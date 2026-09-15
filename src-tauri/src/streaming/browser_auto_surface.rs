//! A background receiver is off-canvas, not a hidden renderer. WebView2's
//! IsVisible=false suspends rAF/ResizeObserver and can deadlock player startup.
//! Bounds are relative to the parent's client area, so this surface is clipped
//! out on every monitor, including when the parent moves/minimizes/trays.
pub const LEFT: f64 = -2560.0;

pub fn prepare(view: &tauri::Webview) -> tauri::Result<()> {
    // No flash at (0, 0), no focus call, no visible OS window. Native muting is
    // established by the caller before navigation to an actual channel.
    view.hide()?;
    view.set_auto_resize(false)?;
    view.set_position(tauri::LogicalPosition::new(LEFT, 0.0))?;
    view.set_size(tauri::LogicalSize::new(1280.0, 720.0))?;
    view.show()
}
