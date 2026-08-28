use war_protocol::{Capabilities, NodeStates};
use windows::Win32::UI::Accessibility::*;

pub unsafe fn inspect(element: &IUIAutomationElement, states: &mut NodeStates) -> Capabilities {
    let mut result = Capabilities::empty();
    if element
        .GetCachedPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
        .is_ok()
    {
        result |= Capabilities::INVOKE;
    }
    if let Ok(pattern) = element.GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
    {
        result |= Capabilities::GET_VALUE;
        if !pattern
            .CachedIsReadOnly()
            .map(|value| value.as_bool())
            .unwrap_or(true)
        {
            result |= Capabilities::SET_VALUE;
        }
    }
    if let Ok(pattern) =
        element.GetCachedPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
    {
        result |= Capabilities::TOGGLE;
        states.checked = pattern
            .CachedToggleState()
            .ok()
            .map(|v| v == ToggleState_On);
    }
    if let Ok(pattern) =
        element.GetCachedPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
    {
        result |= Capabilities::SELECT;
        states.selected = pattern.CachedIsSelected().ok().map(|v| v.as_bool());
    }
    if let Ok(pattern) = element
        .GetCachedPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    {
        result |= Capabilities::EXPAND | Capabilities::COLLAPSE;
        states.expanded = pattern
            .CachedExpandCollapseState()
            .ok()
            .map(|v| v == ExpandCollapseState_Expanded);
    }
    if element
        .GetCachedPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
        .is_ok()
    {
        result |= Capabilities::SCROLL;
    }
    result
}

pub unsafe fn current_value(element: &IUIAutomationElement) -> Option<String> {
    element
        .GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .ok()
        .and_then(|p| p.CachedValue().ok())
        .map(|v| v.to_string())
}
