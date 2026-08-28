use crate::{
    events::{EventHandlers, EventTicker},
    tree,
};
use crossbeam_channel::{Receiver, Sender};
use std::{collections::HashMap, thread, time::Duration};
use war_core::{ProviderActionResult, ProviderNodeRef};
use war_protocol::{
    Action, DesktopEvent, Key, Modifiers, MouseButton, NodeId, Property, RawSnapshot, ScrollAmount,
    SnapshotScope, Target, WarError, WarResult,
};
use windows::core::BSTR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::*;

pub enum UiaCommand {
    Snapshot(SnapshotScope, Sender<WarResult<RawSnapshot>>),
    Execute(
        Option<ProviderNodeRef>,
        Action,
        Sender<WarResult<ProviderActionResult>>,
    ),
    Subscribe(Sender<DesktopEvent>),
    Shutdown,
}

pub fn spawn_worker() -> WarResult<Sender<UiaCommand>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
    thread::Builder::new()
        .name("war-uia-mta".into())
        .spawn(move || worker(rx, ready_tx))
        .map_err(|e| WarError::Provider(e.to_string()))?;
    ready_rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|error| match error {
            crossbeam_channel::RecvTimeoutError::Timeout => WarError::Timeout {
                operation: "UIA worker initialization".into(),
                timeout_ms: 3_000,
            },
            crossbeam_channel::RecvTimeoutError::Disconnected => {
                WarError::Provider("UIA worker initialization disconnected".into())
            }
        })??;
    Ok(tx)
}

fn worker(rx: Receiver<UiaCommand>, ready: Sender<WarResult<()>>) {
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
        let _ = ready.send(Err(WarError::Provider(
            windows::core::Error::from_hresult(initialized).to_string(),
        )));
        return;
    }
    let automation: windows::core::Result<IUIAutomation> =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) };
    let Ok(automation) = automation else {
        let _ = ready.send(Err(WarError::Provider(automation.unwrap_err().to_string())));
        unsafe { CoUninitialize() };
        return;
    };
    let (dirty_tx, dirty_rx) = crossbeam_channel::bounded::<()>(1);
    // Event hooks are installed only after a scoped snapshot has established a
    // safe subtree. Global hooks can deadlock some Win32 providers during
    // ElementFromHandle resolution.
    let handlers = Some(EventHandlers::new(dirty_tx));
    let _ = ready.send(Ok(()));
    let mut state = WorkerState::new(automation, handlers);
    let mut fallback_ticker = EventTicker::new(Duration::from_millis(500));
    let mut health_ticker = EventTicker::new(Duration::from_secs(10));
    loop {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(UiaCommand::Snapshot(scope, reply)) => {
                let _ = reply.send(state.snapshot(scope));
            }
            Ok(UiaCommand::Execute(node, action, reply)) => {
                let result = state.execute(node, &action);
                if result.is_ok() {
                    state.notify_dirty();
                }
                let _ = reply.send(result);
            }
            Ok(UiaCommand::Subscribe(sink)) => state.subscribe(sink),
            Ok(UiaCommand::Shutdown) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                break
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                let subscribed = !state.subscribers.is_empty();
                let fallback_due = !state.event_hooks_active && fallback_ticker.due();
                if dirty_rx.try_recv().is_ok()
                    || (subscribed && (fallback_due || health_ticker.due()))
                {
                    state.notify_dirty();
                }
            }
        }
    }
    if let Some(handlers) = &mut state.handlers {
        unsafe {
            handlers.uninstall(&state.automation);
        }
    }
    unsafe { CoUninitialize() };
}

struct WorkerState {
    automation: IUIAutomation,
    handlers: Option<EventHandlers>,
    epoch: u64,
    elements: HashMap<NodeId, IUIAutomationElement>,
    root: NodeId,
    event_roots: Vec<NodeId>,
    subscribers: Vec<Sender<DesktopEvent>>,
    event_hooks_active: bool,
    process_id: u32,
}
impl WorkerState {
    fn new(automation: IUIAutomation, handlers: Option<EventHandlers>) -> Self {
        Self {
            automation,
            handlers,
            epoch: 0,
            elements: HashMap::new(),
            root: 0,
            event_roots: Vec::new(),
            subscribers: Vec::new(),
            event_hooks_active: false,
            process_id: 0,
        }
    }
    fn snapshot(&mut self, scope: SnapshotScope) -> WarResult<RawSnapshot> {
        self.epoch += 1;
        // A dynamic provider can destroy the old subtree while BuildUpdatedCache
        // is reading the new one. Keeping callbacks rooted in that old subtree
        // during the cache build can corrupt UIAutomationCore's heap (observed
        // with Chromium navigation), so event hooks never span a refresh.
        self.suspend_event_hooks();
        let mut retry_delay = Duration::from_millis(20);
        let result = loop {
            match unsafe {
                tree::snapshot(&self.automation, scope.clone(), self.epoch, &self.elements)
            } {
                Ok(result) => break result,
                Err(error)
                    if is_transient_snapshot_error(&error)
                        && retry_delay <= Duration::from_millis(160) =>
                {
                    thread::sleep(retry_delay);
                    retry_delay *= 2;
                }
                Err(error) => return Err(error),
            }
        };
        self.root = result.snapshot.root;
        self.process_id = result
            .snapshot
            .nodes
            .get(&self.root)
            .map(|node| node.fingerprint.process_id)
            .unwrap_or_default();
        self.event_roots = result.event_roots;
        self.elements = result.elements;
        self.install_event_hooks();
        Ok(result.snapshot)
    }

    fn subscribe(&mut self, sink: Sender<DesktopEvent>) {
        self.subscribers.push(sink);
        self.install_event_hooks();
    }

    fn suspend_event_hooks(&mut self) {
        if self.event_hooks_active {
            if let Some(handlers) = &mut self.handlers {
                unsafe { handlers.uninstall(&self.automation) };
            }
        }
        self.event_hooks_active = false;
    }

    fn install_event_hooks(&mut self) {
        let process_name = war_win32::process_name(self.process_id);
        if self.subscribers.is_empty()
            || self.event_hooks_active
            || !native_event_hooks_allowed(process_name.as_deref())
        {
            return;
        }
        let roots: Vec<_> = self
            .event_roots
            .iter()
            .filter_map(|id| self.elements.get(id).cloned())
            .collect();
        if let Some(handlers) = &mut self.handlers {
            // Subtree hooks can also be denied for elevated/protected windows; the
            // periodic dirty tick is the compatibility fallback.
            self.event_hooks_active = !roots.is_empty()
                && unsafe { handlers.watch_subtrees(&self.automation, &roots) }.is_ok();
        }
    }

    fn notify_dirty(&mut self) {
        let event = DesktopEvent::PropertyChanged {
            node: self.root,
            property: Property::State,
        };
        self.subscribers
            .retain(|sink| sink.send(event.clone()).is_ok());
        if self.subscribers.is_empty() {
            self.suspend_event_hooks();
        }
    }
    fn element(&self, node: Option<ProviderNodeRef>) -> WarResult<IUIAutomationElement> {
        let id = node
            .ok_or_else(|| WarError::TargetNotFound("action needs a target".into()))?
            .id;
        self.elements
            .get(&id)
            .cloned()
            .ok_or_else(|| WarError::TargetNotFound(format!("provider node {id}")))
    }
    fn execute(
        &mut self,
        node: Option<ProviderNodeRef>,
        action: &Action,
    ) -> WarResult<ProviderActionResult> {
        // Actions can synchronously rebuild an application's accessibility tree.
        // Reinstall hooks only after the following stable snapshot.
        self.suspend_event_hooks();
        match action {
            Action::TypeText { text } => {
                self.ensure_foreground()?;
                war_win32::type_text(text)?;
                return ok("send_input.text", false);
            }
            Action::Key { key, modifiers } => {
                self.ensure_foreground()?;
                war_win32::press_key(key.clone(), *modifiers)?;
                return ok("send_input.key", false);
            }
            Action::Click {
                target: Target::Coordinates(point),
                button,
            } => {
                self.ensure_foreground()?;
                let hit_process = war_win32::window_at(*point)?
                    .map(war_win32::window_info)
                    .map(|window| window.process_id)
                    .unwrap_or_default();
                if hit_process != self.process_id {
                    return Err(WarError::ForegroundMismatch {
                        target_process: self.process_id,
                        foreground_process: hit_process,
                    });
                }
                war_win32::click(*point, *button)?;
                return ok("send_input.click", false);
            }
            _ => {}
        }
        let element = self.element(node)?;
        unsafe { execute_element(&element, action) }
    }

    fn ensure_foreground(&self) -> WarResult<()> {
        let foreground_process = war_win32::foreground_process_id();
        if self.process_id != 0 && self.process_id == foreground_process {
            Ok(())
        } else {
            Err(WarError::ForegroundMismatch {
                target_process: self.process_id,
                foreground_process,
            })
        }
    }
}

fn is_transient_snapshot_error(error: &WarError) -> bool {
    matches!(error, WarError::Provider(message) if
        message.contains("Malformed Cacheresponse")
            || message.contains("0x80004005")
            || message.contains("Pattern not found")
            || message.contains("0x80040201"))
}

fn native_event_hooks_allowed(process_name: Option<&str>) -> bool {
    !process_name.is_some_and(|name| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "chrome.exe" | "msedge.exe"
        )
    })
}

unsafe fn execute_element(
    element: &IUIAutomationElement,
    action: &Action,
) -> WarResult<ProviderActionResult> {
    match action {
        Action::Invoke { .. } => {
            if let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            {
                pattern.Invoke().map_err(provider_error)?;
                return ok("uia.invoke", false);
            }
            if element.SetFocus().is_ok()
                && foreground_matches(element).is_ok()
                && war_win32::press_key(Key::Space, Modifiers::default()).is_ok()
            {
                return ok("focus+space", true);
            }
            click_element(element, MouseButton::Left)?;
            ok("send_input.click", true)
        }
        Action::SetValue { value, .. } => {
            if let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            {
                pattern
                    .SetValue(&BSTR::from(value))
                    .map_err(provider_error)?;
                return ok("uia.value", false);
            }
            element.SetFocus().map_err(provider_error)?;
            foreground_matches(element)?;
            war_win32::select_all()?;
            war_win32::type_text(value)?;
            ok("focus+select_all+send_input", true)
        }
        Action::Toggle { value, .. } => {
            if let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
            {
                let wanted = *value;
                if wanted.is_none()
                    || pattern
                        .CurrentToggleState()
                        .ok()
                        .map(|s| s == ToggleState_On)
                        != wanted
                {
                    pattern.Toggle().map_err(provider_error)?;
                }
                return ok("uia.toggle", false);
            }
            click_element(element, MouseButton::Left)?;
            ok("send_input.click", true)
        }
        Action::Select { .. } => {
            if let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                UIA_SelectionItemPatternId,
            ) {
                pattern.Select().map_err(provider_error)?;
                return ok("uia.select", false);
            }
            element.SetFocus().map_err(provider_error)?;
            foreground_matches(element)?;
            war_win32::press_key(Key::Space, Modifiers::default())?;
            ok("focus+space", true)
        }
        Action::Focus { .. } => {
            element.SetFocus().map_err(provider_error)?;
            ok("uia.focus", false)
        }
        Action::Scroll { amount, .. } => {
            if let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
            {
                pattern
                    .Scroll(ScrollAmount_NoAmount, scroll(*amount))
                    .map_err(provider_error)?;
                return ok("uia.scroll", false);
            }
            element.SetFocus().map_err(provider_error)?;
            foreground_matches(element)?;
            let key = match amount {
                ScrollAmount::LargeDecrement => Key::Home,
                ScrollAmount::SmallDecrement => Key::Up,
                ScrollAmount::NoAmount => return ok("noop", true),
                ScrollAmount::LargeIncrement => Key::End,
                ScrollAmount::SmallIncrement => Key::Down,
            };
            war_win32::press_key(key, Modifiers::default())?;
            ok("focus+key", true)
        }
        Action::Click { button, .. } => {
            click_element(element, *button)?;
            ok("send_input.click", true)
        }
        Action::PointerGesture {
            button,
            points,
            duration_ms,
            ..
        } => {
            gesture_element(element, *button, points, *duration_ms)?;
            ok("send_input.pointer_gesture", true)
        }
        Action::TypeText { .. } | Action::Key { .. } => unreachable!(),
    }
}

unsafe fn gesture_element(
    element: &IUIAutomationElement,
    button: MouseButton,
    points: &[war_protocol::NormalizedPoint],
    duration_ms: u64,
) -> WarResult<()> {
    foreground_matches(element)?;
    let rect = element.CurrentBoundingRectangle().map_err(provider_error)?;
    let width = (rect.right - rect.left) as f64;
    let height = (rect.bottom - rect.top) as f64;
    if width <= 0.0 || height <= 0.0 {
        return Err(WarError::CapabilityUnavailable(
            "pointer gesture target has empty bounds".into(),
        ));
    }
    let screen_points = points
        .iter()
        .map(|point| war_protocol::Point {
            x: rect.left as f64 + width * point.x,
            y: rect.top as f64 + height * point.y,
        })
        .collect::<Vec<_>>();
    if let Some(target) = element
        .CachedNativeWindowHandle()
        .ok()
        .and_then(war_win32::root_window)
    {
        for point in &screen_points {
            if let Some(hit) = war_win32::window_at(*point)?.and_then(war_win32::root_window) {
                if target != hit {
                    return Err(WarError::HitTestMismatch {
                        target_window: war_win32::hwnd_to_id(target),
                        hit_window: war_win32::hwnd_to_id(hit),
                    });
                }
            }
        }
    }
    war_win32::pointer_gesture(
        &screen_points,
        button,
        std::time::Duration::from_millis(duration_ms),
    )
}

unsafe fn click_element(element: &IUIAutomationElement, button: MouseButton) -> WarResult<()> {
    foreground_matches(element)?;
    let rect = element.CurrentBoundingRectangle().map_err(provider_error)?;
    let point = war_protocol::Point {
        x: (rect.left + rect.right) as f64 / 2.0,
        y: (rect.top + rect.bottom) as f64 / 2.0,
    };
    let target = element.CachedNativeWindowHandle().ok();
    let hit = war_win32::window_at(point)?;
    if let (Some(target), Some(hit)) = (
        target.and_then(war_win32::root_window),
        hit.and_then(war_win32::root_window),
    ) {
        if target != hit {
            return Err(WarError::HitTestMismatch {
                target_window: war_win32::hwnd_to_id(target),
                hit_window: war_win32::hwnd_to_id(hit),
            });
        }
    }
    war_win32::click(point, button)
}
fn foreground_matches(element: &IUIAutomationElement) -> WarResult<()> {
    let target_process = unsafe { element.CachedProcessId() }
        .map(|process| process as u32)
        .map_err(provider_error)?;
    let foreground_process = war_win32::foreground_process_id();
    if target_process != 0 && target_process == foreground_process {
        Ok(())
    } else {
        Err(WarError::ForegroundMismatch {
            target_process,
            foreground_process,
        })
    }
}
fn scroll(value: ScrollAmount) -> windows::Win32::UI::Accessibility::ScrollAmount {
    match value {
        ScrollAmount::LargeDecrement => ScrollAmount_LargeDecrement,
        ScrollAmount::SmallDecrement => ScrollAmount_SmallDecrement,
        ScrollAmount::NoAmount => ScrollAmount_NoAmount,
        ScrollAmount::LargeIncrement => ScrollAmount_LargeIncrement,
        ScrollAmount::SmallIncrement => ScrollAmount_SmallIncrement,
    }
}
fn ok(method: &str, fallback_used: bool) -> WarResult<ProviderActionResult> {
    Ok(ProviderActionResult {
        method: method.into(),
        fallback_used,
    })
}

fn provider_error(error: windows::core::Error) -> WarError {
    WarError::Provider(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_known_transient_cache_failures() {
        assert!(is_transient_snapshot_error(&WarError::Provider(
            "Malformed Cacheresponse pTreeStructure String (0x80004005)".into()
        )));
        assert!(is_transient_snapshot_error(&WarError::Provider(
            "Pattern not found (0x80040201)".into()
        )));
        assert!(!is_transient_snapshot_error(&WarError::Provider(
            "access denied (0x80070005)".into()
        )));
        assert!(!is_transient_snapshot_error(&WarError::TargetNotFound(
            "window".into()
        )));
    }

    #[test]
    fn chromium_uses_polling_instead_of_native_subtree_events() {
        assert!(!native_event_hooks_allowed(Some("chrome.exe")));
        assert!(!native_event_hooks_allowed(Some("MSEDGE.EXE")));
        assert!(native_event_hooks_allowed(Some("notepad.exe")));
        assert!(native_event_hooks_allowed(None));
    }
}
