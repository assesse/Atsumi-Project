//! Trusted desktop facade; renderer supplies provider IDs, never paths, cookies or tool arguments.
use crate::{interface::commands::AppState, platform_media::*};
use tauri::Manager;
use std::sync::{Arc, OnceLock};

fn replay_chat_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2))).clone()
        .try_acquire_owned().map_err(|_| "다른 채팅 기록을 읽는 중입니다. 잠시 후 다시 시도해 주세요.".into())
}
fn chat_asset_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4))).clone().try_acquire_owned()
        .map_err(|_| "이모티콘 이미지를 읽는 중입니다.".into())
}
fn live_profile_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4))).clone().try_acquire_owned()
        .map_err(|_| "프로필 이미지를 읽는 중입니다.".into())
}
fn current_profile_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2))).clone().try_acquire_owned()
        .map_err(|_| "채널 사진을 읽는 중입니다. 잠시 후 다시 시도해 주세요.".into())
}
fn library_group_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2))).clone().try_acquire_owned()
        .map_err(|_| "녹화 목록을 읽는 중입니다. 잠시 후 다시 시도해 주세요.".into())
}
fn screenshot_read_slot() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2))).clone().try_acquire_owned()
        .map_err(|_| "스크린샷을 읽는 중입니다. 잠시 후 다시 시도해 주세요.".into())
}

fn trusted(window: &tauri::Webview) -> Result<tauri::AppHandle, String> {
    let url = window.url().map_err(|_| "앱 창을 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(window.label(), &url, cfg!(debug_assertions), window.app_handle().config().build.dev_url.as_ref()) {
        return Err("Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(window.app_handle().clone())
}
#[tauri::command]
pub async fn soop_auto_record_snapshot(window: tauri::Webview) -> Result<crate::soop::auto_record::AutoSnapshot, String> {
    let app = trusted(&window)?;
    let state = app.try_state::<AppState>().ok_or("앱을 준비 중입니다.")?;
    Ok(state.soop_auto_recorder().snapshot())
}
#[tauri::command]
pub async fn soop_auto_record_set(window: tauri::Webview, channel_id: String, name: String, enabled: bool, expected_revision: u64) -> Result<crate::soop::auto_record::AutoSnapshot, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.try_state::<AppState>().ok_or("앱을 준비 중입니다.")?;
        state.begin_platform_work(|| state.soop_auto_recorder().set(channel_id, name, enabled, expected_revision))
    }).await.map_err(|_| "녹화 예약을 변경하지 못했습니다.".to_string())?
}
async fn run<T: Send + 'static>(window: tauri::Webview, action: impl FnOnce(&AppState, PlatformMediaService) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.try_state::<AppState>().ok_or("앱을 준비 중입니다.")?;
        let service = state.platform_media(&app)?;
        action(&state, service)
    }).await.map_err(|_| "영상 작업을 완료하지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn platform_media_download(window: tauri::Webview, reference: MediaRef, request_id: String) -> Result<MediaJob,String> {
    run(window, move |state, service| {
        let settings = state.settings_snapshot().map_err(|_| "저장 폴더 설정을 읽지 못했습니다.")?;
        if settings.download_root.trim().is_empty() { return Err("설정에서 다운로드 폴더를 먼저 선택하세요.".into()); }
        state.begin_platform_work(|| service.download(reference,request_id,settings.download_root.into()))
    }).await
}
#[tauri::command]
pub async fn platform_media_live_open(window: tauri::Webview, reference: MediaRef, request_id: String) -> Result<live::LiveSnapshot,String> {
    run(window, move |state, service| {
        let settings = state.settings_snapshot().map_err(|_| "저장 폴더 설정을 읽지 못했습니다.")?;
        if settings.download_root.trim().is_empty() { return Err("설정에서 다운로드 폴더를 먼저 선택하세요.".into()); }
        state.begin_platform_work(|| service.live_open(reference, request_id, settings.download_root.into()))
    }).await
}
#[tauri::command]
pub async fn platform_media_live_close(window: tauri::Webview, token: String) -> Result<(),String> {
    run(window, move |_, service| service.live_close(&token)).await
}
#[tauri::command]
pub async fn platform_media_live_snapshot(window: tauri::Webview, token: String) -> Result<live::LiveSnapshot,String> {
    run(window, move |_, service| service.live_snapshot(&token)).await
}
#[tauri::command]
pub async fn platform_media_live_profile(window: tauri::Webview, token: String) -> Result<Option<String>,String> {
    let app = trusted(&window)?;
    let permit = live_profile_slot()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let state = app.try_state::<AppState>().ok_or("앱을 준비 중입니다.")?;
        // A valid viewer already owns an initialized service. Unknown tokens
        // must not trigger journal recovery, filesystem writes or worker startup.
        state.existing_platform_media().ok_or("라이브 세션이 없습니다.")?.live_profile(&token)
    }).await.map_err(|_| "프로필 이미지를 읽지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn platform_media_live_chat(window: tauri::Webview, token: String, after: u64, limit: usize) -> Result<live_chat::LiveChatPage,String> {
    run(window, move |_, service| service.live_chat(&token, after, limit)).await
}
#[tauri::command]
pub async fn platform_media_live_chat_asset(window: tauri::Webview, token: String, asset_id: String) -> Result<live_chat::ChatAssetRead,String> {
    trusted(&window)?;
    let permit = chat_asset_slot()?;
    run(window, move |_, service| { let _permit = permit; service.live_chat_asset(&token, &asset_id) }).await
}
#[tauri::command]
pub async fn platform_media_replay_chat_asset(window: tauri::Webview, library_id: String, asset_id: String) -> Result<live_chat::ChatAssetRead,String> {
    trusted(&window)?;
    let permit = chat_asset_slot()?;
    run(window, move |_, service| { let _permit = permit; service.replay_chat_asset(&library_id, &asset_id) }).await
}
#[tauri::command]
pub async fn platform_media_live_record(window: tauri::Webview, token: String, enabled: bool) -> Result<live::LiveSnapshot,String> {
    run(window, move |state, service| state.begin_platform_work(|| service.live_record(&token, enabled))).await
}
#[tauri::command]
pub async fn platform_media_download_batch(window: tauri::Webview, references: Vec<MediaRef>, request_id: String) -> Result<Vec<MediaJob>,String> {
    run(window, move |state, service| {
        let settings = state.settings_snapshot().map_err(|_| "저장 폴더 설정을 읽지 못했습니다.")?;
        if settings.download_root.trim().is_empty() { return Err("설정에서 다운로드 폴더를 먼저 선택하세요.".into()); }
        state.begin_platform_work(|| service.download_batch(references, request_id, settings.download_root.into()))
    }).await
}
#[tauri::command]
pub async fn platform_media_jobs(window: tauri::Webview, provider: String) -> Result<Vec<MediaJob>,String> {
    run(window, move |_, service| {
        if !matches!(provider.as_str(),"youtube"|"soop") { return Err("서비스를 확인해 주세요.".into()); }
        Ok(service.jobs(Some(&provider)))
    }).await
}
#[tauri::command]
pub async fn platform_media_cancel(window: tauri::Webview, job_id: String, revision: u64) -> Result<MediaJob,String> {
    run(window, move |_, service| service.cancel(&job_id,revision)).await
}
#[tauri::command]
pub async fn platform_media_retry(window: tauri::Webview, job_id: String, revision: u64) -> Result<MediaJob,String> {
    run(window, move |state, service| state.begin_platform_work(|| service.retry(&job_id,revision))).await
}
#[tauri::command]
pub async fn platform_media_library(window: tauri::Webview, provider: String, cursor: Option<String>, limit: usize) -> Result<LibraryPage,String> {
    run(window, move |_, service| service.library(&provider,cursor.as_deref(),limit)).await
}
#[tauri::command]
pub async fn platform_media_library_groups(window: tauri::Webview, provider: String, cursor: Option<String>, limit: usize) -> Result<groups::GroupsPage,String> {
    trusted(&window)?;
    let permit = library_group_slot()?;
    run(window, move |_, service| { let _permit = permit; service.library_groups(&provider, cursor.as_deref(), limit) }).await
}
#[tauri::command]
pub async fn platform_media_library_group_members(window: tauri::Webview, provider: String, group_key: String, cursor: Option<String>, limit: usize) -> Result<groups::MembersPage,String> {
    trusted(&window)?;
    let permit = library_group_slot()?;
    run(window, move |_, service| { let _permit = permit; service.library_group_members(&provider, &group_key, cursor.as_deref(), limit) }).await
}
#[tauri::command]
pub async fn platform_media_profile(window: tauri::Webview, library_id: String, current: Option<bool>) -> Result<Option<String>,String> {
    trusted(&window)?;
    if current == Some(true) {
        let permit = current_profile_slot()?;
        run(window,move |state,service| { let _permit = permit; state.begin_platform_work(|| service.current_profile(&library_id)) }).await
    } else { run(window,move |_,service|service.profile(&library_id)).await }
}
#[tauri::command]
pub async fn platform_media_thumbnail(window: tauri::Webview, library_id: String) -> Result<Option<String>,String> {
    run(window, move |_, service| service.thumbnail(&library_id)).await
}
#[tauri::command]
pub async fn platform_media_replay_chat(window: tauri::Webview, library_id: String, position_ms: u64, cursor: Option<String>, limit: usize) -> Result<replay_chat::ReplayChatPage,String> {
    // Validate before taking a slot; keep the permit inside the blocking closure,
    // so a dropped IPC future cannot release a still-running index operation.
    trusted(&window)?;
    let permit = replay_chat_slot()?;
    run(window, move |_, service| {
        let _permit = permit;
        service.replay_chat(&library_id, position_ms, cursor.as_deref(), limit)
    }).await
}
#[tauri::command]
pub async fn platform_media_open(window: tauri::Webview, library_id: String) -> Result<Playback,String> {
    run(window, move |state, service| state.begin_platform_work(|| service.open(&library_id))).await
}
#[tauri::command]
pub async fn platform_media_close(window: tauri::Webview, token: String) -> Result<(),String> {
    run(window, move |_, service| {service.close(&token);Ok(())}).await
}
#[tauri::command]
pub async fn platform_media_open_folder(window: tauri::Webview, library_id: String) -> Result<(),String> {
    run(window, move |_, service| service.open_folder(&library_id)).await
}
#[tauri::command]
pub async fn platform_media_delete(window: tauri::Webview, library_id: String, revision: u64) -> Result<(),String> {
    run(window, move |state, service| state.begin_platform_work(|| service.delete(&library_id,revision))).await
}

#[tauri::command]
pub async fn platform_media_screenshot_begin(window: tauri::Webview, token: String, request_id: String, size: usize, width: u32, height: u32) -> Result<screenshots::Begin, String> {
    run(window, move |state, service| {
        let _work = state.begin_platform_work(|| service.screenshot_work())?;
        service.screenshot_begin(&token, &request_id, size, width, height)
    }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_chunk(window: tauri::Webview, token: String, capture_id: String, index: usize, data: String) -> Result<(), String> {
    run(window, move |state, service| {
        let _work = state.begin_platform_work(|| service.screenshot_work())?;
        service.screenshot_chunk(&token, &capture_id, index, &data)
    }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_finish(window: tauri::Webview, token: String, capture_id: String) -> Result<screenshots::Item, String> {
    run(window, move |state, service| {
        let _work = state.begin_platform_work(|| service.screenshot_work())?;
        service.screenshot_finish(&token, &capture_id)
    }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_abort(window: tauri::Webview, token: String, capture_id: String) -> Result<(), String> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.try_state::<AppState>().ok_or("앱을 준비 중입니다.")?;
        match state.existing_platform_media() {
            Some(service) => service.screenshot_abort(&token, &capture_id),
            None => Ok(()),
        }
    }).await.map_err(|_| "스크린샷 전송을 중단하지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn platform_media_screenshot_undo(window: tauri::Webview, token: String, id: String) -> Result<(), String> {
    run(window, move |state, service| {
        let _work = state.begin_platform_work(|| service.screenshot_work())?;
        service.screenshot_undo(&token, &id)
    }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_list(window: tauri::Webview, provider: String, cursor: Option<String>, limit: usize) -> Result<screenshots::Page, String> {
    trusted(&window)?;
    let permit = screenshot_read_slot()?;
    run(window, move |_, service| { let _permit = permit; service.screenshot_list(&provider, cursor.as_deref(), limit) }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_image(window: tauri::Webview, provider: String, id: String, full: bool) -> Result<String, String> {
    trusted(&window)?;
    let permit = screenshot_read_slot()?;
    run(window, move |_, service| { let _permit = permit; service.screenshot_image(&provider, &id, full) }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_delete(window: tauri::Webview, provider: String, ids: Vec<String>) -> Result<screenshots::DeleteResult, String> {
    run(window, move |state, service| {
        let _work = state.begin_platform_work(|| service.screenshot_work())?;
        service.screenshot_delete(&provider, &ids)
    }).await
}
#[tauri::command]
pub async fn platform_media_screenshot_folder(window: tauri::Webview, provider: String, id: String) -> Result<(), String> {
    run(window, move |_, service| {
        let path = service.screenshot_folder(&provider, &id)?;
        #[cfg(windows)] {
            use std::os::windows::ffi::OsStrExt;
            use windows::{core::{w, PCWSTR}, Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL}};
            let text: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe { ShellExecuteW(None, w!("open"), PCWSTR(text.as_ptr()), None, None, SW_SHOWNORMAL) }.0 as isize <= 32 {
                return Err("저장 폴더를 열지 못했습니다.".into());
            }
        }
        #[cfg(not(windows))] { let _ = path; return Err("이 환경에서는 저장 폴더를 열 수 없습니다.".into()); }
        #[cfg(windows)] { Ok(()) }
    }).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn screenshot_read_work_is_bounded_and_released_on_drop() {
        let first = super::screenshot_read_slot().unwrap();
        let second = super::screenshot_read_slot().unwrap();
        assert!(super::screenshot_read_slot().is_err());
        drop(first);
        let next = super::screenshot_read_slot().unwrap();
        drop((second, next));
    }
    #[test]
    fn library_group_work_is_bounded_and_released_on_drop() {
        let first = super::library_group_slot().unwrap();
        let second = super::library_group_slot().unwrap();
        assert!(super::library_group_slot().is_err());
        drop(first);
        let next = super::library_group_slot().unwrap();
        drop((second, next));
    }
    #[test]
    fn live_profile_work_is_bounded_and_released_on_drop() {
        let permits = (0..4).map(|_| super::live_profile_slot().unwrap()).collect::<Vec<_>>();
        assert!(super::live_profile_slot().is_err());
        drop(permits);
        assert!(super::live_profile_slot().is_ok());
    }
    #[test]
    fn replay_chat_work_is_bounded_and_released_on_drop() {
        let first = super::replay_chat_slot().unwrap();
        let second = super::replay_chat_slot().unwrap();
        assert!(super::replay_chat_slot().is_err());
        drop(first);
        let next = super::replay_chat_slot().unwrap();
        drop((second, next));
    }
}
