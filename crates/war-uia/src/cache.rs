use windows::Win32::UI::Accessibility::*;

pub unsafe fn create_cache(
    automation: &IUIAutomation,
) -> windows::core::Result<IUIAutomationCacheRequest> {
    let cache = automation.CreateCacheRequest()?;
    cache.SetTreeScope(TreeScope_Subtree)?;
    let control_view = automation.ControlViewCondition()?;
    cache.SetTreeFilter(&control_view)?;
    cache.SetAutomationElementMode(AutomationElementMode_Full)?;
    for property in [
        UIA_NamePropertyId,
        UIA_AutomationIdPropertyId,
        UIA_ControlTypePropertyId,
        UIA_BoundingRectanglePropertyId,
        UIA_IsEnabledPropertyId,
        UIA_IsOffscreenPropertyId,
        UIA_IsKeyboardFocusablePropertyId,
        UIA_IsPasswordPropertyId,
        UIA_IsDialogPropertyId,
        UIA_HasKeyboardFocusPropertyId,
        UIA_ProcessIdPropertyId,
        UIA_NativeWindowHandlePropertyId,
        UIA_HelpTextPropertyId,
        UIA_ValueValuePropertyId,
        UIA_ValueIsReadOnlyPropertyId,
        UIA_ToggleToggleStatePropertyId,
        UIA_SelectionItemIsSelectedPropertyId,
        UIA_ExpandCollapseExpandCollapseStatePropertyId,
    ] {
        cache.AddProperty(property)?;
    }
    for pattern in [
        UIA_InvokePatternId,
        UIA_ValuePatternId,
        UIA_TogglePatternId,
        UIA_SelectionItemPatternId,
        UIA_ExpandCollapsePatternId,
        UIA_ScrollPatternId,
    ] {
        cache.AddPattern(pattern)?;
    }
    Ok(cache)
}
