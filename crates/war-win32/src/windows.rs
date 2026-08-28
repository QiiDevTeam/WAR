use war_protocol::{WarError, WarResult, WindowId, WindowInfo};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetTopWindow, GetWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, WindowFromPoint, GA_ROOT, GW_HWNDNEXT,
};

pub fn hwnd_to_id(hwnd: HWND) -> WindowId {
    hwnd.0 as usize as u64
}
pub fn id_to_hwnd(id: WindowId) -> HWND {
    HWND(id as usize as *mut _)
}

pub fn window_title(hwnd: HWND) -> WarResult<Option<String>> {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len == 0 {
        return Ok(None);
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if copied == 0 {
        return Err(WarError::Provider("GetWindowTextW failed".into()));
    }
    Ok(Some(String::from_utf16_lossy(&buf[..copied as usize])))
}

pub fn window_info(hwnd: HWND) -> WindowInfo {
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    WindowInfo {
        id: hwnd_to_id(hwnd),
        process_id,
        app: crate::process_name(process_id),
        title: window_title(hwnd).ok().flatten(),
    }
}

pub fn process_window(process_id: u32) -> Option<HWND> {
    process_windows(process_id).into_iter().next()
}

pub fn process_windows(process_id: u32) -> Vec<HWND> {
    let mut result = Vec::new();
    unsafe {
        let Ok(mut window) = GetTopWindow(None) else {
            return result;
        };
        loop {
            let mut owner = 0;
            GetWindowThreadProcessId(window, Some(&mut owner));
            if owner == process_id && IsWindowVisible(window).as_bool() {
                result.push(window);
            }
            let Ok(next) = GetWindow(window, GW_HWNDNEXT) else {
                break;
            };
            window = next;
        }
    }
    result
}

pub fn root_window(hwnd: HWND) -> Option<HWND> {
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    (!root.0.is_null()).then_some(root)
}

pub fn window_at(point: war_protocol::Point) -> WarResult<Option<HWND>> {
    crate::validate_screen_point(point)?;
    let window = unsafe {
        WindowFromPoint(POINT {
            x: point.x.round() as i32,
            y: point.y.round() as i32,
        })
    };
    Ok((!window.0.is_null()).then_some(window))
}
