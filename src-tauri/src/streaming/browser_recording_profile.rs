//! Local archived identity; opening a channel is an explicit trusted-UI action.
use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingProfile {
    name: Option<String>,
    image: Option<String>,
}

fn recording(app: &AppHandle, id: &str) -> Result<BrowserRecording, StreamError> {
    host(app)?
        .inner
        .store
        .lock()
        .map_err(|_| unavailable())?
        .snapshot()?
        .into_iter()
        .find(|r| r.id == id && !r.deletion_pending)
        .ok_or_else(unavailable)
}
#[tauri::command]
pub async fn chzzk_recording_profile(
    app: AppHandle,
    window: Webview,
    recording_id: String,
) -> ApiResult<RecordingProfile> {
    if let Err(error) = require_main(&window) {
        return Err::<RecordingProfile, _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let r = recording(&app, &recording_id)?;
        let (name, image) = super::super::replay_assets::channel_profile::read(
            Path::new(&r.output_dir),
            &r.channel_id,
        );
        Ok(RecordingProfile { name, image })
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_recording_open_channel(
    app: AppHandle,
    window: Webview,
    recording_id: String,
) -> ApiResult<()> {
    if let Err(error) = require_main(&window) {
        return Err::<(), _>(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let r = recording(&app, &recording_id)?;
        if r.channel_id.len() != 32 || !r.channel_id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(unavailable());
        }
        let url = format!("https://chzzk.naver.com/{}", r.channel_id);
        #[cfg(windows)]
        {
            use windows::{
                core::PCWSTR,
                Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            };
            let wide = url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let result = unsafe {
                ShellExecuteW(
                    None,
                    windows::core::w!("open"),
                    PCWSTR(wide.as_ptr()),
                    None,
                    None,
                    SW_SHOWNORMAL,
                )
            };
            if result.0 as isize <= 32 {
                return Err(unavailable());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = url;
            Err(unavailable())
        }
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
