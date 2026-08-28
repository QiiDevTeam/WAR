use war_protocol::{DesktopNode, RawSnapshot};

pub fn normalize(mut raw: RawSnapshot) -> RawSnapshot {
    for node in raw.nodes.values_mut() {
        node.name = clean(node.name.take());
        node.automation_id = clean(node.automation_id.take());
        node.value = clean_preserving_empty(node.value.take());
        node.description = clean(node.description.take());
        if node.bounds.is_some_and(|rect| rect.is_empty()) {
            node.bounds = None;
        }
        node.children.retain(|id| *id != node.id && raw.root != *id);
    }
    raw
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|v| !v.is_empty())
}

fn clean_preserving_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string())
}

pub fn normalized_node(node: &DesktopNode) -> bool {
    let hidden = node.states.offscreen
        && !node.states.focused
        && node.states.selected != Some(true)
        && ((node.role == war_protocol::Role::TabItem && node.states.selected == Some(false))
            || node.capabilities.is_empty());
    if hidden {
        return false;
    }
    node.name.is_some()
        || node.value.is_some()
        || !node.capabilities.is_empty()
        || node.states.focused
}
