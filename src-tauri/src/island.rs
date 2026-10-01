// The island window: a transparent, frameless, always-on-top window centred at
// the top of the primary monitor's work area. The front end tells us how big
// the island is in each mode and the window follows, so it never covers more
// of the screen than the island itself.

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

pub const WINDOW_LABEL: &str = "island";

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

/// Sizes the window to `width` × `height` logical px and centres it on the top
/// edge of the primary monitor's work area (below a top taskbar, if any).
pub fn place(app: &AppHandle, width: f64, height: f64) {
    let Some(win) = window(app) else { return };
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| app.available_monitors().ok().and_then(|m| m.into_iter().next()));
    let Some(m) = monitor else { return };

    let scale = m.scale_factor();
    let area = m.work_area();
    let pw = (width * scale).round().max(1.0) as u32;
    let ph = (height * scale).round().max(1.0) as u32;
    let x = area.position.x + (area.size.width as i32 - pw as i32) / 2;
    let y = area.position.y;

    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    // Moving onto a monitor with another scale can rescale the window: re-assert.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
}

/// Keeps the island out of Alt-Tab. (skipTaskbar already keeps it off the taskbar.)
pub fn hide_from_alt_tab(win: &WebviewWindow) {
    let Ok(handle) = win.hwnd() else { return };
    let hwnd = HWND(handle.0 as *mut _);
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}
