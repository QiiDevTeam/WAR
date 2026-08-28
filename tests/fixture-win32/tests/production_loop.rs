use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use war_core::{DesktopProvider, ProviderNodeRef};
use war_protocol::{
    Action, ActionBatch, Capabilities, Condition, MouseButton, NormalizedPoint, Role,
    SemanticTarget, SnapshotScope, StatePredicate, Target,
};
use war_runtime::{JsonlRequest, WarRuntime};
use war_uia::UiaProvider;

fn run_one(
    runtime: &WarRuntime,
    snapshot: &war_protocol::SemanticSnapshot,
    action: Action,
    postcondition: Option<Condition>,
) -> war_runtime::ExecutionReport {
    runtime
        .act(&ActionBatch {
            expected_session_id: Some(snapshot.session_id.clone()),
            expected_epoch: Some(snapshot.epoch),
            timeout_ms: Some(2_000),
            actions: vec![action],
            precondition: None,
            postcondition,
            stop_on_error: true,
        })
        .unwrap()
}

struct TestApp(u32);
impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/PID", &self.0.to_string(), "/F"])
            .output();
    }
}

struct NativeFixture(Child);
impl Drop for NativeFixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch_native_fixture_with_overlay(overlay: bool) -> (NativeFixture, u64) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_war-fixture-win32"));
    command.stdout(Stdio::piped());
    if overlay {
        command.env("WAR_FIXTURE_OVERLAY", "1");
    }
    let mut child = command.spawn().expect("launch native fixture");
    let stdout = child.stdout.take().expect("fixture stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("fixture HWND line");
    let hwnd = line
        .trim()
        .strip_prefix("HWND=")
        .expect("fixture HWND prefix")
        .parse()
        .expect("fixture HWND value");
    (NativeFixture(child), hwnd)
}

fn launch_native_fixture() -> (NativeFixture, u64) {
    launch_native_fixture_with_overlay(false)
}

fn launch_paint() -> (TestApp, u64) {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "$p=Start-Process -FilePath 'mspaint.exe' -PassThru; Write-Output $p.Id",
        ])
        .output()
        .expect("launch Paint through Windows app activation");
    assert!(output.status.success(), "Paint activation failed");
    let process_id: u32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("Paint process id");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(window) = war_win32::process_window(process_id) {
            return (TestApp(process_id), war_win32::hwnd_to_id(window));
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Paint did not create a visible window");
}

#[test]
#[ignore = "requires an interactive Windows desktop; run the production gate explicitly"]
fn explicit_window_scope_is_fast_private_and_survives_actions() {
    let (_paint, hwnd) = launch_paint();
    let runtime = WarRuntime::new(Arc::new(UiaProvider::new().unwrap()));

    let started = Instant::now();
    let first = runtime.observe(SnapshotScope::Window(hwnd)).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "snapshot took {:?}",
        started.elapsed()
    );
    assert_eq!(first.window.process_id, _paint.0);
    assert_eq!(first.window.id, hwnd);
    assert!(
        first.nodes.len() <= 256,
        "unexpected nodes: {}",
        first.nodes.len()
    );
    assert!(
        !first
            .nodes
            .iter()
            .any(|node| node.name.as_deref() == Some("桌面")),
        "window scope escaped into the desktop"
    );

    let zoom = first
        .nodes
        .iter()
        .find(|node| {
            node.automation_id.as_deref() == Some("EditableText")
                && node.role == Role::TextInput
                && node.capabilities.contains(Capabilities::SET_VALUE)
        })
        .expect("Paint zoom input")
        .id;
    let mut samples = Vec::with_capacity(20);
    let mut latest_epoch = first.epoch;
    for _ in 0..20 {
        let sample_started = Instant::now();
        let sample = runtime.observe(SnapshotScope::Window(hwnd)).unwrap();
        latest_epoch = sample.epoch;
        samples.push(sample_started.elapsed());
        assert_eq!(sample.window.id, hwnd);
        assert_eq!(
            sample
                .nodes
                .iter()
                .find(|node| node.automation_id.as_deref() == Some("EditableText"))
                .map(|node| node.id),
            Some(zoom),
            "stable ref changed during repeated snapshots"
        );
    }
    samples.sort_unstable();
    let p95 = samples[18];
    eprintln!("Paint warm snapshot p95={p95:?}");
    assert!(
        p95 < Duration::from_millis(250),
        "Paint warm snapshot p95 exceeded 250ms: {p95:?}"
    );
    let action_started = Instant::now();
    let report = runtime
        .act(&ActionBatch {
            expected_session_id: Some(first.session_id.clone()),
            expected_epoch: Some(latest_epoch),
            timeout_ms: None,
            actions: vec![Action::SetValue {
                target: Target::Ref(zoom),
                value: "125%".into(),
            }],
            precondition: None,
            postcondition: Some(Condition::ValueEquals {
                target: Target::Ref(zoom),
                value: "125%".into(),
            }),
            stop_on_error: true,
        })
        .unwrap();
    assert!(
        action_started.elapsed() < Duration::from_secs(2),
        "action took {:?}",
        action_started.elapsed()
    );
    assert_eq!(report.snapshot.window.process_id, _paint.0);
    assert_eq!(
        report
            .snapshot
            .nodes
            .iter()
            .find(|node| node.id == zoom)
            .and_then(|node| node.value.as_deref()),
        Some("125%")
    );

    let canvas = report
        .snapshot
        .nodes
        .iter()
        .find(|node| {
            node.automation_id.as_deref() == Some("image")
                && node.capabilities.contains(Capabilities::POINTER_GESTURE)
        })
        .expect("Paint canvas supports pointer gestures")
        .id;
    war_win32::set_foreground(war_win32::id_to_hwnd(hwnd)).unwrap();
    let gesture = runtime
        .act(&ActionBatch {
            expected_session_id: Some(report.snapshot.session_id.clone()),
            expected_epoch: Some(report.snapshot.epoch),
            timeout_ms: Some(2_000),
            actions: vec![Action::PointerGesture {
                target: Target::Ref(canvas),
                button: MouseButton::Left,
                points: vec![
                    NormalizedPoint { x: 0.40, y: 0.45 },
                    NormalizedPoint { x: 0.45, y: 0.50 },
                    NormalizedPoint { x: 0.50, y: 0.45 },
                ],
                duration_ms: 120,
            }],
            precondition: None,
            postcondition: Some(Condition::StateEquals {
                target: Target::Semantic(SemanticTarget {
                    role: Some(Role::Button),
                    name: Some("撤消".into()),
                    automation_id: None,
                    required_capabilities: Capabilities::empty(),
                    ancestor: None,
                }),
                state: StatePredicate::Enabled(true),
            }),
            stop_on_error: true,
        })
        .expect("verified Paint pointer gesture");
    assert_eq!(gesture.outcome.verified, Some(true));
    assert_eq!(
        gesture.outcome.actions[0].method.as_deref(),
        Some("send_input.pointer_gesture")
    );
}

#[test]
#[ignore = "requires an interactive Windows desktop; run the production gate explicitly"]
fn native_win32_provider_does_not_deadlock_or_expose_static_text_as_clickable() {
    let (fixture, hwnd) = launch_native_fixture();
    let runtime = WarRuntime::new(Arc::new(
        UiaProvider::with_timeout(Duration::from_millis(750)).unwrap(),
    ));

    let started = Instant::now();
    let snapshot = runtime
        .observe(SnapshotScope::Window(hwnd))
        .expect("native controls should not deadlock UIA");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "native snapshot returned too slowly: {:?}",
        started.elapsed()
    );
    assert!(snapshot.nodes.iter().any(|node| {
        node.name.as_deref() == Some("Apply")
            && node
                .capabilities
                .contains(Capabilities::INVOKE | Capabilities::CLICK)
    }));
    assert!(snapshot
        .nodes
        .iter()
        .filter(|node| node.role == Role::Text)
        .all(|node| !node
            .capabilities
            .intersects(Capabilities::CLICK | Capabilities::FOCUS)));
    let password = snapshot
        .nodes
        .iter()
        .find(|node| node.automation_id.as_deref() == Some("105"))
        .expect("fixture password input");
    assert_eq!(password.value, None);
    assert!(!password.capabilities.contains(Capabilities::GET_VALUE));
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("super-secret"));
    assert!(!snapshot
        .nodes
        .iter()
        .any(|node| node.name.as_deref() == Some("WAR Native Auxiliary")));

    let process_snapshot = runtime
        .observe(SnapshotScope::Process(fixture.0.id()))
        .expect("process scope should aggregate visible top-level windows");
    assert_eq!(process_snapshot.window.process_id, fixture.0.id());
    for title in ["WAR Native Fixture", "WAR Native Auxiliary"] {
        assert!(
            process_snapshot
                .nodes
                .iter()
                .any(|node| node.name.as_deref() == Some(title)),
            "process scope omitted {title}"
        );
    }
    let input = process_snapshot
        .nodes
        .iter()
        .find(|node| node.automation_id.as_deref() == Some("101"))
        .expect("process-scoped input")
        .id;
    let checkbox = process_snapshot
        .nodes
        .iter()
        .find(|node| node.automation_id.as_deref() == Some("104"))
        .expect("process-scoped checkbox")
        .id;
    let report = runtime
        .act(&ActionBatch {
            expected_session_id: Some(process_snapshot.session_id.clone()),
            expected_epoch: Some(process_snapshot.epoch),
            timeout_ms: None,
            actions: vec![
                Action::SetValue {
                    target: Target::Ref(input),
                    value: "agent-ready".into(),
                },
                Action::Toggle {
                    target: Target::Ref(checkbox),
                    value: Some(true),
                },
            ],
            precondition: None,
            postcondition: Some(Condition::All {
                conditions: vec![
                    Condition::ValueEquals {
                        target: Target::Ref(input),
                        value: "agent-ready".into(),
                    },
                    Condition::StateEquals {
                        target: Target::Ref(checkbox),
                        state: war_protocol::StatePredicate::Checked(true),
                    },
                ],
            }),
            stop_on_error: true,
        })
        .expect("process-scoped semantic batch");
    assert_eq!(report.outcome.verified, Some(true));
    assert_eq!(
        report
            .outcome
            .actions
            .iter()
            .map(|outcome| outcome.method.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("uia.value"), Some("uia.toggle")]
    );

    let mut current = report.snapshot;
    let item = current
        .nodes
        .iter()
        .find(|node| node.role == Role::ListItem && node.name.as_deref() == Some("Item 20"))
        .expect("scrollable fixture list item")
        .id;
    let selected = run_one(
        &runtime,
        &current,
        Action::Select {
            target: Target::Ref(item),
        },
        Some(Condition::StateEquals {
            target: Target::Ref(item),
            state: war_protocol::StatePredicate::Selected(true),
        }),
    );
    assert_eq!(
        selected.outcome.actions[0].method.as_deref(),
        Some("uia.select")
    );
    current = selected.snapshot;

    let list = current
        .nodes
        .iter()
        .find(|node| node.role == Role::List && node.capabilities.contains(Capabilities::SCROLL))
        .expect("scrollable fixture list")
        .id;
    let scrolled = run_one(
        &runtime,
        &current,
        Action::Scroll {
            target: Target::Ref(list),
            amount: war_protocol::ScrollAmount::SmallIncrement,
        },
        None,
    );
    assert_eq!(
        scrolled.outcome.actions[0].method.as_deref(),
        Some("uia.scroll")
    );
    current = scrolled.snapshot;

    let apply = current
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Apply"))
        .unwrap()
        .id;
    let invoked = run_one(
        &runtime,
        &current,
        Action::Invoke {
            target: Target::Ref(apply),
        },
        None,
    );
    assert_eq!(
        invoked.outcome.actions[0].method.as_deref(),
        Some("uia.invoke")
    );
    current = invoked.snapshot;

    war_win32::set_foreground(war_win32::id_to_hwnd(hwnd)).expect("foreground fixture window");
    thread::sleep(Duration::from_millis(50));
    let clicked = run_one(
        &runtime,
        &current,
        Action::Click {
            target: Target::Ref(apply),
            button: war_protocol::MouseButton::Left,
        },
        None,
    );
    assert_eq!(
        clicked.outcome.actions[0].method.as_deref(),
        Some("send_input.click")
    );
    assert_eq!(clicked.outcome.actions[0].fallback_used, Some(true));
    current = clicked.snapshot;

    let cleared = run_one(
        &runtime,
        &current,
        Action::SetValue {
            target: Target::Ref(input),
            value: String::new(),
        },
        Some(Condition::ValueEquals {
            target: Target::Ref(input),
            value: String::new(),
        }),
    );
    let focused = run_one(
        &runtime,
        &cleared.snapshot,
        Action::Focus {
            target: Target::Ref(input),
        },
        Some(Condition::StateEquals {
            target: Target::Ref(input),
            state: war_protocol::StatePredicate::Focused(true),
        }),
    );
    war_win32::set_foreground(war_win32::id_to_hwnd(hwnd)).expect("foreground focused input");
    thread::sleep(Duration::from_millis(50));
    let typed = run_one(
        &runtime,
        &focused.snapshot,
        Action::TypeText {
            text: "typed".into(),
        },
        Some(Condition::ValueEquals {
            target: Target::Ref(input),
            value: "typed".into(),
        }),
    );
    assert_eq!(
        typed.outcome.actions[0].method.as_deref(),
        Some("send_input.text")
    );
    let keyed = run_one(
        &runtime,
        &typed.snapshot,
        Action::Key {
            key: war_protocol::Key::Backspace,
            modifiers: war_protocol::Modifiers::default(),
        },
        Some(Condition::ValueEquals {
            target: Target::Ref(input),
            value: "type".into(),
        }),
    );
    assert_eq!(
        keyed.outcome.actions[0].method.as_deref(),
        Some("send_input.key")
    );
}

#[test]
#[ignore = "requires an interactive Windows desktop; run the production gate explicitly"]
fn external_property_change_emits_event_within_half_a_second() {
    let (_fixture, hwnd) = launch_native_fixture();
    let provider = UiaProvider::new().unwrap();
    let subscription = provider.subscribe().unwrap();
    provider.snapshot(SnapshotScope::Window(hwnd)).unwrap();
    while subscription.receiver().try_recv().is_ok() {}

    let actor = UiaProvider::new().unwrap();
    let actor_snapshot = actor.snapshot(SnapshotScope::Window(hwnd)).unwrap();
    let input = actor_snapshot
        .nodes
        .values()
        .find(|node| node.automation_id.as_deref() == Some("101"))
        .expect("fixture input");
    actor
        .execute(
            Some(ProviderNodeRef { id: input.id }),
            &Action::SetValue {
                target: Target::Ref(input.id),
                value: "external change".into(),
            },
        )
        .expect("change edit through an independent UIA client");

    subscription
        .receiver()
        .recv_timeout(Duration::from_millis(500))
        .expect("scoped UIA property event within 500ms");
    let updated = provider.snapshot(SnapshotScope::Window(hwnd)).unwrap();
    assert!(updated.nodes.values().any(|node| {
        node.automation_id.as_deref() == Some("101")
            && node.value.as_deref() == Some("external change")
    }));
}

#[test]
#[ignore = "requires an interactive Windows desktop; run the production gate explicitly"]
fn real_jsonl_agent_session_observes_acts_and_verifies() {
    let (_fixture, hwnd) = launch_native_fixture();
    let runtime = WarRuntime::new(Arc::new(UiaProvider::new().unwrap()));
    let snapshot_response = runtime.handle_jsonl(
        serde_json::from_value::<JsonlRequest>(serde_json::json!({
            "id": "observe-1",
            "method": "snapshot",
            "scope": {"kind":"window", "value": hwnd}
        }))
        .unwrap(),
    );
    assert!(snapshot_response.error.is_none());
    let snapshot: war_protocol::SemanticSnapshot =
        serde_json::from_value(snapshot_response.result.unwrap()["snapshot"].clone()).unwrap();
    let input = snapshot
        .nodes
        .iter()
        .find(|node| node.automation_id.as_deref() == Some("101"))
        .unwrap()
        .id;
    let act_response = runtime.handle_jsonl(
        serde_json::from_value::<JsonlRequest>(serde_json::json!({
            "id": "act-1",
            "method": "act",
            "expected_session_id": snapshot.session_id,
            "expected_epoch": snapshot.epoch,
            "timeout_ms": 2_000,
            "actions": [{
                "set_value": {"target": format!("@{input}"), "value": "jsonl-ready"}
            }],
            "postcondition": {
                "type": "value_equals",
                "target": format!("@{input}"),
                "value": "jsonl-ready"
            },
            "stop_on_error": true
        }))
        .unwrap(),
    );
    assert!(act_response.error.is_none(), "{:?}", act_response.error);
    let result = act_response.result.unwrap();
    assert_eq!(result["verified"], true);
    assert_eq!(result["actions"][0]["method"], "uia.value");
    assert_eq!(result["actions"][0]["fallback_used"], false);
}

#[test]
#[ignore = "requires an interactive Windows desktop; run the production gate explicitly"]
fn obscured_native_target_is_refused_even_within_the_same_process() {
    let (_fixture, hwnd) = launch_native_fixture_with_overlay(true);
    let runtime = WarRuntime::new(Arc::new(UiaProvider::new().unwrap()));
    let snapshot = runtime.observe(SnapshotScope::Window(hwnd)).unwrap();
    let apply = snapshot
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Apply"))
        .expect("covered Apply button")
        .id;

    war_win32::set_foreground(war_win32::id_to_hwnd(hwnd)).expect("foreground main fixture");
    thread::sleep(Duration::from_millis(50));
    let report = run_one(
        &runtime,
        &snapshot,
        Action::Click {
            target: Target::Ref(apply),
            button: war_protocol::MouseButton::Left,
        },
        None,
    );
    assert!(!report.outcome.actions[0].dispatched);
    assert!(report.outcome.actions[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("obscured")));
}
