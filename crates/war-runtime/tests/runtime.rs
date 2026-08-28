use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use war_core::*;
use war_protocol::*;
use war_runtime::JsonlRequest;
use war_runtime::WarRuntime;

struct MockProvider {
    value: Mutex<String>,
    pending_value: Mutex<Option<(String, u8)>>,
    delay_snapshots: u8,
    epoch: Mutex<u64>,
    scopes: Mutex<Vec<SnapshotScope>>,
}
impl MockProvider {
    fn new() -> Self {
        Self {
            value: Mutex::new("old".into()),
            pending_value: Mutex::new(None),
            delay_snapshots: 0,
            epoch: Mutex::new(0),
            scopes: Mutex::new(Vec::new()),
        }
    }
    fn delayed(delay_snapshots: u8) -> Self {
        Self {
            delay_snapshots,
            ..Self::new()
        }
    }
}
impl DesktopProvider for MockProvider {
    fn kind(&self) -> NodeSource {
        NodeSource::Uia
    }
    fn snapshot(&self, scope: SnapshotScope) -> WarResult<RawSnapshot> {
        self.scopes.lock().unwrap().push(scope);
        let ready = {
            let mut pending = self.pending_value.lock().unwrap();
            match pending.as_mut() {
                Some((_, remaining)) if *remaining > 0 => {
                    *remaining -= 1;
                    None
                }
                Some(_) => pending.take().map(|(value, _)| value),
                None => None,
            }
        };
        if let Some(value) = ready {
            *self.value.lock().unwrap() = value;
        }
        let mut epoch = self.epoch.lock().unwrap();
        *epoch += 1;
        let root = DesktopNode {
            id: 1,
            source: NodeSource::Uia,
            role: Role::Window,
            name: Some("Fixture".into()),
            automation_id: Some("window".into()),
            value: None,
            description: None,
            bounds: Some(Rect {
                left: 10.0,
                top: 20.0,
                width: 640.0,
                height: 480.0,
            }),
            states: NodeStates {
                enabled: true,
                ..Default::default()
            },
            capabilities: Capabilities::empty(),
            parent: None,
            children: vec![200, 300],
            fingerprint: NodeFingerprint {
                process_id: 7,
                ..Default::default()
            },
        };
        let input = DesktopNode {
            id: 200,
            source: NodeSource::Uia,
            role: Role::TextInput,
            name: Some("Name".into()),
            automation_id: Some("name".into()),
            value: Some(self.value.lock().unwrap().clone()),
            description: None,
            bounds: Some(Rect {
                left: 30.0,
                top: 40.0,
                width: 120.0,
                height: 24.0,
            }),
            states: NodeStates {
                enabled: true,
                focused: true,
                ..Default::default()
            },
            capabilities: Capabilities::SET_VALUE,
            parent: Some(1),
            children: vec![],
            fingerprint: NodeFingerprint {
                process_id: 7,
                ..Default::default()
            },
        };
        let link = DesktopNode {
            id: 300,
            source: NodeSource::Uia,
            role: Role::Link,
            name: Some("A useful video".into()),
            automation_id: None,
            value: Some("https://www.example.test/video/42".into()),
            description: None,
            bounds: Some(Rect {
                left: 30.0,
                top: 80.0,
                width: 240.0,
                height: 30.0,
            }),
            states: NodeStates {
                enabled: true,
                ..Default::default()
            },
            capabilities: Capabilities::INVOKE | Capabilities::GET_VALUE,
            parent: Some(1),
            children: vec![],
            fingerprint: NodeFingerprint {
                process_id: 7,
                ..Default::default()
            },
        };
        Ok(RawSnapshot {
            epoch: *epoch,
            root: 1,
            nodes: HashMap::from([(1, root), (200, input), (300, link)]),
        })
    }
    fn execute(
        &self,
        _node: Option<ProviderNodeRef>,
        action: &Action,
    ) -> WarResult<ProviderActionResult> {
        if let Action::SetValue { value, .. } = action {
            if self.delay_snapshots == 0 {
                *self.value.lock().unwrap() = value.clone();
            } else {
                *self.pending_value.lock().unwrap() = Some((value.clone(), self.delay_snapshots));
            }
            Ok(ProviderActionResult {
                method: "mock.value".into(),
                fallback_used: false,
            })
        } else {
            Err(WarError::CapabilityUnavailable("mock".into()))
        }
    }
    fn subscribe(&self) -> WarResult<Subscription> {
        Ok(Subscription::new(crossbeam_channel::never()))
    }
}

#[test]
fn batch_executes_refreshes_diffs_and_verifies() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = first
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("Name"))
        .unwrap()
        .id;
    let report = runtime
        .act(&ActionBatch {
            expected_session_id: Some(first.session_id.clone()),
            expected_epoch: Some(first.epoch),
            timeout_ms: None,
            actions: vec![Action::SetValue {
                target: Target::Ref(input),
                value: "new".into(),
            }],
            precondition: None,
            postcondition: Some(Condition::ValueEquals {
                target: Target::Ref(input),
                value: "new".into(),
            }),
            stop_on_error: true,
        })
        .unwrap();
    assert_eq!(report.outcome.verified, Some(true));
    assert_eq!(report.outcome.status, ExecutionStatus::Verified);
    assert_eq!(report.delta.changed.len(), 1);
}

#[test]
fn waits_for_async_postcondition_instead_of_sampling_once() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::delayed(2)));
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Name"))
        .unwrap()
        .id;
    let report = runtime
        .act(&ActionBatch {
            expected_session_id: Some(first.session_id.clone()),
            expected_epoch: Some(first.epoch),
            timeout_ms: Some(500),
            actions: vec![Action::SetValue {
                target: Target::Ref(input),
                value: "eventual".into(),
            }],
            precondition: None,
            postcondition: Some(Condition::ValueEquals {
                target: Target::Ref(input),
                value: "eventual".into(),
            }),
            stop_on_error: true,
        })
        .unwrap();
    assert_eq!(report.outcome.status, ExecutionStatus::Verified);
    assert!(report.observations >= 3);
}

#[test]
fn reports_dispatch_without_claiming_unverified_success() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Name"))
        .unwrap()
        .id;
    let report = runtime
        .act(&ActionBatch {
            expected_session_id: Some(first.session_id.clone()),
            expected_epoch: Some(first.epoch),
            timeout_ms: Some(100),
            actions: vec![Action::SetValue {
                target: Target::Ref(input),
                value: "old".into(),
            }],
            precondition: None,
            postcondition: None,
            stop_on_error: true,
        })
        .unwrap();
    assert_eq!(report.outcome.status, ExecutionStatus::DispatchedUnverified);
    assert_eq!(report.outcome.effect, ObservedEffect::NoChange);
    assert_eq!(report.outcome.verified, None);
}

#[test]
fn rejects_refs_from_another_runtime_even_when_epoch_and_ref_match() {
    let runtime_a = WarRuntime::new(Arc::new(MockProvider::new()));
    let runtime_b = WarRuntime::new(Arc::new(MockProvider::new()));
    let from_a = runtime_a.observe(SnapshotScope::FocusedWindow).unwrap();
    let from_b = runtime_b.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = from_a
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Name"))
        .unwrap()
        .id;
    assert_eq!(from_a.epoch, from_b.epoch);
    assert!(from_b.nodes.iter().any(|node| node.id == input));
    let error = runtime_b
        .act(&ActionBatch {
            expected_session_id: Some(from_a.session_id),
            expected_epoch: Some(from_b.epoch),
            timeout_ms: Some(100),
            actions: vec![Action::SetValue {
                target: Target::Ref(input),
                value: "unsafe".into(),
            }],
            precondition: None,
            postcondition: None,
            stop_on_error: true,
        })
        .unwrap_err();
    assert!(matches!(error, WarError::StaleSession { .. }));
}

#[test]
fn rejects_unsupported_action_before_provider_execution() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Name"))
        .unwrap();
    let error = runtime
        .act(&ActionBatch {
            expected_session_id: Some(first.session_id.clone()),
            expected_epoch: Some(first.epoch),
            timeout_ms: None,
            actions: vec![Action::Invoke {
                target: Target::Ref(input.id),
            }],
            precondition: None,
            postcondition: None,
            stop_on_error: true,
        })
        .unwrap_err();
    assert!(matches!(error, WarError::CapabilityUnavailable(_)));
}

#[test]
fn rejects_stale_epoch_and_jsonl_refs_without_epoch() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Name"))
        .unwrap();
    runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let stale = runtime
        .act(&ActionBatch {
            expected_session_id: Some(first.session_id.clone()),
            expected_epoch: Some(first.epoch),
            timeout_ms: None,
            actions: vec![Action::SetValue {
                target: Target::Ref(input.id),
                value: "unsafe".into(),
            }],
            precondition: None,
            postcondition: None,
            stop_on_error: true,
        })
        .unwrap_err();
    assert!(matches!(stale, WarError::StaleSnapshot { .. }));

    let response = runtime.handle_jsonl(JsonlRequest {
        id: serde_json::json!(9),
        method: "act".into(),
        params: serde_json::json!({
            "actions": [{"set_value":{"target":format!("@{}", input.id),"value":"unsafe"}}],
            "stop_on_error": true
        }),
        extra: serde_json::Map::new(),
    });
    assert!(response.result.is_none());
    assert_eq!(
        response.error.unwrap().get("kind").and_then(|v| v.as_str()),
        Some("invalid_request")
    );
}

#[test]
fn accepts_flat_jsonl_shape_from_protocol_examples() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let request: JsonlRequest = serde_json::from_value(serde_json::json!({
        "id": 1,
        "method": "snapshot",
        "scope": "focused_window"
    }))
    .unwrap();
    let response = runtime.handle_jsonl(request);
    assert!(response.error.is_none());
    let result = response.result.unwrap();
    assert_eq!(result["snapshot"]["epoch"], 1);
    assert!(result.get("text").is_none());
}

#[test]
fn inspect_observes_resolves_and_projects_bounds_in_one_compact_response() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let response = runtime.handle_jsonl(
        serde_json::from_value(serde_json::json!({
            "id": 3,
            "method": "inspect",
            "params": {
                "automation_id": "name",
                "fields": ["bounds", "capabilities", "lineage"]
            }
        }))
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let encoded = serde_json::to_vec(&response).unwrap();
    assert!(
        encoded.len() <= 512,
        "inspect response was {} bytes",
        encoded.len()
    );
    let result = response.result.unwrap();
    assert_eq!(result["node"]["name"], "Name");
    assert_eq!(result["node"]["bounds"]["left"], 30.0);
    assert_eq!(result["lineage"].as_array().unwrap().len(), 2);
    assert!(result["session_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
}

#[test]
fn query_filters_server_side_and_returns_only_projected_matches() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let response = runtime.handle_jsonl(
        serde_json::from_value(serde_json::json!({
            "id": 4,
            "method": "query",
            "params": {
                "role": "link",
                "value_contains": "/video/",
                "required_capabilities": "INVOKE",
                "limit": 5,
                "fields": ["value", "capabilities"]
            }
        }))
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let encoded = serde_json::to_vec(&response).unwrap();
    assert!(
        encoded.len() <= 768,
        "query response was {} bytes",
        encoded.len()
    );
    let result = response.result.unwrap();
    assert_eq!(result["returned"], 1);
    assert_eq!(result["total_matches"], 1);
    assert_eq!(result["matches"][0]["name"], "A useful video");
    assert_eq!(
        result["matches"][0]["value"],
        "https://www.example.test/video/42"
    );
}

#[test]
fn wait_polls_inside_runtime_until_a_query_matches() {
    let provider = Arc::new(MockProvider::delayed(2));
    *provider.pending_value.lock().unwrap() = Some(("eventual value".into(), 2));
    let runtime = WarRuntime::new(provider);
    let response = runtime.handle_jsonl(
        serde_json::from_value(serde_json::json!({
            "id": 5,
            "method": "wait",
            "params": {
                "role": "text_input",
                "value_contains": "eventual",
                "timeout_ms": 1000,
                "poll_interval_ms": 50,
                "fields": ["value"]
            }
        }))
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    assert_eq!(result["returned"], 1);
    assert_eq!(result["matches"][0]["value"], "eventual value");
    assert!(result["observations"].as_u64().unwrap() >= 3);
}

#[test]
fn query_and_wait_enforce_bounded_requests() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    for (method, params) in [
        ("query", serde_json::json!({"limit":51,"role":"link"})),
        (
            "wait",
            serde_json::json!({"timeout_ms":60001,"role":"link"}),
        ),
        ("query", serde_json::json!({})),
        (
            "wait",
            serde_json::json!({"scope":{"kind":"node","value":999},"role":"link"}),
        ),
    ] {
        let response = runtime.handle_jsonl(JsonlRequest {
            id: serde_json::json!(6),
            method: method.into(),
            params,
            extra: serde_json::Map::new(),
        });
        assert!(response.result.is_none());
        assert_eq!(response.error.unwrap()["kind"], "invalid_request");
    }
}

#[test]
fn action_summary_omits_delta_but_keeps_verification() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let snapshot = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = snapshot
        .nodes
        .iter()
        .find(|node| node.automation_id.as_deref() == Some("name"))
        .unwrap();
    let response = runtime.handle_jsonl(
        serde_json::from_value(serde_json::json!({
            "id": 7,
            "method": "act",
            "params": {
                "expected_session_id": snapshot.session_id,
                "expected_epoch": snapshot.epoch,
                "actions": [{"set_value":{"target":format!("@{}", input.id),"value":"compact"}}],
                "postcondition":{"type":"value_equals","target":format!("@{}", input.id),"value":"compact"},
                "format":"summary"
            }
        }))
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    assert_eq!(result["status"], "verified");
    assert!(result.get("delta").is_none());
    assert!(result.get("text").is_none());
}

#[test]
fn find_returns_capability_filtered_lineage_and_ref_guards() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let response = runtime.handle_jsonl(
        serde_json::from_value(serde_json::json!({
            "id": 2,
            "method": "find",
            "params": {
                "name": "Name",
                "required_capabilities": "SET_VALUE"
            }
        }))
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    assert!(result["session_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    assert_eq!(result["epoch"], 1);
    assert_eq!(result["lineage"].as_array().unwrap().len(), 2);
    assert_eq!(result["lineage"][0]["name"], "Fixture");
    assert_eq!(result["lineage"][1]["name"], "Name");
}

#[test]
fn jsonl_session_observes_then_acts_with_correlated_responses() {
    let runtime = WarRuntime::new(Arc::new(MockProvider::new()));
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let input = format!(
        "{{\"id\":1,\"method\":\"snapshot\",\"params\":{{}}}}\n\
         {{\"id\":2,\"method\":\"act\",\"params\":{{\"expected_session_id\":{},\"expected_epoch\":2,\
         \"actions\":[{{\"set_value\":{{\"target\":\"@2\",\"value\":\"jsonl\"}}}}],\
         \"postcondition\":{{\"type\":\"value_equals\",\"target\":\"@2\",\"value\":\"jsonl\"}},\
         \"stop_on_error\":true}}}}\n",
        serde_json::to_string(&first.session_id).unwrap()
    );
    let mut output = Vec::new();
    runtime
        .serve_jsonl(std::io::Cursor::new(input), &mut output)
        .unwrap();
    let responses: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[1]["id"], 2);
    assert_eq!(responses[1]["result"]["verified"], true);
}

#[test]
fn node_scope_maps_stable_ref_to_provider_id() {
    let provider = Arc::new(MockProvider::new());
    let runtime = WarRuntime::new(provider.clone());
    let first = runtime.observe(SnapshotScope::FocusedWindow).unwrap();
    let stable = first
        .nodes
        .iter()
        .find(|node| node.name.as_deref() == Some("Name"))
        .unwrap()
        .id;
    assert_ne!(stable, 200);
    let subtree = runtime.observe(SnapshotScope::Node(stable)).unwrap();
    assert!(subtree.nodes.iter().any(|node| node.id == stable));
    assert_eq!(
        provider.scopes.lock().unwrap().last(),
        Some(&SnapshotScope::Node(200))
    );
    runtime
        .act(&ActionBatch {
            expected_session_id: Some(subtree.session_id.clone()),
            expected_epoch: Some(subtree.epoch),
            timeout_ms: None,
            actions: vec![Action::SetValue {
                target: Target::Ref(stable),
                value: "subtree".into(),
            }],
            precondition: None,
            postcondition: Some(Condition::ValueEquals {
                target: Target::Ref(stable),
                value: "subtree".into(),
            }),
            stop_on_error: true,
        })
        .unwrap();
}
