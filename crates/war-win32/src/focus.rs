use war_protocol::{WarError, WarResult};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

pub fn foreground_window() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd)
}

pub fn foreground_process_id() -> u32 {
    foreground_window()
        .map(crate::window_info)
        .map(|window| window.process_id)
        .unwrap_or_default()
}

pub fn set_foreground(hwnd: HWND) -> WarResult<()> {
    if unsafe { SetForegroundWindow(hwnd) }.as_bool() {
        Ok(())
    } else {
        Err(WarError::Provider("SetForegroundWindow was denied".into()))
    }
}
