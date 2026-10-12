//! Small Win32 helpers shared by the window code.

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowTextLengthW, GetWindowTextW, MB_ICONERROR, MB_OK, MessageBoxW, PostMessageW,
    SendMessageW, SetWindowTextW,
};

/// NUL-terminated UTF-16 copy of `s`, as Win32 expects for string arguments.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Current text of a window or control.
pub fn text_of(hwnd: HWND) -> String {
    // SAFETY: `hwnd` is a window of this process. The call takes no pointers.
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    // SAFETY: `buf` has room for `buf.len()` units, including the terminating NUL.
    let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..copied.max(0) as usize])
}

pub fn set_text(hwnd: HWND, text: &str) {
    let wide_text = wide(text);
    // SAFETY: `wide_text` is NUL-terminated and outlives the call.
    unsafe { SetWindowTextW(hwnd, wide_text.as_ptr()) };
}

/// Scales a 96-DPI pixel value to `dpi`, rounding to the nearest pixel.
pub fn scale(px: i32, dpi: u32) -> i32 {
    ((i64::from(px) * i64::from(dpi) + 48) / 96) as i32
}

/// Modal error box. `hwnd` may be null before the main window exists.
pub fn message(hwnd: HWND, title: &str, text: &str) {
    let (wide_text, wide_title) = (wide(text), wide(title));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            hwnd,
            wide_text.as_ptr(),
            wide_title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

pub fn enable(hwnd: HWND, on: bool) {
    // SAFETY: `hwnd` is a window of this process. The call takes no pointers.
    unsafe { EnableWindow(hwnd, i32::from(on)) };
}

/// Asynchronous PostMessageW. Carries no pointers, so a message can cross threads.
pub fn post(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    // SAFETY: the arguments are plain integers.
    unsafe { PostMessageW(hwnd, msg, wparam, lparam) };
}

/// Synchronous SendMessageW. Any pointer carried in `lparam` must stay valid for the call.
pub fn send(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: callers pass integers or pointers to live locals (see the doc comment).
    unsafe { SendMessageW(hwnd, msg, wparam, lparam) }
}
