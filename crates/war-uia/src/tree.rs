use crate::{cache::create_cache, convert, patterns};
use std::collections::HashMap;
use war_protocol::{
    Capabilities, DesktopNode, NodeFingerprint, NodeId, NodeSource, NodeStates, RawSnapshot, Rect,
    Role, SnapshotScope, WarError, WarResult,
};
use windows::Win32::UI::Accessibility::*;
use windows_core::Interface;

const MAX_RAW_NODES: usize = 2_048;
const MAX_DEPTH: u8 = 64;

pub struct TreeResult {
    pub snapshot: RawSnapshot,
    pub elements: HashMap<NodeId, IUIAutomationElement>,
    pub event_roots: Vec<NodeId>,
}

pub unsafe fn snapshot(
    automation: &IUIAutomation,
    scope: SnapshotScope,
    epoch: u64,
    old: &HashMap<NodeId, IUIAutomationElement>,
) -> WarResult<TreeResult> {
    if let SnapshotScope::Process(process_id) = scope {
        return snapshot_process(automation, process_id, epoch);
    }
    let start = scope_element(automation, scope, old).map_err(provider_error)?;
    let cache = create_cache(automation).map_err(provider_error)?;
    // One cross-process cache build fetches the complete control-view subtree.
    // Traversal below reads only cached children/properties/patterns.
    let root_element = start
        .BuildUpdatedCache(&cache)
        .map_err(|error| WarError::Provider(format!("BuildUpdatedCache: {error}")))?;
    let mut nodes = HashMap::new();
    let mut elements = HashMap::new();
    let mut next = 1;
    let root = visit_cached(
        &root_element,
        None,
        0,
        0,
        &mut next,
        &mut nodes,
        &mut elements,
    )
    .map_err(|error| WarError::Provider(format!("visit_cached: {error}")))?;
    Ok(TreeResult {
        snapshot: RawSnapshot { epoch, root, nodes },
        elements,
        event_roots: vec![root],
    })
}

unsafe fn snapshot_process(
    automation: &IUIAutomation,
    process_id: u32,
    epoch: u64,
) -> WarResult<TreeResult> {
    let windows = war_win32::process_windows(process_id);
    if windows.is_empty() {
        return Err(WarError::TargetNotFound(format!(
            "process {process_id} has no visible top-level window"
        )));
    }
    let cache = create_cache(automation).map_err(provider_error)?;
    let root = 1;
    let mut next = 2;
    let mut nodes = HashMap::new();
    let mut elements = HashMap::new();
    let mut children = Vec::new();
    let mut event_roots = Vec::new();
    for (index, window) in windows.into_iter().enumerate() {
        if nodes.len() + 1 >= MAX_RAW_NODES {
            break;
        }
        let Ok(element) = automation
            .ElementFromHandle(window)
            .and_then(|element| element.BuildUpdatedCache(&cache))
        else {
            // One protected/helper window must not hide every other window in
            // an explicitly scoped process.
            continue;
        };
        let child = visit_cached(
            &element,
            Some(root),
            index.min(u16::MAX as usize) as u16,
            1,
            &mut next,
            &mut nodes,
            &mut elements,
        )
        .map_err(provider_error)?;
        children.push(child);
        event_roots.push(child);
    }
    if children.is_empty() {
        return Err(WarError::Provider(format!(
            "process {process_id} has no accessible top-level UIA window"
        )));
    }
    nodes.insert(
        root,
        DesktopNode {
            id: root,
            source: NodeSource::Win32,
            role: Role::Group,
            name: Some(format!("Process {process_id}")),
            automation_id: None,
            value: None,
            description: None,
            bounds: None,
            states: NodeStates {
                enabled: true,
                ..Default::default()
            },
            capabilities: Capabilities::empty(),
            parent: None,
            children,
            fingerprint: NodeFingerprint {
                process_id,
                role: Role::Group,
                ..Default::default()
            },
        },
    );
    Ok(TreeResult {
        snapshot: RawSnapshot { epoch, root, nodes },
        elements,
        event_roots,
    })
}

unsafe fn scope_element(
    automation: &IUIAutomation,
    scope: SnapshotScope,
    old: &HashMap<NodeId, IUIAutomationElement>,
) -> windows::core::Result<IUIAutomationElement> {
    match scope {
        SnapshotScope::Desktop => automation.GetRootElement(),
        SnapshotScope::Window(id) => automation.ElementFromHandle(war_win32::id_to_hwnd(id)),
        SnapshotScope::Node(id) => old.get(&id).cloned().ok_or_else(not_found),
        SnapshotScope::FocusedSubtree => automation.GetFocusedElement(),
        SnapshotScope::Process(_) => unreachable!("process scope is handled as a window set"),
        SnapshotScope::FocusedWindow => war_win32::foreground_window()
            .ok_or_else(not_found)
            .and_then(|window| automation.ElementFromHandle(window)),
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn visit_cached(
    element: &IUIAutomationElement,
    parent: Option<NodeId>,
    sibling: u16,
    depth: u8,
    next: &mut NodeId,
    nodes: &mut HashMap<NodeId, DesktopNode>,
    elements: &mut HashMap<NodeId, IUIAutomationElement>,
) -> windows::core::Result<NodeId> {
    let id = *next;
    *next += 1;
    let control = element
        .CachedControlType()
        .unwrap_or(UIA_CustomControlTypeId);
    let mut role = convert::role(control);
    if role == Role::Window
        && element
            .cast::<IUIAutomationElement9>()
            .and_then(|element| element.CachedIsDialog())
            .map(|value| value.as_bool())
            .unwrap_or(false)
    {
        role = Role::Dialog;
    }
    let mut states = NodeStates {
        enabled: element
            .CachedIsEnabled()
            .map(|value| value.as_bool())
            .unwrap_or(true),
        focused: element
            .CachedHasKeyboardFocus()
            .map(|value| value.as_bool())
            .unwrap_or(false),
        focusable: element
            .CachedIsKeyboardFocusable()
            .map(|value| value.as_bool())
            .unwrap_or(false),
        offscreen: element
            .CachedIsOffscreen()
            .map(|value| value.as_bool())
            .unwrap_or(false),
        ..Default::default()
    };
    let bounds = element.CachedBoundingRectangle().ok().map(|rect| Rect {
        left: rect.left as f64,
        top: rect.top as f64,
        width: (rect.right - rect.left) as f64,
        height: (rect.bottom - rect.top) as f64,
    });
    let mut capabilities = patterns::inspect(element, &mut states);
    let is_password = element
        .CachedIsPassword()
        .map(|value| value.as_bool())
        .unwrap_or(false);
    if is_password {
        capabilities.remove(Capabilities::GET_VALUE);
    }
    if states.enabled && states.focusable {
        capabilities |= Capabilities::FOCUS;
    }
    if supports_type_text(&states, capabilities) {
        capabilities |= Capabilities::TYPE_TEXT;
    }
    if !states.enabled {
        capabilities &= Capabilities::GET_VALUE;
    }
    if supports_pointer_gesture(role, &states, bounds) {
        capabilities |= Capabilities::POINTER_GESTURE;
    }
    if is_clickable(role, &states, bounds, capabilities) {
        capabilities |= Capabilities::CLICK;
    }
    let process_id = element.CachedProcessId().unwrap_or_default() as u32;
    let mut node = DesktopNode {
        id,
        source: NodeSource::Uia,
        role,
        name: limited(bstr(element.CachedName().ok()), 256),
        automation_id: limited(bstr(element.CachedAutomationId().ok()), 128),
        value: safe_value(is_password, role, patterns::current_value(element)),
        description: limited(bstr(element.CachedHelpText().ok()), 512),
        bounds,
        states,
        capabilities,
        parent,
        children: Vec::new(),
        fingerprint: NodeFingerprint {
            process_id,
            role: Role::Unknown,
            automation_id: None,
            name_hash: None,
            ancestor_hash: 0,
            sibling_hint: sibling,
        },
    };
    elements.insert(id, element.clone());

    if depth < MAX_DEPTH && nodes.len() < MAX_RAW_NODES {
        if let Ok(children) = element.GetCachedChildren() {
            let length = children.Length().unwrap_or_default().max(0);
            for index in 0..length {
                if nodes.len() + 1 >= MAX_RAW_NODES {
                    break;
                }
                let child = children.GetElement(index)?;
                let child_id = visit_cached(
                    &child,
                    Some(id),
                    index.min(u16::MAX as i32) as u16,
                    depth + 1,
                    next,
                    nodes,
                    elements,
                )?;
                node.children.push(child_id);
            }
        }
    }
    nodes.insert(id, node);
    Ok(id)
}

fn is_clickable(
    role: Role,
    states: &NodeStates,
    bounds: Option<Rect>,
    capabilities: Capabilities,
) -> bool {
    if !states.enabled || states.offscreen || bounds.map_or(true, Rect::is_empty) {
        return false;
    }
    capabilities.intersects(
        Capabilities::INVOKE
            | Capabilities::TOGGLE
            | Capabilities::SELECT
            | Capabilities::EXPAND
            | Capabilities::COLLAPSE,
    ) || matches!(
        role,
        Role::Button
            | Role::Toggle
            | Role::Checkbox
            | Role::Radio
            | Role::TextInput
            | Role::TextArea
            | Role::ComboBox
            | Role::ListItem
            | Role::TreeItem
            | Role::MenuItem
            | Role::TabItem
            | Role::Slider
            | Role::Canvas
            | Role::Link
    )
}

fn supports_type_text(states: &NodeStates, capabilities: Capabilities) -> bool {
    states.enabled && states.focusable && capabilities.contains(Capabilities::SET_VALUE)
}

fn supports_pointer_gesture(role: Role, states: &NodeStates, bounds: Option<Rect>) -> bool {
    states.enabled
        && !states.offscreen
        && bounds.is_some_and(|bounds| !bounds.is_empty())
        && matches!(
            role,
            Role::Canvas | Role::Pane | Role::Group | Role::Image | Role::Document | Role::Slider
        )
}

fn limited(value: Option<String>, max_chars: usize) -> Option<String> {
    value.map(|value| {
        if value.chars().count() <= max_chars {
            value
        } else {
            let mut truncated: String = value.chars().take(max_chars).collect();
            truncated.push('…');
            truncated
        }
    })
}

fn value_limit(role: Role) -> usize {
    match role {
        Role::Document | Role::TextArea => 1_024,
        Role::TextInput | Role::ComboBox => 512,
        _ => 256,
    }
}

fn safe_value(is_password: bool, role: Role, value: Option<String>) -> Option<String> {
    if is_password {
        None
    } else {
        limited(value, value_limit(role))
    }
}

fn bstr(value: Option<windows::core::BSTR>) -> Option<String> {
    value
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
}

fn not_found() -> windows::core::Error {
    windows::core::Error::new(
        windows::core::HRESULT(0x80070490u32 as i32),
        "snapshot scope has no accessible window",
    )
}

fn provider_error(error: windows::core::Error) -> WarError {
    WarError::Provider(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_values_are_removed_and_long_values_are_bounded() {
        assert_eq!(
            safe_value(true, Role::TextInput, Some("secret".into())),
            None
        );
        let value = "界".repeat(600);
        let bounded = safe_value(false, Role::TextInput, Some(value)).unwrap();
        assert_eq!(bounded.chars().count(), 513);
        assert!(bounded.ends_with('…'));
    }

    #[test]
    fn focusable_set_value_control_supports_typing_regardless_of_role() {
        let states = NodeStates {
            enabled: true,
            focusable: true,
            ..Default::default()
        };
        assert!(supports_type_text(&states, Capabilities::SET_VALUE));
        assert!(!supports_type_text(&states, Capabilities::GET_VALUE));
    }

    #[test]
    fn visible_bounded_canvas_like_control_supports_pointer_gestures() {
        let states = NodeStates {
            enabled: true,
            ..Default::default()
        };
        let bounds = Some(Rect {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 50.0,
        });
        assert!(supports_pointer_gesture(Role::Group, &states, bounds));
        assert!(!supports_pointer_gesture(Role::Button, &states, bounds));
    }
}
