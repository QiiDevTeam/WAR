use crate::normalized_node;
use std::collections::HashSet;
use war_protocol::{NodeId, RawSnapshot, Role};

pub fn retained_nodes(raw: &RawSnapshot) -> HashSet<NodeId> {
    let mut retained = HashSet::new();
    for node in raw.nodes.values() {
        let structural = matches!(
            node.role,
            Role::Window
                | Role::Dialog
                | Role::Menu
                | Role::List
                | Role::Tree
                | Role::Tab
                | Role::Document
        );
        if node.id == raw.root || structural || normalized_node(node) {
            retain_ancestors(raw, node.id, &mut retained);
        }
    }
    retained
}

fn retain_ancestors(raw: &RawSnapshot, mut id: NodeId, retained: &mut HashSet<NodeId>) {
    while retained.insert(id) {
        let Some(parent) = raw.nodes.get(&id).and_then(|n| n.parent) else {
            break;
        };
        id = parent;
    }
}
