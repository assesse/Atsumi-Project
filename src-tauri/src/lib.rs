pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod interface;
pub mod source;
pub mod thumbnail;

#[cfg(test)]
mod tests;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use application::{
    ApplicationService, ArtifactRepository, ArtifactStore, AutoFindSource, AutoFindSupervisor,
    AutomationRepository, DetailOriginalSupervisor, DisabledDuplicateRelationProvider,
    DownloadOverlapRepository, DownloadPipelineRepository, DownloadSourcePort, DownloadSupervisor,
    DuplicateRepository, DuplicateSupervisor, InternalDuplicateRepository,
    InternalDuplicateSupervisor, StateRepository,
};
use domain::{
    AutoFindRun, DownloadJobProjection, DuplicateScanRun, InternalArtifactScanProgress,
    InternalScanRun,
};
use infrastructure::{
    CompositeThumbnailResolver, FilesystemArtifactStore, HitomiLiveAdapter, HitomiLiveConfig,
    SqliteRepository, WindowsFolderPicker,
};
use interface::{AppQuitRequest, AppState};
use tauri::{
    http::{Method, Request, Response, StatusCode},
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconEvent},
    Emitter, Manager,
};
use thumbnail::{
    ThumbnailCompletionEventDto, ThumbnailCoordinator, ThumbnailCoordinatorConfig,
    ThumbnailResolver,
};

fn apply_pending_factory_reset(data_dir: &std::path::Path) -> std::io::Result<()> {
    let marker = data_dir.join("factory-reset.pending");
    if !marker.exists() {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let backup = data_dir.join(format!("factory-reset-backup-{stamp}"));
    std::fs::create_dir_all(&backup)?;
    for suffix in ["", "-wal", "-shm"] {
        let source = data_dir.join(format!("atsumi-next.sqlite3{suffix}"));
        if source.exists() {
            std::fs::rename(&source, backup.join(format!("atsumi-next.sqlite3{suffix}")))?;
        }
    }
    std::fs::remove_file(marker)?;
    Ok(())
}

const TRAY_WORK_STATUS_ID: &str = "tray-work-status";
const TRAY_QUIT_ID: &str = "tray-quit";
const TRAY_EVENT_REFRESH_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayQuitAction {
    AwaitValidatedShutdown,
    RestoreAndRequestConfirmation,
}

fn tray_quit_action<E>(result: Result<bool, E>) -> TrayQuitAction {
    match result {
        Ok(true) => TrayQuitAction::AwaitValidatedShutdown,
        Ok(false) | Err(_) => TrayQuitAction::RestoreAndRequestConfirmation,
    }
}

pub(crate) fn shutdown_all_then_exit<Internal, Duplicates, AutoFind, Downloads, Exit>(
    shutdown_internal_duplicates: Internal,
    shutdown_duplicates: Duplicates,
    shutdown_auto_find: AutoFind,
    shutdown_downloads: Downloads,
    exit: Exit,
) where
    Internal: FnOnce(),
    Duplicates: FnOnce(),
    AutoFind: FnOnce(),
    Downloads: FnOnce(),
    Exit: FnOnce(),
{
    shutdown_internal_duplicates();
    shutdown_duplicates();
    shutdown_auto_find();
    shutdown_downloads();
    exit();
}

pub(crate) fn minimize_to_tray<T, E>(hide: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    hide()
}

fn detail_original_protocol_request_id(request: &Request<Vec<u8>>) -> Result<String, StatusCode> {
    if request.method() != Method::GET {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    if request.uri().query().is_some() {
        return Err(StatusCode::NOT_FOUND);
    }
    let Some(request_id) = request
        .uri()
        .path()
        .strip_prefix('/')
        .filter(|id| !id.is_empty() && !id.contains('/'))
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    application::canonical_request_id(request_id).map_err(|_| StatusCode::NOT_FOUND)
}

fn detail_original_protocol_response(
    status: StatusCode,
    bytes: Vec<u8>,
    content_type: Option<&str>,
) -> Response<Vec<u8>> {
    let mut response = Response::builder()
        .status(status)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff");
    if let Some(content_type) = content_type {
        response = response
            .header("content-type", content_type)
            .header("content-length", bytes.len());
    }
    response
        .body(bytes)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

fn danbooru_media_protocol_token(request: &Request<Vec<u8>>) -> Result<String, StatusCode> {
    if request.method() != Method::GET {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    if request.uri().query().is_some() {
        return Err(StatusCode::NOT_FOUND);
    }
    request
        .uri()
        .path()
        .strip_prefix('/')
        .filter(|token| !token.is_empty() && !token.contains('/'))
        .map(str::to_owned)
        .ok_or(StatusCode::NOT_FOUND)
}

fn danbooru_media_protocol_response(
    status: StatusCode,
    bytes: Vec<u8>,
    content_type: Option<&str>,
) -> Response<Vec<u8>> {
    let cache_control = if status.is_success() {
        "private, max-age=86400"
    } else {
        "no-store"
    };
    let mut response = Response::builder()
        .status(status)
        .header("cache-control", cache_control)
        .header("cross-origin-resource-policy", "cross-origin")
        .header("x-content-type-options", "nosniff");
    if let Some(content_type) = content_type {
        response = response
            .header("content-type", content_type)
            .header("content-length", bytes.len());
    }
    response
        .body(bytes)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

#[cfg(test)]
mod detail_original_protocol_tests {
    use super::*;

    const REQUEST_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    #[test]
    fn protocol_accepts_only_a_single_canonical_uuid_get_path() {
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("http://detail-original.localhost/{REQUEST_ID}"))
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            detail_original_protocol_request_id(&request),
            Ok(REQUEST_ID.into())
        );

        let query = Request::builder()
            .method(Method::GET)
            .uri(format!(
                "http://detail-original.localhost/{REQUEST_ID}?extra=1"
            ))
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            detail_original_protocol_request_id(&query),
            Err(StatusCode::NOT_FOUND)
        );

        let traversal = Request::builder()
            .method(Method::GET)
            .uri("http://detail-original.localhost/../outside")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            detail_original_protocol_request_id(&traversal),
            Err(StatusCode::NOT_FOUND)
        );

        let post = Request::builder()
            .method(Method::POST)
            .uri(format!("http://detail-original.localhost/{REQUEST_ID}"))
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            detail_original_protocol_request_id(&post),
            Err(StatusCode::METHOD_NOT_ALLOWED)
        );
    }

    #[test]
    fn protocol_success_response_is_non_cacheable_and_nosniff() {
        let response =
            detail_original_protocol_response(StatusCode::OK, vec![1, 2, 3], Some("image/webp"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/webp");
        assert_eq!(response.headers()["content-length"], "3");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }

    #[test]
    fn danbooru_media_protocol_accepts_only_one_get_token() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("http://danbooru-media.localhost/aGVsbG8")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            danbooru_media_protocol_token(&request),
            Ok("aGVsbG8".into())
        );
        let nested = Request::builder()
            .method(Method::GET)
            .uri("http://danbooru-media.localhost/a/b")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            danbooru_media_protocol_token(&nested),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn danbooru_media_response_is_cacheable_and_cross_origin_safe() {
        let response =
            danbooru_media_protocol_response(StatusCode::OK, vec![1, 2], Some("image/jpeg"));
        assert_eq!(response.headers()["content-type"], "image/jpeg");
        assert_eq!(response.headers()["content-length"], "2");
        assert_eq!(
            response.headers()["cache-control"],
            "private, max-age=86400"
        );
        assert_eq!(
            response.headers()["cross-origin-resource-policy"],
            "cross-origin"
        );
    }

    #[test]
    fn danbooru_media_error_responses_are_not_cached() {
        for status in [
            StatusCode::NOT_FOUND,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            StatusCode::PAYLOAD_TOO_LARGE,
            StatusCode::BAD_GATEWAY,
        ] {
            let response = danbooru_media_protocol_response(status, Vec::new(), None);
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
}

struct TrayMenuState {
    work_status: MenuItem<tauri::Wry>,
    event_refresh_pending: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrayWorkCounts {
    active_downloads: u64,
    auto_find_active: bool,
    duplicate_scan_active: bool,
    internal_duplicate_scan_active: bool,
}

fn tray_work_status_label(work: Option<TrayWorkCounts>) -> String {
    let Some(work) = work else {
        return "작업 상태: 확인 불가".into();
    };
    let mut kinds = Vec::new();
    if work.active_downloads > 0 {
        kinds.push(format!("다운로드 {}개", work.active_downloads));
    }
    if work.auto_find_active {
        kinds.push("Auto Find".into());
    }
    if work.duplicate_scan_active {
        kinds.push("작품 중복 검사".into());
    }
    if work.internal_duplicate_scan_active {
        kinds.push("내부 중복 검사".into());
    }
    if kinds.is_empty() {
        "작업 상태: 진행 중인 작업 없음".into()
    } else {
        format!("작업 상태: {}", kinds.join(" · "))
    }
}

fn restore_main_window(app: &tauri::AppHandle, action: &str) -> Option<tauri::WebviewWindow> {
    let Some(window) = app.get_webview_window("main") else {
        tracing::warn!("could not {action}; the main Atsumi window is unavailable");
        return None;
    };
    if let Err(error) = window.show() {
        tracing::warn!(error = %error, "could not show Atsumi from the tray");
    }
    if let Err(error) = window.unminimize() {
        tracing::warn!(error = %error, "could not unminimize Atsumi from the tray");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(error = %error, "could not focus Atsumi from the tray");
    }
    Some(window)
}

fn request_tray_exit_confirmation(app: &tauri::AppHandle) {
    let Some(window) = restore_main_window(app, "request tray exit confirmation") else {
        return;
    };
    if let Err(error) = window.emit(
        "app:exit-requested",
        serde_json::json!({ "source": "tray_menu" }),
    ) {
        tracing::warn!(error = %error, "could not request the exit confirmation dialog from the tray");
    }
}

fn refresh_tray_work_status(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let work = match state.active_work_snapshot() {
        Ok(snapshot) => Some(TrayWorkCounts {
            active_downloads: snapshot.downloads.active_count,
            auto_find_active: snapshot.auto_find.is_some(),
            duplicate_scan_active: snapshot.duplicate_scan.is_some(),
            internal_duplicate_scan_active: snapshot.internal_duplicate_scan.is_some(),
        }),
        Err(error) => {
            tracing::warn!(error = %error, "could not refresh tray work status");
            None
        }
    };
    let label = tray_work_status_label(work);
    if let Some(menu_state) = app.try_state::<TrayMenuState>() {
        if let Err(error) = menu_state.work_status.set_text(&label) {
            tracing::warn!(error = %error, "could not update tray work status");
        }
    }
    if let Some(tray) = app.tray_by_id("main") {
        if let Err(error) = tray.set_tooltip(Some(&label)) {
            tracing::warn!(error = %error, "could not update tray tooltip");
        }
    }
}

fn claim_tray_event_refresh(pending: &AtomicBool) -> bool {
    pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// Event relays can deliver a rapid progress burst.  Coalesce that burst into
/// one deferred refresh while leaving explicit tray interactions immediate.
fn schedule_tray_work_status_refresh(app: &tauri::AppHandle) {
    let Some(menu_state) = app.try_state::<TrayMenuState>() else {
        return;
    };
    let pending = Arc::clone(&menu_state.event_refresh_pending);
    if !claim_tray_event_refresh(&pending) {
        return;
    }
    let worker_pending = Arc::clone(&pending);
    let refresh_app = app.clone();
    if let Err(error) = thread::Builder::new()
        .name("atsumi-tray-work-refresh".into())
        .spawn(move || {
            thread::sleep(TRAY_EVENT_REFRESH_DEBOUNCE);
            // Release the coalescing slot before the read. An event that lands
            // while this snapshot/menu update is running then schedules one
            // trailing refresh instead of being lost with a stale label.
            worker_pending.store(false, Ordering::Release);
            refresh_tray_work_status(&refresh_app);
        })
    {
        pending.store(false, Ordering::Release);
        tracing::warn!(error = %error, "could not schedule tray work status refresh");
    }
}

fn request_tray_quit(app: &tauri::AppHandle) {
    let result = if let Some(state) = app.try_state::<AppState>() {
        match state.request_graceful_quit(
            app.clone(),
            AppQuitRequest {
                expected_work_set_fingerprint: String::new(),
                confirm_active_work: false,
                force_when_status_unknown: false,
            },
        ) {
            Ok(result) => Ok(result.accepted),
            Err(error) => {
                tracing::warn!(error = ?error, "could not start graceful quit from the tray");
                Err(())
            }
        }
    } else {
        Err(())
    };
    if tray_quit_action(result) == TrayQuitAction::RestoreAndRequestConfirmation {
        request_tray_exit_confirmation(app);
    }
}

pub fn run() -> tauri::Result<()> {
    infrastructure::telemetry::init();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol("detail-original", |context, request, responder| {
            let request_id = match detail_original_protocol_request_id(&request) {
                Ok(request_id) => request_id,
                Err(status) => {
                    responder.respond(detail_original_protocol_response(status, Vec::new(), None));
                    return;
                }
            };
            let state = context.app_handle().state::<AppState>();
            let Some((path, content_type)) = state.detail_original_media_file(&request_id) else {
                responder.respond(detail_original_protocol_response(StatusCode::NOT_FOUND, Vec::new(), None));
                return;
            };
            tracing::info!(request_id = %request_id, "detail original media_requested");
            thread::spawn(move || {
                let response = match std::fs::read(path) {
                    Ok(bytes) => detail_original_protocol_response(StatusCode::OK, bytes, Some(&content_type)),
                    Err(_) => detail_original_protocol_response(StatusCode::NOT_FOUND, Vec::new(), None),
                };
                responder.respond(response);
            });
        })
        .register_asynchronous_uri_scheme_protocol("danbooru-media", |context, request, responder| {
            let token = match danbooru_media_protocol_token(&request) {
                Ok(token) => token,
                Err(status) => {
                    responder.respond(danbooru_media_protocol_response(status, Vec::new(), None));
                    return;
                }
            };
            let client = Arc::clone(&context.app_handle().state::<AppState>().danbooru);
            thread::spawn(move || {
                let response = match client.media(&token) {
                    Ok(media) => danbooru_media_protocol_response(
                        StatusCode::OK,
                        media.bytes,
                        Some(&media.content_type),
                    ),
                    Err(interface::danbooru::DanbooruError::UnsafeMediaUrl) => {
                        danbooru_media_protocol_response(StatusCode::NOT_FOUND, Vec::new(), None)
                    }
                    Err(interface::danbooru::DanbooruError::NotFound)
                    | Err(interface::danbooru::DanbooruError::MediaUnavailable) => {
                        danbooru_media_protocol_response(StatusCode::NOT_FOUND, Vec::new(), None)
                    }
                    Err(interface::danbooru::DanbooruError::UnsupportedMedia) => {
                        danbooru_media_protocol_response(
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            Vec::new(),
                            None,
                        )
                    }
                    Err(interface::danbooru::DanbooruError::DownloadTooLarge) => {
                        danbooru_media_protocol_response(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            Vec::new(),
                            None,
                        )
                    }
                    Err(error) => {
                        tracing::warn!(error = ?error, "Danbooru display media could not be loaded");
                        danbooru_media_protocol_response(
                            StatusCode::BAD_GATEWAY,
                            Vec::new(),
                            None,
                        )
                    }
                };
                responder.respond(response);
            });
        })
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                if let Err(error) = window
                    .show()
                    .and_then(|_| window.unminimize())
                    .and_then(|_| window.set_focus())
                {
                    tracing::warn!(error = %error, "could not focus the existing Atsumi window");
                }
            }
        }))
        .on_tray_icon_event(|app, event| {
            let refresh_status = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Right,
                    button_state: MouseButtonState::Up,
                    ..
                }
            );
            if refresh_status {
                refresh_tray_work_status(app);
            }
            let restore = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            );
            if !restore {
                return;
            }
            restore_main_window(app, "restore Atsumi from the tray");
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) =
                    window.emit("app:exit-requested", serde_json::json!({ "source": "window_close" }))
                {
                    tracing::warn!(error = %error, "could not request the exit confirmation dialog");
                }
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            apply_pending_factory_reset(&data_dir)?;
            let database_path = data_dir.join("atsumi-next.sqlite3");
            let repository = SqliteRepository::open(&database_path)?;
            let repository = Arc::new(repository);
            let settings = ApplicationService::new(repository.clone()).settings_get()?;
            let download_root_configured = !settings.download_root.trim().is_empty();
            let live_source = Arc::new(HitomiLiveAdapter::new(HitomiLiveConfig {
                max_concurrent_requests: settings.concurrent_image_requests as usize,
                request_start_interval: Duration::from_millis(
                    settings.request_start_interval_ms,
                ),
                ..HitomiLiveConfig::default()
            })?);
            let service = ApplicationService::new(repository.clone())
                .with_download_repository(repository.clone())
                .with_search_repository(live_source.clone())
                .with_automation_repository(repository.clone())
                .with_tag_catalog(repository.clone(), live_source.clone());
            let danbooru = Arc::new(interface::DanbooruClient::new().map_err(|_| {
                std::io::Error::other("could not initialize the Danbooru client")
            })?);
            let recovered_entries = service.download_recover_interrupted()?;
            let automation_repository: Arc<dyn AutomationRepository> = repository.clone();
            let auto_find_settings: Arc<dyn StateRepository> = repository.clone();
            let auto_find_source: Arc<dyn AutoFindSource> = live_source.clone();
            let (auto_find_event_tx, auto_find_event_rx) = mpsc::channel::<AutoFindRun>();
            let auto_find = AutoFindSupervisor::new(
                automation_repository,
                auto_find_settings,
                auto_find_source,
                auto_find_event_tx,
            );
            let recovered_auto_find_runs = auto_find.recover_interrupted()?;
            let auto_find_app = app.handle().clone();
            thread::Builder::new()
                .name("atsumi-auto-find-events".into())
                .spawn(move || {
                    while let Ok(run) = auto_find_event_rx.recv() {
                        if let Err(error) = auto_find_app.emit("auto-find:changed", &run) {
                            tracing::warn!(error = %error, "could not emit auto-find:changed");
                        }
                        schedule_tray_work_status_refresh(&auto_find_app);
                    }
                })?;
            let artifact_store: Arc<dyn ArtifactStore> =
                Arc::new(FilesystemArtifactStore::new());
            let thumbnail_config = ThumbnailCoordinatorConfig {
                max_concurrency: settings.concurrent_image_requests as usize,
                request_start_interval: Duration::from_millis(settings.request_start_interval_ms),
                ..ThumbnailCoordinatorConfig::default()
            };
            let remote_thumbnail_resolver: Arc<dyn ThumbnailResolver> = live_source.clone();
            let artifact_repository: Arc<dyn ArtifactRepository> = repository.clone();
            let thumbnail_settings: Arc<dyn StateRepository> = repository.clone();
            let thumbnail_resolver: Arc<dyn ThumbnailResolver> = Arc::new(
                CompositeThumbnailResolver::new(
                    remote_thumbnail_resolver,
                    Arc::clone(&artifact_repository),
                    thumbnail_settings,
                    Arc::clone(&artifact_store),
                ),
            );
            let thumbnails = ThumbnailCoordinator::new(thumbnail_resolver, thumbnail_config)?;
            let (thumbnail_completion_tx, thumbnail_completion_rx) =
                mpsc::channel::<ThumbnailCompletionEventDto>();
            let thumbnail_app = app.handle().clone();
            thread::Builder::new()
                .name("atsumi-thumbnail-events".into())
                .spawn(move || {
                    while let Ok(event) = thumbnail_completion_rx.recv() {
                        if let Err(error) = thumbnail_app.emit("thumbnail:ready", &event) {
                            tracing::warn!(error = %error, "could not emit thumbnail:ready");
                        }
                    }
                })?;
            let detail_original_repository: Arc<dyn DownloadPipelineRepository> = repository.clone();
            let detail_originals = DetailOriginalSupervisor::new_with_artifacts(
                live_source.clone(),
                detail_original_repository,
                Arc::clone(&artifact_store),
                &data_dir,
            )?;
            let duplicate_repository: Arc<dyn DuplicateRepository> = repository.clone();
            let duplicate_settings: Arc<dyn StateRepository> = repository.clone();
            let (duplicate_event_tx, duplicate_event_rx) = mpsc::channel::<DuplicateScanRun>();
            let duplicates = DuplicateSupervisor::new(
                Arc::clone(&duplicate_repository),
                duplicate_settings,
                Arc::clone(&artifact_store),
                Arc::new(DisabledDuplicateRelationProvider),
                duplicate_event_tx,
            );
            let recovered_duplicate_runs = duplicates.recover_interrupted()?;
            let duplicate_app = app.handle().clone();
            thread::Builder::new()
                .name("atsumi-duplicate-events".into())
                .spawn(move || {
                    while let Ok(run) = duplicate_event_rx.recv() {
                        if let Err(error) = duplicate_app.emit("duplicate:changed", &run) {
                            tracing::warn!(error = %error, "could not emit duplicate:changed");
                        }
                        schedule_tray_work_status_refresh(&duplicate_app);
                    }
                })?;
            let internal_repository: Arc<dyn InternalDuplicateRepository> = repository.clone();
            let internal_artifact_repository: Arc<dyn ArtifactRepository> = repository.clone();
            let internal_settings: Arc<dyn StateRepository> = repository.clone();
            let (internal_event_tx, internal_event_rx) = mpsc::channel::<InternalScanRun>();
            let (internal_progress_tx, internal_progress_rx) =
                mpsc::channel::<InternalArtifactScanProgress>();
            let internal_duplicates = InternalDuplicateSupervisor::new_with_progress_events(
                internal_repository,
                duplicate_repository,
                internal_artifact_repository,
                internal_settings,
                Arc::clone(&artifact_store),
                internal_event_tx,
                internal_progress_tx,
            );
            let recovered_internal_runs = internal_duplicates.recover_interrupted()?;
            let reconciled_internal_pages = if download_root_configured {
                match internal_duplicates.reconcile_pending_page_moves() {
                    Ok(count) => count,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "startup internal page quarantine reconciliation was deferred"
                        );
                        0
                    }
                }
            } else {
                0
            };
            let internal_app = app.handle().clone();
            thread::Builder::new()
                .name("atsumi-internal-duplicate-events".into())
                .spawn(move || {
                    while let Ok(run) = internal_event_rx.recv() {
                        if let Err(error) = internal_app.emit("internal-duplicate:changed", &run) {
                            tracing::warn!(
                                error = %error,
                                "could not emit internal-duplicate:changed"
                            );
                        }
                        schedule_tray_work_status_refresh(&internal_app);
                    }
                })?;
            let internal_progress_app = app.handle().clone();
            thread::Builder::new()
                .name("atsumi-internal-duplicate-progress-events".into())
                .spawn(move || {
                    while let Ok(progress) = internal_progress_rx.recv() {
                        if let Err(error) = internal_progress_app
                            .emit("internal-duplicate:artifact-progress", &progress)
                        {
                            tracing::warn!(
                                error = %error,
                                "could not emit internal-duplicate:artifact-progress"
                            );
                        }
                    }
                })?;
            let download_repository: Arc<dyn DownloadOverlapRepository> = repository.clone();
            let settings_repository: Arc<dyn StateRepository> = repository.clone();
            let download_source: Arc<dyn DownloadSourcePort> = live_source.clone();
            let (download_event_tx, download_event_rx) =
                mpsc::channel::<DownloadJobProjection>();
            let download_app = app.handle().clone();
            thread::Builder::new()
                .name("atsumi-download-events".into())
                .spawn(move || {
                    while let Ok(projection) = download_event_rx.recv() {
                        if let Err(error) = download_app.emit("job:changed", &projection.job) {
                            tracing::warn!(error = %error, "could not emit job:changed");
                        }
                        if let Err(error) =
                            download_app.emit("download:changed", &projection.download)
                        {
                            tracing::warn!(error = %error, "could not emit download:changed");
                        }
                        schedule_tray_work_status_refresh(&download_app);
                    }
                })?;
            let downloads = DownloadSupervisor::new(
                download_repository,
                settings_repository,
                download_source,
                Arc::clone(&artifact_store),
                download_event_tx,
                2,
            )?;
            let (startup_recovery_issues, resumed_jobs) =
                if download_root_configured {
                    match downloads.recover_startup_state() {
                        Ok(report) => (report.issues.len(), report.resumed_jobs),
                        Err(_) => {
                            tracing::warn!(
                                "startup download recovery was deferred; no ambiguous file was changed"
                            );
                            (1, 0)
                        }
                    }
                } else {
                    (0, 0)
                };
            app.manage(AppState::new(
                service,
                danbooru,
                thumbnails,
                thumbnail_completion_tx,
                detail_originals,
                downloads,
                auto_find,
                duplicates,
                internal_duplicates,
                Arc::new(WindowsFolderPicker::new()),
                artifact_store,
                live_source.clone(),
                data_dir,
            ));
            let tray_status = MenuItem::with_id(
                app,
                TRAY_WORK_STATUS_ID,
                "작업 상태: 진행 중인 작업 없음",
                false,
                None::<&str>,
            )?;
            let tray_quit = MenuItem::with_id(app, TRAY_QUIT_ID, "종료", true, None::<&str>)?;
            let tray_separator = PredefinedMenuItem::separator(app)?;
            let tray_menu = Menu::with_items(app, &[&tray_status, &tray_separator, &tray_quit])?;
            let tray = app
                .tray_by_id("main")
                .ok_or_else(|| tauri::Error::AssetNotFound("main tray icon".into()))?;
            tray.set_menu(Some(tray_menu))?;
            tray.on_menu_event(|app, event| {
                if event.id() == TRAY_QUIT_ID {
                    request_tray_quit(app);
                }
            });
            app.manage(TrayMenuState {
                work_status: tray_status,
                event_refresh_pending: Arc::new(AtomicBool::new(false)),
            });
            refresh_tray_work_status(app.handle());
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
                window.unminimize()?;
                window.set_focus()?;
            }
            tracing::info!(
                database_file = "atsumi-next.sqlite3",
                app_version = env!("CARGO_PKG_VERSION"),
                recovered_entries,
                recovered_auto_find_runs,
                recovered_duplicate_runs,
                recovered_internal_runs,
                reconciled_internal_pages,
                startup_recovery_issues,
                resumed_jobs,
                "Atsumi backend initialized"
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            interface::commands::settings_get,
            interface::danbooru::danbooru_search,
            interface::danbooru::danbooru_random,
            interface::danbooru::danbooru_related,
            interface::danbooru::danbooru_autocomplete,
            interface::danbooru::danbooru_download,
            interface::danbooru::danbooru_downloads_list,
            interface::commands::settings_update,
            interface::commands::storage_usage_get,
            interface::commands::folder_name_template_preview,
            interface::commands::window_placement_get,
            interface::commands::window_placement_update,
            interface::commands::search_submit,
            interface::commands::search_page_get,
            interface::commands::search_page_cancel,
            interface::commands::gallery_detail_get,
            interface::commands::favorites_list,
            interface::commands::favorite_set,
            interface::commands::search_history_list,
            interface::commands::tag_catalog_status,
            interface::commands::tag_catalog_refresh,
            interface::commands::tag_suggestions_search,
            interface::commands::auto_find_snapshot,
            interface::commands::auto_find_refresh,
            interface::commands::auto_find_cancel,
            interface::commands::auto_find_exclude,
            interface::commands::exploration_exclusions_list,
            interface::commands::exploration_exclusions_restore,
            interface::commands::exploration_data_reset,
            interface::commands::maintenance_preview,
            interface::commands::maintenance_execute,
            interface::commands::duplicate_snapshot,
            interface::commands::duplicate_scan_start,
            interface::commands::duplicate_scan_cancel,
            interface::commands::duplicate_review_get,
            interface::commands::duplicate_decision_apply,
            interface::commands::download_overlap_review_get,
            interface::commands::download_overlap_decision_apply,
            interface::commands::internal_duplicate_snapshot,
            interface::commands::internal_duplicate_active_artifact,
            interface::commands::internal_duplicate_scan_start,
            interface::commands::internal_duplicate_scan_cancel,
            interface::commands::internal_duplicate_review_get,
            interface::commands::internal_removal_plan,
            interface::commands::internal_removal_apply,
            interface::commands::internal_removal_undo,
            interface::commands::download_queue_add,
            interface::commands::download_entries_list,
            interface::commands::download_library_page_list,
            interface::commands::download_retry,
            interface::commands::download_cancel,
            interface::commands::download_quarantine,
            interface::commands::download_quarantine_undo,
            interface::commands::artifact_open_first,
            interface::commands::artifact_open_folder,
            interface::commands::app_reconcile,
            interface::commands::thumbnail_request,
            interface::commands::thumbnail_cancel,
            interface::commands::thumbnail_invalidate,
            interface::commands::thumbnail_reprioritize,
            interface::commands::thumbnail_stats,
            interface::commands::thumbnail_cache_clear,
            interface::commands::detail_original_prepare,
            interface::commands::detail_original_dispose,
            interface::commands::app_minimize_to_tray,
            interface::commands::app_active_work_snapshot,
            interface::commands::app_quit,
        ])
        .run(tauri::generate_context!());

    if let Err(ref error) = result {
        tracing::error!(error_type = %std::any::type_name_of_val(error), "Atsumi exited with an error");
    }
    result
}

#[cfg(test)]
mod tray_menu_tests {
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
    };

    use super::{
        claim_tray_event_refresh, minimize_to_tray, shutdown_all_then_exit, tray_quit_action,
        tray_work_status_label, TrayQuitAction, TrayWorkCounts,
    };

    #[test]
    fn tray_quit_restores_confirmation_for_active_or_unknown_work() {
        assert_eq!(
            tray_quit_action(Ok::<_, ()>(false)),
            TrayQuitAction::RestoreAndRequestConfirmation
        );
        assert_eq!(
            tray_quit_action(Err::<bool, _>("active work status unavailable")),
            TrayQuitAction::RestoreAndRequestConfirmation
        );
    }

    #[test]
    fn tray_quit_awaits_only_a_validated_shutdown() {
        assert_eq!(
            tray_quit_action(Ok::<_, ()>(true)),
            TrayQuitAction::AwaitValidatedShutdown
        );
    }

    #[test]
    fn shutdown_joins_every_worker_in_order_before_exit() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let record = |event: &'static str| {
            let events = Rc::clone(&events);
            move || events.borrow_mut().push(event)
        };

        shutdown_all_then_exit(
            record("internal_duplicates"),
            record("duplicates"),
            record("auto_find"),
            record("downloads"),
            record("exit"),
        );

        assert_eq!(
            events.borrow().as_slice(),
            [
                "internal_duplicates",
                "duplicates",
                "auto_find",
                "downloads",
                "exit",
            ]
        );
    }

    #[test]
    fn minimize_to_tray_performs_only_the_supplied_hide_operation() {
        let events = RefCell::new(Vec::new());

        let result = minimize_to_tray(|| {
            events.borrow_mut().push("hide");
            Ok::<_, ()>(())
        });

        assert_eq!(result, Ok(()));
        assert_eq!(events.into_inner(), ["hide"]);
    }

    #[test]
    fn labels_aggregate_active_work_without_exposing_work_identities() {
        assert_eq!(
            tray_work_status_label(Some(TrayWorkCounts {
                active_downloads: 0,
                auto_find_active: false,
                duplicate_scan_active: false,
                internal_duplicate_scan_active: false,
            })),
            "작업 상태: 진행 중인 작업 없음"
        );
        assert_eq!(
            tray_work_status_label(Some(TrayWorkCounts {
                active_downloads: 2,
                auto_find_active: false,
                duplicate_scan_active: false,
                internal_duplicate_scan_active: false,
            })),
            "작업 상태: 다운로드 2개"
        );
        assert_eq!(
            tray_work_status_label(Some(TrayWorkCounts {
                active_downloads: 2,
                auto_find_active: true,
                duplicate_scan_active: true,
                internal_duplicate_scan_active: true,
            })),
            "작업 상태: 다운로드 2개 · Auto Find · 작품 중복 검사 · 내부 중복 검사"
        );
        assert_eq!(
            tray_work_status_label(Some(TrayWorkCounts {
                active_downloads: 0,
                auto_find_active: false,
                duplicate_scan_active: false,
                internal_duplicate_scan_active: true,
            })),
            "작업 상태: 내부 중복 검사"
        );
        assert_eq!(tray_work_status_label(None), "작업 상태: 확인 불가");
    }

    #[test]
    fn event_refresh_claim_coalesces_a_burst_until_the_worker_completes() {
        let pending = AtomicBool::new(false);
        assert!(claim_tray_event_refresh(&pending));
        assert!(!claim_tray_event_refresh(&pending));
        pending.store(false, Ordering::Release);
        assert!(claim_tray_event_refresh(&pending));
    }
}
