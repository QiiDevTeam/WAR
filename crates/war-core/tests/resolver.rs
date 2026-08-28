use war_core::resolve_target;
use war_protocol::{
    Capabilities, NodeStates, Role, SemanticNode, SemanticSnapshot, SemanticTarget, Target,
    WindowInfo,
};

fn node(id: u64, depth: u8, role: Role, name: &str) -> SemanticNode {
    SemanticNode {
        id,
        role,
        name: Some(name.into()),
        automation_id: None,
        value: None,
        states: NodeStates::default(),
        capabilities: Capabilities::empty(),
        depth,
    }
}

#[test]
fn ancestor_disambiguates_identical_descendants() {
    let snapshot = SemanticSnapshot {
        session_id: String::new(),
        epoch: 1,
        window: WindowInfo::default(),
        nodes: vec![
            node(1, 0, Role::Window, "App"),
            node(2, 1, Role::Group, "Shipping"),
            node(3, 2, Role::TextInput, "Address"),
            node(4, 1, Role::Group, "Billing"),
            node(5, 2, Role::TextInput, "Address"),
        ],
        total_nodes: 5,
        truncated: false,
        focused: None,
    };
    let resolved = resolve_target(
        &snapshot,
        &Target::Semantic(SemanticTarget {
            role: Some(Role::TextInput),
            name: Some("Address".into()),
            automation_id: None,
            required_capabilities: Capabilities::empty(),
            ancestor: Some(Box::new(SemanticTarget {
                role: Some(Role::Group),
                name: Some("Billing".into()),
                automation_id: None,
                required_capabilities: Capabilities::empty(),
                ancestor: None,
            })),
        }),
    )
    .unwrap();
    assert_eq!(resolved.id, 5);
}
