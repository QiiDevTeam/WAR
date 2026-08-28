use std::fmt::Write;
use war_protocol::{Capabilities, SemanticNode, SemanticSnapshot, SnapshotDelta};

pub fn render_snapshot(snapshot: &SemanticSnapshot) -> String {
    let mut out = format!("epoch={}\n", snapshot.epoch);
    if let Some(app) = &snapshot.window.app {
        let _ = writeln!(out, "app={app:?}");
    }
    if let Some(title) = &snapshot.window.title {
        let _ = writeln!(out, "window={title:?}");
    }
    if snapshot.truncated {
        let _ = writeln!(
            out,
            "nodes={}/{} truncated",
            snapshot.nodes.len(),
            snapshot.total_nodes
        );
    }
    out.push('\n');
    for node in &snapshot.nodes {
        let _ = writeln!(out, "{}", render_node(node));
    }
    out
}

pub fn render_delta(delta: &SnapshotDelta) -> String {
    let mut out = format!("epoch {} -> {}\n\n", delta.from_epoch, delta.to_epoch);
    for node in &delta.added {
        let _ = writeln!(out, "+ {}", render_node(node));
    }
    for id in &delta.removed {
        let _ = writeln!(out, "- @{id}");
    }
    for change in &delta.changed {
        let _ = writeln!(
            out,
            "~ @{} {} -> {}",
            change.id,
            concise(&change.before),
            concise(&change.after)
        );
    }
    if let Some(focus) = &delta.focus_changed {
        let _ = writeln!(
            out,
            "focus: {:?} -> {:?}",
            focus.from.map(|id| format!("@{id}")),
            focus.to.map(|id| format!("@{id}"))
        );
    }
    out
}

pub fn render_node(node: &SemanticNode) -> String {
    let mut out = format!(
        "{}@{} {}",
        "  ".repeat(node.depth as usize),
        node.id,
        node.role
    );
    if let Some(name) = &node.name {
        let _ = write!(out, " {name:?}");
    }
    if let Some(value) = &node.value {
        let _ = write!(out, " = {value:?}");
    }
    if node.states.focused {
        out.push_str(" focused");
    }
    if node.states.selected == Some(true) {
        out.push_str(" selected");
    }
    if node.states.checked == Some(true) {
        out.push_str(" checked");
    }
    let capabilities = capability_names(node.capabilities);
    if !capabilities.is_empty() {
        let _ = write!(out, " [{}]", capabilities.join(","));
    }
    out
}

fn concise(node: &SemanticNode) -> String {
    format!(
        "{} {:?} value={:?} state={:?}",
        node.role, node.name, node.value, node.states
    )
}

fn capability_names(c: Capabilities) -> Vec<&'static str> {
    [
        (Capabilities::INVOKE, "invoke"),
        (Capabilities::SET_VALUE, "set_value"),
        (Capabilities::GET_VALUE, "get_value"),
        (Capabilities::TOGGLE, "toggle"),
        (Capabilities::SELECT, "select"),
        (Capabilities::EXPAND, "expand"),
        (Capabilities::COLLAPSE, "collapse"),
        (Capabilities::SCROLL, "scroll"),
        (Capabilities::FOCUS, "focus"),
        (Capabilities::CLICK, "click"),
        (Capabilities::TYPE_TEXT, "type_text"),
        (Capabilities::POINTER_GESTURE, "pointer_gesture"),
    ]
    .into_iter()
    .filter_map(|(flag, name)| c.contains(flag).then_some(name))
    .collect()
}
