use war_core::{LocalVerifier, Verifier};
use war_protocol::*;

fn snapshot(value: &str, focused: bool) -> SemanticSnapshot {
    SemanticSnapshot {
        session_id: String::new(),
        epoch: 1,
        window: WindowInfo::default(),
        focused: focused.then_some(1),
        total_nodes: 1,
        truncated: false,
        nodes: vec![SemanticNode {
            id: 1,
            role: Role::TextInput,
            name: Some("Name".into()),
            automation_id: None,
            value: Some(value.into()),
            states: NodeStates {
                enabled: true,
                focused,
                ..Default::default()
            },
            capabilities: Capabilities::SET_VALUE,
            depth: 0,
        }],
    }
}

#[test]
fn evaluates_nested_conditions_locally() {
    let before = snapshot("old", false);
    let after = snapshot("new", true);
    let condition = Condition::All {
        conditions: vec![
            Condition::ValueEquals {
                target: Target::Ref(1),
                value: "new".into(),
            },
            Condition::StateEquals {
                target: Target::Ref(1),
                state: StatePredicate::Focused(true),
            },
        ],
    };
    assert!(LocalVerifier.verify(&before, &after, &condition).success);
}

#[test]
fn window_conditions_use_dialog_nodes_not_only_primary_title() {
    let before = snapshot("old", false);
    let mut after = before.clone();
    after.epoch = 2;
    after.total_nodes += 1;
    after.nodes.push(SemanticNode {
        id: 2,
        role: Role::Dialog,
        name: Some("Save As".into()),
        automation_id: None,
        value: None,
        states: NodeStates {
            enabled: true,
            ..Default::default()
        },
        capabilities: Capabilities::empty(),
        depth: 0,
    });
    assert!(
        LocalVerifier
            .verify(
                &before,
                &after,
                &Condition::WindowOpened {
                    name: "Save As".into()
                }
            )
            .success
    );
    assert!(
        LocalVerifier
            .verify(
                &after,
                &before,
                &Condition::WindowClosed {
                    name: "Save As".into()
                }
            )
            .success
    );
}
