use crossbeam_channel::Sender;
use std::time::{Duration, Instant};
use windows::core::{implement, Ref};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;

pub struct EventTicker {
    last: Instant,
    interval: Duration,
}
impl EventTicker {
    pub fn new(interval: Duration) -> Self {
        Self {
            last: Instant::now(),
            interval,
        }
    }
    pub fn due(&mut self) -> bool {
        if self.last.elapsed() >= self.interval {
            self.last = Instant::now();
            true
        } else {
            false
        }
    }
}

#[implement(IUIAutomationPropertyChangedEventHandler)]
struct PropertyHandler {
    dirty: Sender<()>,
}

impl IUIAutomationPropertyChangedEventHandler_Impl for PropertyHandler_Impl {
    fn HandlePropertyChangedEvent(
        &self,
        _sender: Ref<'_, IUIAutomationElement>,
        _property_id: UIA_PROPERTY_ID,
        _new_value: &VARIANT,
    ) -> windows::core::Result<()> {
        let _ = self.dirty.try_send(());
        Ok(())
    }
}

#[implement(IUIAutomationStructureChangedEventHandler)]
struct StructureHandler {
    dirty: Sender<()>,
}

impl IUIAutomationStructureChangedEventHandler_Impl for StructureHandler_Impl {
    fn HandleStructureChangedEvent(
        &self,
        _sender: Ref<'_, IUIAutomationElement>,
        _change: StructureChangeType,
        _runtime_id: *const windows::Win32::System::Com::SAFEARRAY,
    ) -> windows::core::Result<()> {
        let _ = self.dirty.try_send(());
        Ok(())
    }
}

pub struct EventHandlers {
    structure: IUIAutomationStructureChangedEventHandler,
    property: IUIAutomationPropertyChangedEventHandler,
    roots: Vec<IUIAutomationElement>,
}

impl EventHandlers {
    pub fn new(dirty: Sender<()>) -> Self {
        let structure: IUIAutomationStructureChangedEventHandler = StructureHandler {
            dirty: dirty.clone(),
        }
        .into();
        let property: IUIAutomationPropertyChangedEventHandler = PropertyHandler { dirty }.into();
        Self {
            structure,
            property,
            roots: Vec::new(),
        }
    }

    pub unsafe fn watch_subtrees(
        &mut self,
        automation: &IUIAutomation,
        roots: &[IUIAutomationElement],
    ) -> windows::core::Result<()> {
        self.remove_all(automation);
        let properties = [
            UIA_NamePropertyId,
            UIA_ValueValuePropertyId,
            UIA_ValueIsReadOnlyPropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsEnabledPropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_IsPasswordPropertyId,
            UIA_IsDialogPropertyId,
            UIA_HasKeyboardFocusPropertyId,
            UIA_ToggleToggleStatePropertyId,
            UIA_SelectionItemIsSelectedPropertyId,
            UIA_ExpandCollapseExpandCollapseStatePropertyId,
        ];
        for root in roots {
            if let Err(error) = automation.AddStructureChangedEventHandler(
                root,
                TreeScope_Subtree,
                None::<&IUIAutomationCacheRequest>,
                &self.structure,
            ) {
                self.remove_all(automation);
                return Err(error);
            }
            if let Err(error) = automation.AddPropertyChangedEventHandlerNativeArray(
                root,
                TreeScope_Subtree,
                None::<&IUIAutomationCacheRequest>,
                &self.property,
                &properties,
            ) {
                let _ = automation.RemoveStructureChangedEventHandler(root, &self.structure);
                self.remove_all(automation);
                return Err(error);
            }
            self.roots.push(root.clone());
        }
        Ok(())
    }

    pub unsafe fn uninstall(&mut self, automation: &IUIAutomation) {
        self.remove_all(automation);
    }

    unsafe fn remove_all(&mut self, automation: &IUIAutomation) {
        for root in self.roots.drain(..) {
            let _ = automation.RemoveStructureChangedEventHandler(&root, &self.structure);
            let _ = automation.RemovePropertyChangedEventHandler(&root, &self.property);
        }
    }
}
