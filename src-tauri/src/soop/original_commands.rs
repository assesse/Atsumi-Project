//! Only the trusted Atsumi main renderer may operate SOOP native receivers.
use super::original::{AccountSnapshot, OriginalSnapshot, ScreenshotSnapshot, SoopOriginalHost, VodSnapshot};
use crate::{interface::commands::AppState, platform_native_geometry::NativeViewport};
use tauri::{AppHandle, Manager, Webview};
fn trusted(view: &Webview) -> Result<AppHandle, String> {
    let url = view.url().map_err(|_| "앱 창을 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(
        view.label(),
        &url,
        cfg!(debug_assertions),
        view.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(view.app_handle().clone())
}
#[tauri::command]
pub async fn soop_original_open(
    window: Webview,
    channel_id: String,
    request_id: String,
) -> Result<OriginalSnapshot, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state
            .settings_snapshot()
            .map_err(|_| "저장 폴더 설정을 읽지 못했습니다.")?;
        if settings.download_root.trim().is_empty() {
            return Err("설정에서 다운로드 폴더를 선택해 주세요.".into());
        }
        let media = state.platform_media(&app)?;
        state.begin_platform_work(|| {
            app.state::<SoopOriginalHost>().open(
                &app,
                media,
                channel_id,
                request_id,
                settings.download_root.into(),
            )
        })
    })
    .await
    .map_err(|_| "SOOP 방송을 열지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn soop_original_snapshot(
    window: Webview,
    token: String,
) -> Result<OriginalSnapshot, String> {
    let app = trusted(&window)?;
    app.state::<SoopOriginalHost>().snapshot(&token)
}
#[tauri::command]
pub async fn soop_original_viewport(
    window: Webview,
    token: String,
    viewport: NativeViewport,
) -> Result<(), String> {
    let app = trusted(&window)?;
    app.state::<SoopOriginalHost>()
        .viewport(&app, &token, viewport)
}
#[tauri::command]
pub async fn soop_original_close(window: Webview, token: String) -> Result<(), String> {
    let app = trusted(&window)?;
    app.state::<SoopOriginalHost>().close(&app, &token)
}
#[tauri::command]
pub async fn soop_original_controls(window:Webview,token:String,can_close:bool,capture_busy:bool)->Result<(),String>{
    let app=trusted(&window)?;app.state::<SoopOriginalHost>().controls(&app,&token,can_close,capture_busy)
}
#[tauri::command]
pub async fn soop_original_resume(window:Webview,token:String)->Result<(),String>{
    let app=trusted(&window)?;app.state::<SoopOriginalHost>().resume(&app,&token)
}
#[tauri::command]
pub async fn soop_original_record(
    window: Webview,
    token: String,
    enabled: bool,
) -> Result<OriginalSnapshot, String> {
    let app = trusted(&window)?;
    app.state::<AppState>().begin_platform_work(|| {
        app.state::<SoopOriginalHost>()
            .record(&app, &token, enabled)
    })
}
#[tauri::command]
pub async fn soop_original_login(window: Webview) -> Result<AccountSnapshot, String> {
    let app = trusted(&window)?;
    app.state::<SoopOriginalHost>().login(&app)
}
#[tauri::command]
pub async fn soop_original_logout(window: Webview) -> Result<AccountSnapshot, String> {
    let app = trusted(&window)?;
    app.state::<SoopOriginalHost>().logout(&app)
}
#[tauri::command]
pub async fn soop_original_account(window: Webview) -> Result<AccountSnapshot, String> {
    let app = trusted(&window)?;
    Ok(app.state::<SoopOriginalHost>().account(&app))
}
#[tauri::command]
pub async fn soop_original_screenshot(
    window: Webview,
    token: String,
    request_id: String,
) -> Result<ScreenshotSnapshot, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<SoopOriginalHost>()
            .screenshot(&app, &token, &request_id)
    })
    .await
    .map_err(|_| "스크린샷을 저장하지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn soop_original_screenshot_undo(
    window: Webview,
    token: String,
    id: String,
) -> Result<(), String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<SoopOriginalHost>().undo_screenshot(&token, &id)
    })
    .await
    .map_err(|_| "스크린샷 저장 취소를 확인하지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn soop_original_fullscreen(
    window: Webview,
    token: String,
    enabled: bool,
) -> Result<(), String> {
    let app = trusted(&window)?;
    app.state::<SoopOriginalHost>()
        .fullscreen(&app, &token, enabled)
}

/// Watch-only original VOD needs neither a download folder nor a media receiver lease.
#[tauri::command]
pub async fn soop_vod_open(window: Webview, video_id: String, request_id: String) -> Result<VodSnapshot, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>().begin_platform_work(|| app.state::<SoopOriginalHost>().vod_open(&app, video_id, request_id))
    }).await.map_err(|_| "SOOP 영상을 열지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn soop_vod_snapshot(window: Webview, token: String) -> Result<VodSnapshot, String> {
    let app = trusted(&window)?; app.state::<SoopOriginalHost>().vod_snapshot(&token)
}
#[tauri::command]
pub async fn soop_vod_viewport(window: Webview, token: String, viewport: NativeViewport) -> Result<(), String> {
    let app = trusted(&window)?; app.state::<SoopOriginalHost>().vod_viewport(&app, &token, viewport)
}
#[tauri::command]
pub async fn soop_vod_close(window: Webview, token: String) -> Result<(), String> {
    let app = trusted(&window)?; app.state::<SoopOriginalHost>().vod_close(&app, &token)
}
#[tauri::command]
pub async fn soop_vod_fullscreen(window: Webview, token: String, enabled: bool) -> Result<(), String> {
    let app = trusted(&window)?; app.state::<SoopOriginalHost>().vod_fullscreen(&app, &token, enabled)
}
