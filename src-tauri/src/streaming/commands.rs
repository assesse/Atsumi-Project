use super::model::*;
use crate::interface::{ApiResult, AppState};
use tauri::{AppHandle, Manager};

async fn blocking<T: serde::Serialize + Send + 'static>(
    operation: impl FnOnce() -> Result<T, StreamError> + Send + 'static,
) -> ApiResult<T> {
    match tauri::async_runtime::spawn_blocking(operation).await {
        Ok(result) => result.into(),
        Err(_) => Err::<T, _>(StreamError::new(
            "STREAMING_WORKER_FAILED",
            "방송 작업을 완료하지 못했습니다.",
            true,
        ))
        .into(),
    }
}

/// Installation/relaunch must not race with a new recording start.
#[tauri::command]
pub async fn streaming_update_reserve(app: AppHandle) -> ApiResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        state.official_browser()?.reserve_update()
    })
    .await
}

#[tauri::command]
pub async fn streaming_update_release(app: AppHandle) -> ApiResult<()> {
    blocking(move || {
        if let Ok(browser) = app.state::<AppState>().official_browser() {
            browser.release_update();
        }

        Ok(())
    })
    .await
}
