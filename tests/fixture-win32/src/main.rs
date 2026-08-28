use std::io::Write;
use windows::{
    core::w,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    },
};

const INPUT_ID: i32 = 101;
const APPLY_ID: i32 = 102;

fn main() -> windows::core::Result<()> {
    let overlay = std::env::var_os("WAR_FIXTURE_OVERLAY").is_some();
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("WAR Native Fixture"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            700,
            420,
            None,
            None,
            Some(instance),
            None,
        )?;
        create_controls(window, instance)?;
        let mut auxiliary_x = CW_USEDEFAULT;
        let mut auxiliary_y = CW_USEDEFAULT;
        let mut auxiliary_style = WINDOW_EX_STYLE::default();
        if overlay {
            let mut rect = RECT::default();
            GetWindowRect(window, &mut rect)?;
            auxiliary_x = rect.left + 10;
            auxiliary_y = rect.top + 50;
            auxiliary_style = WS_EX_TOPMOST;
        }
        CreateWindowExW(
            auxiliary_style,
            w!("STATIC"),
            w!("WAR Native Auxiliary"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            auxiliary_x,
            auxiliary_y,
            320,
            180,
            None,
            None,
            Some(instance),
            None,
        )?;
        let input = GetDlgItem(Some(window), INPUT_ID)?;
        let _ = SetFocus(Some(input));

        println!("HWND={}", window.0 as usize);
        std::io::stdout().flush().ok();

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe fn create_controls(parent: HWND, instance: HINSTANCE) -> windows::core::Result<()> {
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Agent input"),
        WS_CHILD | WS_VISIBLE,
        24,
        22,
        110,
        24,
        Some(parent),
        None,
        Some(instance),
        None,
    )?;
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("EDIT"),
        w!("before"),
        WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        140,
        18,
        330,
        30,
        Some(parent),
        Some(HMENU(INPUT_ID as usize as *mut _)),
        Some(instance),
        None,
    )?;
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Apply"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        24,
        72,
        100,
        34,
        Some(parent),
        Some(HMENU(APPLY_ID as usize as *mut _)),
        Some(instance),
        None,
    )?;
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("before"),
        WS_CHILD | WS_VISIBLE,
        145,
        79,
        325,
        24,
        Some(parent),
        None,
        Some(instance),
        None,
    )?;
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Enable feature"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        24,
        128,
        180,
        30,
        Some(parent),
        Some(HMENU(104usize as *mut _)),
        Some(instance),
        None,
    )?;
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Password"),
        WS_CHILD | WS_VISIBLE,
        230,
        132,
        90,
        24,
        Some(parent),
        None,
        Some(instance),
        None,
    )?;
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("EDIT"),
        w!("super-secret"),
        WS_CHILD
            | WS_VISIBLE
            | WS_BORDER
            | WS_TABSTOP
            | WINDOW_STYLE((ES_AUTOHSCROLL | ES_PASSWORD) as u32),
        320,
        128,
        150,
        30,
        Some(parent),
        Some(HMENU(105usize as *mut _)),
        Some(instance),
        None,
    )?;
    let list = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("LISTBOX"),
        None,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(LBS_STANDARD as u32),
        24,
        185,
        260,
        155,
        Some(parent),
        Some(HMENU(106usize as *mut _)),
        Some(instance),
        None,
    )?;
    for index in 1..=30 {
        let item: Vec<u16> = format!("Item {index:02}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        SendMessageW(
            list,
            LB_ADDSTRING,
            Some(WPARAM(0)),
            Some(LPARAM(item.as_ptr() as isize)),
        );
    }
    Ok(())
}
