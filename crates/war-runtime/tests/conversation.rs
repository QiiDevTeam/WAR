use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use war_core::{DesktopProvider, ProviderActionResult, ProviderNodeRef, Subscription};
use war_protocol::{
    Action, Capabilities, DesktopNode, NodeFingerprint, NodeSource, NodeStates, RawSnapshot, Rect,
    Role, SnapshotScope, WarError, WarResult,
};
use war_runtime::{JsonlRequest, WarRuntime};

#[derive(Default)]
struct ChatState {
    epoch: u64,
    selected: bool,
    pending_selection_snapshots: u8,
    draft: String,
    outgoing: Vec<String>,
}

struct MockConversationProvider {
    state: Mutex<ChatState>,
}

impl MockConversationProvider {
    fn new() -> Self {
        Self {
            state: Mutex::new(ChatState::default()),
        }
    }
}

impl DesktopProvider for MockConversationProvider {
    fn kind(&self) -> NodeSource {
        NodeSource::Uia
    }

    fn snapshot(&self, _scope: SnapshotScope) -> WarResult<RawSnapshot> {
        let mut state = self.state.lock().unwrap();
        state.epoch += 1;
        if state.pending_selection_snapshots > 0 {
            state.pending_selection_snapshots -= 1;
            if state.pending_selection_snapshots == 0 {
                state.selected = true;
            }
        }

        let mut nodes = HashMap::new();
        nodes.insert(
            1,
            node(
                1,
                Role::Window,
                "QQ",
                None,
                vec![10, 20],
                Capabilities::empty(),
                true,
            ),
        );
        nodes.insert(
            10,
            node(
                10,
                Role::List,
                "会话列表",
                Some(1),
                vec![11],
                Capabilities::empty(),
                true,
            ),
        );
        nodes.insert(
            11,
            node(
                11,
                Role::Group,
                "Amiracle",
                Some(10),
                vec![12],
                Capabilities::INVOKE | Capabilities::CLICK,
                true,
            ),
        );
        nodes.insert(
            12,
            node(
                12,
                Role::Text,
                "Amiracle",
                Some(11),
                vec![],
                Capabilities::empty(),
                true,
            ),
        );
        let mut main_children = Vec::new();
        if state.selected {
            main_children.extend([21, 22]);
            for (index, message) in state.outgoing.iter().enumerate() {
                let id = 100 + index as u64;
                main_children.push(id);
                nodes.insert(
                    id,
                    node(
                        id,
                        Role::Text,
                        message,
                        Some(20),
                        vec![],
                        Capabilities::empty(),
                        true,
                    ),
                );
            }
            let mut editor = node(
                21,
                Role::Group,
                "Amiracle",
                Some(20),
                vec![],
                Capabilities::SET_VALUE
                    | Capabilities::GET_VALUE
                    | Capabilities::FOCUS
                    | Capabilities::TYPE_TEXT,
                true,
            );
            editor.value = Some(state.draft.clone());
            editor.states.focused = true;
            editor.states.focusable = true;
            nodes.insert(21, editor);
            nodes.insert(
                22,
                node(
                    22,
                    Role::Button,
                    "发送",
                    Some(20),
                    vec![],
                    if state.draft.is_empty() {
                        Capabilities::empty()
                    } else {
                        Capabilities::INVOKE | Capabilities::CLICK
                    },
                    !state.draft.is_empty(),
                ),
            );
        }
        nodes.insert(
            20,
            node(
                20,
                Role::Pane,
                "主面板",
                Some(1),
                main_children,
                Capabilities::empty(),
                true,
            ),
        );
        Ok(RawSnapshot {
            epoch: state.epoch,
            root: 1,
            nodes,
        })
    }

    fn execute(
        &self,
        node: Option<ProviderNodeRef>,
        action: &Action,
    ) -> WarResult<ProviderActionResult> {
        let mut state = self.state.lock().unwrap();
        let id = node.map(|node| node.id);
        match (id, action) {
            // Electron exposes Invoke but QQ performs no selection for this row.
            (Some(11), Action::Invoke { .. }) => {}
            (Some(11), Action::Click { .. }) => state.pending_selection_snapshots = 2,
            (Some(21), Action::SetValue { value, .. }) if state.selected => {
                state.draft = value.clone()
            }
            (Some(22), Action::Invoke { .. }) if !state.draft.is_empty() => {
                let message = std::mem::take(&mut state.draft);
                state.outgoing.push(message);
            }
            _ => {
                return Err(WarError::CapabilityUnavailable(
                    "mock conversation action".into(),
                ))
            }
        }
        Ok(ProviderActionResult {
            method: "mock.uia".into(),
            fallback_used: matches!(action, Action::Click { .. }),
        })
    }

    fn subscribe(&self) -> WarResult<Subscription> {
        Ok(Subscription::new(crossbeam_channel::never()))
    }
}

fn node(
    id: u64,
    role: Role,
    name: &str,
    parent: Option<u64>,
    children: Vec<u64>,
    capabilities: Capabilities,
    enabled: bool,
) -> DesktopNode {
    DesktopNode {
        id,
        source: NodeSource::Uia,
        role,
        name: Some(name.into()),
        automation_id: None,
        value: None,
        description: None,
        bounds: Some(Rect {
            left: 10.0,
            top: id as f64,
            width: 100.0,
            height: 20.0,
        }),
        states: NodeStates {
            enabled,
            ..Default::default()
        },
        capabilities,
        parent,
        children,
        fingerprint: NodeFingerprint {
            process_id: 77,
            role,
            sibling_hint: id as u16,
            ..Default::default()
        },
    }
}

#[test]
fn one_compact_request_handles_noop_invoke_async_selection_and_verified_send() {
    let baseline_runtime = WarRuntime::new(Arc::new(MockConversationProvider::new()));
    let baseline = baseline_runtime.handle_jsonl(
        serde_json::from_value(serde_json::json!({
            "id": 0,
            "method": "snapshot",
            "params": {}
        }))
        .unwrap(),
    );
    let baseline_bytes = serde_json::to_vec(&baseline).unwrap().len();
    let provider = Arc::new(MockConversationProvider::new());
    let runtime = WarRuntime::new(provider.clone());
    let request: JsonlRequest = serde_json::from_value(serde_json::json!({
        "id": 1,
        "method": "send_message",
        "params": {
            "scope": "focused_window",
            "recipient": "Amiracle",
            "text": "test",
            "timeout_ms": 2_000
        }
    }))
    .unwrap();

    let response = runtime.handle_jsonl(request);
    assert!(response.error.is_none(), "{:?}", response.error);
    let encoded = serde_json::to_vec(&response).unwrap();
    assert!(encoded.len() <= 512, "response was {} bytes", encoded.len());
    assert!(
        encoded.len() * 4 < baseline_bytes,
        "compact workflow response {} bytes was not 4x smaller than one snapshot {} bytes",
        encoded.len(),
        baseline_bytes
    );
    let result = response.result.unwrap();
    assert_eq!(result["status"], "verified");
    assert_eq!(result["activation"], "click_fallback");
    assert_eq!(result["actions"], 4);
    assert_eq!(provider.state.lock().unwrap().outgoing, ["test"]);
}
