use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::{
    application::{
        ApplicationError, ApplicationService, ArtifactStore, AutoFindSupervisor,
        DetailOriginalError, DetailOriginalPrepareRequest, DetailOriginalPrepared,
        DetailOriginalSupervisor, DownloadPipelineError, DownloadPipelineErrorCode,
        DownloadRootPicker, DownloadSupervisor, DuplicateSupervisor, InternalDuplicateSupervisor,
        ReconcileReport,
    },
    domain::{
        AutoFindExclusionResult, AutoFindRun, AutoFindSnapshot, DownloadChangedEvent,
        DownloadEntry, DownloadLibraryPage, DownloadListRequest, DownloadOverlapDecisionRequest,
        DownloadOverlapDecisionResult, DownloadOverlapReview, DownloadPage,
        DuplicateDecisionRequest, DuplicateReview, DuplicateScanRun, DuplicateSnapshot,
        ExplorationDataResetRequest, ExplorationDataResetResult, FavoriteKey,
        FavoriteMutationResult, FavoriteRecord, GalleryDetail, GalleryPage,
        InternalArtifactScanProgress, InternalDuplicateReview, InternalDuplicateSnapshot,
        InternalRemovalApplyRequest, InternalRemovalPlan, InternalRemovalPlanRequest,
        InternalRemovalResult, InternalRemovalUndoRequest, InternalScanRequest, InternalScanRun,
        JobRef, MaintenanceAction, MaintenancePreview, MaintenanceResult, SearchHistoryEntry,
        SearchRequest, SearchSubmission, SettingsPatch, SettingsSnapshot, TagCatalogStatus,
        TagSuggestion, TagSuggestionRequest, ValidationError, WindowPlacement,
        WindowPlacementSnapshot,
    },
    infrastructure::HitomiLiveAdapter,
    thumbnail::{
        CancellationToken, ThumbnailCacheClearDto, ThumbnailCompletionEventDto,
        ThumbnailCoordinator, ThumbnailCoordinatorError, ThumbnailInvalidationDto, ThumbnailKey,
        ThumbnailPriority, ThumbnailRequestDto, ThumbnailRequestTokenDto,
        ThumbnailRuntimeConfigDto, ThumbnailWorkerStatsDto,
    },
};

use super::storage_usage::{collect_storage_usage, StorageUsageSnapshot};
use super::{
    api::{
        AppActiveAutoFindSnapshot, AppActiveDownloadsSnapshot, AppActiveDuplicateScanSnapshot,
        AppActiveInternalDuplicateScanSnapshot, AppActiveWorkSnapshot, AppQuitRejectionReason,
        AppQuitRequest, AppQuitResult,
    },
    ApiAction, ApiError, ApiResult,
};

#[derive(Clone)]
struct ManagedWorkGate {
    inner: Arc<ManagedWorkGateInner>,
}

struct ManagedWorkGateInner {
    control: Mutex<ManagedWorkGateState>,
    quitting: AtomicBool,
}

#[derive(Default)]
struct ManagedWorkGateState {
    accepted_quit: Option<AppQuitResult>,
}

impl Default for ManagedWorkGate {
    fn default() -> Self {
        Self {
            inner: Arc::new(ManagedWorkGateInner {
                control: Mutex::new(ManagedWorkGateState::default()),
                quitting: AtomicBool::new(false),
            }),
        }
    }
}

impl ManagedWorkGate {
    fn run<T>(
        &self,
        operation: impl FnOnce() -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let _control = self.inner.control.lock().map_err(|_| {
            crate::application::RepositoryError::Other(
                "managed work gate mutex was poisoned".into(),
            )
        })?;
        if self.inner.quitting.load(Ordering::Acquire) {
            return Err(ApplicationError::AppQuitInProgress);
        }
        operation()
    }

    fn evaluate_quit_locked(
        &self,
        control: &mut ManagedWorkGateState,
        request: &AppQuitRequest,
        snapshot: Option<AppActiveWorkSnapshot>,
    ) -> (AppQuitResult, bool) {
        if self.inner.quitting.load(Ordering::Acquire) {
            return (
                control.accepted_quit.clone().unwrap_or(AppQuitResult {
                    accepted: true,
                    reason: None,
                    snapshot: None,
                }),
                false,
            );
        }
        if let Some(snapshot) = snapshot.as_ref() {
            if let Some(reason) = quit_rejection_reason(request, snapshot) {
                return (
                    AppQuitResult {
                        accepted: false,
                        reason: Some(reason),
                        snapshot: Some(snapshot.clone()),
                    },
                    false,
                );
            }
        } else if !request.confirm_active_work {
            return (
                AppQuitResult {
                    accepted: false,
                    reason: Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired),
                    snapshot: None,
                },
                false,
            );
        }

        let accepted = AppQuitResult {
            accepted: true,
            reason: None,
            snapshot,
        };
        control.accepted_quit = Some(accepted.clone());
        self.inner.quitting.store(true, Ordering::Release);
        (accepted, true)
    }
}

fn prepare_then_commit_managed_work<P, T>(
    managed_work: &ManagedWorkGate,
    prepare: impl FnOnce() -> Result<P, ApplicationError>,
    commit: impl FnOnce(P) -> Result<T, ApplicationError>,
) -> Result<T, ApplicationError> {
    let prepared = prepare()?;
    managed_work.run(|| commit(prepared))
}

fn now_unix_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn active_work_fingerprint<'a>(
    active_download_entry_ids: impl IntoIterator<Item = &'a str>,
    auto_find_run_id: Option<&str>,
    duplicate_run_id: Option<&str>,
    internal_duplicate_run_id: Option<&str>,
) -> String {
    fn update_identity(hasher: &mut Sha256, kind: &str, identity: Option<&str>) {
        hasher.update(u64::try_from(kind.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(kind.as_bytes());
        match identity {
            Some(identity) => {
                hasher.update([1]);
                hasher.update(
                    u64::try_from(identity.len())
                        .unwrap_or(u64::MAX)
                        .to_be_bytes(),
                );
                hasher.update(identity.as_bytes());
            }
            None => hasher.update([0]),
        }
    }

    let mut downloads = active_download_entry_ids.into_iter().collect::<Vec<_>>();
    downloads.sort_unstable();
    downloads.dedup();

    let mut hasher = Sha256::new();
    hasher.update(b"atsumi-active-work-v1");
    for entry_id in downloads {
        update_identity(&mut hasher, "download", Some(entry_id));
    }
    update_identity(&mut hasher, "auto_find", auto_find_run_id);
    update_identity(&mut hasher, "duplicate_scan", duplicate_run_id);
    update_identity(
        &mut hasher,
        "internal_duplicate_scan",
        internal_duplicate_run_id,
    );
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn active_work_status_error(source_code: &str) -> ApiError {
    ApiError {
        code: "APP_ACTIVE_WORK_STATUS_UNAVAILABLE".into(),
        message: "The application could not determine whether background work is active".into(),
        retryable: true,
        action: Some(ApiAction::Retry),
        details: Some(std::collections::BTreeMap::from([(
            "sourceCode".into(),
            serde_json::json!(source_code),
        )])),
    }
}

fn quit_rejection_reason(
    request: &AppQuitRequest,
    snapshot: &AppActiveWorkSnapshot,
) -> Option<AppQuitRejectionReason> {
    let expected = request.expected_work_set_fingerprint.trim();
    if (!expected.is_empty() && expected != snapshot.work_set_fingerprint)
        || (request.confirm_active_work && expected.is_empty())
    {
        return Some(AppQuitRejectionReason::ActiveWorkChanged);
    }
    if snapshot.has_active_work() && !request.confirm_active_work {
        return Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired);
    }
    None
}

pub struct AppState {
    service: ApplicationService,
    thumbnails: ThumbnailCoordinator,
    thumbnail_completions: Sender<ThumbnailCompletionEventDto>,
    detail_originals: DetailOriginalSupervisor,
    downloads: DownloadSupervisor,
    auto_find: AutoFindSupervisor,
    duplicates: DuplicateSupervisor,
    internal_duplicates: InternalDuplicateSupervisor,
    download_root_picker: Arc<dyn DownloadRootPicker>,
    artifact_store: Arc<dyn ArtifactStore>,
    live_source: Arc<HitomiLiveAdapter>,
    data_dir: PathBuf,
    search_pages: SearchPageRequests,
    maintenance_previews: Mutex<HashMap<String, MaintenanceAction>>,
    managed_work: ManagedWorkGate,
}

#[derive(Default)]
struct SearchPageRequests {
    inner: Mutex<SearchPageRequestsInner>,
}

#[derive(Default)]
struct SearchPageRequestsInner {
    active: HashMap<String, CancellationToken>,
    cancelled: HashSet<String>,
    cancelled_order: VecDeque<String>,
}

impl SearchPageRequests {
    fn start(&self, request_id: &str) -> CancellationToken {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let token = CancellationToken::new();
        if inner.cancelled.remove(request_id) {
            inner
                .cancelled_order
                .retain(|candidate| candidate != request_id);
            token.cancel();
        }
        if let Some(previous) = inner.active.insert(request_id.to_owned(), token.clone()) {
            previous.cancel();
        }
        token
    }

    fn cancel(&self, request_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(token) = inner.active.get(request_id) {
            token.cancel();
            return true;
        }
        if inner.cancelled.insert(request_id.to_owned()) {
            inner.cancelled_order.push_back(request_id.to_owned());
        }
        while inner.cancelled_order.len() > 256 {
            if let Some(oldest) = inner.cancelled_order.pop_front() {
                inner.cancelled.remove(&oldest);
            }
        }
        true
    }

    fn finish(&self, request_id: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.active.remove(request_id);
        inner.cancelled.remove(request_id);
        inner
            .cancelled_order
            .retain(|candidate| candidate != request_id);
    }

    fn cancel_all(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        for token in inner.active.values() {
            token.cancel();
        }
        inner.active.clear();
        inner.cancelled.clear();
        inner.cancelled_order.clear();
    }
}

impl AppState {
    // This is the single composition root for application services and ports.
    // Keeping dependencies explicit here makes production/test wiring auditable.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        service: ApplicationService,
        thumbnails: ThumbnailCoordinator,
        thumbnail_completions: Sender<ThumbnailCompletionEventDto>,
        detail_originals: DetailOriginalSupervisor,
        downloads: DownloadSupervisor,
        auto_find: AutoFindSupervisor,
        duplicates: DuplicateSupervisor,
        internal_duplicates: InternalDuplicateSupervisor,
        download_root_picker: Arc<dyn DownloadRootPicker>,
        artifact_store: Arc<dyn ArtifactStore>,
        live_source: Arc<HitomiLiveAdapter>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            service,
            thumbnails,
            thumbnail_completions,
            detail_originals,
            downloads,
            auto_find,
            duplicates,
            internal_duplicates,
            download_root_picker,
            artifact_store,
            live_source,
            data_dir,
            search_pages: SearchPageRequests::default(),
            maintenance_previews: Mutex::new(HashMap::new()),
            managed_work: ManagedWorkGate::default(),
        }
    }

    pub(crate) fn active_work_snapshot(&self) -> Result<AppActiveWorkSnapshot, ApplicationError> {
        let _control = self.managed_work.inner.control.lock().map_err(|_| {
            crate::application::RepositoryError::Other(
                "managed work gate mutex was poisoned".into(),
            )
        })?;
        self.active_work_snapshot_locked()
    }

    fn active_work_snapshot_locked(&self) -> Result<AppActiveWorkSnapshot, ApplicationError> {
        let mut active_download_ids = self.service.download_active_entry_ids()?;
        active_download_ids.sort();
        active_download_ids.dedup();
        let auto_find = self.auto_find.active_run_snapshot()?;
        let duplicate_scan = self.duplicates.active_run_snapshot()?;
        let internal_duplicate_scan = self.internal_duplicates.active_run_snapshot()?;
        let work_set_fingerprint = active_work_fingerprint(
            active_download_ids.iter().map(|entry_id| entry_id.as_str()),
            auto_find.as_ref().map(|run| run.run_id.as_str()),
            duplicate_scan.as_ref().map(|run| run.run_id.as_str()),
            internal_duplicate_scan
                .as_ref()
                .map(|run| run.run_id.as_str()),
        );
        Ok(AppActiveWorkSnapshot {
            queried_at: now_unix_ms(),
            work_set_fingerprint,
            downloads: AppActiveDownloadsSnapshot {
                active_count: u64::try_from(active_download_ids.len()).unwrap_or(u64::MAX),
            },
            auto_find: auto_find.map(|run| AppActiveAutoFindSnapshot {
                run_id: run.run_id,
                completed_favorites: run.completed_favorites,
                total_favorites: run.total_favorites,
                candidates_found: run.candidates_found,
            }),
            duplicate_scan: duplicate_scan.map(|run| AppActiveDuplicateScanSnapshot {
                run_id: run.run_id,
                hashed_artifacts: run.hashed_artifacts,
                total_artifacts: run.total_artifacts,
                compared_pairs: run.compared_pairs,
                total_pairs: run.total_pairs,
                candidates_found: run.candidates_found,
            }),
            internal_duplicate_scan: internal_duplicate_scan.map(|run| {
                AppActiveInternalDuplicateScanSnapshot {
                    run_id: run.run_id,
                    scanned_artifacts: run.scanned_artifacts,
                    total_artifacts: run.total_artifacts,
                    skipped_artifacts: run.skipped_artifacts,
                    groups_found: run.groups_found,
                }
            }),
        })
    }

    pub(crate) fn request_graceful_quit(
        &self,
        app: AppHandle,
        request: AppQuitRequest,
    ) -> Result<AppQuitResult, ApiError> {
        let (result, should_shutdown) = {
            let (mut control, gate_status_unknown) = match self.managed_work.inner.control.lock() {
                Ok(control) => (control, false),
                Err(error) if request.force_when_status_unknown => {
                    tracing::warn!(
                        "managed work gate was unavailable; honoring explicit forced quit"
                    );
                    (error.into_inner(), true)
                }
                Err(_) => {
                    return Err(active_work_status_error("managed_work_gate_unavailable"));
                }
            };
            if self.managed_work.inner.quitting.load(Ordering::Acquire) {
                self.managed_work
                    .evaluate_quit_locked(&mut control, &request, None)
            } else {
                let snapshot = if gate_status_unknown {
                    None
                } else {
                    match self.active_work_snapshot_locked() {
                        Ok(snapshot) => Some(snapshot),
                        Err(error) if request.force_when_status_unknown => {
                            let api = ApiError::from(error);
                            tracing::warn!(
                                error_code = %api.code,
                                "active work status unavailable; honoring explicit forced quit"
                            );
                            None
                        }
                        Err(error) => {
                            let api = ApiError::from(error);
                            tracing::warn!(
                                error_code = %api.code,
                                "active work status could not be checked before quit"
                            );
                            return Err(active_work_status_error(&api.code));
                        }
                    }
                };
                self.managed_work
                    .evaluate_quit_locked(&mut control, &request, snapshot)
            }
        };
        if should_shutdown {
            self.spawn_graceful_shutdown(app);
        }
        Ok(result)
    }

    fn spawn_graceful_shutdown(&self, app: AppHandle) {
        let downloads = self.downloads.clone();
        let auto_find = self.auto_find.clone();
        let duplicates = self.duplicates.clone();
        let internal_duplicates = self.internal_duplicates.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
                crate::shutdown_all_then_exit(
                    || internal_duplicates.shutdown_and_wait(),
                    || duplicates.shutdown_and_wait(),
                    || auto_find.shutdown_and_wait(),
                    || downloads.shutdown_and_wait(),
                    || app.exit(0),
                );
            })
            .await
            {
                tracing::warn!(error = %error, "background workers did not finish shutdown cleanly");
            }
        });
    }

    fn managed_work(&self) -> ManagedWorkGate {
        self.managed_work.clone()
    }

    fn remember_maintenance_preview(&self, action: MaintenanceAction) -> String {
        let id = format!("maintenance-{}", uuid::Uuid::new_v4());
        self.maintenance_previews
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), action);
        id
    }

    fn consume_maintenance_preview(&self, preview_id: &str, action: &MaintenanceAction) -> bool {
        self.maintenance_previews
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(preview_id)
            .is_some_and(|previewed| previewed == *action)
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn detail_original_prepare(
    state: State<'_, AppState>,
    request: DetailOriginalPrepareRequest,
) -> Result<ApiResult<DetailOriginalPrepared>, ApiError> {
    let originals = state.detail_originals.clone();
    let failure_context = request.clone();
    Ok(
        match tauri::async_runtime::spawn_blocking(move || originals.prepare(request)).await {
            Ok(Ok(prepared)) => ApiResult::success(prepared),
            Ok(Err(error)) => {
                ApiResult::failure(detail_original_api_error(error, &failure_context))
            }
            Err(error) => {
                tracing::error!(error = %error, "detail original prepare task did not complete");
                ApiResult::failure(ApiError {
                    code: "BACKEND_TASK_FAILED".into(),
                    message: "The backend could not complete the request".into(),
                    retryable: true,
                    action: Some(super::ApiAction::Retry),
                    details: None,
                })
            }
        },
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn detail_original_dispose(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<ApiResult<bool>, ApiError> {
    Ok(ApiResult::success(
        state.detail_originals.dispose(&request_id),
    ))
}

impl AppState {
    pub(crate) fn detail_original_media_file(
        &self,
        request_id: &str,
    ) -> Option<(std::path::PathBuf, String)> {
        self.detail_originals.media_file(request_id)
    }
}

fn detail_original_api_error(
    error: DetailOriginalError,
    request: &DetailOriginalPrepareRequest,
) -> ApiError {
    use serde_json::json;
    let source_code = match &error {
        DetailOriginalError::SourceFailed { source_code } => Some(source_code.clone()),
        _ => None,
    };
    let stage = match &error {
        DetailOriginalError::InvalidRequest => "validation",
        DetailOriginalError::Cancelled => "cancelled",
        DetailOriginalError::SourceFailed { .. } => "source",
        DetailOriginalError::ConversionFailed => "conversion",
        DetailOriginalError::WriteFailed => "write",
        DetailOriginalError::Unavailable => "finalize",
        DetailOriginalError::ArtifactUnavailable => "artifact",
    };
    let mut details = std::collections::BTreeMap::from([
        ("requestId".into(), json!(request.request_id)),
        ("galleryId".into(), json!(request.gallery_id)),
        ("sourcePage".into(), json!(request.source_page)),
        ("stage".into(), json!(stage)),
    ]);
    if let Some(source_code) = source_code {
        details.insert("sourceCode".into(), json!(source_code));
    }
    if let Some(entry_id) = request.entry_id.as_ref() {
        details.insert("entryId".into(), json!(entry_id));
    }
    ApiError {
        code: error.code().into(),
        message: error.to_string(),
        retryable: false,
        action: Some(super::ApiAction::None),
        details: Some(details),
    }
}

#[tauri::command]
pub async fn favorites_list(
    state: State<'_, AppState>,
) -> Result<ApiResult<Vec<FavoriteRecord>>, ApiError> {
    Ok(state.service.favorites_list().into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn favorite_set(
    state: State<'_, AppState>,
    key: FavoriteKey,
    enabled: bool,
) -> Result<ApiResult<FavoriteMutationResult>, ApiError> {
    Ok(state.service.favorite_set(key, enabled).into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_history_list(
    state: State<'_, AppState>,
    limit: u32,
) -> Result<ApiResult<Vec<SearchHistoryEntry>>, ApiError> {
    Ok(state.service.search_history_list(limit).into())
}

#[tauri::command]
pub async fn tag_catalog_status(
    state: State<'_, AppState>,
) -> Result<ApiResult<TagCatalogStatus>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("tag_catalog_status", move || service.tag_catalog_status()).await)
}

#[tauri::command]
pub async fn tag_catalog_refresh(
    state: State<'_, AppState>,
) -> Result<ApiResult<TagCatalogStatus>, ApiError> {
    let service = state.service.clone();
    Ok(
        run_application_blocking("tag_catalog_refresh", move || service.tag_catalog_refresh())
            .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn tag_suggestions_search(
    state: State<'_, AppState>,
    request: TagSuggestionRequest,
) -> Result<ApiResult<Vec<TagSuggestion>>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("tag_suggestions_search", move || {
        service.tag_suggestions_search(request)
    })
    .await)
}

#[tauri::command]
pub async fn auto_find_snapshot(
    state: State<'_, AppState>,
) -> Result<ApiResult<AutoFindSnapshot>, ApiError> {
    Ok(state.service.auto_find_snapshot().into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn exploration_data_reset(
    state: State<'_, AppState>,
    request: ExplorationDataResetRequest,
) -> Result<ApiResult<ExplorationDataResetResult>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("exploration_data_reset", move || {
        service.exploration_data_reset(request)
    })
    .await)
}

#[tauri::command]
pub async fn auto_find_refresh(
    state: State<'_, AppState>,
) -> Result<ApiResult<AutoFindRun>, ApiError> {
    let managed_work = state.managed_work();
    let auto_find = state.auto_find.clone();
    Ok(run_application_blocking("auto_find_refresh", move || {
        prepare_then_commit_managed_work(
            &managed_work,
            || auto_find.prepare_refresh(),
            |prepared| auto_find.commit_refresh(prepared),
        )
    })
    .await)
}

#[tauri::command]
pub async fn auto_find_cancel(
    state: State<'_, AppState>,
) -> Result<ApiResult<AutoFindRun>, ApiError> {
    Ok(state.auto_find.cancel().into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auto_find_exclude(
    state: State<'_, AppState>,
    gallery_ids: Vec<i64>,
    reason: String,
) -> Result<ApiResult<AutoFindExclusionResult>, ApiError> {
    Ok(state.service.auto_find_exclude(gallery_ids, reason).into())
}

#[tauri::command]
pub async fn exploration_exclusions_list(
    state: State<'_, AppState>,
) -> Result<ApiResult<Vec<crate::domain::ExplorationExclusion>>, ApiError> {
    let service = state.service.clone();
    Ok(
        run_application_blocking("exploration_exclusions_list", move || {
            service.exploration_exclusions_list()
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn exploration_exclusions_restore(
    state: State<'_, AppState>,
    gallery_ids: Vec<i64>,
) -> Result<ApiResult<crate::domain::ExplorationExclusionRestoreResult>, ApiError> {
    let service = state.service.clone();
    Ok(
        run_application_blocking("exploration_exclusions_restore", move || {
            service.exploration_exclusions_restore(gallery_ids)
        })
        .await,
    )
}

#[tauri::command]
pub async fn duplicate_snapshot(
    state: State<'_, AppState>,
) -> Result<ApiResult<DuplicateSnapshot>, ApiError> {
    let duplicates = state.duplicates.clone();
    Ok(run_application_blocking("duplicate_snapshot", move || duplicates.snapshot()).await)
}

#[tauri::command]
pub async fn duplicate_scan_start(
    state: State<'_, AppState>,
) -> Result<ApiResult<DuplicateScanRun>, ApiError> {
    let managed_work = state.managed_work();
    let duplicates = state.duplicates.clone();
    Ok(run_application_blocking("duplicate_scan_start", move || {
        prepare_then_commit_managed_work(
            &managed_work,
            || duplicates.prepare_start(),
            |prepared| duplicates.commit_start(prepared),
        )
    })
    .await)
}

#[tauri::command]
pub async fn duplicate_scan_cancel(
    state: State<'_, AppState>,
) -> Result<ApiResult<DuplicateScanRun>, ApiError> {
    let duplicates = state.duplicates.clone();
    Ok(run_application_blocking("duplicate_scan_cancel", move || duplicates.cancel()).await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn duplicate_review_get(
    state: State<'_, AppState>,
    candidate_id: String,
) -> Result<ApiResult<DuplicateReview>, ApiError> {
    let duplicates = state.duplicates.clone();
    Ok(run_application_blocking("duplicate_review_get", move || {
        duplicates.review_get(&candidate_id)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn duplicate_decision_apply(
    state: State<'_, AppState>,
    request: DuplicateDecisionRequest,
) -> Result<ApiResult<DuplicateReview>, ApiError> {
    let duplicates = state.duplicates.clone();
    Ok(
        run_application_blocking("duplicate_decision_apply", move || {
            duplicates.decision_apply(request)
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_overlap_review_get(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<ApiResult<DownloadOverlapReview>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(
        run_application_blocking("download_overlap_review_get", move || {
            downloads
                .overlap_review_get(&review_id)?
                .ok_or(ApplicationError::DownloadOverlapReviewNotFound(review_id))
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_overlap_decision_apply(
    state: State<'_, AppState>,
    request: DownloadOverlapDecisionRequest,
) -> Result<ApiResult<DownloadOverlapDecisionResult>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(
        run_application_blocking("download_overlap_decision_apply", move || {
            downloads.overlap_decision_apply(request)
        })
        .await,
    )
}

#[tauri::command]
pub async fn internal_duplicate_snapshot(
    state: State<'_, AppState>,
) -> Result<ApiResult<InternalDuplicateSnapshot>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(
        run_application_blocking("internal_duplicate_snapshot", move || supervisor.snapshot())
            .await,
    )
}

#[tauri::command]
pub async fn internal_duplicate_active_artifact(
    state: State<'_, AppState>,
) -> Result<ApiResult<Option<InternalArtifactScanProgress>>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(
        run_application_blocking("internal_duplicate_active_artifact", move || {
            supervisor.active_artifact_progress()
        })
        .await,
    )
}

#[tauri::command]
pub async fn internal_duplicate_scan_start(
    state: State<'_, AppState>,
    request: InternalScanRequest,
) -> Result<ApiResult<InternalScanRun>, ApiError> {
    let managed_work = state.managed_work();
    let supervisor = state.internal_duplicates.clone();
    Ok(
        run_application_blocking("internal_duplicate_scan_start", move || {
            prepare_then_commit_managed_work(
                &managed_work,
                || supervisor.prepare_start(request),
                |prepared| supervisor.commit_start(prepared),
            )
        })
        .await,
    )
}

#[tauri::command]
pub async fn internal_duplicate_scan_cancel(
    state: State<'_, AppState>,
) -> Result<ApiResult<InternalScanRun>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(
        run_application_blocking("internal_duplicate_scan_cancel", move || {
            supervisor.cancel()
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn internal_duplicate_review_get(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<ApiResult<InternalDuplicateReview>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(
        run_application_blocking("internal_duplicate_review_get", move || {
            supervisor.review_get(&entry_id)
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn internal_removal_plan(
    state: State<'_, AppState>,
    request: InternalRemovalPlanRequest,
) -> Result<ApiResult<InternalRemovalPlan>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(run_application_blocking("internal_removal_plan", move || {
        supervisor.removal_plan(request)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn internal_removal_apply(
    state: State<'_, AppState>,
    request: InternalRemovalApplyRequest,
) -> Result<ApiResult<InternalRemovalResult>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(run_application_blocking("internal_removal_apply", move || {
        supervisor.removal_apply(request)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn internal_removal_undo(
    state: State<'_, AppState>,
    request: InternalRemovalUndoRequest,
) -> Result<ApiResult<InternalRemovalResult>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    Ok(run_application_blocking("internal_removal_undo", move || {
        supervisor.removal_undo(request)
    })
    .await)
}

#[tauri::command]
pub async fn settings_get(
    state: State<'_, AppState>,
) -> Result<ApiResult<SettingsSnapshot>, ApiError> {
    Ok(state.service.settings_get().into())
}

#[tauri::command]
pub async fn storage_usage_get(
    state: State<'_, AppState>,
) -> Result<ApiResult<StorageUsageSnapshot>, ApiError> {
    let settings = match state.service.settings_get() {
        Ok(settings) => settings,
        Err(error) => return Ok(ApiResult::failure(error.into())),
    };
    let data_dir = state.data_dir.clone();
    let download_root =
        (!settings.download_root.trim().is_empty()).then(|| PathBuf::from(settings.download_root));
    let memory_cache_bytes =
        u64::try_from(state.thumbnails.stats().success_cache_bytes).unwrap_or(u64::MAX);
    Ok(
        match tauri::async_runtime::spawn_blocking(move || {
            collect_storage_usage(&data_dir, download_root.as_deref(), memory_cache_bytes)
        })
        .await
        {
            Ok(snapshot) => ApiResult::success(snapshot),
            Err(error) => ApiResult::failure(blocking_task_error("storage_usage_get", &error)),
        },
    )
}

#[tauri::command]
pub async fn folder_name_template_preview(
    state: State<'_, AppState>,
    template: String,
) -> Result<ApiResult<String>, ApiError> {
    Ok(state.service.folder_name_template_preview(&template).into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn settings_update(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
    expected_revision: u64,
) -> Result<ApiResult<SettingsSnapshot>, ApiError> {
    match state.service.settings_update(patch, expected_revision) {
        Ok(snapshot) => {
            if let Err(error) = state.thumbnails.reconfigure(ThumbnailRuntimeConfigDto {
                concurrent_image_requests: snapshot.concurrent_image_requests,
                request_start_interval_ms: snapshot.request_start_interval_ms,
            }) {
                tracing::warn!(error = %error, "could not apply thumbnail worker settings");
            }
            if let Err(error) = app.emit("settings:changed", &snapshot) {
                tracing::warn!(error = %error, "could not emit settings:changed");
            }
            Ok(ApiResult::success(snapshot))
        }
        Err(error) => Ok(ApiResult::failure(error.into())),
    }
}

#[tauri::command]
pub async fn window_placement_get(
    state: State<'_, AppState>,
) -> Result<ApiResult<WindowPlacementSnapshot>, ApiError> {
    Ok(state.service.window_placement_get().into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn window_placement_update(
    state: State<'_, AppState>,
    placement: WindowPlacement,
    expected_revision: u64,
) -> Result<ApiResult<WindowPlacementSnapshot>, ApiError> {
    Ok(state
        .service
        .window_placement_update(placement, expected_revision)
        .into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_submit(
    state: State<'_, AppState>,
    request: SearchRequest,
) -> Result<ApiResult<SearchSubmission>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("search_submit", move || service.search_submit(request)).await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_queue_add(
    app: AppHandle,
    state: State<'_, AppState>,
    galleries: Vec<i64>,
    request_id: String,
) -> Result<ApiResult<Vec<DownloadEntry>>, ApiError> {
    if let Err(error) = ensure_download_root(
        &app,
        state.service.clone(),
        Arc::clone(&state.download_root_picker),
        Arc::clone(&state.artifact_store),
    )
    .await
    {
        return Ok(ApiResult::failure(error));
    }
    let managed_work = state.managed_work();
    match managed_work.run(|| {
        let launch = state.service.download_queue_add(galleries, request_id)?;
        state
            .downloads
            .enqueue_all(launch.jobs)
            .map_err(ApplicationError::from)?;
        Ok(launch.entries)
    }) {
        Ok(entries) => Ok(ApiResult::success(entries)),
        Err(error) => Ok(ApiResult::failure(error.into())),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_entries_list(
    state: State<'_, AppState>,
    request: DownloadListRequest,
) -> Result<ApiResult<DownloadPage>, ApiError> {
    Ok(state.service.download_entries_list(request).into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_library_page_list(
    state: State<'_, AppState>,
    request: DownloadListRequest,
) -> Result<ApiResult<DownloadLibraryPage>, ApiError> {
    let service = state.service.clone();
    Ok(
        run_application_blocking("download_library_page_list", move || {
            service.download_library_page_list(request)
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn thumbnail_request(
    state: State<'_, AppState>,
    request: ThumbnailRequestDto,
) -> Result<ApiResult<ThumbnailRequestTokenDto>, ApiError> {
    match state
        .thumbnails
        .request_with_completion(request, state.thumbnail_completions.clone())
    {
        Ok(token) => Ok(ApiResult::success(token)),
        Err(error) => Ok(ApiResult::failure(thumbnail_coordinator_error(error))),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub fn thumbnail_cancel(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<ApiResult<bool>, ApiError> {
    Ok(ApiResult::success(
        state.thumbnails.cancel(request_id.trim()),
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub fn thumbnail_reprioritize(
    state: State<'_, AppState>,
    request_id: String,
    priority: ThumbnailPriority,
) -> Result<ApiResult<bool>, ApiError> {
    Ok(ApiResult::success(
        state.thumbnails.reprioritize(request_id.trim(), priority),
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub fn thumbnail_invalidate(
    state: State<'_, AppState>,
    key: ThumbnailKey,
) -> Result<ApiResult<ThumbnailInvalidationDto>, ApiError> {
    match state.thumbnails.invalidate(&key) {
        Ok(result) => Ok(ApiResult::success(result)),
        Err(error) => Ok(ApiResult::failure(ApiError {
            code: "THUMBNAIL_REQUEST_INVALID".into(),
            message: error.to_string(),
            retryable: false,
            action: Some(super::ApiAction::None),
            details: None,
        })),
    }
}

#[tauri::command]
pub fn thumbnail_stats(
    state: State<'_, AppState>,
) -> Result<ApiResult<ThumbnailWorkerStatsDto>, ApiError> {
    Ok(ApiResult::success(state.thumbnails.stats()))
}

#[tauri::command]
pub fn thumbnail_cache_clear(
    state: State<'_, AppState>,
) -> Result<ApiResult<ThumbnailCacheClearDto>, ApiError> {
    Ok(ApiResult::success(state.thumbnails.clear_cache()))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_retry(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
) -> Result<ApiResult<Vec<JobRef>>, ApiError> {
    let managed_work = state.managed_work();
    match managed_work.run(|| {
        let job_refs = state.service.download_retry(entry_ids)?;
        state
            .downloads
            .enqueue_retries(&job_refs)
            .map_err(ApplicationError::from)?;
        Ok(job_refs)
    }) {
        Ok(job_refs) => Ok(ApiResult::success(job_refs)),
        Err(error) => Ok(ApiResult::failure(error.into())),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
) -> Result<ApiResult<Vec<DownloadEntry>>, ApiError> {
    let cancellation_ids = entry_ids.clone();
    match state.service.download_cancel(entry_ids) {
        Ok(entries) => {
            state.downloads.cancel_entries(&cancellation_ids);
            for entry in &entries {
                let event = DownloadChangedEvent {
                    entry_id: entry.entry_id.to_string(),
                    gallery_id: entry.gallery_id.get(),
                    revision: entry.revision,
                    state: entry.state,
                    progress: entry.progress,
                    attempt: entry.attempt,
                    error_code: entry.error_code.clone(),
                    error_message: entry.error_message.clone(),
                    review_kind: entry.review_kind,
                    review_id: entry.review_id.clone(),
                };
                if let Err(error) = app.emit("download:changed", event) {
                    tracing::warn!(
                        entry_id = %entry.entry_id,
                        error = %error,
                        "could not emit download:changed"
                    );
                }
            }
            Ok(ApiResult::success(entries))
        }
        Err(error) => Ok(ApiResult::failure(error.into())),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn artifact_open_first(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<ApiResult<()>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(run_application_blocking("artifact_open_first", move || {
        downloads.open_first(entry_id)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn artifact_open_folder(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<ApiResult<()>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(run_application_blocking("artifact_open_folder", move || {
        downloads.open_folder(entry_id)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_quarantine(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
    reason: String,
) -> Result<ApiResult<Vec<DownloadEntry>>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(run_application_blocking("download_quarantine", move || {
        downloads.quarantine_entries(entry_ids, reason)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_quarantine_undo(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
) -> Result<ApiResult<Vec<DownloadEntry>>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(
        run_application_blocking("download_quarantine_undo", move || {
            downloads.restore_entries(entry_ids)
        })
        .await,
    )
}

#[tauri::command]
pub async fn app_reconcile(
    state: State<'_, AppState>,
) -> Result<ApiResult<ReconcileReport>, ApiError> {
    let managed_work = state.managed_work();
    let downloads = state.downloads.clone();
    Ok(run_application_blocking("app_reconcile", move || {
        let mut report = downloads.reconcile_without_resume()?;
        managed_work.run(|| downloads.resume_after_reconcile(&mut report))?;
        Ok(report)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn maintenance_preview(
    state: State<'_, AppState>,
    action: MaintenanceAction,
) -> Result<ApiResult<MaintenancePreview>, ApiError> {
    if let Err(error) = action.validate() {
        return Ok(ApiResult::failure(ApplicationError::from(error).into()));
    }
    let preview_id = state.remember_maintenance_preview(action.clone());
    let (original_files_deleted, user_decisions_preserved, restart_required, steps, warnings) =
        match &action {
            MaintenanceAction::QuickRepair => (
                false,
                true,
                false,
                vec![
                    "완료된 썸네일·검색 cache를 비웁니다".into(),
                    "중단된 다운로드와 검사 작업을 안전하게 복구합니다".into(),
                    "보류된 격리·복원 작업을 다시 확인합니다".into(),
                ],
                vec!["유효한 HTTP host cooldown과 Retry-After는 유지됩니다".into()],
            ),
            MaintenanceAction::RebuildLibrary { .. } => (
                false,
                true,
                false,
                vec![
                    "SQLite artifact, manifest와 저장 파일을 검사합니다".into(),
                    "선택한 파생 분석만 다시 실행합니다".into(),
                ],
                vec![
                    "모호한 final/.part 충돌은 덮어쓰거나 삭제하지 않고 recovery로 보냅니다".into(),
                ],
            ),
            MaintenanceAction::FactoryReset { .. } => (
                false,
                false,
                true,
                vec![
                    "모든 worker를 취소하고 종료합니다".into(),
                    "다음 시작 전에 앱 SQLite 상태를 recovery backup으로 옮깁니다".into(),
                    "새 SQLite DB와 기본 설정으로 다시 시작합니다".into(),
                ],
                vec!["외부 download root와 quarantine/recovery 원본 파일은 유지됩니다".into()],
            ),
        };
    Ok(ApiResult::success(MaintenancePreview {
        preview_id,
        action,
        original_files_deleted,
        user_decisions_preserved,
        restart_required,
        warnings,
        steps,
    }))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn maintenance_execute(
    app: AppHandle,
    state: State<'_, AppState>,
    preview_id: String,
    action: MaintenanceAction,
) -> Result<ApiResult<MaintenanceResult>, ApiError> {
    if let Err(error) = action.validate() {
        return Ok(ApiResult::failure(ApplicationError::from(error).into()));
    }
    if !state.consume_maintenance_preview(preview_id.trim(), &action) {
        return Ok(ApiResult::failure(
            ApplicationError::from(ValidationError::new(
                "previewId",
                "a matching maintenance preview is required before execution",
            ))
            .into(),
        ));
    }
    if matches!(action, MaintenanceAction::QuickRepair) {
        state.search_pages.cancel_all();
    }

    let thumbnails = state.thumbnails.clone();
    let live_source = Arc::clone(&state.live_source);
    let downloads = state.downloads.clone();
    let auto_find = state.auto_find.clone();
    let duplicates = state.duplicates.clone();
    let internal_duplicates = state.internal_duplicates.clone();
    let data_dir = state.data_dir.clone();
    let managed_work = state.managed_work();
    let execute_action = action.clone();
    let result = run_application_blocking("maintenance_execute", move || {
        let mut completed_steps = Vec::new();
        let mut warnings = Vec::new();
        match &execute_action {
            MaintenanceAction::QuickRepair => {
                thumbnails.clear_cache();
                live_source.clear_derived_caches();
                let mut download_recovery = downloads.recover_startup_state_without_resume()?;
                auto_find.recover_interrupted()?;
                duplicates.recover_interrupted()?;
                internal_duplicates.recover_interrupted()?;
                match internal_duplicates.reconcile_pending_page_moves() {
                    Ok(_) => {}
                    Err(error) => warnings.push(format!("internal page recovery deferred: {error}")),
                }
                completed_steps.extend([
                    "thumbnail and source caches cleared".into(),
                    "interrupted work recovery completed".into(),
                ]);
                managed_work
                    .run(|| downloads.resume_after_reconcile(&mut download_recovery))?;
                Ok(MaintenanceResult { action: execute_action.clone(), completed_steps, warnings, restart_required: false })
            }
            MaintenanceAction::RebuildLibrary {
                rebuild_thumbnail_data,
                rebuild_duplicate_analysis,
                rebuild_internal_analysis,
                rebuild_auto_find_results,
            } => {
                let mut report = downloads.reconcile_without_resume()?;
                completed_steps.push(format!("{} artifacts inspected", report.inspected_artifacts));
                if *rebuild_thumbnail_data {
                    thumbnails.clear_cache();
                    live_source.clear_derived_caches();
                    completed_steps.push("thumbnail derived caches cleared".into());
                }
                let duplicate_prepared = rebuild_duplicate_analysis
                    .then(|| duplicates.prepare_start())
                    .transpose()?;
                let internal_prepared = rebuild_internal_analysis
                    .then(|| internal_duplicates.prepare_start_all())
                    .transpose()?;
                let auto_find_prepared = rebuild_auto_find_results
                    .then(|| auto_find.prepare_refresh())
                    .transpose()?;
                managed_work.run(|| {
                    downloads.resume_after_reconcile(&mut report)?;
                    if let Some(prepared) = duplicate_prepared {
                        duplicates.commit_start(prepared)?;
                        completed_steps.push("gallery duplicate analysis started".into());
                    }
                    if let Some(prepared) = internal_prepared {
                        internal_duplicates.commit_start(prepared)?;
                        completed_steps.push("internal duplicate analysis started".into());
                    }
                    if let Some(prepared) = auto_find_prepared {
                        auto_find.commit_refresh(prepared)?;
                        completed_steps.push("Auto Find refresh started".into());
                    }
                    Ok(())
                })?;
                Ok(MaintenanceResult { action: execute_action.clone(), completed_steps, warnings, restart_required: false })
            }
            MaintenanceAction::FactoryReset { .. } => {
                internal_duplicates.shutdown_and_wait();
                duplicates.shutdown_and_wait();
                auto_find.shutdown_and_wait();
                downloads.shutdown_and_wait();
                std::fs::write(data_dir.join("factory-reset.pending"), b"v1\n")
                    .map_err(|error| ApplicationError::from(crate::application::RepositoryError::Other(format!("could not schedule factory reset: {error}"))))?;
                Ok(MaintenanceResult {
                    action: execute_action,
                    completed_steps: vec!["factory reset scheduled for the next startup".into()],
                    warnings: vec!["the app will now exit; external originals are unchanged".into()],
                    restart_required: true,
                })
            }
        }
    }).await;
    if matches!(action, MaintenanceAction::FactoryReset { .. })
        && matches!(result, ApiResult::Success(_))
    {
        app.exit(0);
    }
    Ok(result)
}

#[tauri::command]
pub async fn app_minimize_to_tray(window: WebviewWindow) -> Result<ApiResult<()>, ApiError> {
    match crate::minimize_to_tray(|| window.hide()) {
        Ok(()) => Ok(ApiResult::success(())),
        Err(error) => Ok(ApiResult::failure(ApiError {
            code: "WINDOW_HIDE_FAILED".into(),
            message: format!("could not hide Atsumi to the tray: {error}"),
            retryable: true,
            action: Some(super::ApiAction::Retry),
            details: None,
        })),
    }
}

#[tauri::command]
pub fn app_active_work_snapshot(
    state: State<'_, AppState>,
) -> Result<ApiResult<AppActiveWorkSnapshot>, ApiError> {
    Ok(match state.active_work_snapshot() {
        Ok(snapshot) => ApiResult::success(snapshot),
        Err(error) => {
            let source = ApiError::from(error);
            tracing::warn!(error_code = %source.code, "could not inspect active work");
            ApiResult::failure(active_work_status_error(&source.code))
        }
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn app_quit(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AppQuitRequest,
) -> Result<ApiResult<AppQuitResult>, ApiError> {
    Ok(match state.request_graceful_quit(app, request) {
        Ok(result) => ApiResult::success(result),
        Err(error) => ApiResult::failure(error),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_page_get(
    state: State<'_, AppState>,
    query_id: String,
    page: u32,
    request_id: String,
) -> Result<ApiResult<GalleryPage>, ApiError> {
    let request_id = request_id.trim().to_owned();
    if request_id.is_empty() || request_id.len() > 200 {
        return Ok(ApiResult::failure(
            ApplicationError::from(ValidationError::new(
                "requestId",
                "must contain between 1 and 200 bytes",
            ))
            .into(),
        ));
    }
    let cancellation = state.search_pages.start(&request_id);
    let service = state.service.clone();
    let result = run_application_blocking("search_page_get", move || {
        service.search_page_get_cancellable(query_id, page, &cancellation)
    })
    .await;
    state.search_pages.finish(&request_id);
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn search_page_cancel(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<ApiResult<bool>, ApiError> {
    let request_id = request_id.trim();
    if request_id.is_empty() || request_id.len() > 200 {
        return Ok(ApiResult::failure(
            ApplicationError::from(ValidationError::new(
                "requestId",
                "must contain between 1 and 200 bytes",
            ))
            .into(),
        ));
    }
    Ok(ApiResult::success(state.search_pages.cancel(request_id)))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn gallery_detail_get(
    state: State<'_, AppState>,
    gallery_id: i64,
) -> Result<ApiResult<GalleryDetail>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("gallery_detail_get", move || {
        service.gallery_detail_get(gallery_id)
    })
    .await)
}

async fn ensure_download_root(
    app: &AppHandle,
    service: ApplicationService,
    picker: Arc<dyn DownloadRootPicker>,
    store: Arc<dyn ArtifactStore>,
) -> Result<(), ApiError> {
    let current = service.settings_get().map_err(ApiError::from)?;
    if !current.download_root.trim().is_empty() {
        let root = PathBuf::from(current.download_root);
        return match tauri::async_runtime::spawn_blocking(move || {
            store.validate_download_root(&root)
        })
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(ApplicationError::from(error).into()),
            Err(error) => Err(blocking_task_error("download_root_validate", &error)),
        };
    }

    let selected = match tauri::async_runtime::spawn_blocking(move || {
        let selected = picker.pick_download_root()?;
        selected
            .map(|path| store.validate_download_root(&path))
            .transpose()
    })
    .await
    {
        Ok(Ok(selected)) => selected,
        Ok(Err(error)) => return Err(ApplicationError::from(error).into()),
        Err(error) => return Err(blocking_task_error("download_root_choose", &error)),
    };
    let Some(selected) = selected else {
        return Err(ApplicationError::from(DownloadPipelineError::new(
            DownloadPipelineErrorCode::RootSelectionCancelled,
            "Download folder selection was cancelled; no queue entry was created",
            false,
        ))
        .into());
    };
    let selected = selected.to_str().ok_or_else(|| {
        ApiError::from(ApplicationError::from(DownloadPipelineError::new(
            DownloadPipelineErrorCode::RootUnavailable,
            "The selected folder path cannot be represented safely",
            false,
        )))
    })?;
    let updated = service
        .settings_update(
            SettingsPatch {
                download_root: Some(selected.to_owned()),
                ..SettingsPatch::default()
            },
            current.revision,
        )
        .map_err(ApiError::from)?;
    if let Err(error) = app.emit("settings:changed", &updated) {
        tracing::warn!(error = %error, "could not emit settings:changed after folder selection");
    }
    Ok(())
}

fn blocking_task_error(operation_id: &'static str, error: &tauri::Error) -> ApiError {
    tracing::error!(operation_id, error = %error, "blocking backend task did not complete");
    ApiError {
        code: "BACKEND_TASK_FAILED".into(),
        message: "The backend could not complete the request".into(),
        retryable: true,
        action: Some(super::ApiAction::Retry),
        details: None,
    }
}

async fn run_application_blocking<T, F>(operation_id: &'static str, operation: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApplicationError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(operation).await {
        Ok(result) => result.into(),
        Err(error) => {
            let (cancelled, panicked) = match &error {
                tauri::Error::JoinError(join_error) => {
                    (join_error.is_cancelled(), join_error.is_panic())
                }
                _ => (false, false),
            };
            tracing::error!(
                operation_id,
                cancelled,
                panicked,
                "blocking application task did not complete"
            );
            ApiResult::failure(ApiError {
                code: "BACKEND_TASK_FAILED".into(),
                message: "The backend could not complete the request".into(),
                retryable: true,
                action: Some(super::ApiAction::Retry),
                details: None,
            })
        }
    }
}

fn thumbnail_coordinator_error(error: ThumbnailCoordinatorError) -> ApiError {
    let (code, retryable) = match &error {
        ThumbnailCoordinatorError::InvalidConfiguration(_)
        | ThumbnailCoordinatorError::InvalidKey(_) => ("THUMBNAIL_REQUEST_INVALID", false),
        ThumbnailCoordinatorError::Closed => ("THUMBNAIL_COORDINATOR_CLOSED", true),
        ThumbnailCoordinatorError::WorkerStart(_) => ("THUMBNAIL_WORKER_UNAVAILABLE", true),
    };
    ApiError {
        code: code.into(),
        message: error.to_string(),
        retryable,
        action: Some(if retryable {
            super::ApiAction::Retry
        } else {
            super::ApiAction::None
        }),
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread};

    use super::*;
    use crate::domain::ValidationError;

    #[test]
    fn application_blocking_helper_runs_off_the_calling_thread() {
        let caller = thread::current().id();
        let result = tauri::async_runtime::block_on(run_application_blocking(
            "test_blocking_boundary",
            || Ok(thread::current().id()),
        ));

        match result {
            ApiResult::Success(worker) => assert_ne!(worker, caller),
            ApiResult::Failure(error) => panic!("blocking call unexpectedly failed: {error:?}"),
        }
    }

    #[test]
    fn application_blocking_helper_preserves_api_errors() {
        let result =
            tauri::async_runtime::block_on(run_application_blocking("test_error_boundary", || {
                Err::<(), _>(
                    ValidationError::new("query", "must not contain unsupported syntax").into(),
                )
            }));

        match result {
            ApiResult::Failure(error) => {
                assert_eq!(error.code, "VALIDATION_ERROR");
                assert!(!error.retryable);
            }
            ApiResult::Success(()) => panic!("validation error unexpectedly succeeded"),
        }
    }

    #[test]
    fn search_page_cancellation_is_replayed_when_cancel_arrives_before_start() {
        let requests = SearchPageRequests::default();
        assert!(requests.cancel("request-before-start"));
        let token = requests.start("request-before-start");
        assert!(token.is_cancelled());
        requests.finish("request-before-start");
    }

    #[test]
    fn search_page_cancellation_reaches_an_active_request() {
        let requests = SearchPageRequests::default();
        let token = requests.start("request-active");
        assert!(!token.is_cancelled());
        assert!(requests.cancel("request-active"));
        assert!(token.is_cancelled());
        requests.finish("request-active");
    }

    fn active_work_snapshot(active_downloads: u64) -> AppActiveWorkSnapshot {
        AppActiveWorkSnapshot {
            queried_at: "123".into(),
            work_set_fingerprint: "current-work".into(),
            downloads: AppActiveDownloadsSnapshot {
                active_count: active_downloads,
            },
            auto_find: None,
            duplicate_scan: None,
            internal_duplicate_scan: None,
        }
    }

    fn all_active_work_snapshot() -> AppActiveWorkSnapshot {
        AppActiveWorkSnapshot {
            queried_at: "123".into(),
            work_set_fingerprint: "all-work".into(),
            downloads: AppActiveDownloadsSnapshot { active_count: 2 },
            auto_find: Some(AppActiveAutoFindSnapshot {
                run_id: "auto-run".into(),
                completed_favorites: 1,
                total_favorites: 4,
                candidates_found: 2,
            }),
            duplicate_scan: Some(AppActiveDuplicateScanSnapshot {
                run_id: "duplicate-run".into(),
                hashed_artifacts: 3,
                total_artifacts: 8,
                compared_pairs: 5,
                total_pairs: 10,
                candidates_found: 1,
            }),
            internal_duplicate_scan: Some(AppActiveInternalDuplicateScanSnapshot {
                run_id: "internal-run".into(),
                scanned_artifacts: 2,
                total_artifacts: 7,
                skipped_artifacts: 1,
                groups_found: 3,
            }),
        }
    }

    #[test]
    fn active_work_fingerprint_is_order_independent_and_identity_only() {
        let first = active_work_fingerprint(
            ["entry-b", "entry-a", "entry-a"],
            Some("auto-run"),
            Some("duplicate-run"),
            None,
        );
        let reordered = active_work_fingerprint(
            ["entry-a", "entry-b"],
            Some("auto-run"),
            Some("duplicate-run"),
            None,
        );
        assert_eq!(first, reordered);
        assert_eq!(first.len(), 64);

        let changed = active_work_fingerprint(
            ["entry-a", "entry-c"],
            Some("auto-run"),
            Some("duplicate-run"),
            None,
        );
        assert_ne!(first, changed);

        let changed_run = active_work_fingerprint(
            ["entry-a", "entry-b"],
            Some("new-auto-run"),
            Some("duplicate-run"),
            None,
        );
        assert_ne!(first, changed_run);
    }

    #[test]
    fn active_work_fingerprint_does_not_include_progress() {
        let before_progress = AppActiveAutoFindSnapshot {
            run_id: "auto-run".into(),
            completed_favorites: 1,
            total_favorites: 10,
            candidates_found: 0,
        };
        let after_progress = AppActiveAutoFindSnapshot {
            completed_favorites: 9,
            candidates_found: 42,
            ..before_progress.clone()
        };
        let before =
            active_work_fingerprint(["entry-a"], Some(&before_progress.run_id), None, None);
        let after = active_work_fingerprint(["entry-a"], Some(&after_progress.run_id), None, None);
        assert_eq!(before, after);
    }

    #[test]
    fn quit_requires_confirmation_for_the_current_active_work_set() {
        let snapshot = active_work_snapshot(1);
        let request = AppQuitRequest {
            expected_work_set_fingerprint: snapshot.work_set_fingerprint.clone(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        assert_eq!(
            quit_rejection_reason(&request, &snapshot),
            Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired)
        );

        let confirmed = AppQuitRequest {
            confirm_active_work: true,
            ..request
        };
        assert_eq!(quit_rejection_reason(&confirmed, &snapshot), None);
    }

    #[test]
    fn quit_rejects_a_changed_or_unproven_work_set() {
        let snapshot = active_work_snapshot(1);
        let changed = AppQuitRequest {
            expected_work_set_fingerprint: "previous-work".into(),
            confirm_active_work: true,
            force_when_status_unknown: false,
        };
        assert_eq!(
            quit_rejection_reason(&changed, &snapshot),
            Some(AppQuitRejectionReason::ActiveWorkChanged)
        );

        let unproven = AppQuitRequest {
            expected_work_set_fingerprint: String::new(),
            confirm_active_work: true,
            force_when_status_unknown: false,
        };
        assert_eq!(
            quit_rejection_reason(&unproven, &snapshot),
            Some(AppQuitRejectionReason::ActiveWorkChanged)
        );
    }

    #[test]
    fn tray_style_empty_fingerprint_is_safe_only_when_no_work_is_active() {
        let request = AppQuitRequest {
            expected_work_set_fingerprint: String::new(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        assert_eq!(
            quit_rejection_reason(&request, &active_work_snapshot(0)),
            None
        );
        assert_eq!(
            quit_rejection_reason(&request, &active_work_snapshot(1)),
            Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired)
        );
    }

    #[test]
    fn managed_work_gate_rejects_new_work_after_quit_is_committed() {
        let gate = ManagedWorkGate::default();
        assert_eq!(gate.run(|| Ok(7)).expect("work should start"), 7);
        gate.inner.quitting.store(true, Ordering::Release);
        assert!(matches!(
            gate.run(|| Ok::<_, ApplicationError>(8)),
            Err(ApplicationError::AppQuitInProgress)
        ));
    }

    #[test]
    fn managed_work_preflight_does_not_hold_the_quit_gate_and_cannot_commit_after_quit() {
        let gate = ManagedWorkGate::default();
        let worker_gate = gate.clone();
        let committed = Arc::new(AtomicBool::new(false));
        let worker_committed = Arc::clone(&committed);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            prepare_then_commit_managed_work(
                &worker_gate,
                || {
                    entered_tx.send(()).expect("signal preflight");
                    release_rx.recv().expect("release preflight");
                    Ok(())
                },
                |()| {
                    worker_committed.store(true, Ordering::Release);
                    Ok(())
                },
            )
        });

        entered_rx.recv().expect("preflight should begin");
        let mut control = gate
            .inner
            .control
            .try_lock()
            .expect("artifact preflight must not hold the quit gate");
        let snapshot = active_work_snapshot(0);
        let request = AppQuitRequest {
            expected_work_set_fingerprint: snapshot.work_set_fingerprint.clone(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        let (accepted, should_shutdown) =
            gate.evaluate_quit_locked(&mut control, &request, Some(snapshot));
        assert!(accepted.accepted);
        assert!(should_shutdown);
        drop(control);
        release_tx.send(()).expect("finish preflight");

        assert!(matches!(
            worker.join().expect("join preflight worker"),
            Err(ApplicationError::AppQuitInProgress)
        ));
        assert!(!committed.load(Ordering::Acquire));
    }

    #[test]
    fn active_work_snapshot_counts_no_work_each_kind_and_all_kinds() {
        let empty = active_work_snapshot(0);
        assert!(!empty.has_active_work());
        assert_eq!(empty.active_work_count(), 0);

        let downloads = active_work_snapshot(2);
        assert!(downloads.has_active_work());
        assert_eq!(downloads.active_work_count(), 2);

        let all = all_active_work_snapshot();
        assert!(all.has_active_work());
        assert_eq!(all.active_work_count(), 5);

        let mut auto_find_only = active_work_snapshot(0);
        auto_find_only.auto_find = all.auto_find.clone();
        assert_eq!(auto_find_only.active_work_count(), 1);
        let mut duplicate_only = active_work_snapshot(0);
        duplicate_only.duplicate_scan = all.duplicate_scan.clone();
        assert_eq!(duplicate_only.active_work_count(), 1);
        let mut internal_only = active_work_snapshot(0);
        internal_only.internal_duplicate_scan = all.internal_duplicate_scan.clone();
        assert_eq!(internal_only.active_work_count(), 1);
    }

    #[test]
    fn rejected_quit_has_no_commit_and_accepted_quit_is_single_flight() {
        let gate = ManagedWorkGate::default();
        let snapshot = all_active_work_snapshot();
        let unconfirmed = AppQuitRequest {
            expected_work_set_fingerprint: snapshot.work_set_fingerprint.clone(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        let mut control = gate.inner.control.lock().expect("managed work gate");
        let (rejected, should_shutdown) =
            gate.evaluate_quit_locked(&mut control, &unconfirmed, Some(snapshot.clone()));
        assert!(!rejected.accepted);
        assert!(!should_shutdown);
        assert!(!gate.inner.quitting.load(Ordering::Acquire));
        assert!(control.accepted_quit.is_none());

        let confirmed = AppQuitRequest {
            confirm_active_work: true,
            ..unconfirmed
        };
        let (accepted, should_shutdown) =
            gate.evaluate_quit_locked(&mut control, &confirmed, Some(snapshot));
        assert!(accepted.accepted);
        assert!(should_shutdown);
        assert!(gate.inner.quitting.load(Ordering::Acquire));

        let unrelated_second_request = AppQuitRequest {
            expected_work_set_fingerprint: "stale".into(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        let (same_result, should_shutdown_again) = gate.evaluate_quit_locked(
            &mut control,
            &unrelated_second_request,
            Some(active_work_snapshot(0)),
        );
        assert_eq!(same_result, accepted);
        assert!(!should_shutdown_again);
    }

    #[test]
    fn unknown_status_requires_an_explicit_confirmed_force_decision() {
        let gate = ManagedWorkGate::default();
        let mut control = gate.inner.control.lock().expect("managed work gate");
        let unconfirmed_force = AppQuitRequest {
            expected_work_set_fingerprint: String::new(),
            confirm_active_work: false,
            force_when_status_unknown: true,
        };
        let (rejected, should_shutdown) =
            gate.evaluate_quit_locked(&mut control, &unconfirmed_force, None);
        assert!(!rejected.accepted);
        assert_eq!(
            rejected.reason,
            Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired)
        );
        assert!(!should_shutdown);
        assert!(!gate.inner.quitting.load(Ordering::Acquire));

        let confirmed_force = AppQuitRequest {
            confirm_active_work: true,
            ..unconfirmed_force
        };
        let (accepted, should_shutdown) =
            gate.evaluate_quit_locked(&mut control, &confirmed_force, None);
        assert!(accepted.accepted);
        assert!(accepted.snapshot.is_none());
        assert!(should_shutdown);
    }
}
