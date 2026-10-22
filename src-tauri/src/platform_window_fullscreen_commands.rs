//! Presentation-only commands for the trusted main renderer; no media authority.
use crate::platform_window_fullscreen::{self as fullscreen, Grant};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Webview};

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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquireOutcome {
    grant: Option<Grant>,
    error: Option<String>,
}

#[tauri::command]
pub async fn platform_window_fullscreen_acquire(
    window: Webview,
    owner: String,
    document_epoch: u64,
) -> Result<AcquireOutcome, String> {
    let app = trusted(&window)?;
    // Frontend owners are independently minted UUIDs, never provider/channel identifiers.
    if uuid::Uuid::parse_str(&owner).is_err() || document_epoch == 0 {
        return Err("전체화면 요청이 변경되었습니다.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let valid_app = app.clone();
        let lost_app = app.clone();
        match fullscreen::acquire(
            &app,
            owner,
            document_epoch,
            Arc::new(move || valid_app.get_window("main").is_some()),
            Arc::new(|_| {}),
            Arc::new(move |grant| {
                let _ = lost_app.emit_to("main", "platform-window-fullscreen:lost", grant);
            }),
        ) {
            Ok(grant) => AcquireOutcome {
                grant: Some(grant),
                error: None,
            },
            Err(error) => AcquireOutcome {
                grant: error.retained_grant,
                error: Some(error.message),
            },
        }
    })
    .await
    .map_err(|_| "전체화면을 전환하지 못했습니다.".into())
}

#[tauri::command]
pub async fn platform_window_fullscreen_release(
    window: Webview,
    grant: Grant,
) -> Result<bool, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || fullscreen::release(&app, grant))
        .await
        .map_err(|_| "전체화면을 종료하지 못했습니다.".to_string())?
}

#[tauri::command]
pub async fn platform_window_fullscreen_current(
    window: Webview,
    grant: Grant,
) -> Result<bool, String> {
    let app = trusted(&window)?;
    Ok(fullscreen::is_current(&app, &grant))
}
