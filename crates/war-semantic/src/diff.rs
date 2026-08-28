use std::collections::HashMap;
use war_protocol::{
    FocusChange, NodeChange, NodeId, SemanticNode, SemanticSnapshot, SnapshotDelta,
};

pub fn diff(before: &SemanticSnapshot, after: &SemanticSnapshot) -> SnapshotDelta {
    let old: HashMap<NodeId, &SemanticNode> = before.nodes.iter().map(|n| (n.id, n)).collect();
    let new: HashMap<NodeId, &SemanticNode> = after.nodes.iter().map(|n| (n.id, n)).collect();
    let added = after
        .nodes
        .iter()
        .filter(|n| !old.contains_key(&n.id))
        .cloned()
        .collect();
    let removed = before
        .nodes
        .iter()
        .filter(|n| !new.contains_key(&n.id))
        .map(|n| n.id)
        .collect();
    let changed = after
        .nodes
        .iter()
        .filter_map(|node| {
            old.get(&node.id)
                .filter(|old| ***old != *node)
                .map(|old| NodeChange {
                    id: node.id,
                    before: (*old).clone(),
                    after: node.clone(),
                })
        })
        .collect();
    let focus_changed = (before.focused != after.focused).then_some(FocusChange {
        from: before.focused,
        to: after.focused,
    });
    SnapshotDelta {
        from_epoch: before.epoch,
        to_epoch: after.epoch,
        added,
        removed,
        changed,
        focus_changed,
    }
}
