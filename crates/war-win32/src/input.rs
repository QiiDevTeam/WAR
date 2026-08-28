use std::mem::size_of;
use war_protocol::{Key, Modifiers, MouseButton, Point, WarError, WarResult};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SetCursorPos, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

fn send(inputs: &[INPUT]) -> WarResult<()> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        Err(WarError::Provider(
            "SendInput did not inject all inputs".into(),
        ))
    } else {
        Ok(())
    }
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn absolute_mouse_input(point: Point) -> WarResult<INPUT> {
    validate_screen_point(point)?;
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 1 || height <= 1 {
        return Err(WarError::Provider(
            "virtual desktop has invalid dimensions".into(),
        ));
    }
    let dx = ((point.x - left as f64) * 65_535.0 / (width - 1) as f64)
        .round()
        .clamp(0.0, 65_535.0) as i32;
    let dy = ((point.y - top as f64) * 65_535.0 / (height - 1) as f64)
        .round()
        .clamp(0.0, 65_535.0) as i32;
    Ok(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    })
}

fn button_flags(button: MouseButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }
}

pub fn validate_screen_point(point: Point) -> WarResult<()> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < i32::MIN as f64
        || point.x > i32::MAX as f64
        || point.y < i32::MIN as f64
        || point.y > i32::MAX as f64
    {
        return Err(WarError::InvalidRequest(
            "click coordinates must be finite 32-bit screen positions".into(),
        ));
    }
    Ok(())
}

pub fn click(point: Point, button: MouseButton) -> WarResult<()> {
    validate_screen_point(point)?;
    unsafe { SetCursorPos(point.x.round() as i32, point.y.round() as i32) }
        .map_err(|e| WarError::Provider(e.to_string()))?;
    let (down, up) = button_flags(button);
    send(&[mouse_input(down), mouse_input(up)])
}

struct ButtonReleaseGuard {
    up: MOUSE_EVENT_FLAGS,
    active: bool,
}

impl Drop for ButtonReleaseGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = send(&[mouse_input(self.up)]);
        }
    }
}

/// Injects a continuous pointer path and always releases the pressed button on failure.
pub fn pointer_gesture(
    points: &[Point],
    button: MouseButton,
    duration: std::time::Duration,
) -> WarResult<()> {
    if !(2..=4096).contains(&points.len()) {
        return Err(WarError::InvalidRequest(
            "pointer gesture requires between 2 and 4096 screen points".into(),
        ));
    }
    if duration.is_zero() || duration > std::time::Duration::from_secs(60) {
        return Err(WarError::InvalidRequest(
            "pointer gesture duration must be between 1 ms and 60 seconds".into(),
        ));
    }
    let moves = points
        .iter()
        .copied()
        .map(absolute_mouse_input)
        .collect::<WarResult<Vec<_>>>()?;
    let (down, up) = button_flags(button);
    send(&[moves[0], mouse_input(down)])?;
    let mut release = ButtonReleaseGuard { up, active: true };
    let started = std::time::Instant::now();
    let segments = (moves.len() - 1) as f64;
    for (index, movement) in moves.iter().enumerate().skip(1) {
        let target_elapsed = duration.mul_f64(index as f64 / segments);
        if let Some(pause) = target_elapsed.checked_sub(started.elapsed()) {
            std::thread::sleep(pause);
        }
        send(std::slice::from_ref(movement))?;
    }
    send(&[mouse_input(up)])?;
    release.active = false;
    Ok(())
}

fn key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn virtual_key(key: &Key) -> Option<VIRTUAL_KEY> {
    Some(match key {
        Key::Enter => VK_RETURN,
        Key::Escape => VK_ESCAPE,
        Key::Tab => VK_TAB,
        Key::Space => VK_SPACE,
        Key::Backspace => VK_BACK,
        Key::Delete => VK_DELETE,
        Key::Home => VK_HOME,
        Key::End => VK_END,
        Key::Left => VK_LEFT,
        Key::Right => VK_RIGHT,
        Key::Up => VK_UP,
        Key::Down => VK_DOWN,
        Key::Character(_) => return None,
    })
}

pub fn press_key(key: Key, modifiers: Modifiers) -> WarResult<()> {
    if let Key::Character(c) = key {
        if modifiers == Modifiers::default() {
            return type_text(&c.to_string());
        }
    }
    let vk = virtual_key(&key).ok_or_else(|| {
        WarError::InvalidRequest("modified Unicode character keys are unsupported".into())
    })?;
    let mut inputs = Vec::new();
    for (enabled, modifier) in [
        (modifiers.ctrl, VK_CONTROL),
        (modifiers.alt, VK_MENU),
        (modifiers.shift, VK_SHIFT),
        (modifiers.meta, VK_LWIN),
    ] {
        if enabled {
            inputs.push(key_input(modifier, KEYBD_EVENT_FLAGS(0)));
        }
    }
    inputs.push(key_input(vk, KEYBD_EVENT_FLAGS(0)));
    inputs.push(key_input(vk, KEYEVENTF_KEYUP));
    for (enabled, modifier) in [
        (modifiers.meta, VK_LWIN),
        (modifiers.shift, VK_SHIFT),
        (modifiers.alt, VK_MENU),
        (modifiers.ctrl, VK_CONTROL),
    ] {
        if enabled {
            inputs.push(key_input(modifier, KEYEVENTF_KEYUP));
        }
    }
    send(&inputs)
}

pub fn type_text(text: &str) -> WarResult<()> {
    if text.len() > 64 * 1024 {
        return Err(WarError::InvalidRequest(
            "text input exceeds the 64 KiB limit".into(),
        ));
    }
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    ..Default::default()
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        };
        inputs.extend([down, up]);
    }
    for chunk in inputs.chunks(256) {
        send(chunk)?;
    }
    Ok(())
}

pub fn select_all() -> WarResult<()> {
    press_key(
        Key::Character('a'),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_input_before_touching_global_state() {
        assert!(matches!(
            click(
                Point {
                    x: f64::NAN,
                    y: 0.0
                },
                MouseButton::Left
            ),
            Err(WarError::InvalidRequest(_))
        ));
        assert!(matches!(
            pointer_gesture(
                &[Point { x: 1.0, y: 1.0 }],
                MouseButton::Left,
                std::time::Duration::from_millis(10)
            ),
            Err(WarError::InvalidRequest(_))
        ));
        assert!(matches!(
            type_text(&"x".repeat(64 * 1024 + 1)),
            Err(WarError::InvalidRequest(_))
        ));
    }
}
