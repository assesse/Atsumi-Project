//! Trusted-main commands. Native WebView callbacks must never wait on their own UI thread.
use super::multiview::{MultiViewEntry, MultiViewSnapshot};
use super::*;

#[tauri::command]
pub async fn chzzk_multiview_request_control(
    app: AppHandle,
    window: Webview,
    pane_id: String,
    action: ControlAction,
    epoch: u64,
) -> ApiResult<MultiViewSnapshot> {
    tauri::async_runtime::spawn_blocking(move || {
        require_main(&window)?;
        host(&app)?.request_pane_control_from_ui(&app, &pane_id, action, epoch)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Stable frontend IPC argument contract"
)]
pub async fn chzzk_multiview_confirm_control(
    app: AppHandle,
    window: Webview,
    pane_id: String,
    request_id: String,
    approve: bool,
    rights_acknowledged: bool,
    capture_chat: bool,
    epoch: u64,
) -> ApiResult<MultiViewSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err::<MultiViewSnapshot, _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.confirm_pane_control(
            &app,
            &pane_id,
            &request_id,
            approve,
            rights_acknowledged,
            capture_chat,
            epoch,
        )
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_multiview_ack_ui_action(
    app: AppHandle,
    window: Webview,
    pane_id: String,
    id: String,
    epoch: u64,
) -> ApiResult<MultiViewSnapshot> {
    (|| {
        require_main(&window)?;
        host(&app)?.ack_pane_ui_action(&pane_id, &id, epoch)
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_multiview_set_pane_audio(
    app: AppHandle,
    window: Webview,
    pane_id: String,
    enabled: bool,
    epoch: u64,
) -> ApiResult<MultiViewSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err::<MultiViewSnapshot, _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let browser = host(&app)?;
        browser.set_pane_audio(&app, &pane_id, enabled, epoch)?;
        browser.multiview_snapshot()
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}

#[tauri::command]
pub async fn chzzk_multiview_configure(
    app: AppHandle,
    window: Webview,
    entries: Vec<MultiViewEntry>,
) -> ApiResult<MultiViewSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err::<MultiViewSnapshot, _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || host(&app)?.configure_multiview(&app, entries))
        .await
        .unwrap_or_else(|_| Err(unavailable()))
        .into()
}
#[tauri::command]
pub async fn chzzk_multiview_snapshot(
    app: AppHandle,
    window: Webview,
) -> ApiResult<MultiViewSnapshot> {
    (|| {
        require_main(&window)?;
        host(&app)?.multiview_snapshot()
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_multiview_close(
    app: AppHandle,
    window: Webview,
    epoch: u64,
) -> ApiResult<MultiViewSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err::<MultiViewSnapshot, _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let browser = host(&app)?;
        browser.close_multiview(&app, Some(epoch))?;
        browser.multiview_snapshot()
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_multiview_set_audio(
    app: AppHandle,
    window: Webview,
    channel_id: Option<String>,
    epoch: u64,
) -> ApiResult<MultiViewSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err::<MultiViewSnapshot, _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let browser = host(&app)?;
        browser.set_multiview_audio(&app, channel_id, epoch)?;
        browser.multiview_snapshot()
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_multiview_set_viewport(
    app: AppHandle,
    window: Webview,
    pane_id: String,
    viewport: BrowserViewport,
) -> ApiResult<()> {
    if let Err(error) = require_main(&window) {
        return Err::<(), _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.set_multiview_viewport(&app, &pane_id, viewport)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
