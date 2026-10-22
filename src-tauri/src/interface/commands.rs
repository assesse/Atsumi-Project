use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::{
    application::{
        ApplicationError, ApplicationService, ArtifactStore, ArtistPreview, AutoFindSupervisor,
        DetailOriginalError, DetailOriginalPrepareRequest, DetailOriginalPrepared,
        DetailOriginalSupervisor, DownloadPipelineError, DownloadPipelineErrorCode,
        DownloadRootPicker, DownloadSupervisor, DuplicateSupervisor, GalleryPreview,
        GalleryPreviewService, GalleryPreviewSetRequest, InternalDuplicateSupervisor,
        ReconcileReport,
    },
    domain::{
        AutoFindExclusionResult, AutoFindRun, AutoFindSnapshot, DownloadChangedEvent,
        DownloadEntry, DownloadLibraryPage, DownloadListRequest,
        DownloadOverlapAutomationHistoryItem, DownloadOverlapAutomationHistoryListRequest,
        DownloadOverlapAutomationHistoryPage, DownloadOverlapDecisionRequest,
        DownloadOverlapDecisionResult, DownloadOverlapReview, DownloadPage,
        DuplicateDecisionRequest, DuplicateReview, DuplicateScanRun, DuplicateSnapshot,
        ExplorationDataResetRequest, ExplorationDataResetResult, FavoriteKey,
        FavoriteMutationResult, FavoriteRecord, GalleryDetail, GalleryPage, GallerySummary,
        InternalArtifactScanProgress, InternalDuplicateReview, InternalDuplicateSnapshot,
        InternalRemovalApplyRequest, InternalRemovalPlan, InternalRemovalPlanRequest,
        InternalRemovalResult, InternalRemovalUndoRequest, InternalScanRequest, InternalScanRun,
        JobRef, MaintenanceAction, MaintenancePreview, MaintenanceResult, SearchHistoryEntry,
        SearchRequest, SearchSubmission, SettingsPatch, SettingsSnapshot, TagCatalogStatus,
        TagSuggestion, TagSuggestionRequest, ValidationError, WindowPlacement,
        WindowPlacementSnapshot,
    },
    infrastructure::{
        DownloadOverlapMergeRequest, DownloadOverlapMergeResult, DownloadOverlapMergeSide,
        ExcludedArtifactService, HitomiLiveAdapter, OverlapMergeService, ThumbnailDiskCache,
    },
    thumbnail::{
        CancellationToken, ThumbnailCacheClearDto, ThumbnailCoordinator, ThumbnailCoordinatorError,
        ThumbnailInvalidationDto, ThumbnailKey, ThumbnailPriority, ThumbnailRequestDto,
        ThumbnailRequestTokenDto, ThumbnailRuntimeConfigDto, ThumbnailWorkerStatsDto,
    },
};

use super::storage_usage::{collect_storage_usage, StorageUsageSnapshot};
use super::{
    api::{
        AppActiveAutoFindSnapshot, AppActiveDownloadsSnapshot, AppActiveDuplicateScanSnapshot,
        AppActiveInternalDuplicateScanSnapshot, AppActiveLiveReceiversSnapshot,
        AppActiveRecordingsSnapshot, AppActiveWorkSnapshot,
        AppQuitRejectionReason, AppQuitRequest, AppQuitResult,
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
    // A failed reset cannot reopen services that have already been shut down.
    // Keep admission closed separately from whether a quit is being dispatched.
    maintenance_stopped: AtomicBool,
    platform_initializers: AtomicUsize,
}

#[derive(Default)]
struct ManagedWorkGateState {
    accepted_quit: Option<AppQuitResult>,
    updating: bool,
}

impl Default for ManagedWorkGate {
    fn default() -> Self {
        Self {
            inner: Arc::new(ManagedWorkGateInner {
                control: Mutex::new(ManagedWorkGateState::default()),
                quitting: AtomicBool::new(false),
                maintenance_stopped: AtomicBool::new(false),
                platform_initializers: AtomicUsize::new(0),
            }),
        }
    }
}

impl ManagedWorkGate {
    fn begin_platform_initialization(&self) -> Result<PlatformInitialization, String> {
        let control = self.inner.control.lock().map_err(|_| "작업 상태를 확인하지 못했습니다.")?;
        if self.admission_closed() || control.updating {
            return Err("종료·업데이트 중에는 영상 서비스를 준비할 수 없습니다.".into());
        }
        self.inner.platform_initializers.fetch_add(1, Ordering::AcqRel);
        Ok(PlatformInitialization { gate: self.clone() })
    }

    fn platform_initializing(&self) -> bool {
        self.inner.platform_initializers.load(Ordering::Acquire) > 0
    }

    fn admission_closed(&self) -> bool {
        self.inner.quitting.load(Ordering::Acquire)
            || self.inner.maintenance_stopped.load(Ordering::Acquire)
    }

    fn reserve_factory_reset(&self) -> Result<FactoryResetReservation, ApplicationError> {
        let _control = self.inner.control.lock().map_err(|_| {
            crate::application::RepositoryError::Other(
                "managed work gate mutex was poisoned".into(),
            )
        })?;
        if self.admission_closed() || _control.updating {
            return Err(ApplicationError::AppQuitInProgress);
        }
        self.inner.maintenance_stopped.store(true, Ordering::Release);
        self.inner.quitting.store(true, Ordering::Release);
        Ok(FactoryResetReservation { gate: self.clone(), committed: false })
    }

    fn run<T>(
        &self,
        operation: impl FnOnce() -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let _control = self.inner.control.lock().map_err(|_| {
            crate::application::RepositoryError::Other(
                "managed work gate mutex was poisoned".into(),
            )
        })?;
        if self.admission_closed() || _control.updating {
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

struct PlatformInitialization {
    gate: ManagedWorkGate,
}

impl Drop for PlatformInitialization {
    fn drop(&mut self) {
        self.gate.inner.platform_initializers.fetch_sub(1, Ordering::AcqRel);
    }
}

// Admission must already be closed. Do not snapshot an empty OnceLock before
// its admitted initializer has finished journal recovery and worker creation.
fn settled_platform_service<T: Clone>(
    gate: &ManagedWorkGate,
    service: &OnceLock<T>,
    mut wait: impl FnMut(),
) -> Option<T> {
    while gate.platform_initializing() { wait(); }
    service.get().cloned()
}

struct FactoryResetReservation {
    gate: ManagedWorkGate,
    committed: bool,
}

impl Drop for FactoryResetReservation {
    fn drop(&mut self) {
        if !self.committed {
            let mut control = self.gate.inner.control.lock().unwrap_or_else(|error| error.into_inner());
            control.accepted_quit = None;
            // A normal quit can retry the drain, but no work/update may start.
            self.gate.inner.quitting.store(false, Ordering::Release);
        }
    }
}

fn finish_factory_reset(
    mut reservation: FactoryResetReservation,
    drain: impl FnOnce() -> Result<(), ApplicationError>,
    publish_marker: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), ApplicationError> {
    drain()?;
    publish_marker().map_err(|error| {
        ApplicationError::from(crate::application::RepositoryError::Other(format!(
            "could not schedule factory reset: {error}"
        )))
    })?;
    reservation.committed = true;
    Ok(())
}

fn drain_managed_workers(
    stop: impl FnOnce(),
    mut pending: impl FnMut() -> bool,
    mut wait: impl FnMut(),
    drain_legacy: impl FnOnce(),
) {
    stop();
    while pending() {
        wait();
    }
    drain_legacy();
}

fn publish_factory_reset_marker(data_dir: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    let temporary = data_dir.join(format!(".factory-reset-{}.pending", uuid::Uuid::new_v4()));
    let target = data_dir.join("factory-reset.pending");
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let written = file.write_all(b"v1\n").and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| publish_marker_no_replace(&temporary, &target));
    // Only our unpublished unique file is removed; never the destination marker.
    let _ = std::fs::remove_file(&temporary);
    result
}

fn publish_marker_no_replace(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH}};
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr()), MOVEFILE_WRITE_THROUGH) }
            .map_err(|error| std::io::Error::other(error.to_string()))
    }
    #[cfg(not(windows))]
    {
        // A same-directory hard-link publication is atomic and cannot clobber
        // an existing target; the caller removes only its temporary link.
        std::fs::hard_link(source, target)
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

/// Display-only classification of the already deduplicated ownership IDs.
/// Keep the original IDs intact for quit fingerprints and actual worker drain.
fn display_work_counts(ids: &[String]) -> (AppActiveDownloadsSnapshot, Option<AppActiveLiveReceiversSnapshot>) {
    let receiver_count = ids.iter().filter(|id| {
        id.strip_prefix("platform-live:")
            .or_else(|| id.strip_prefix("platform-official:"))
            .is_some_and(|identity| !identity.is_empty())
    }).count();
    (
        AppActiveDownloadsSnapshot {
            active_count: u64::try_from(ids.len() - receiver_count).unwrap_or(u64::MAX),
        },
        (receiver_count > 0).then(|| AppActiveLiveReceiversSnapshot {
            active_count: u64::try_from(receiver_count).unwrap_or(u64::MAX),
        }),
    )
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

fn fingerprint_with_recordings(base: String, recording_ids: &[String]) -> String {
    if recording_ids.is_empty() {
        return base;
    }
    let mut ids = recording_ids.to_vec();
    ids.sort();
    ids.dedup();
    let mut hash = Sha256::new();
    hash.update(b"atsumi-work-with-recordings-v1");
    hash.update(base.as_bytes());
    for id in ids {
        hash.update((id.len() as u64).to_be_bytes());
        hash.update(id.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    pub(crate) danbooru: Arc<super::danbooru::DanbooruClient>,
    thumbnails: ThumbnailCoordinator,
    thumbnail_disk_cache: Option<Arc<ThumbnailDiskCache>>,
    pub(crate) thumbnail_transport: Arc<crate::thumbnail::transport::ThumbnailTransport>,
    detail_originals: DetailOriginalSupervisor,
    downloads: DownloadSupervisor,
    auto_find: AutoFindSupervisor,
    duplicates: DuplicateSupervisor,
    internal_duplicates: InternalDuplicateSupervisor,
    gallery_previews: Option<GalleryPreviewService>,
    excluded_artifacts: Option<Arc<ExcludedArtifactService>>,
    overlap_merges: Option<Arc<OverlapMergeService>>,

    browser: crate::streaming::startup::DeferredBrowser,
    platform_media: Arc<OnceLock<Result<crate::platform_media::PlatformMediaService, String>>>,
    soop_auto_recorder: Arc<crate::soop::auto_record::SoopAutoRecorder>,
    youtube_auto_recorder: Arc<crate::youtube::auto_record::YouTubeAutoRecorder>,
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

fn platform_development_resource_root() -> Result<PathBuf, String> {
    // Keep the native resolver's trusted compile-time path lexical. The raw
    // archive's guarded tool lease deliberately rejects ParentDir components.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|workspace| workspace.join(".runtime"))
        .ok_or_else(|| "개발용 앱 도구 위치를 확인하지 못했습니다.".into())
}

impl AppState {
    // This is the single composition root for application services and ports.
    // Keeping dependencies explicit here makes production/test wiring auditable.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        service: ApplicationService,
        danbooru: Arc<super::danbooru::DanbooruClient>,
        thumbnails: ThumbnailCoordinator,
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
            danbooru,
            thumbnails,
            thumbnail_disk_cache: None,
            thumbnail_transport: Arc::new(
                crate::thumbnail::transport::ThumbnailTransport::default(),
            ),
            detail_originals,
            downloads,
            auto_find,
            duplicates,
            internal_duplicates,
            gallery_previews: None,
            excluded_artifacts: None,
            overlap_merges: None,

            browser: crate::streaming::startup::DeferredBrowser::default(),
            platform_media: Arc::new(OnceLock::new()),
            soop_auto_recorder: Arc::new(crate::soop::auto_record::SoopAutoRecorder::new(data_dir.clone())),
            youtube_auto_recorder: Arc::new(crate::youtube::auto_record::YouTubeAutoRecorder::new(data_dir.clone())),
            download_root_picker,
            artifact_store,
            live_source,
            data_dir,
            search_pages: SearchPageRequests::default(),
            maintenance_previews: Mutex::new(HashMap::new()),
            managed_work: ManagedWorkGate::default(),
        }
    }

    pub fn with_gallery_previews(mut self, previews: GalleryPreviewService) -> Self {
        self.gallery_previews = Some(previews);
        self
    }

    pub fn with_thumbnail_disk_cache(mut self, cache: Arc<ThumbnailDiskCache>) -> Self {
        self.thumbnail_disk_cache = Some(cache);
        self
    }

    pub fn with_excluded_artifacts(mut self, excluded: Arc<ExcludedArtifactService>) -> Self {
        self.excluded_artifacts = Some(excluded);
        self
    }

    pub fn with_overlap_merges(mut self, merges: Arc<OverlapMergeService>) -> Self {
        self.overlap_merges = Some(merges);
        self
    }

    pub fn with_official_browser(
        mut self,
        browser: Option<crate::streaming::browser::OfficialBrowser>,
    ) -> Self {
        self.browser = crate::streaming::startup::DeferredBrowser::ready(browser);
        self
    }

    pub(crate) fn deferred_browser(&self) -> crate::streaming::startup::DeferredBrowser {
        self.browser.clone()
    }

    pub(crate) fn official_browser(
        &self,
    ) -> Result<crate::streaming::browser::OfficialBrowser, crate::streaming::model::StreamError>
    {
        self.browser.get()
    }

    pub(crate) fn start_browser_managed(
        &self,
        app: &AppHandle,
        rights: bool,
        capture_chat: bool,
    ) -> Result<(), crate::streaming::model::StreamError> {
        self.start_browser_managed_for(app, None, rights, capture_chat)
    }
    pub(crate) fn start_browser_managed_for(
        &self,
        app: &AppHandle,
        context: Option<&str>,
        rights: bool,
        capture_chat: bool,
    ) -> Result<(), crate::streaming::model::StreamError> {
        self.start_browser_managed_checked(app, context, rights, capture_chat, None)
    }
    pub(crate) fn start_browser_managed_checked(
        &self,
        app: &AppHandle,
        context: Option<&str>,
        rights: bool,
        capture_chat: bool,
        expected_channel: Option<&str>,
    ) -> Result<(), crate::streaming::model::StreamError> {
        use crate::streaming::model::StreamError;
        let browser = self.official_browser()?;
        let settings = self.settings_snapshot().map_err(|_| {
            StreamError::new("SETTINGS_UNAVAILABLE", "저장 위치를 읽지 못했습니다.", true)
        })?;
        let _control = self.managed_work.inner.control.lock().map_err(|_| {
            StreamError::new(
                "APP_WORK_UNAVAILABLE",
                "앱 작업 상태를 확인하지 못했습니다.",
                true,
            )
        })?;
        if self.managed_work.admission_closed() || _control.updating {
            return Err(StreamError::new(
                "APP_QUITTING",
                "앱 종료 중에는 녹화를 시작할 수 없습니다.",
                false,
            ));
        }
        browser.capture_context(context)?.arm_checked(
            app,
            PathBuf::from(settings.download_root),
            rights,
            capture_chat,
            expected_channel,
        )
    }

    fn preview_service(&self) -> Result<GalleryPreviewService, ApplicationError> {
        self.gallery_previews.clone().ok_or_else(|| {
            crate::application::RepositoryError::Other("gallery previews are unavailable".into())
                .into()
        })
    }

    pub(crate) fn settings_snapshot(&self) -> Result<SettingsSnapshot, ApplicationError> {
        self.service.settings_get()
    }

    /// Lazy on a blocking command worker: tool/journal inspection must not delay the first frame.
    pub(crate) fn platform_media(&self, app: &AppHandle) -> Result<crate::platform_media::PlatformMediaService, String> {
        use tauri::Manager;
        if self.is_quitting() { return Err("앱을 종료하는 중입니다.".into()); }
        if let Some(service) = self.platform_media.get() { return service.clone(); }
        // Only registration holds the admission lock. Disk I/O stays on the
        // calling blocking worker; normal UI/quit requests remain responsive.
        let _initialization = self.managed_work.begin_platform_initialization()?;
        let result = self.platform_media.get_or_init(|| {
            let root = if cfg!(debug_assertions) {
                platform_development_resource_root()?
            } else { app.path().resource_dir().map_err(|_| "앱 도구 위치를 확인하지 못했습니다.")? };
            let media = if cfg!(debug_assertions) {
                root.join("media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin")
            } else { root.join("media-tools/bin") };
            // Keep new tools under the resource root supported by older portable updaters.
            let platform = if cfg!(debug_assertions) {
                root.join("platform-tools/bin")
            } else { root.join("media-tools/platform-tools/bin") };
            crate::platform_media::PlatformMediaService::new(self.data_dir.clone(), crate::platform_media::PlatformMediaTools {
                yt_dlp: platform.join("yt-dlp.exe"), deno: platform.join("deno.exe"),
                ffmpeg: media.join("ffmpeg.exe"), ffprobe: media.join("ffprobe.exe"),
            }, app.config().build.dev_url.clone())
        }).clone();
        if self.is_quitting() {
            if let Ok(service) = &result { service.shutdown(); }
            return Err("앱을 종료하는 중입니다.".into());
        }
        result
    }

    pub(crate) fn existing_platform_media(&self) -> Option<crate::platform_media::PlatformMediaService> {
        self.platform_media.get().and_then(|value| value.as_ref().ok()).cloned()
    }

    pub(crate) fn soop_auto_recorder(&self) -> Arc<crate::soop::auto_record::SoopAutoRecorder> {
        self.soop_auto_recorder.clone()
    }

    pub(crate) fn youtube_auto_recorder(&self) -> Arc<crate::youtube::auto_record::YouTubeAutoRecorder> {
        self.youtube_auto_recorder.clone()
    }

    /// Register provider work atomically with quit/update reservation. Network I/O
    /// and waiting must happen after this closure returns its RAII work permit.
    pub(crate) fn begin_platform_work<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let control = self
            .managed_work
            .inner
            .control
            .lock()
            .map_err(|_| "작업 상태를 확인하지 못했습니다.")?;
        if control.updating || self.is_quitting() {
            return Err("종료·업데이트 중에는 다운로드를 시작할 수 없습니다.".into());
        }
        operation()
    }

    pub(crate) fn active_work_snapshot(&self) -> Result<AppActiveWorkSnapshot, ApplicationError> {
        let _control = self.managed_work.inner.control.lock().map_err(|_| {
            crate::application::RepositoryError::Other(
                "managed work gate mutex was poisoned".into(),
            )
        })?;
        self.active_work_snapshot_locked()
    }

    pub(crate) fn reserve_update(&self) -> Result<(), crate::streaming::model::StreamError> {
        use crate::streaming::model::StreamError;
        let fail = || {
            StreamError::new(
                "UPDATE_BUSY",
                "작업 상태를 확인하지 못해 업데이트를 보류했습니다.",
                true,
            )
        };
        let mut control = self.managed_work.inner.control.lock().map_err(|_| fail())?;
        if control.updating || self.is_quitting() {
            return Err(fail());
        }
        if self
            .active_work_snapshot_locked()
            .map_err(|_| fail())?
            .has_active_work()
        {
            return Err(StreamError::new(
                "UPDATE_WORK_ACTIVE",
                "다운로드·검사·탐색·녹화를 마친 뒤 업데이트해 주세요.",
                true,
            ));
        }
        self.official_browser()?.reserve_update()?;
        control.updating = true;
        Ok(())
    }

    pub(crate) fn release_update(&self) {
        if let Ok(mut control) = self.managed_work.inner.control.lock() {
            if !self.is_quitting() {
                control.updating = false;
                if let Ok(browser) = self.official_browser() {
                    browser.release_update();
                }
            }
        }
    }

    pub(crate) fn update_reserved(&self) -> bool {
        self.managed_work
            .inner
            .control
            .lock()
            .is_ok_and(|control| control.updating && !self.is_quitting())
    }

    pub(crate) fn finish_portable_update(&self, app: AppHandle) -> Result<(), String> {
        let control = self
            .managed_work
            .inner
            .control
            .lock()
            .map_err(|_| "업데이트 잠금을 확인하지 못했습니다.")?;
        if !control.updating
            || self.is_quitting()
            || self
                .active_work_snapshot_locked()
                .map_err(|_| "작업 상태 확인 실패")?
                .has_active_work()
        {
            return Err("진행 중인 작업 때문에 업데이트를 보류했습니다.".into());
        }
        self.managed_work
            .inner
            .quitting
            .store(true, Ordering::Release);
        drop(control);
        self.spawn_graceful_shutdown(app);
        Ok(())
    }

    fn active_work_snapshot_locked(&self) -> Result<AppActiveWorkSnapshot, ApplicationError> {
        let mut active_download_ids: Vec<String> = self
            .service
            .download_active_entry_ids()?
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        active_download_ids.extend(
            crate::pinterest::active_download_ids()
                .into_iter()
                .map(|id| format!("pinterest:{id}")),
        );
        active_download_ids.extend(
            crate::pixiv::active_download_ids()
                .into_iter()
                .map(|id| format!("pixiv:{id}")),
        );
        active_download_ids.extend(
            crate::x_feed::archive::active_ids()
                .into_iter()
                .map(|id| format!("x-archive:{id}")),
        );
        if let Some(service) = self.existing_platform_media() {
            active_download_ids.extend(service.active_ids());
        }
        if self.managed_work.platform_initializing() {
            active_download_ids.push("platform-media:initializing".into());
        }
        active_download_ids.extend(self.soop_auto_recorder.active_ids());
        active_download_ids.extend(self.youtube_auto_recorder.active_ids());
        active_download_ids.sort();
        active_download_ids.dedup();
        let (downloads, live_receivers) = display_work_counts(&active_download_ids);
        let auto_find = self.auto_find.active_run_snapshot()?;
        let duplicate_scan = self.duplicates.active_run_snapshot()?;
        let internal_duplicate_scan = self.internal_duplicates.active_run_snapshot()?;
        let recording_ids = self
            .browser
            .get()
            .map_or_else(|_| Vec::new(), |browser| browser.active_ids());
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
            work_set_fingerprint: fingerprint_with_recordings(work_set_fingerprint, &recording_ids),
            recordings: (!recording_ids.is_empty()).then_some(AppActiveRecordingsSnapshot {
                active_count: recording_ids.len() as u64,
            }),
            downloads,
            live_receivers,
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

    /// The caller must first close admission under ManagedWorkGate. Neither a
    /// normal exit nor a factory-reset marker may bypass this same drain.
    fn blocking_work_drain(&self, app: AppHandle) -> impl FnOnce() -> Result<(), ApplicationError> + Send + 'static {
        let soop_auto_recorder = self.soop_auto_recorder.clone();
        let youtube_auto_recorder = self.youtube_auto_recorder.clone();
        let platform_slot = self.platform_media.clone();
        let gate = self.managed_work.clone();
        let browser = self.browser.clone();
        let downloads = self.downloads.clone();
        let auto_find = self.auto_find.clone();
        let duplicates = self.duplicates.clone();
        let internal_duplicates = self.internal_duplicates.clone();
        move || {
            let platform_media = settled_platform_service(&gate, &platform_slot, || {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }).and_then(Result::ok);
            drain_managed_workers(
            || {
                soop_auto_recorder.shutdown();
                youtube_auto_recorder.shutdown();
                if let Some(host) = app.try_state::<crate::soop::original::SoopOriginalHost>() {
                    host.shutdown(&app);
                }
                if let Some(host) = app.try_state::<crate::youtube::original::YouTubeOriginalHost>() {
                    host.shutdown(&app);
                }
                if let Some(browser) = app.try_state::<crate::pinterest_browser::PinterestBrowser>() {
                    browser.detach(&app);
                }
                if let Some(session) = app.try_state::<crate::pinterest_session::PinterestSession>() {
                    session.detach(&app);
                }
                if let Some(session) = app.try_state::<crate::x_feed::XSession>() {
                    session.shutdown(&app);
                }
                if let Some(account) = app.try_state::<crate::pixiv::account::PixivAccount>() {
                    account.shutdown(&app);
                }
                crate::pinterest::cancel_all_downloads();
                crate::pixiv::cancel_all_downloads();
                crate::x_feed::archive::cancel_all();
                crate::x_feed::archive_ipc::clear_cache();
                if let Some(service) = &platform_media { service.shutdown(); }
                browser.shutdown_and_wait(&app);
            },
            || crate::pinterest::active_download_count() > 0
                    || crate::pixiv::active_download_count() > 0
                    || !crate::x_feed::archive::active_ids().is_empty()
                    || crate::x_feed::surface::search::busy()
                    || !soop_auto_recorder.active_ids().is_empty()
                    || !youtube_auto_recorder.active_ids().is_empty()
                    || platform_media.as_ref().is_some_and(|service| !service.active_ids().is_empty()),
            || std::thread::sleep(std::time::Duration::from_millis(50)),
            || {
                crate::shutdown_all_then_exit(
                    || internal_duplicates.shutdown_and_wait(),
                    || duplicates.shutdown_and_wait(),
                    || auto_find.shutdown_and_wait(),
                    || downloads.shutdown_and_wait(),
                    || {},
                );
            });
            if let Some(error) = platform_media.and_then(|service| service.shutdown_error()) {
                return Err(crate::application::RepositoryError::Other(error).into());
            }
            Ok(())
        }
    }

    fn spawn_graceful_shutdown(&self, app: AppHandle) {
        let drain = self.blocking_work_drain(app.clone());
        tauri::async_runtime::spawn(async move {
            if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
                // All real workers have stopped before this result is returned.
                // A failed final journal write is recoverable from the last valid
                // journal and retained originals; it is not a successful save.
                if let Err(error) = drain() {
                    tracing::warn!(error = %error, "work stopped; journal persistence failed, originals retained for recovery");
                }
                app.exit(0);
            })
            .await
            {
                tracing::warn!(error = %error, "background workers did not finish shutdown cleanly");
            }
        });
    }

    pub(crate) fn is_quitting(&self) -> bool {
        // Also suppress delayed startup after a failed maintenance shutdown.
        // Quit dispatch itself checks `quitting`, so the user can still exit.
        self.managed_work.admission_closed()
    }

    /// Only used when bootstrap was cancelled before any queued work resumed.
    pub(crate) fn cancel_startup(&self, app: AppHandle) {
        self.managed_work
            .inner
            .quitting
            .store(true, Ordering::Release);
        self.spawn_graceful_shutdown(app);
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

#[tauri::command(rename_all = "camelCase")]
pub async fn search_history_remove(
    state: State<'_, AppState>,
    history_id: i64,
) -> Result<ApiResult<u64>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("search_history_remove", move || {
        service.search_history_remove(history_id)
    })
    .await)
}

#[tauri::command]
pub async fn search_history_clear(state: State<'_, AppState>) -> Result<ApiResult<u64>, ApiError> {
    let service = state.service.clone();
    Ok(run_application_blocking("search_history_clear", move || {
        service.search_history_clear()
    })
    .await)
}

#[tauri::command]
pub async fn download_root_choose(
    window: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<ApiResult<Option<String>>, ApiError> {
    if window.label() != "main" {
        return Ok(ApiResult::failure(
            ApplicationError::from(ValidationError::new("window", "main window required")).into(),
        ));
    }
    let picker = Arc::clone(&state.download_root_picker);
    // Choose only: settings are not persisted until the user presses Save.
    Ok(run_application_blocking("download_root_choose", move || {
        picker
            .pick_download_root()?
            .map(|path| {
                path.into_os_string().into_string().map_err(|_| {
                    ApplicationError::from(ValidationError::new(
                        "downloadRoot",
                        "invalid path encoding",
                    ))
                })
            })
            .transpose()
    })
    .await)
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
pub async fn exploration_exclusion_context(
    state: State<'_, AppState>,
    gallery_id: i64,
) -> Result<ApiResult<crate::domain::ExplorationExclusionContext>, ApiError> {
    let service = state.service.clone();
    Ok(
        run_application_blocking("exploration_exclusion_context", move || {
            service.exploration_exclusion_context(gallery_id)
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
    let excluded = state.excluded_artifacts.clone();
    let previews = state.gallery_previews.clone();
    Ok(
        run_application_blocking("exploration_exclusions_restore", move || {
            let result = match excluded {
                Some(excluded) => excluded.restore_then(&gallery_ids, || {
                    service.exploration_exclusions_restore(gallery_ids.clone())
                })?,
                None => service.exploration_exclusions_restore(gallery_ids.clone())?,
            };
            if let Some(previews) = previews {
                for id in gallery_ids {
                    if let Ok(id) = crate::domain::GalleryId::new(id) {
                        previews.enqueue(id);
                    }
                }
            }
            Ok(result)
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
    gallery_ids: Option<Vec<i64>>,
) -> Result<ApiResult<DuplicateScanRun>, ApiError> {
    let managed_work = state.managed_work();
    let duplicates = state.duplicates.clone();
    Ok(run_application_blocking("duplicate_scan_start", move || {
        prepare_then_commit_managed_work(
            &managed_work,
            || match gallery_ids {
                Some(ids) => duplicates.prepare_selected(ids),
                None => duplicates.prepare_start(),
            },
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
    let excluded = state.excluded_artifacts.clone();
    let previews = state.gallery_previews.clone();
    Ok(
        run_application_blocking("duplicate_decision_apply", move || {
            let review = duplicates.decision_apply(request)?;
            if let Some(excluded) = excluded {
                match excluded.reconcile() {
                    Ok(report) => {
                        for issue in &report.issues { tracing::warn!(issue, "excluded folder move was deferred"); }
                        if let Some(previews) = previews {
                            for id in report.gallery_ids { if let Ok(id) = crate::domain::GalleryId::new(id) { previews.enqueue(id); } }
                        }
                    }
                    Err(error) => tracing::warn!(error = %error, "excluded folder reconciliation was deferred"),
                }
            }
            Ok(review)
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
pub async fn download_overlap_automation_history_list(
    state: State<'_, AppState>,
    request: DownloadOverlapAutomationHistoryListRequest,
) -> Result<ApiResult<DownloadOverlapAutomationHistoryPage>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(
        run_application_blocking("download_overlap_automation_history_list", move || {
            downloads.overlap_automation_history_list(request)
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_overlap_automation_history_acknowledge(
    state: State<'_, AppState>,
    review_id: String,
) -> Result<ApiResult<DownloadOverlapAutomationHistoryItem>, ApiError> {
    let downloads = state.downloads.clone();
    Ok(run_application_blocking(
        "download_overlap_automation_history_acknowledge",
        move || downloads.overlap_automation_history_acknowledge(review_id),
    )
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_overlap_decision_apply(
    state: State<'_, AppState>,
    request: DownloadOverlapDecisionRequest,
) -> Result<ApiResult<DownloadOverlapDecisionResult>, ApiError> {
    let downloads = state.downloads.clone();
    let merges = state.overlap_merges.clone();
    let managed_work = state.managed_work.clone();
    let excluded = state.excluded_artifacts.clone();
    Ok(
        run_application_blocking("download_overlap_decision_apply", move || {
            if request.review_id.starts_with("completed-pair:") {
                return Err(ApplicationError::DownloadOverlapDecisionInvalid("완료 앨범의 원래 대조 결과를 다시 열어 판정해 주세요".into()));
            }
            if request.review_id.starts_with("duplicate:") {
                return managed_work.run(|| {
                    let merges = merges.ok_or_else(|| crate::application::RepositoryError::Other("page merge service is unavailable".into()))?;
                    let request = merges.prepare_completed_decision(request)?;
                    let review_id = request.review_id.clone();
                    let result = downloads.with_overlap_review_lock(&review_id, || merges.decide_completed_pair(request))?;
                    if let Some(excluded) = excluded {
                        match excluded.reconcile() {
                            Ok(report) => for issue in report.issues { tracing::warn!(issue, "completed pair exclusion relocation deferred"); },
                            Err(error) => tracing::warn!(%error, "completed pair exclusion relocation deferred"),
                        }
                    }
                    Ok(result)
                });
            }
            downloads.overlap_decision_apply(request)
        })
        .await,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_overlap_merge(
    state: State<'_, AppState>,
    request: DownloadOverlapMergeRequest,
) -> Result<ApiResult<DownloadOverlapMergeResult>, ApiError> {
    let downloads = state.downloads.clone();
    let merges = state.overlap_merges.clone();
    let managed_work = state.managed_work.clone();
    let thumbnails = state.thumbnails.clone();
    let disk = state.thumbnail_disk_cache.clone();
    let previews = state.gallery_previews.clone();
    let excluded = state.excluded_artifacts.clone();
    Ok(run_application_blocking("download_overlap_merge", move || {
        managed_work.run(|| {
            let merges = merges.ok_or_else(|| {
                crate::application::RepositoryError::Other("page merge service is unavailable".into())
            })?;
            let request = merges.prepare_completed_merge(request)?;
            let review_id = request.review_id.clone();
            let review = downloads.overlap_review_get(&review_id)?
                .ok_or_else(|| ApplicationError::DownloadOverlapReviewNotFound(review_id.clone()))?;
            let candidate = review.candidates.iter().find(|item| item.candidate_id == request.candidate_id)
                .ok_or_else(|| ApplicationError::DownloadOverlapDecisionInvalid("병합할 후보가 변경되었습니다".into()))?;
            let source_is_existing = matches!(request.source_side, DownloadOverlapMergeSide::Existing);
            let target = if source_is_existing { &review.incoming } else { &candidate.existing };
            let mut keys = vec![ThumbnailKey::GalleryCover { gallery_id: target.gallery_id.get() }];
            for pair in &candidate.page_pairs {
                let (source_page, target_page) = if source_is_existing {
                    (pair.existing_source_page, pair.incoming_source_page)
                } else { (pair.incoming_source_page, pair.existing_source_page) };
                if request.source_pages.contains(&source_page) {
                    keys.push(ThumbnailKey::ArtifactPage { entry_id: target.entry_id.clone(), source_page: target_page });
                    keys.push(ThumbnailKey::GalleryPage { gallery_id: target.gallery_id.get(), source_page: target_page });
                }
            }
            if request.selected_pages.is_some() {
                for number in 1..=target.page_count + request.source_pages.len() as u32 {
                    keys.push(ThumbnailKey::ArtifactPage { entry_id: target.entry_id.clone(), source_page: number });
                    keys.push(ThumbnailKey::GalleryPage { gallery_id: target.gallery_id.get(), source_page: number });
                }
            }
            let result = downloads.with_overlap_merge_lock(&review_id, || merges.apply(request))?;
            // A committed merge must never be reported as failed because a
            // recreatable preview or delayed exclusion refresh failed afterward.
            for key in keys {
                let _ = thumbnails.invalidate(&key);
                if let Some(cache) = disk.as_ref() {
                    if let Err(error) = cache.invalidate(&key) {
                        tracing::warn!(merge_id = %result.merge_id, error = %error, "merged page disk thumbnail invalidation deferred");
                    }
                }
                let _ = thumbnails.invalidate(&key);
            }
            if let Some(previews) = previews {
                previews.enqueue(result.target_gallery_id);
                if let Err(error) = previews.publish_saved(&[result.source_gallery_id, result.target_gallery_id], true) {
                    tracing::warn!(merge_id = %result.merge_id, error = %error, "merged gallery preview update deferred");
                }
            }
            if result.source_excluded {
                if let Some(excluded) = excluded {
                    match excluded.reconcile() {
                        Ok(report) => for issue in report.issues {
                            tracing::warn!(merge_id = %result.merge_id, issue, "merged source folder relocation deferred");
                        },
                        Err(error) => tracing::warn!(merge_id = %result.merge_id, error = %error, "merged source folder relocation deferred"),
                    }
                }
            }
            Ok(result)
        })
    }).await)
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
    let previews = state.gallery_previews.clone();
    Ok(run_application_blocking("internal_removal_apply", move || {
        let result = supervisor.removal_apply(request)?;
        if let Some(previews) = previews {
            let ids = result
                .records
                .iter()
                .map(|record| record.gallery_id)
                .collect::<Vec<_>>();
            if let Err(error) = previews.publish_saved(&ids, false) {
                tracing::warn!(error = %error, "could not publish updated gallery preview");
            }
        }
        Ok(result)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn internal_removal_undo(
    state: State<'_, AppState>,
    request: InternalRemovalUndoRequest,
) -> Result<ApiResult<InternalRemovalResult>, ApiError> {
    let supervisor = state.internal_duplicates.clone();
    let previews = state.gallery_previews.clone();
    Ok(run_application_blocking("internal_removal_undo", move || {
        let result = supervisor.removal_undo(request)?;
        if let Some(previews) = previews {
            let ids = result
                .records
                .iter()
                .map(|record| record.gallery_id)
                .collect::<Vec<_>>();
            if let Err(error) = previews.publish_saved(&ids, false) {
                tracing::warn!(error = %error, "could not publish updated gallery preview");
            }
        }
        Ok(result)
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
pub async fn download_popularity_snapshot(
    state: State<'_, AppState>,
    period: crate::download_popularity::Period,
) -> Result<crate::download_popularity::PopularitySnapshot, String> {
    let source = state.live_source.clone();
    let path = state.data_dir.join("atsumi-next.sqlite3");
    tauri::async_runtime::spawn_blocking(move || {
        crate::download_popularity::snapshot(&path, &source, period)
    })
    .await
    .map_err(|e| e.to_string())?
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
    let cache_limit_changed = patch.cache_limit_gb.is_some();
    let processing_changed = patch.high_performance_processing.is_some();
    match state.service.settings_update(patch, expected_revision) {
        Ok(snapshot) => {
            if processing_changed {
                crate::application::image_work_budget::configure(
                    snapshot.high_performance_processing,
                );
            }
            if let Err(error) = state.thumbnails.reconfigure(ThumbnailRuntimeConfigDto {
                concurrent_image_requests: snapshot.concurrent_image_requests,
                request_start_interval_ms: snapshot.request_start_interval_ms,
            }) {
                tracing::warn!(error = %error, "could not apply thumbnail worker settings");
            }
            if let Err(error) = app.emit("settings:changed", &snapshot) {
                tracing::warn!(error = %error, "could not emit settings:changed");
            }
            if let Some(cache) = state
                .thumbnail_disk_cache
                .clone()
                .filter(|_| cache_limit_changed)
            {
                let max_bytes = u64::from(snapshot.cache_limit_gb) * 1024 * 1024 * 1024;
                match tauri::async_runtime::spawn_blocking(move || cache.set_max_bytes(max_bytes))
                    .await
                {
                    Ok(Ok(())) => {}
                    error => tracing::warn!(?error, "could not apply disk thumbnail cache limit"),
                }
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
pub async fn gallery_preview_list(
    state: State<'_, AppState>,
    gallery_ids: Vec<i64>,
) -> Result<ApiResult<Vec<GalleryPreview>>, ApiError> {
    let previews = match state.preview_service() {
        Ok(service) => service,
        Err(error) => return Ok(Err::<Vec<GalleryPreview>, _>(error).into()),
    };
    Ok(run_application_blocking("gallery_preview_list", move || {
        let ids = gallery_ids
            .into_iter()
            .map(crate::domain::GalleryId::new)
            .collect::<Result<Vec<_>, _>>()?;
        previews.list(&ids)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn gallery_preview_set(
    state: State<'_, AppState>,
    request: GalleryPreviewSetRequest,
) -> Result<ApiResult<GalleryPreview>, ApiError> {
    let previews = match state.preview_service() {
        Ok(service) => service,
        Err(error) => return Ok(Err::<GalleryPreview, _>(error).into()),
    };
    Ok(run_application_blocking("gallery_preview_set", move || previews.set(request)).await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn artist_preview_list(
    state: State<'_, AppState>,
    artists: Vec<String>,
) -> Result<ApiResult<Vec<ArtistPreview>>, ApiError> {
    let previews = match state.preview_service() {
        Ok(service) => service,
        Err(error) => return Ok(Err::<Vec<ArtistPreview>, _>(error).into()),
    };
    Ok(run_application_blocking("artist_preview_list", move || {
        previews.artist_list(&artists)
    })
    .await)
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
    window: tauri::Webview,
    request: ThumbnailRequestDto,
    epoch: String,
) -> Result<ApiResult<ThumbnailRequestTokenDto>, ApiError> {
    use tauri::Emitter;
    // CHZZK receivers are child webviews of the same native window. Request
    // the calling document, not WebviewWindow (which rejects multi-webview
    // windows before this handler can run).
    if window.label() != "main" || state.thumbnail_transport.epoch() != epoch {
        return Ok(ApiResult::failure(ApiError {
            code: "THUMBNAIL_STALE_DOCUMENT".into(),
            message: "The thumbnail document was replaced".into(),
            retryable: false,
            action: Some(super::ApiAction::None),
            details: None,
        }));
    }
    let transport = Arc::clone(&state.thumbnail_transport);
    match state.thumbnails.request_with_callback(
        request,
        Arc::new(move |event| {
            let request_id = event.request_id.clone();
            if let Some(value) = transport.publish(&epoch, event) {
                if window.emit("thumbnail:ready", value).is_err() {
                    transport.cancel_request(&request_id);
                }
            }
        }),
    ) {
        Ok(token) => Ok(ApiResult::success(token)),
        Err(error) => Ok(ApiResult::failure(thumbnail_coordinator_error(error))),
    }
}

#[tauri::command]
pub fn thumbnail_session(state: State<'_, AppState>) -> ApiResult<String> {
    ApiResult::success(state.thumbnail_transport.epoch())
}

#[tauri::command]
pub fn thumbnail_read(
    window: tauri::Webview,
    state: State<'_, AppState>,
    token: String,
) -> Result<tauri::ipc::Response, String> {
    if window.label() != "main" {
        return Err("Thumbnail bodies are restricted to the main document".into());
    }
    state
        .thumbnail_transport
        .read(&token)
        .map(tauri::ipc::Response::new)
        .ok_or_else(|| "Thumbnail capability expired or was already consumed".into())
}

#[tauri::command]
pub fn thumbnail_release(
    window: tauri::Webview,
    state: State<'_, AppState>,
    token: String,
) -> ApiResult<bool> {
    ApiResult::success(window.label() == "main" && state.thumbnail_transport.release(&token))
}

#[tauri::command(rename_all = "camelCase")]
pub fn thumbnail_cancel(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<ApiResult<bool>, ApiError> {
    state.thumbnail_transport.cancel_request(request_id.trim());
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
pub async fn thumbnail_invalidate(
    state: State<'_, AppState>,
    key: ThumbnailKey,
) -> Result<ApiResult<ThumbnailInvalidationDto>, ApiError> {
    if let Err(error) = key.validate() {
        return Ok(ApiResult::failure(ApiError {
            code: "THUMBNAIL_REQUEST_INVALID".into(),
            message: error.to_string(),
            retryable: false,
            action: Some(super::ApiAction::None),
            details: None,
        }));
    }
    state.thumbnail_transport.invalidate(&key);
    let thumbnails = state.thumbnails.clone();
    let disk = state.thumbnail_disk_cache.clone();
    Ok(run_application_blocking("thumbnail_invalidate", move || {
        let mut result = thumbnails.invalidate(&key).expect("key was validated");
        let disk_result = disk
            .as_ref()
            .map(|cache| cache.invalidate(&key))
            .transpose();
        let after = thumbnails.invalidate(&key).expect("key was validated");
        result.success_cache_removed |= after.success_cache_removed;
        result.negative_cache_removed |= after.negative_cache_removed;
        result.success_cache_removed |= disk_result.map_err(thumbnail_disk_error)?.unwrap_or(false);
        Ok(result)
    })
    .await)
}

#[tauri::command]
pub fn thumbnail_stats(
    state: State<'_, AppState>,
) -> Result<ApiResult<ThumbnailWorkerStatsDto>, ApiError> {
    Ok(ApiResult::success(state.thumbnails.stats()))
}

#[tauri::command]
pub async fn thumbnail_cache_clear(
    state: State<'_, AppState>,
) -> Result<ApiResult<ThumbnailCacheClearDto>, ApiError> {
    let thumbnails = state.thumbnails.clone();
    let disk = state.thumbnail_disk_cache.clone();
    Ok(run_application_blocking("thumbnail_cache_clear", move || {
        clear_thumbnail_caches(&thumbnails, disk.as_deref())
    })
    .await)
}

fn thumbnail_disk_error(error: std::io::Error) -> ApplicationError {
    crate::application::RepositoryError::Other(format!(
        "thumbnail disk cache operation failed: {error}"
    ))
    .into()
}

fn clear_thumbnail_caches(
    thumbnails: &ThumbnailCoordinator,
    disk: Option<&ThumbnailDiskCache>,
) -> Result<ThumbnailCacheClearDto, ApplicationError> {
    let mut result = thumbnails.clear_cache();
    let disk_result = disk.map(ThumbnailDiskCache::clear).transpose();
    // Also fence work which started during disk cleanup, even when cleanup failed.
    let after = thumbnails.clear_cache();
    result.success_entries_removed += after.success_entries_removed;
    result.success_bytes_removed += after.success_bytes_removed;
    result.negative_entries_removed += after.negative_entries_removed;
    disk_result.map_err(thumbnail_disk_error)?;
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_retry(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
    fill_available: Option<bool>,
) -> Result<ApiResult<Vec<JobRef>>, ApiError> {
    let managed_work = state.managed_work();
    match managed_work.run(|| {
        let job_refs = if fill_available.unwrap_or(false) {
            state.service.download_retry_available(entry_ids)?
        } else {
            state.service.download_retry(entry_ids)?
        };
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
                    linked_gallery_id: entry.linked_gallery_id,
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
    let previews = state.gallery_previews.clone();
    Ok(run_application_blocking("download_quarantine", move || {
        let result = downloads.quarantine_entries(entry_ids, reason)?;
        if let Some(previews) = previews {
            let ids = result
                .iter()
                .map(|entry| entry.gallery_id)
                .collect::<Vec<_>>();
            if let Err(error) = previews.publish_saved(&ids, true) {
                tracing::warn!(error = %error, "could not publish updated gallery availability");
            }
        }
        Ok(result)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn download_quarantine_undo(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
) -> Result<ApiResult<Vec<DownloadEntry>>, ApiError> {
    let downloads = state.downloads.clone();
    let previews = state.gallery_previews.clone();
    Ok(
        run_application_blocking("download_quarantine_undo", move || {
            let result = downloads.restore_entries(entry_ids)?;
            if let Some(previews) = previews {
                let ids = result.iter().map(|entry| entry.gallery_id).collect::<Vec<_>>();
                if let Err(error) = previews.publish_saved(&ids, true) { tracing::warn!(error = %error, "could not publish updated gallery availability"); }
            }
            Ok(result)
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
    if matches!(action, MaintenanceAction::FactoryReset { .. }) {
        let reservation = match state.managed_work.reserve_factory_reset() {
            Ok(reservation) => reservation,
            Err(error) => return Ok(ApiResult::failure(error.into())),
        };
        let drain = state.blocking_work_drain(app.clone());
        let data_dir = state.data_dir.clone();
        let result = run_application_blocking("maintenance_factory_reset", move || {
            finish_factory_reset(reservation, drain, || publish_factory_reset_marker(&data_dir))?;
            Ok(MaintenanceResult {
                action,
                completed_steps: vec!["factory reset scheduled for the next startup".into()],
                warnings: vec!["the app will now exit; external originals are unchanged".into()],
                restart_required: true,
            })
        }).await;
        return Ok(match result {
            ApiResult::Success(result) => {
                app.exit(0);
                ApiResult::Success(result)
            }
            ApiResult::Failure(_) => ApiResult::failure(ApiError {
                code: "FACTORY_RESET_INCOMPLETE".into(),
                message: "초기화를 예약하지 못했습니다. 새 작업은 차단된 상태입니다. 앱을 정상 종료한 뒤 다시 열어 주세요. 기존 파일은 삭제하지 않았습니다.".into(),
                retryable: false,
                action: Some(ApiAction::None),
                details: None,
            }),
        });
    }
    if matches!(action, MaintenanceAction::QuickRepair) {
        state.search_pages.cancel_all();
    }

    let thumbnails = state.thumbnails.clone();
    let live_source = Arc::clone(&state.live_source);
    let thumbnail_disk_cache = state.thumbnail_disk_cache.clone();
    let downloads = state.downloads.clone();
    let auto_find = state.auto_find.clone();
    let duplicates = state.duplicates.clone();
    let internal_duplicates = state.internal_duplicates.clone();
    let managed_work = state.managed_work();
    let execute_action = action.clone();

    let result = run_application_blocking("maintenance_execute", move || {
        let mut completed_steps = Vec::new();
        let mut warnings = Vec::new();
        match &execute_action {
            MaintenanceAction::QuickRepair => {
                if let Err(error) = clear_thumbnail_caches(&thumbnails, thumbnail_disk_cache.as_deref()) {
                    warnings.push(error.to_string());
                }
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
                    if let Err(error) = clear_thumbnail_caches(&thumbnails, thumbnail_disk_cache.as_deref()) {
                        warnings.push(error.to_string());
                    }
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
                // Handled by the reserved, shared-drain branch above.
                Err(ApplicationError::AppQuitInProgress)
            }
        }
    }).await;
    Ok(result)
}

#[tauri::command]
pub async fn app_minimize_to_tray(window: Window) -> Result<ApiResult<()>, ApiError> {
    match crate::minimize_to_tray(|| window.hide()) {
        Ok(()) => {
            let _ = window.emit("platform-media:tray-muted", true);
            if let Some(host) = window.app_handle().try_state::<crate::soop::original::SoopOriginalHost>() {
                host.set_tray_muted(window.app_handle(), true);
            }
            if let Some(host) = window.app_handle().try_state::<crate::youtube::original::YouTubeOriginalHost>() {
                host.set_tray_muted(window.app_handle(), true);
            }
            Ok(ApiResult::success(()))
        },
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
pub async fn gallery_summary_get(
    state: State<'_, AppState>,
    gallery_id: i64,
) -> Result<ApiResult<GallerySummary>, ApiError> {
    let service = state.service.clone();
    let merges = state.overlap_merges.clone();
    Ok(run_application_blocking("gallery_summary_get", move || {
        let mut summary = service.gallery_summary_get(gallery_id)?;
        if let Some(merges) = merges {
            if let Some(count) = merges.composed_page_count(gallery_id)? {
                summary.pages = count;
            }
        }
        Ok(summary)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn gallery_detail_get(
    state: State<'_, AppState>,
    gallery_id: i64,
) -> Result<ApiResult<GalleryDetail>, ApiError> {
    let service = state.service.clone();
    let merges = state.overlap_merges.clone();
    Ok(run_application_blocking("gallery_detail_get", move || {
        let mut detail = service.gallery_detail_get(gallery_id)?;
        if let Some(merges) = merges {
            if let Some(count) = merges.composed_page_count(gallery_id)? {
                detail.summary.pages = count;
                // Online dimensions refer to the original order, not this edition.
                detail.page_dimensions.clear();
            }
        }
        Ok(detail)
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
    let span = tracing::info_span!("diagnostic_operation", diag_operation = operation_id);
    match tauri::async_runtime::spawn_blocking(move || {
        let _span = span.enter();
        let result = operation().map_err(ApiError::from);
        tracing::info!(diag_stage = "finished", success = result.is_ok());
        if let Err(error) = &result {
            tracing::warn!(
                error_code = error.code.as_str(),
                retryable = error.retryable
            );
        }
        result
    })
    .await
    {
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
    fn platform_development_tools_have_no_parent_traversal_components() {
        let root = platform_development_resource_root().unwrap();
        assert!(root.is_absolute());
        assert_eq!(root.file_name(), Some(std::ffi::OsStr::new(".runtime")));
        assert_eq!(root.parent(), std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent());
        assert!(!root.components().any(|part| matches!(part,
            std::path::Component::ParentDir | std::path::Component::CurDir)));
    }

    #[test]
    fn thumbnail_cleanup_clears_persistent_and_memory_entries() {
        use crate::thumbnail::{
            FixtureThumbnailResolver, ThumbnailConsumer, ThumbnailCoordinatorConfig,
        };
        let temporary = tempfile::tempdir().unwrap();
        let disk = ThumbnailDiskCache::new(temporary.path().join("cache"), 4096);
        let thumbnails = ThumbnailCoordinator::with_resolver(
            FixtureThumbnailResolver::new(),
            ThumbnailCoordinatorConfig::default(),
        )
        .unwrap();
        let key = ThumbnailKey::gallery_cover(1).unwrap();
        let delivery = thumbnails
            .request(ThumbnailRequestDto {
                key: key.clone(),
                consumer: ThumbnailConsumer::Downloads,
                priority: ThumbnailPriority::Visible,
            })
            .unwrap()
            .recv()
            .unwrap();
        disk.put(
            &key,
            "test-profile",
            disk.ticket(),
            &delivery.thumbnail,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(disk.usage().entries, 1);
        let result = clear_thumbnail_caches(&thumbnails, Some(&disk)).unwrap();
        assert_eq!(result.success_entries_removed, 1);
        assert_eq!(disk.usage().entries, 0);
        assert_eq!(thumbnails.clear_cache().success_entries_removed, 0);
        let restarted = ThumbnailDiskCache::new(temporary.path().join("cache"), 4096);
        assert!(restarted
            .get(&key, "test-profile", restarted.ticket())
            .is_none());
    }

    #[test]
    fn thumbnail_cleanup_reports_unavailable_disk_and_still_clears_memory() {
        use crate::thumbnail::{
            FixtureThumbnailResolver, ThumbnailConsumer, ThumbnailCoordinatorConfig,
        };
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("not-a-cache-directory");
        std::fs::write(&path, b"keep").unwrap();
        let disk = ThumbnailDiskCache::new(&path, 4096);
        let thumbnails = ThumbnailCoordinator::with_resolver(
            FixtureThumbnailResolver::new(),
            ThumbnailCoordinatorConfig::default(),
        )
        .unwrap();
        thumbnails
            .request(ThumbnailRequestDto {
                key: ThumbnailKey::gallery_cover(1).unwrap(),
                consumer: ThumbnailConsumer::Downloads,
                priority: ThumbnailPriority::Visible,
            })
            .unwrap()
            .recv()
            .unwrap();
        assert!(clear_thumbnail_caches(&thumbnails, Some(&disk)).is_err());
        assert_eq!(thumbnails.clear_cache().success_entries_removed, 0);
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
    }

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
            recordings: None,
            live_receivers: None,
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
            recordings: None,
            live_receivers: None,
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
    fn live_receiver_display_counts_preserve_all_ownership_and_unknown_work() {
        let ids = ["platform-official:receiver-a", "platform-live:receiver-b", "platform-media:download-c", "platform-screenshot:capture", "platform-live:", "platform-officially:unknown"]
            .map(str::to_owned);
        let original = ids.clone();
        let fingerprint = active_work_fingerprint(ids.iter().map(String::as_str), None, None, None);
        let (downloads, receivers) = display_work_counts(&ids);
        assert_eq!(downloads.active_count, 4);
        assert_eq!(receivers.as_ref().map(|value| value.active_count), Some(2));
        assert_eq!(ids, original);
        assert_eq!(fingerprint, active_work_fingerprint(ids.iter().map(String::as_str), None, None, None));

        let (empty, no_receivers) = display_work_counts(&[]);
        assert_eq!(empty.active_count, 0);
        assert!(no_receivers.is_none());
        let (watch_only, receiver) = display_work_counts(&["platform-official:receiver-a".into()]);
        assert_eq!(watch_only.active_count, 0);
        assert_eq!(receiver.unwrap().active_count, 1);
    }

    #[test]
    fn live_receiver_only_quit_keeps_confirmation_and_stale_owner_fencing() {
        let ids = vec!["platform-official:receiver-a".into()];
        let mut snapshot = active_work_snapshot(0);
        (snapshot.downloads, snapshot.live_receivers) = display_work_counts(&ids);
        snapshot.work_set_fingerprint = active_work_fingerprint(ids.iter().map(String::as_str), None, None, None);
        assert!(snapshot.has_active_work());
        assert_eq!(snapshot.active_work_count(), 1);
        let request = AppQuitRequest {
            expected_work_set_fingerprint: snapshot.work_set_fingerprint.clone(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        assert_eq!(quit_rejection_reason(&request, &snapshot), Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired));
        let confirmed = AppQuitRequest { confirm_active_work: true, ..request };
        assert_eq!(quit_rejection_reason(&confirmed, &snapshot), None);
        snapshot.work_set_fingerprint = active_work_fingerprint(["platform-official:receiver-b"], None, None, None);
        assert_eq!(quit_rejection_reason(&confirmed, &snapshot), Some(AppQuitRejectionReason::ActiveWorkChanged));

        snapshot.downloads.active_count = 2;
        assert_eq!(snapshot.active_work_count(), 3);
        snapshot.downloads.active_count = u64::MAX;
        assert_eq!(snapshot.active_work_count(), u64::MAX);
        snapshot.downloads.active_count = 0;
        snapshot.live_receivers = Some(AppActiveLiveReceiversSnapshot { active_count: 0 });
        assert!(!snapshot.has_active_work());
        assert_eq!(snapshot.active_work_count(), 0);
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
    fn recording_only_work_requires_confirmation_and_has_identity_fingerprint() {
        let mut snapshot = active_work_snapshot(0);
        snapshot.recordings = Some(AppActiveRecordingsSnapshot { active_count: 1 });
        assert!(snapshot.has_active_work());
        assert_eq!(snapshot.active_work_count(), 1);
        let request = AppQuitRequest {
            expected_work_set_fingerprint: snapshot.work_set_fingerprint.clone(),
            confirm_active_work: false,
            force_when_status_unknown: false,
        };
        assert_eq!(
            quit_rejection_reason(&request, &snapshot),
            Some(AppQuitRejectionReason::ActiveWorkConfirmationRequired)
        );
        let ids = vec!["session-b".into(), "session-a".into()];
        let reversed = vec!["session-a".into(), "session-b".into(), "session-b".into()];
        assert_eq!(
            fingerprint_with_recordings("base".into(), &ids),
            fingerprint_with_recordings("base".into(), &reversed)
        );
        assert_ne!(
            fingerprint_with_recordings("base".into(), &ids),
            fingerprint_with_recordings("base".into(), &["session-c".into()])
        );
        assert_eq!(fingerprint_with_recordings("base".into(), &[]), "base");
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
    fn update_reservation_blocks_new_work_and_releases_after_failure() {
        let gate = ManagedWorkGate::default();
        gate.inner.control.lock().unwrap().updating = true;
        assert!(gate.run(|| Ok::<_, ApplicationError>(7)).is_err());
        gate.inner.control.lock().unwrap().updating = false;
        assert_eq!(gate.run(|| Ok(8)).unwrap(), 8);
    }

    #[test]
    fn platform_initialization_registers_without_holding_admission_lock() {
        let gate = ManagedWorkGate::default();
        let permit = gate.begin_platform_initialization().unwrap();
        assert!(gate.platform_initializing());
        assert!(gate.inner.control.try_lock().is_ok());
        let reset = gate.reserve_factory_reset().unwrap();
        assert!(gate.begin_platform_initialization().is_err());
        drop(permit);
        assert!(!gate.platform_initializing());
        drop(reset);
        assert!(gate.begin_platform_initialization().is_err());
        let gate = ManagedWorkGate::default();
        gate.inner.control.lock().unwrap().updating = true;
        assert!(gate.begin_platform_initialization().is_err());
        assert!(!gate.platform_initializing());
    }

    #[test]
    fn platform_initialization_drain_reads_only_the_settled_slot() {
        let gate = ManagedWorkGate::default();
        let slot = OnceLock::new();
        let mut permit = Some(gate.begin_platform_initialization().unwrap());
        let _reset = gate.reserve_factory_reset().unwrap();
        let mut waited = false;
        let service = settled_platform_service(&gate, &slot, || {
            assert!(!waited);
            assert!(slot.get().is_none());
            waited = true;
            slot.set("completed service").unwrap();
            drop(permit.take());
        });
        assert!(waited);
        assert_eq!(service, Some("completed service"));
    }

    #[test]
    fn platform_initialization_panic_releases_its_owned_count() {
        let gate = ManagedWorkGate::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = gate.begin_platform_initialization().unwrap();
            panic!("fixture initialization failure");
        }));
        assert!(result.is_err());
        assert!(!gate.platform_initializing());
        let _reset = gate.reserve_factory_reset().unwrap();
        let slot: OnceLock<String> = OnceLock::new();
        assert_eq!(settled_platform_service(&gate, &slot, || panic!("no remaining initializer")), None);
    }

    #[test]
    fn factory_reset_reserves_admission_and_rejects_competing_quit_dispatch() {
        let gate = ManagedWorkGate::default();
        let reservation = gate.reserve_factory_reset().unwrap();
        assert!(gate.run(|| Ok(())).is_err());
        assert!(gate.reserve_factory_reset().is_err());
        let mut control = gate.inner.control.lock().unwrap();
        let request = AppQuitRequest {
            expected_work_set_fingerprint: String::new(),
            confirm_active_work: true,
            force_when_status_unknown: false,
        };
        let (accepted, dispatch) = gate.evaluate_quit_locked(&mut control, &request, None);
        assert!(accepted.accepted);
        assert!(!dispatch);
        drop(control);
        drop(reservation);
        assert!(gate.admission_closed());
        assert!(!gate.inner.quitting.load(Ordering::Acquire));
        assert!(gate.run(|| Ok(())).is_err());
        assert!(gate.reserve_factory_reset().is_err());
        let mut control = gate.inner.control.lock().unwrap();
        let (accepted, dispatch) = gate.evaluate_quit_locked(&mut control, &request, None);
        assert!(accepted.accepted);
        assert!(dispatch, "explicit quit must work after a failed reset");
    }

    #[test]
    fn factory_reset_rejects_update_or_quit_without_changing_gate() {
        let gate = ManagedWorkGate::default();
        gate.inner.control.lock().unwrap().updating = true;
        assert!(gate.reserve_factory_reset().is_err());
        assert!(!gate.admission_closed());
        gate.inner.control.lock().unwrap().updating = false;
        gate.inner.quitting.store(true, Ordering::Release);
        assert!(gate.reserve_factory_reset().is_err());
        assert!(!gate.inner.maintenance_stopped.load(Ordering::Acquire));
    }

    #[test]
    fn factory_reset_stale_preflight_cannot_commit() {
        let gate = ManagedWorkGate::default();
        let worker_gate = gate.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let committed = Arc::new(AtomicBool::new(false));
        let worker_committed = committed.clone();
        let worker = thread::spawn(move || prepare_then_commit_managed_work(&worker_gate, || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        }, |()| { worker_committed.store(true, Ordering::Release); Ok(()) }));
        entered_rx.recv().unwrap();
        let reservation = gate.reserve_factory_reset().unwrap();
        release_tx.send(()).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(!committed.load(Ordering::Acquire));
        drop(reservation);
    }

    #[test]
    fn factory_reset_publishes_only_after_drain_and_keeps_successful_quit_reserved() {
        let gate = ManagedWorkGate::default();
        let events = std::cell::RefCell::new(Vec::new());
        finish_factory_reset(gate.reserve_factory_reset().unwrap(), || {
            assert!(gate.run(|| Ok(())).is_err());
            events.borrow_mut().push("drained");
            Ok(())
        }, || {
            assert_eq!(*events.borrow(), ["drained"]);
            events.borrow_mut().push("marker");
            Ok(())
        }).unwrap();
        assert_eq!(events.into_inner(), ["drained", "marker"]);
        assert!(gate.inner.quitting.load(Ordering::Acquire));
        assert!(gate.admission_closed());
    }

    #[test]
    fn factory_reset_shared_drain_waits_for_every_platform_before_marker() {
        let gate = ManagedWorkGate::default();
        let events = std::cell::RefCell::new(Vec::new());
        // Platform counts settle separately, including a late scheduler worker.
        let polls = std::cell::Cell::new(0usize);
        let remaining = [[1, 1, 1, 1], [0, 1, 1, 1], [0, 0, 1, 1], [0, 0, 0, 1], [0, 0, 0, 0]];
        finish_factory_reset(gate.reserve_factory_reset().unwrap(), || {
            drain_managed_workers(
                || events.borrow_mut().push("stop"),
                || remaining[polls.get()].iter().any(|count| *count > 0),
                || { polls.set(polls.get() + 1); events.borrow_mut().push("wait"); },
                || events.borrow_mut().push("legacy-drained"),
            );
            Ok(())
        }, || { events.borrow_mut().push("marker"); Ok(()) }).unwrap();
        assert_eq!(events.into_inner(), ["stop", "wait", "wait", "wait", "wait", "legacy-drained", "marker"]);
    }

    #[test]
    fn factory_reset_failed_marker_and_panicked_drain_never_reopen_admission() {
        let gate = ManagedWorkGate::default();
        let result = finish_factory_reset(gate.reserve_factory_reset().unwrap(), || Ok(()), || {
            Err(std::io::Error::other("fixture marker failure"))
        });
        assert!(result.is_err());
        assert!(gate.admission_closed());
        assert!(!gate.inner.quitting.load(Ordering::Acquire));

        let gate = ManagedWorkGate::default();
        let published = AtomicBool::new(false);
        let reservation = gate.reserve_factory_reset().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            finish_factory_reset(reservation, || panic!("fixture drain failure"), || {
                published.store(true, Ordering::Release); Ok(())
            })
        }));
        assert!(result.is_err());
        assert!(!published.load(Ordering::Acquire));
        assert!(gate.admission_closed());
        assert!(!gate.inner.quitting.load(Ordering::Acquire));
    }

    #[test]
    fn factory_reset_terminal_persistence_failure_never_publishes_marker() {
        let gate = ManagedWorkGate::default();
        let published = AtomicBool::new(false);
        let result = finish_factory_reset(gate.reserve_factory_reset().unwrap(), || {
            Err(crate::application::RepositoryError::Other("fixture journal write failed".into()).into())
        }, || { published.store(true, Ordering::Release); Ok(()) });
        assert!(result.is_err());
        assert!(!published.load(Ordering::Acquire));
        assert!(gate.admission_closed());
        assert!(!gate.inner.quitting.load(Ordering::Acquire));
    }

    #[test]
    fn factory_reset_marker_is_complete_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        publish_factory_reset_marker(root.path()).unwrap();
        let marker = root.path().join("factory-reset.pending");
        assert_eq!(std::fs::read(&marker).unwrap(), b"v1\n");
        std::fs::write(&marker, b"existing marker").unwrap();
        assert!(publish_factory_reset_marker(root.path()).is_err());
        assert_eq!(std::fs::read(&marker).unwrap(), b"existing marker");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn factory_reset_marker_failure_preserves_existing_directory_and_no_temp() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("factory-reset.pending");
        std::fs::create_dir(&marker).unwrap();
        let sentinel = marker.join("owned-elsewhere");
        std::fs::write(&sentinel, b"keep").unwrap();
        assert!(publish_factory_reset_marker(root.path()).is_err());
        assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
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
