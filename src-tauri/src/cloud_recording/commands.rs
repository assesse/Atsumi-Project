//! Trusted desktop commands. Native dialogs and catalogs own every filesystem path.
use super::{CloudError, CloudSnapshot, CloudSourceRef};
use crate::interface::commands::AppState;
use tauri::Manager;

#[path = "live_replay_commands.rs"]
pub mod live_replay;

fn error(code: &str, message: impl Into<String>) -> CloudError {
    CloudError { code: code.into(), message: message.into(), retryable: false }
}
fn trusted(window: &tauri::Webview) -> Result<tauri::AppHandle, CloudError> {
    let url = window.url().map_err(|_| error("APP_WINDOW", "앱 창을 확인하지 못했습니다."))?;
    if !crate::frontend_origin::trusted_main(window.label(), &url, cfg!(debug_assertions), window.app_handle().config().build.dev_url.as_ref()) {
        return Err(error("APP_WINDOW", "Atsumi 기본 창에서만 사용할 수 있습니다."));
    }
    Ok(window.app_handle().clone())
}
async fn run<T: Send + 'static>(
    window: tauri::Webview,
    mutate: bool,
    action: impl FnOnce(&AppState, &tauri::AppHandle) -> Result<T, CloudError> + Send + 'static,
) -> Result<T, CloudError> {
    let app = trusted(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.try_state::<AppState>().ok_or_else(|| error("APP_STARTING", "앱을 준비 중입니다."))?;
        let _permit = if mutate { Some(state.begin_cloud_work().map_err(|message| error("APP_QUITTING", message))?) } else { None };
        action(&state, &app)
    }).await.map_err(|_| error("CLOUD_WORKER", "클라우드 작업을 완료하지 못했습니다."))?
}
fn service(state: &AppState, app: &tauri::AppHandle) -> Result<std::sync::Arc<super::CloudService>, CloudError> {
    state.cloud_recording(app).map_err(|message| error("CLOUD_UNAVAILABLE", message))
}

fn live_source_error() -> CloudError {
    error("CLOUD_LIVE_SOURCE", "현재 시청 중인 방송을 선택해 주세요. 방송이 변경되었다면 다시 선택해 주세요.")
}
#[tauri::command]
pub async fn cloud_live_snapshot(window: tauri::Webview) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, false, |state, app| service(state, app)?.live_snapshot()).await
}
#[tauri::command]
pub async fn cloud_live_sources(window: tauri::Webview, provider: String) -> Result<Vec<String>, CloudError> {
    run(window, false, move |state, app| match provider.as_str() {
        "soop" => Ok(app.try_state::<crate::soop::original::SoopOriginalHost>().map(|host| host.relay_available_channels()).unwrap_or_default()),
        "chzzk" => Ok(state.deferred_browser().get().map(|browser| browser.relay_available_channels()).unwrap_or_default()),
        _ => Err(live_source_error()),
    }).await
}
#[tauri::command]
pub async fn cloud_live_authorize(window: tauri::Webview) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, true, |state, app| service(state, app)?.authorize_live()).await
}
#[tauri::command]
pub async fn cloud_live_disconnect(window: tauri::Webview) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, true, |state, app| service(state, app)?.disconnect_live()).await
}
#[tauri::command]
pub async fn cloud_live_select(window: tauri::Webview, provider: String, channel_id: String) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, true, move |state, app| {
        if channel_id.is_empty() || channel_id.len() > 128 { return Err(live_source_error()); }
        let source = match provider.as_str() {
            "soop" => super::live_relay::NativeRelaySource::Soop(app.try_state::<crate::soop::original::SoopOriginalHost>()
                .ok_or_else(live_source_error)?.select_relay_source_by_channel(&channel_id).map_err(|_| live_source_error())?),
            "chzzk" => super::live_relay::NativeRelaySource::Chzzk(state.deferred_browser().get().map_err(|_| live_source_error())?
                .select_relay_source_by_channel(&channel_id).map_err(|_| live_source_error())?),
            _ => return Err(live_source_error()),
        };
        service(state, app)?.select_live(app, source)
    }).await
}
#[tauri::command]
pub async fn cloud_live_start(window: tauri::Webview, selection_id: String, expected_revision: u64, plan: super::model::CloudLivePlan) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, true, move |state, app| service(state, app)?.start_live(app.clone(), &selection_id, expected_revision, plan)).await
}
#[tauri::command]
pub async fn cloud_live_stop(window: tauri::Webview, selection_id: String, expected_revision: u64) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, true, move |state, app| service(state, app)?.stop_live(&selection_id, expected_revision)).await
}
#[tauri::command]
pub async fn cloud_live_verify_archive(window: tauri::Webview, selection_id: String, expected_revision: u64) -> Result<super::model::CloudLiveSnapshot, CloudError> {
    run(window, true, move |state, app| service(state, app)?.verify_live_archive(&selection_id, expected_revision)).await
}

#[tauri::command]
pub async fn cloud_recording_snapshot(window: tauri::Webview) -> Result<CloudSnapshot, CloudError> {
    run(window, false, |state, app| service(state, app)?.snapshot()).await
}
#[tauri::command]
pub async fn cloud_recording_automatic_set(window: tauri::Webview, target: super::CloudAutomaticTarget, enabled: bool, expected_revision: u64, visibility: Option<super::model::CloudVisibility>) -> Result<CloudSnapshot, CloudError> {
    run(window, true, move |state, app| service(state, app)?.automatic_set_with_visibility(target,enabled,expected_revision,visibility)).await
}
#[tauri::command]
pub async fn cloud_recording_describe(window: tauri::Webview, source: CloudSourceRef) -> Result<super::source::SourceDescription, CloudError> {
    run(window, false, move |state, app| {
        let platform = state.platform_media(app).map_err(|message| error("SOURCE_UNAVAILABLE", message))?;
        super::source::describe(&state.deferred_browser(), &platform, state.cloud_recording_data_dir(), &source).map_err(|message| error("SOURCE_UNAVAILABLE", message))
    }).await
}
#[tauri::command]
pub async fn cloud_recording_get_plan(window: tauri::Webview, plan_id: String) -> Result<super::source::SourceDescription, CloudError> {
    run(window, false, move |state, _| super::source::get_plan(state.cloud_recording_data_dir(), &plan_id).map_err(|message| error("SOURCE_PLAN", message))).await
}

fn chat_unavailable() -> CloudError {
    error("CLOUD_CHAT_UNAVAILABLE", "저장된 채팅·후원 기록을 열지 못했습니다. 기록의 보관 상태를 확인해 주세요.")
}
fn replay_authorization(state: &AppState, app: &tauri::AppHandle, job_id: &str) -> Result<super::replay::CloudReplayAuthorization, CloudError> {
    service(state, app)?.replay_authorization(state.cloud_recording_data_dir(), job_id)
}
fn platform_chat(state: &AppState, app: &tauri::AppHandle) -> Result<crate::platform_media::PlatformMediaService, CloudError> {
    state.platform_media(app).map_err(|_| chat_unavailable())
}
fn complete_platform_description(authorization: &mut super::replay::CloudReplayAuthorization) {
    // The caller has just verified completed catalog ownership and exact original
    // identity. A legacy whole platform file therefore has no inferred tail.
    if authorization.source.plan_id.is_none() {
        authorization.description.coverage.state = "complete".into();
        authorization.description.coverage.reason = None;
    }
}
#[tauri::command]
pub async fn cloud_recording_replay_describe(window: tauri::Webview, job_id: String) -> Result<super::source::SourceDescription, CloudError> {
    run(window, false, move |state, app| {
        let mut authorization = replay_authorization(state, app, &job_id)?;
        if authorization.source.provider != "chzzk" {
            let platform = platform_chat(state, app)?;
            platform.validate_cloud_chat_source(&authorization).map_err(|_| chat_unavailable())?;
            authorization.description.coverage.capture_warning = platform.cloud_capture_warning(&authorization).map_err(|_| chat_unavailable())?;
            complete_platform_description(&mut authorization);
        }
        Ok(authorization.description)
    }).await
}
#[tauri::command]
pub async fn cloud_recording_replay_open(window: tauri::Webview, job_id: String) -> Result<super::replay::CloudReplayOpen, CloudError> {
    run(window, false, move |state, app| {
        let mut authorization = replay_authorization(state, app, &job_id)?;
        let (token, session) = if authorization.source.provider == "chzzk" {
            let replay = app.try_state::<crate::streaming::replay::ReplayService>().ok_or_else(chat_unavailable)?;
            let session = replay.open_cloud_chat(&authorization).map_err(|_| chat_unavailable())?;
            (session.token.clone(), Some(session))
        } else {
            let platform = platform_chat(state, app)?;
            authorization.description.coverage.capture_warning = platform.cloud_capture_warning(&authorization).map_err(|_| chat_unavailable())?;
            let token = platform.open_cloud_chat(&authorization).map_err(|_| chat_unavailable())?;
            complete_platform_description(&mut authorization);
            (token, None)
        };
        Ok(super::replay::CloudReplayOpen { provider: authorization.source.provider,
            token, cohort_id: authorization.cohort_id, description: authorization.description, session })
    }).await
}
#[tauri::command]
pub async fn cloud_recording_replay_close(window: tauri::Webview, provider: String, token: String) -> Result<(), CloudError> {
    run(window, false, move |state, app| {
        match provider.as_str() {
            "chzzk" => app.try_state::<crate::streaming::replay::ReplayService>().ok_or_else(chat_unavailable)?
                .close(&token).map_err(|_| chat_unavailable()),
            "soop" | "youtube" => { platform_chat(state, app)?.close_cloud_chat(&token); Ok(()) },
            _ => Err(chat_unavailable()),
        }
    }).await
}
#[tauri::command]
pub async fn cloud_recording_replay_page(window: tauri::Webview, token: String, position_ms: u64, cursor: Option<String>, limit: usize)
    -> Result<crate::platform_media::replay_chat::ReplayChatPage, CloudError> {
    trusted(&window)?;
    let permit = crate::platform_media_ipc::replay_chat_slot().map_err(|_| error("CLOUD_CHAT_BUSY", "다른 채팅 기록을 읽는 중입니다. 잠시 후 다시 시도해 주세요."))?;
    run(window, false, move |state, app| {
        let _permit = permit;
        platform_chat(state, app)?.cloud_chat_page(&token, position_ms, cursor.as_deref(), limit).map_err(|_| chat_unavailable())
    }).await
}
#[tauri::command]
pub async fn cloud_recording_replay_asset(window: tauri::Webview, token: String, asset_id: String)
    -> Result<crate::platform_media::live_chat::ChatAssetRead, CloudError> {
    trusted(&window)?;
    let permit = crate::platform_media_ipc::chat_asset_slot().map_err(|_| error("CLOUD_CHAT_BUSY", "채팅 이미지를 읽는 중입니다."))?;
    run(window, false, move |state, app| {
        let _permit = permit;
        platform_chat(state, app)?.cloud_chat_asset(&token, &asset_id).map_err(|_| chat_unavailable())
    }).await
}
#[tauri::command]
pub async fn cloud_recording_replay_profile(window: tauri::Webview, token: String) -> Result<Option<String>, CloudError> {
    trusted(&window)?;
    let permit = crate::platform_media_ipc::current_profile_slot().map_err(|_| error("CLOUD_CHAT_BUSY", "채널 사진을 읽는 중입니다."))?;
    run(window, false, move |state, app| {
        let _permit = permit;
        platform_chat(state, app)?.cloud_chat_profile(&token).map_err(|_| chat_unavailable())
    }).await
}
#[tauri::command]
pub async fn cloud_recording_enqueue_recording(window: tauri::Webview, source: CloudSourceRef, visibility: Option<super::model::CloudVisibility>) -> Result<CloudSnapshot, CloudError> {
    run(window, true, move |state, app| {
        super::source::validate_reference(&source).map_err(|message| error("SOURCE_INVALID", message))?;
        if source.part_index.is_some() || source.plan_id.is_some() || source.unit_id.is_some() { return Err(error("SOURCE_INVALID", "녹화 항목을 선택해 주세요.")); }
        service(state, app)?.enqueue_recording_with_visibility(source, visibility.unwrap_or_default())
    }).await
}
#[tauri::command]
pub async fn cloud_recording_import_oauth_client(window: tauri::Webview) -> Result<CloudSnapshot, CloudError> {
    let owner = window.window();
    run(window, true, move |state, app| {
        let service = service(state, app)?;
        match pick_client_file(&owner).map_err(|message| error("CLIENT_PICKER", message))? {
            Some(path) => service.import_client_path(&path),
            None => service.snapshot(),
        }
    }).await
}
#[tauri::command]
pub async fn cloud_recording_authorize(window: tauri::Webview) -> Result<CloudSnapshot, CloudError> {
    run(window, true, |state, app| service(state, app)?.authorize()).await
}
#[tauri::command]
pub async fn cloud_recording_disconnect(window: tauri::Webview) -> Result<CloudSnapshot, CloudError> {
    run(window, true, |state, app| service(state, app)?.disconnect()).await
}
#[tauri::command]
pub async fn cloud_recording_enqueue(window: tauri::Webview, source: CloudSourceRef, visibility: Option<super::model::CloudVisibility>) -> Result<CloudSnapshot, CloudError> {
    run(window, true, move |state, app| {
        super::source::validate_reference(&source).map_err(|message| error("SOURCE_INVALID", message))?;
        service(state, app)?.enqueue_with_visibility(source, visibility.unwrap_or_default())
    }).await
}
#[tauri::command]
pub async fn cloud_recording_pause(window: tauri::Webview, job_id: String) -> Result<CloudSnapshot, CloudError> {
    run(window, true, move |state, app| service(state, app)?.pause(&job_id)).await
}
#[tauri::command]
pub async fn cloud_recording_resume(window: tauri::Webview, job_id: String) -> Result<CloudSnapshot, CloudError> {
    run(window, true, move |state, app| service(state, app)?.resume(&job_id)).await
}

fn authorization_url_allowed(raw: &str) -> bool {
    if raw.len() > 8192 { return false; }
    let Ok(url) = reqwest::Url::parse(raw) else { return false; };
    if url.scheme() != "https" || url.host_str() != Some("accounts.google.com")
        || url.port().is_some() || url.path() != "/o/oauth2/v2/auth"
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return false;
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let one = |key: &str| {
        let mut values = pairs.iter().filter(|(name, _)| name == key).map(|(_, value)| value.as_ref());
        let first = values.next(); if values.next().is_some() { None } else { first }
    };
    let Some(redirect) = one("redirect_uri").and_then(|value| reqwest::Url::parse(value).ok()) else { return false; };
    if redirect.scheme() != "http" || redirect.host_str() != Some("127.0.0.1")
        || redirect.port().is_none_or(|port| port == 0) || !redirect.username().is_empty()
        || redirect.password().is_some() || redirect.fragment().is_some() || redirect.query().is_some() {
        return false;
    }
    one("response_type") == Some("code")
        && one("code_challenge_method") == Some("S256")
        && one("code_challenge").is_some_and(|value| value.len() == 43 && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
        && one("state").is_some_and(|value| value.len() >= 20 && value.len() <= 512)
        && one("client_id").is_some_and(|value| value.ends_with(".apps.googleusercontent.com"))
}
pub(crate) fn open_authorization_browser(raw: &str) -> Result<(), String> {
    if !authorization_url_allowed(raw) { return Err("인증 주소를 확인하지 못했습니다.".into()); }
    #[cfg(windows)] {
        use windows::{core::{w, PCWSTR}, Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL}};
        let target: Vec<_> = raw.encode_utf16().chain(Some(0)).collect();
        let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(target.as_ptr()), None, None, SW_SHOWNORMAL) };
        if result.0 as isize <= 32 { return Err("기본 브라우저를 열지 못했습니다.".into()); }
        Ok(())
    }
    #[cfg(not(windows))] { Err("현재 시스템에서는 Google 인증 창을 열 수 없습니다.".into()) }
}

fn pick_client_file(owner: &tauri::Window) -> Result<Option<std::path::PathBuf>, String> {
    #[cfg(windows)] {
        use windows::{core::{w, HRESULT}, Win32::{
            Foundation::HWND,
            System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED},
            UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH},
        }};
        let owner = owner.hwnd().map_err(|_| "파일 선택기의 앱 창을 확인하지 못했습니다.")?;
        if owner.0.is_null() { return Err("파일 선택기의 앱 창을 확인하지 못했습니다.".into()); }
        let owner = HWND(owner.0 as *mut std::ffi::c_void);
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(|_| "파일 선택기를 준비하지 못했습니다.")?; }
        struct Apartment;
        impl Drop for Apartment { fn drop(&mut self) { unsafe { CoUninitialize() } } }
        let _apartment = Apartment;
        unsafe {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| "파일 선택기를 열지 못했습니다.")?;
            dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST).map_err(|_| "파일 선택기를 준비하지 못했습니다.")?;
            dialog.SetTitle(w!("Google 데스크톱 앱 OAuth JSON 선택")).map_err(|_| "파일 선택기를 준비하지 못했습니다.")?;
            dialog.SetDefaultExtension(w!("json")).map_err(|_| "파일 선택기를 준비하지 못했습니다.")?;
            // Native modality returns activation to the originating app window.
            // Do not force foreground/focus after cancellation if the user switched away.
            if let Err(cause) = dialog.Show(Some(owner)) {
                if cause.code() == HRESULT::from_win32(1223) { return Ok(None); }
                return Err("파일을 선택하지 못했습니다.".into());
            }
            let item = dialog.GetResult().map_err(|_| "선택한 파일을 읽지 못했습니다.")?;
            let path = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|_| "파일 경로를 읽지 못했습니다.")?;
            let result = path.to_string().map(std::path::PathBuf::from).map_err(|_| "파일 경로가 올바르지 않습니다.".into());
            CoTaskMemFree(Some(path.0.cast()));
            result.map(Some)
        }
    }
    #[cfg(not(windows))] { let _ = owner; Err("현재 시스템에서는 OAuth 설정 파일을 선택할 수 없습니다.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid_url() -> String {
        let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
        url.query_pairs_mut().append_pair("client_id", "test.apps.googleusercontent.com")
            .append_pair("redirect_uri", "http://127.0.0.1:43123/callback")
            .append_pair("response_type", "code").append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &"a".repeat(43)).append_pair("state", &"b".repeat(32));
        url.to_string()
    }
    #[test]
    fn opener_restricts_exact_google_pkce_loopback_flow() {
        let valid = valid_url();
        assert!(authorization_url_allowed(&valid));
        for bad in [valid.replace("accounts.google.com", "accounts.google.com.evil.test"),
            valid.replace("https://", "http://"), valid.replace("/o/oauth2/v2/auth", "/other"),
            valid.replace("127.0.0.1", "localhost"), format!("{valid}&state=other"),
            format!("{valid}#fragment"), "file:///C:/private".into(), "javascript:alert(1)".into()] {
            assert!(!authorization_url_allowed(&bad));
        }
    }
}

