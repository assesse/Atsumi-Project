//! Explicit link-access YouTube archival. Original media and sidecars are never deleted.
mod catalog;
pub(crate) mod live_api;
mod automatic;
pub(crate) mod live_relay;
mod live_store;
mod live_service;
mod live_archive;
mod relay_timing_read;
mod live_sidecar;
mod live_replay_session;
mod live_replay_host;
mod live_replay_index;
mod live_replay_disk_index;
mod live_replay_measure;
pub(crate) mod live_fingerprint;
pub(crate) mod live_alignment;
#[cfg(test)]
pub(crate) use live_sidecar::verify_actual_joined_history;
pub mod commands;
mod model;
mod oauth;
pub mod source;
pub(crate) mod replay;
#[cfg(all(test, windows))]
pub(crate) mod replay_receipt_actual;
#[cfg(all(test, windows))]
pub(crate) mod replay_receipt_private_actual;
#[cfg(test)]
mod tests;
mod upload;
mod vault;

use catalog::{Catalog, Job};
use model::now;
pub use model::{
    CloudAccountSnapshot, CloudError, CloudJobSnapshot, CloudSnapshot, CloudSource, CloudSourceRef,
    CloudVisibility, SourcePreparationProgress,
    CloudLiveAccountSnapshot, CloudLiveOperation, CloudLivePlan, CloudLiveSnapshot,
};
pub use automatic::{AutomaticCandidate, AutomaticCandidateResolver, AutomaticReadiness, CloudAutomaticRule, CloudAutomaticSnapshot, CloudAutomaticTarget};
use oauth::BrowserOpener;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const PROCESSING_CHECK_DELAY_MS: u64 = 15_000;
const PROCESSING_WAIT_LIMIT_MS: u64 = 86_400_000;

pub struct SourceResolveContext {
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<dyn Fn(SourcePreparationProgress) + Send + Sync>,
}
pub type SourceResolver = Arc<
    dyn Fn(&CloudSourceRef, &SourceResolveContext) -> Result<CloudSource, String> + Send + Sync,
>;
pub type PlanResolver = Arc<
    dyn Fn(&CloudSourceRef, &SourceResolveContext) -> Result<Vec<CloudSourceRef>, String>
        + Send
        + Sync,
>;
struct Inner {
    catalog: Catalog,
    account_error: Option<CloudError>,
    authorizing: bool,
    auth_workers: usize,
    auth_generation: u64,
    auth_cancel: Option<Arc<AtomicBool>>,
    active: Option<(String, Arc<AtomicBool>)>,
    worker_running: bool,
    closing: bool,
    persistence_error: Option<CloudError>,
    preparation_progress_at: Option<Instant>,
    automatic_running: bool,
    automatic_observations: std::collections::HashMap<String, AutomaticReadiness>,
    automatic_error: Option<CloudError>,
    live: live_service::Runtime,
}
pub struct CloudService {
    inner: Mutex<Inner>,
    wake: Condvar,
    threads: Mutex<Vec<JoinHandle<()>>>,
    vault: vault::Vault,
    resolver: SourceResolver,
    planner: Option<PlanResolver>,
    open_browser: BrowserOpener,
    transport: Arc<dyn upload::Transport>,
}
impl CloudService {
    pub fn new(
        data_dir: PathBuf,
        resolver: SourceResolver,
        open_browser: BrowserOpener,
    ) -> Result<Arc<Self>, CloudError> {
        Self::with_transport(
            data_dir,
            resolver,
            open_browser,
            Arc::new(upload::GoogleTransport::new()?),
        )
    }
    pub fn new_with_planner(
        data_dir: PathBuf,
        resolver: SourceResolver,
        open_browser: BrowserOpener,
        planner: PlanResolver,
    ) -> Result<Arc<Self>, CloudError> {
        Self::with_transport_and_planner(
            data_dir,
            resolver,
            open_browser,
            Arc::new(upload::GoogleTransport::new()?),
            Some(planner),
        )
    }
    fn with_transport(
        data_dir: PathBuf,
        resolver: SourceResolver,
        open_browser: BrowserOpener,
        transport: Arc<dyn upload::Transport>,
    ) -> Result<Arc<Self>, CloudError> {
        Self::with_transport_and_planner(data_dir, resolver, open_browser, transport, None)
    }
    fn with_transport_and_planner(
        data_dir: PathBuf,
        resolver: SourceResolver,
        open_browser: BrowserOpener,
        transport: Arc<dyn upload::Transport>,
        planner: Option<PlanResolver>,
    ) -> Result<Arc<Self>, CloudError> {
        let vault = vault::Vault::open(&data_dir)?;
        let mut catalog = vault.load()?;
        catalog.recover();
        // Manual jobs remain paused. Only selected automatic rules may later
        // resume their persisted intent under the matching connected owner.
        if !catalog.jobs.is_empty() || !catalog.live_receipts.is_empty() {
            vault.save(&catalog)?;
        }
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                catalog,
                account_error: None,
                authorizing: false,
                auth_workers: 0,
                auth_generation: 0,
                auth_cancel: None,
                active: None,
                worker_running: false,
                closing: false,
                persistence_error: None,
                preparation_progress_at: None,
                automatic_running: false,
                automatic_observations: Default::default(),
                automatic_error: None,
                live: Default::default(),
            }),
            wake: Condvar::new(),
            threads: Mutex::new(Vec::new()),
            vault,
            resolver,
            planner,
            open_browser,
            transport,
        }))
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, CloudError> {
        self.inner.lock().map_err(|_| CloudError::storage())
    }
    fn snapshot_inner(inner: &Inner) -> CloudSnapshot {
        let grant = inner.catalog.grant.as_ref();
        CloudSnapshot {
            automatic: automatic::snapshot(inner),
            account: CloudAccountSnapshot {
                configured: inner.catalog.client.is_some(),
                connected: grant.is_some(),
                authorizing: inner.authorizing,
                channel_id: grant.map(|g| g.channel_id.clone()),
                channel_name: grant.map(|g| g.channel_name.clone()),
                error: inner.account_error.clone(),
            },
            jobs: inner
                .catalog
                .jobs
                .iter()
                .map(|job| {
                    let mut view = job.view.clone();
                    view.replay_cohort_id = replay::cohort_id(job);
                    if let Some(error) = &inner.persistence_error {
                        if matches!(
                            view.state.as_str(),
                            "queued" | "validating" | "uploading" | "retry_wait" | "processing"
                        ) {
                            view.state = "failed".into();
                            view.error = Some(error.clone());
                            view.retry_at = None;
                        }
                    }
                    view
                })
                .collect(),
        }
    }
    /// Publish memory only after the encrypted replacement is durable. A write
    /// failure closes admission until restart, rather than showing phantom success.
    fn publish(&self, inner: &mut Inner, candidate: Catalog) -> Result<(), CloudError> {
        if let Some(error) = &inner.persistence_error {
            return Err(error.clone());
        }
        match self.vault.save(&candidate) {
            Ok(()) => {
                inner.catalog = candidate;
                Ok(())
            }
            Err(error) => {
                if let Some((_, cancel)) = &inner.active {
                    cancel.store(true, Ordering::Release);
                }
                if let Some(cancel) = &inner.auth_cancel {
                    cancel.store(true, Ordering::Release);
                }
                inner.live.cancel_admission();
                inner.persistence_error = Some(error.clone());
                inner.account_error = Some(error.clone());
                Err(error)
            }
        }
    }
    pub fn snapshot(&self) -> Result<CloudSnapshot, CloudError> {
        let inner = self.lock()?;
        Ok(Self::snapshot_inner(&inner))
    }
    pub fn import_client_path(&self, path: &Path) -> Result<CloudSnapshot, CloudError> {
        let client = oauth::ClientConfig::read(path)?;
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner.authorizing || inner.worker_running || inner.live.busy() {
            return Err(CloudError::new(
                "CLOUD_BUSY",
                "진행 중인 업로드나 연결을 일시정지한 뒤 설정을 변경해 주세요.",
                true,
            ));
        }
        let changed = inner.catalog.client.as_ref().is_none_or(|v| {
            v.client_id != client.client_id || v.client_secret != client.client_secret
        });
        let mut candidate = inner.catalog.clone();
        candidate.client = Some(client);
        if changed {
            candidate.grant = None;
            candidate.live_grant = None;
            candidate.live_generation = candidate.live_generation.checked_add(1).ok_or_else(CloudError::storage)?;
            for receipt in &mut candidate.live_receipts {
                if receipt.view.state == "selected" {
                    receipt.view.state = "failed".into();
                    receipt.view.stop_requested = true;
                    receipt.view.error = Some(CloudError::new("CLOUD_LIVE_CHANGED", "YouTube 연결 설정이 바뀌었습니다. 방송을 다시 선택해 주세요.", false));
                    receipt.changed()?;
                }
            }
        }
        self.publish(&mut inner, candidate)?;
        if changed {
            inner.auth_generation = inner.auth_generation.wrapping_add(1);
            inner.live.discard_idle();
        }
        inner.account_error = None;
        Ok(Self::snapshot_inner(&inner))
    }
    fn admit(inner: &Inner) -> Result<(), CloudError> {
        if let Some(error) = &inner.persistence_error {
            return Err(error.clone());
        }
        if inner.closing {
            Err(CloudError::cancelled())
        } else {
            Ok(())
        }
    }
    fn add_thread(&self, handle: JoinHandle<()>) -> Result<(), CloudError> {
        let mut threads = self.threads.lock().map_err(|_| CloudError::storage())?;
        let mut index = 0;
        while index < threads.len() {
            if threads[index].is_finished() {
                let _ = threads.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        threads.push(handle);
        Ok(())
    }
    pub fn authorize(self: &Arc<Self>) -> Result<CloudSnapshot, CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner.authorizing || inner.auth_workers > 0 || inner.catalog.grant.is_some() {
            return Ok(Self::snapshot_inner(&inner));
        }
        let client = inner.catalog.client.clone().ok_or_else(|| {
            CloudError::new(
                "CLOUD_CLIENT_REQUIRED",
                "먼저 Google Cloud의 데스크톱 앱용 OAuth JSON을 선택해 주세요.",
                false,
            )
        })?;
        inner.authorizing = true;
        inner.auth_workers += 1;
        inner.account_error = None;
        inner.auth_generation = inner.auth_generation.wrapping_add(1);
        let generation = inner.auth_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        inner.auth_cancel = Some(cancel.clone());
        let service = self.clone();
        let handle = std::thread::Builder::new()
            .name("atsumi-cloud-auth".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    oauth::authorize(
                        service.transport.as_ref(),
                        &client,
                        service.open_browser.as_ref(),
                        &cancel,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(CloudError::new(
                        "CLOUD_AUTH_STOPPED",
                        "Google 연결 작업이 중단됐습니다. 다시 연결해 주세요.",
                        true,
                    ))
                });
                if let Ok(mut inner) = service.lock() {
                    inner.auth_workers = inner.auth_workers.saturating_sub(1);
                    if inner.auth_generation != generation {
                        return;
                    }
                    inner.authorizing = false;
                    inner.auth_cancel = None;
                    if inner.closing || cancel.load(Ordering::Acquire) {
                        return;
                    }
                    match result {
                        Ok(grant) => {
                            let mut candidate = inner.catalog.clone();
                            candidate.automatic.bind_unbound(&grant.channel_id);
                            candidate.grant = Some(grant);
                            if service.publish(&mut inner, candidate).is_ok() {
                                inner.account_error = None;
                            }
                        }
                        Err(error) => inner.account_error = Some(error),
                    }
                }
                service.wake.notify_all();
            })
            .map_err(|_| CloudError::storage());
        match handle {
            Ok(handle) => self.add_thread(handle)?,
            Err(error) => {
                inner.authorizing = false;
                inner.auth_workers = inner.auth_workers.saturating_sub(1);
                inner.auth_cancel = None;
                inner.account_error = Some(error.clone());
                return Err(error);
            }
        }
        Ok(Self::snapshot_inner(&inner))
    }
    pub fn disconnect(&self) -> Result<CloudSnapshot, CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        inner.auth_generation = inner.auth_generation.wrapping_add(1);
        if let Some(cancel) = inner.auth_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        if let Some((_, cancel)) = &inner.active {
            cancel.store(true, Ordering::Release);
        }
        inner.authorizing = false;
        let mut candidate = inner.catalog.clone();
        candidate.grant = None;
        for job in &mut candidate.jobs {
            if matches!(
                job.view.state.as_str(),
                "queued" | "validating" | "uploading" | "processing" | "retry_wait"
            ) {
                job.view.state = "waiting_auth".into();
                job.view.error = Some(CloudError::auth());
                job.view.retry_at = None;
            }
        }
        self.publish(&mut inner, candidate)?;
        inner.account_error = None;
        self.wake.notify_all();
        Ok(Self::snapshot_inner(&inner))
    }
    pub fn enqueue(self: &Arc<Self>, source: CloudSourceRef) -> Result<CloudSnapshot, CloudError> {
        self.enqueue_with_visibility(source, CloudVisibility::Private)
    }
    pub fn enqueue_with_visibility(self: &Arc<Self>, source: CloudSourceRef, visibility: CloudVisibility) -> Result<CloudSnapshot, CloudError> {
        self.enqueue_kind(vec![source], false, None, visibility)
    }
    pub fn enqueue_recording(
        self: &Arc<Self>,
        source: CloudSourceRef,
    ) -> Result<CloudSnapshot, CloudError> {
        self.enqueue_recording_with_visibility(source, CloudVisibility::Private)
    }
    pub fn enqueue_recording_with_visibility(
        self: &Arc<Self>, source: CloudSourceRef, visibility: CloudVisibility,
    ) -> Result<CloudSnapshot, CloudError> {
        if self.planner.is_none()
            || source.plan_id.is_some()
            || source.unit_id.is_some()
            || source.part_index.is_some()
        {
            return Err(CloudError::source());
        }
        self.enqueue_kind(vec![source], true, None, visibility)
    }
    /// The native planner supplies one recording's immutable ordered units.
    /// Publish all missing jobs atomically before a worker can send any bytes.
    pub fn enqueue_many(
        self: &Arc<Self>,
        sources: Vec<CloudSourceRef>,
    ) -> Result<CloudSnapshot, CloudError> {
        self.enqueue_kind(sources, false, None, CloudVisibility::Private)
    }
    fn enqueue_kind(
        self: &Arc<Self>,
        sources: Vec<CloudSourceRef>,
        recording_plan: bool,
        automatic_admission: Option<automatic::Admission>,
        visibility: CloudVisibility,
    ) -> Result<CloudSnapshot, CloudError> {
        let Some(first) = sources.first() else {
            return Err(CloudError::source());
        };
        let mut unique = std::collections::HashSet::new();
        if sources.len() > 2048
            || sources.iter().any(|source| {
                !source.valid()
                    || source.provider != first.provider
                    || source.recording_id != first.recording_id
                    || source.plan_id != first.plan_id
                    || !unique.insert((source.unit_id.clone(), source.part_index))
            })
        {
            return Err(CloudError::source());
        }
        {
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            let owner = inner
                .catalog
                .grant
                .as_ref()
                .ok_or_else(CloudError::auth)?
                .channel_id
                .clone();
            let admitted = automatic_admission.as_ref().map(|admission|automatic::admit(&inner.catalog,admission,&owner)).transpose()?;
            let (automatic_origin, visibility) = admitted.map(|(origin, selected)| (Some(origin), selected)).unwrap_or((None, visibility));
            // Overlapping selected channels/recordings and an earlier manual
            // job must converge on the existing upload, never a second video.
            if automatic_origin.is_some() && inner.catalog.jobs.iter().any(|job|job.channel_id == owner && job.view.source.provider == first.provider && job.view.source.recording_id == first.recording_id) {
                return Ok(Self::snapshot_inner(&inner));
            }
            if recording_plan
                && inner.catalog.jobs.iter().any(|job| {
                    job.channel_id == owner
                        && job.view.source.provider == first.provider
                        && job.view.source.recording_id == first.recording_id
                        && (job.view.recording_plan || job.view.source.plan_id.is_some())
                })
            {
                return Ok(Self::snapshot_inner(&inner));
            }
            let missing: Vec<_> = sources
                .into_iter()
                .filter(|source| {
                    !inner
                        .catalog
                        .jobs
                        .iter()
                        .any(|job| job.view.source == *source && job.channel_id == owner)
                })
                .collect();
            if missing.is_empty() {
                return Ok(Self::snapshot_inner(&inner));
            }
            if inner.catalog.jobs.len().saturating_add(missing.len()) > 2048 {
                return Err(CloudError::new(
                    "CLOUD_JOB_LIMIT",
                    "보관 중인 업로드 작업이 너무 많습니다.",
                    false,
                ));
            }
            let at = now();
            let mut candidate = inner.catalog.clone();
            for source in missing {
                candidate.jobs.push(Job {
                    view: CloudJobSnapshot {
                        id: uuid::Uuid::new_v4().to_string(),
                        source,
                        title: "녹화본 업로드".into(),
                        state: "queued".into(),
                        visibility,
                        recording_plan,
                        uploaded_bytes: 0,
                        total_bytes: 0,
                        video_id: None,
                        replay_cohort_id: None,
                        duration_seconds: 0.0,
                        media_offset_seconds: 0.0,
                        error: None,
                        retry_at: None,
                        preparation: None,
                        created_at: at,
                        updated_at: at,
                    },
                    channel_id: owner.clone(),
                    source_sha256: None,
                    session_url: None,
                    attempts: 0,
                    processing_started_at: None,
                    automatic: automatic_origin.clone(),
                });
            }
            self.publish(&mut inner, candidate)?;
        }
        self.start_worker()?;
        self.snapshot()
    }
    pub fn pause(&self, job_id: &str) -> Result<CloudSnapshot, CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if let Some((id, cancel)) = &inner.active {
            if id == job_id {
                cancel.store(true, Ordering::Release);
            }
        }
        let mut candidate = inner.catalog.clone();
        let job = candidate
            .jobs
            .iter_mut()
            .find(|j| j.view.id == job_id)
            .ok_or_else(CloudError::source)?;
        if job.view.state != "ready" {
            if let Some(origin) = &mut job.automatic { origin.resume_intent = false; origin.paused_by_user = true; }
            job.view.state = "paused".into();
            job.view.error = None;
            job.view.retry_at = None;
            job.view.updated_at = now();
        }
        self.publish(&mut inner, candidate)?;
        self.wake.notify_all();
        Ok(Self::snapshot_inner(&inner))
    }
    pub fn resume(self: &Arc<Self>, job_id: &str) -> Result<CloudSnapshot, CloudError> {
        {
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            let owner = inner
                .catalog
                .grant
                .as_ref()
                .ok_or_else(CloudError::auth)?
                .channel_id
                .clone();
            let automatic_resume = inner.catalog.jobs.iter().find(|job|job.view.id == job_id)
                .and_then(|job|job.automatic.as_ref()).is_some_and(|origin|automatic::current(&inner.catalog,origin,&owner));
            if inner.active.as_ref().is_some_and(|(id, _)| id == job_id) {
                return Err(CloudError::new(
                    "CLOUD_PAUSING",
                    "현재 전송을 정리 중입니다. 잠시 후 이어서 전송해 주세요.",
                    true,
                ));
            }
            let mut candidate = inner.catalog.clone();
            let job = candidate
                .jobs
                .iter_mut()
                .find(|j| j.view.id == job_id)
                .ok_or_else(CloudError::source)?;
            if job.channel_id != owner {
                return Err(CloudError::new(
                    "CLOUD_ACCOUNT_MISMATCH",
                    "이 작업을 시작한 YouTube 채널로 연결해 주세요.",
                    false,
                ));
            }
            if job.view.state != "ready" {
                if let Some(origin) = &mut job.automatic { origin.paused_by_user = false; origin.resume_intent = automatic_resume; }
                // Explicitly retrying an exhausted verification window does not
                // create another video; it only permits further metadata checks.
                if job
                    .view
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == "CLOUD_PROCESSING_WAIT")
                {
                    job.processing_started_at = Some(now());
                }
                job.view.state = "queued".into();
                job.view.error = None;
                job.view.retry_at = None;
                job.view.updated_at = now();
                job.attempts = 0;
            }
            self.publish(&mut inner, candidate)?;
        }
        self.start_worker()?;
        self.snapshot()
    }
    pub fn active_ids(&self) -> Vec<String> {
        let Ok(inner) = self.lock() else {
            return vec!["cloud:status-unavailable".into()];
        };
        let mut ids: Vec<_> = inner
            .catalog
            .jobs
            .iter()
            .filter(|j| {
                inner.persistence_error.is_none()
                    && j.view.video_id.is_none()
                    && matches!(
                        j.view.state.as_str(),
                        "queued" | "validating" | "uploading" | "retry_wait"
                    )
            })
            .map(|j| format!("cloud:{}", j.view.id))
            .collect();
        if let Some((id, _)) = &inner.active {
            let id = format!("cloud:{id}");
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if inner.authorizing || inner.auth_workers > 0 {
            ids.push("cloud:authorization".into());
        }
        ids.extend(inner.live.active_ids());
        // A sleeping remote-processing scheduler is not local active work. A
        // real request is accounted for by `active` until its worker returns.
        if inner.closing && (inner.worker_running || inner.automatic_running) && ids.is_empty() {
            ids.push("cloud:stopping".into());
        }
        ids
    }
    pub fn shutdown_and_wait(&self) -> Result<(), CloudError> {
        self.request_live_shutdown()?;
        // No inner catalog lock is held while replay workers/commands drain.
        let replay_shutdown = self.live_replays()?.shutdown();
        let save = {
            let mut inner = self.lock()?;
            inner.closing = true;
            inner.auth_generation = inner.auth_generation.wrapping_add(1);
            if let Some(cancel) = inner.auth_cancel.take() {
                cancel.store(true, Ordering::Release);
            }
            if let Some((_, cancel)) = &inner.active {
                cancel.store(true, Ordering::Release);
            }
            inner.authorizing = false;
            let mut candidate = inner.catalog.clone();
            for job in &mut candidate.jobs {
                if matches!(
                    job.view.state.as_str(),
                    "queued" | "validating" | "uploading" | "processing" | "retry_wait"
                ) {
                    job.view.state = "paused".into();
                    job.view.retry_at = None;
                    job.view.error = Some(CloudError::cancelled());
                }
            }
            self.publish(&mut inner, candidate)
        };
        self.wake.notify_all();
        let handles = std::mem::take(&mut *self.threads.lock().map_err(|_| CloudError::storage())?);
        let mut failed = false;
        for handle in handles {
            if handle.join().is_err() {
                failed = true;
            }
        }
        self.wait_live_history()?;
        replay_shutdown?;
        if failed {
            return Err(CloudError::storage());
        }
        save
    }
    fn start_worker(self: &Arc<Self>) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if inner.worker_running {
            self.wake.notify_all();
            return Ok(());
        }
        inner.worker_running = true;
        let service = self.clone();
        match std::thread::Builder::new()
            .name("atsumi-cloud-upload".into())
            .spawn(move || service.worker())
        {
            Ok(handle) => self.add_thread(handle),
            Err(_) => {
                inner.worker_running = false;
                Err(CloudError::storage())
            }
        }
    }
    fn worker(self: Arc<Self>) {
        let mut prefer_processing = true;
        loop {
            let selected = {
                let Ok(mut inner) = self.lock() else {
                    return;
                };
                if inner.closing
                    || inner.persistence_error.is_some()
                    || inner.catalog.grant.is_none()
                {
                    inner.worker_running = false;
                    self.wake.notify_all();
                    return;
                }
                let at = now();
                let due = |job: &Job| {
                    job.view.state == "queued"
                        || matches!(job.view.state.as_str(), "retry_wait" | "processing")
                            && job.view.retry_at.is_none_or(|time| time <= at)
                };
                let upload = inner
                    .catalog
                    .jobs
                    .iter()
                    .position(|job| job.view.video_id.is_none() && due(job));
                let processing = inner
                    .catalog
                    .jobs
                    .iter()
                    .position(|job| job.view.video_id.is_some() && due(job));
                let index = if prefer_processing {
                    processing.or(upload)
                } else {
                    upload.or(processing)
                };
                let Some(index) = index else {
                    if let Some(next) = inner
                        .catalog
                        .jobs
                        .iter()
                        .filter(|job| {
                            matches!(job.view.state.as_str(), "retry_wait" | "processing")
                        })
                        .filter_map(|job| job.view.retry_at)
                        .min()
                    {
                        let wait = next.saturating_sub(at).clamp(1, 5_000);
                        let _ = self.wake.wait_timeout(inner, Duration::from_millis(wait));
                        continue;
                    }
                    inner.worker_running = false;
                    self.wake.notify_all();
                    return;
                };
                let cancel = Arc::new(AtomicBool::new(false));
                let mut candidate = inner.catalog.clone();
                let processing = candidate.jobs[index].view.video_id.is_some();
                // Alternate bounded steps when both queues have work; neither
                // a long upload list nor many due checks can starve the other.
                prefer_processing = !processing;
                candidate.jobs[index].view.state = if processing {
                    "processing"
                } else {
                    "validating"
                }
                .into();
                candidate.jobs[index].view.preparation = None;
                inner.preparation_progress_at = None;
                candidate.jobs[index].view.updated_at = now();
                let job = candidate.jobs[index].clone();
                inner.active = Some((job.view.id.clone(), cancel.clone()));
                if self.publish(&mut inner, candidate).is_err() {
                    inner.active = None;
                    inner.worker_running = false;
                    return;
                }
                (job, cancel)
            };
            let (job, cancel) = selected;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.run_job(job.clone(), &cancel)
            }))
            .unwrap_or_else(|_| {
                Err(CloudError::new(
                    "CLOUD_WORKER_STOPPED",
                    "업로드 작업이 중단됐습니다. 원본은 유지됩니다.",
                    true,
                ))
            });
            if let Ok(mut inner) = self.lock() {
                inner.active = None;
                let connected = inner.catalog.grant.is_some();
                let mut candidate = inner.catalog.clone();
                if let Some(current) = candidate.jobs.iter_mut().find(|j| j.view.id == job.view.id)
                {
                    if let Err(error) = result {
                        let interrupted = cancel.load(Ordering::Acquire) || error.code == "CLOUD_PAUSED";
                        if interrupted {
                            if current.view.state != "waiting_auth" {
                                current.view.state = "paused".into();
                            }
                            current.view.retry_at = None;
                        } else if matches!(
                            error.code.as_str(),
                            "CLOUD_AUTH_REQUIRED" | "CLOUD_REAUTH_REQUIRED"
                        ) || !connected
                        {
                            current.view.state = "waiting_auth".into();
                            current.view.retry_at = None;
                        } else if error.retryable
                            && current.attempts < 8
                            && error.code != "CLOUD_STORAGE"
                            && error.code != "CLOUD_PROCESSING_WAIT"
                            && error.code != "CLOUD_PREPARATION"
                            && !matches!(
                                error.code.as_str(),
                                "CLOUD_SOURCE_SPACE" | "CLOUD_SOURCE_IO" | "CLOUD_SOURCE_COVERAGE"
                            )
                        {
                            current.attempts += 1;
                            current.view.state = "retry_wait".into();
                            let delay = if error.code == "CLOUD_QUOTA" {
                                3_600_000
                            } else {
                                (5_000u64 << current.attempts.min(6)).min(300_000)
                            };
                            current.view.retry_at = Some(now().saturating_add(delay));
                        } else {
                            current.view.state = "failed".into();
                            current.view.retry_at = None;
                        }
                        if !interrupted || current.view.state != "waiting_auth" || current.view.error.is_none() {
                            current.view.error = Some(error);
                        }
                    }
                    current.view.updated_at = now();
                }
                if self.publish(&mut inner, candidate).is_err() {
                    inner.worker_running = false;
                    return;
                }
            }
        }
    }
    fn update_job(&self, id: &str, update: impl FnOnce(&mut Job)) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        let mut candidate = inner.catalog.clone();
        let job = candidate
            .jobs
            .iter_mut()
            .find(|j| j.view.id == id)
            .ok_or_else(CloudError::source)?;
        update(job);
        job.view.updated_at = now();
        self.publish(&mut inner, candidate)
    }
    fn token(&self, owner: &str, force: bool) -> Result<String, CloudError> {
        let (client, grant, generation) = {
            let inner = self.lock()?;
            Self::admit(&inner)?;
            let grant = inner.catalog.grant.clone().ok_or_else(CloudError::auth)?;
            if grant.channel_id != owner {
                return Err(CloudError::new(
                    "CLOUD_ACCOUNT_MISMATCH",
                    "작업을 시작한 YouTube 채널과 연결된 채널이 다릅니다.",
                    false,
                ));
            }
            if !force && grant.expires_at > now().saturating_add(60_000) {
                return Ok(grant.access_token);
            }
            (
                inner.catalog.client.clone().ok_or_else(CloudError::auth)?,
                grant,
                inner.auth_generation,
            )
        };
        let refreshed = oauth::refresh(self.transport.as_ref(), &client, &grant);
        let mut inner = self.lock()?;
        if inner.closing
            || inner.auth_generation != generation
            || inner
                .catalog
                .grant
                .as_ref()
                .is_none_or(|g| g.channel_id != owner || g.refresh_token != grant.refresh_token)
        {
            return Err(CloudError::auth());
        }
        match refreshed {
            Ok(refreshed) => {
                let token = refreshed.access_token.clone();
                let mut candidate = inner.catalog.clone();
                candidate.grant = Some(refreshed);
                self.publish(&mut inner, candidate)?;
                Ok(token)
            }
            Err(error) => {
                if matches!(
                    error.code.as_str(),
                    "CLOUD_AUTH_REQUIRED" | "CLOUD_REAUTH_REQUIRED"
                ) {
                    let mut candidate = inner.catalog.clone();
                    candidate.grant = None;
                    for job in &mut candidate.jobs {
                        if matches!(
                            job.view.state.as_str(),
                            "queued" | "validating" | "uploading" | "processing" | "retry_wait"
                        ) {
                            job.view.state = "waiting_auth".into();
                            job.view.error = Some(error.clone());
                            job.view.retry_at = None;
                            job.view.updated_at = now();
                        }
                    }
                    self.publish(&mut inner, candidate)?;
                    inner.account_error = Some(error.clone());
                }
                Err(error)
            }
        }
    }
    fn authorized<T>(
        &self,
        owner: &str,
        action: impl Fn(&str) -> Result<T, CloudError>,
    ) -> Result<T, CloudError> {
        let token = self.token(owner, false)?;
        match action(&token) {
            Err(error) if error.code == "CLOUD_AUTH_REQUIRED" => {
                let token = self.token(owner, true)?;
                action(&token)
            }
            result => result,
        }
    }
    fn update_preparation(
        &self,
        id: &str,
        cancel: &Arc<AtomicBool>,
        progress: SourcePreparationProgress,
    ) {
        if !progress.valid() || cancel.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut inner) = self.lock() else {
            cancel.store(true, Ordering::Release);
            return;
        };
        if inner.closing
            || inner.persistence_error.is_some()
            || !inner
                .active
                .as_ref()
                .is_some_and(|(active_id, active_cancel)| {
                    active_id == id && Arc::ptr_eq(cancel, active_cancel)
                })
        {
            return;
        }
        let Some(current) = inner
            .catalog
            .jobs
            .iter()
            .find(|job| job.view.id == id && job.view.state == "validating")
        else {
            return;
        };
        if current.view.preparation.as_ref() == Some(&progress) {
            return;
        }
        let phase_changed = current
            .view
            .preparation
            .as_ref()
            .is_none_or(|old| old.phase != progress.phase);
        let completed = progress
            .completed_bytes
            .zip(progress.total_bytes)
            .is_some_and(|(done, total)| total > 0 && done == total)
            || progress
                .completed_seconds
                .zip(progress.total_seconds)
                .is_some_and(|(done, total)| done == total);
        if !phase_changed
            && !completed
            && inner
                .preparation_progress_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        let mut candidate = inner.catalog.clone();
        if let Some(job) = candidate.jobs.iter_mut().find(|job| job.view.id == id) {
            job.view.preparation = Some(progress);
            job.view.updated_at = now();
        }
        if self.publish(&mut inner, candidate).is_ok() {
            inner.preparation_progress_at = Some(Instant::now());
        }
    }
    fn expand_recording_plan(
        &self,
        placeholder: &Job,
        sources: Vec<CloudSourceRef>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), CloudError> {
        let Some(first) = sources.first() else {
            return Err(CloudError::source());
        };
        let mut units = std::collections::HashSet::new();
        if sources.len() > 2048
            || first.plan_id.is_none()
            || sources.iter().any(|source| {
                !source.valid()
                    || source.part_index.is_some()
                    || source.plan_id != first.plan_id
                    || source.provider != placeholder.view.source.provider
                    || source.recording_id != placeholder.view.source.recording_id
                    || !units.insert(source.unit_id.clone())
            })
        {
            return Err(CloudError::source());
        }
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        check(cancel)?;
        if !inner
            .active
            .as_ref()
            .is_some_and(|(id, active)| id == &placeholder.view.id && Arc::ptr_eq(active, cancel))
        {
            return Err(CloudError::cancelled());
        }
        if inner
            .catalog
            .grant
            .as_ref()
            .is_none_or(|grant| grant.channel_id != placeholder.channel_id)
        {
            return Err(CloudError::auth());
        }
        let index = inner
            .catalog
            .jobs
            .iter()
            .position(|job| {
                job.view.id == placeholder.view.id
                    && job.view.recording_plan
                    && job.view.state == "validating"
                    && job.view.source == placeholder.view.source
            })
            .ok_or_else(CloudError::cancelled)?;
        // Never merge units from different immutable plans into one recording.
        if inner.catalog.jobs.iter().any(|job| {
            job.channel_id == placeholder.channel_id
                && job.view.source.provider == first.provider
                && job.view.source.recording_id == first.recording_id
                && job.view.source.plan_id.is_some()
                && job.view.source.plan_id != first.plan_id
        }) {
            return Err(CloudError::source());
        }
        let missing: Vec<_> = sources
            .into_iter()
            .filter(|source| {
                !inner.catalog.jobs.iter().any(|job| {
                    job.view.source == *source && job.channel_id == placeholder.channel_id
                })
            })
            .collect();
        if inner
            .catalog
            .jobs
            .len()
            .saturating_sub(1)
            .saturating_add(missing.len())
            > 2048
        {
            return Err(CloudError::new(
                "CLOUD_JOB_LIMIT",
                "보관 중인 업로드 작업이 너무 많습니다.",
                false,
            ));
        }
        let mut candidate = inner.catalog.clone();
        candidate.jobs.remove(index);
        let at = now();
        let children = missing.into_iter().map(|source| {
            let mut child = placeholder.clone();
            child.view.id = uuid::Uuid::new_v4().to_string();
            child.view.source = source;
            child.view.recording_plan = false;
            child.view.state = "queued".into();
            child.view.preparation = None;
            child.view.error = None;
            child.view.retry_at = None;
            child.view.updated_at = at;
            child.attempts = 0;
            child
        });
        candidate.jobs.splice(index..index, children);
        self.publish(&mut inner, candidate)
    }
    fn run_job(self: &Arc<Self>, mut job: Job, cancel: &Arc<AtomicBool>) -> Result<(), CloudError> {
        check(cancel)?;
        // A durable final response is sufficient to query the remote video.
        // Do not reopen, hash, or acquire a media lease for processing checks.
        if job.view.video_id.is_some() {
            return self.check_processing(job, cancel);
        }
        // Preparing a long recording can remux or hash many gigabytes. Resolve
        // an expiring grant before touching the source. Each upload request also
        // checks it again in case it expired during preparation.
        self.token(&job.channel_id, false)?;
        check(cancel)?;
        let weak = Arc::downgrade(self);
        let id = job.view.id.clone();
        let progress_cancel = cancel.clone();
        let context = SourceResolveContext {
            cancel: cancel.clone(),
            progress: Arc::new(move |progress| {
                if let Some(service) = weak.upgrade() {
                    service.update_preparation(&id, &progress_cancel, progress);
                }
            }),
        };
        if job.view.recording_plan {
            let planner = self.planner.as_ref().ok_or_else(CloudError::source)?;
            let sources = planner(&job.view.source, &context).map_err(|error| {
                CloudError::source_failure(&error, true, cancel.load(Ordering::Acquire))
            })?;
            return self.expand_recording_plan(&job, sources, cancel);
        }
        let mut source = (self.resolver)(&job.view.source, &context).map_err(|error| {
            CloudError::source_failure(&error, false, cancel.load(Ordering::Acquire))
        })?;
        check(cancel)?;
        if source.source != job.view.source
            || !source.duration_seconds.is_finite()
            || source.duration_seconds <= 0.0
            || source.duration_seconds > 43_200.0
            || !source.media_offset_seconds.is_finite()
            || !(0.0..=1_000_000_000.0).contains(&source.media_offset_seconds)
        {
            return Err(CloudError::new(
                "CLOUD_SOURCE_LIMIT",
                "한 영상은 12시간 이하여야 합니다. 확인된 녹화 구간을 선택해 주세요.",
                false,
            ));
        }
        let metadata = source.file.metadata().map_err(|_| CloudError::source())?;
        let bytes = metadata.len();
        if !metadata.is_file() || bytes == 0 || bytes > upload::MAX_BYTES {
            return Err(CloudError::new(
                "CLOUD_SOURCE_LIMIT",
                "업로드할 녹화 파일이 비어 있거나 허용 크기를 넘었습니다.",
                false,
            ));
        }
        let hash = hash_source(&mut source.file, cancel)?;
        if job.source_sha256.as_ref().is_some_and(|old| old != &hash)
            || job.view.total_bytes != 0 && job.view.total_bytes != bytes
            || job.view.duration_seconds != 0.0
                && (job.view.duration_seconds - source.duration_seconds).abs() > 0.001
            || (job.view.total_bytes != 0
                && (job.view.media_offset_seconds - source.media_offset_seconds).abs() > 0.001)
        {
            return Err(CloudError::source());
        }
        let title: String = source
            .title
            .chars()
            .filter(|c| *c != '<' && *c != '>' && !c.is_control())
            .take(100)
            .collect();
        let title = if title.trim().is_empty() {
            "아츠미 녹화".into()
        } else {
            title.trim().to_owned()
        };
        job.view.title = title;
        job.view.total_bytes = bytes;
        job.view.duration_seconds = source.duration_seconds;
        job.view.media_offset_seconds = source.media_offset_seconds;
        job.source_sha256 = Some(hash);
        self.update_job(&job.view.id, |current| {
            current.view.title = job.view.title.clone();
            current.view.total_bytes = bytes;
            current.view.duration_seconds = source.duration_seconds;
            current.view.media_offset_seconds = source.media_offset_seconds;
            current.source_sha256 = job.source_sha256.clone();
            current.set_active_phase("validating");
            current.view.preparation = None;
        })?;
        check(cancel)?;
        if job.view.video_id.is_none() {
            let session = match job.session_url.clone() {
                Some(session) => session,
                None => {
                    let session = self.authorized(&job.channel_id, |token| {
                        upload::create_session(
                            self.transport.as_ref(),
                            token,
                            &job.view.title,
                            bytes,
                            &job.view.id,
                            job.view.visibility,
                        )
                    })?;
                    // Durable encrypted session before any media bytes are sent.
                    self.update_job(&job.view.id, |current| {
                        current.session_url = Some(session.clone());
                        current.set_active_phase("uploading");
                    })?;
                    session
                }
            };
            check(cancel)?;
            let mut progress = self.authorized(&job.channel_id, |token| {
                upload::query_session(self.transport.as_ref(), token, &session, bytes)
            })?;
            let mut unchanged = 0;
            loop {
                match progress {
                    upload::Progress::Complete(id) => {
                        let started = now();
                        // Even when pause/shutdown arrived during the final HTTP
                        // response, checkpoint the observed ID before releasing
                        // the upload slot. Preserve the already-paused state.
                        self.update_job(&job.view.id, |current| {
                            current.view.video_id = Some(id.clone());
                            current.view.uploaded_bytes = bytes;
                            current.processing_started_at = Some(started);
                            current.set_active_phase("processing");
                        })?;
                        job.view.video_id = Some(id);
                        job.view.uploaded_bytes = bytes;
                        job.processing_started_at = Some(started);
                        break;
                    }
                    upload::Progress::Offset(offset) => {
                        self.update_job(&job.view.id, |current| {
                            current.view.uploaded_bytes = offset;
                            current.set_active_phase("uploading");
                        })?;
                        check(cancel)?;
                        if offset == bytes {
                            if unchanged >= 3 {
                                return Err(CloudError::new(
                                    "CLOUD_UPLOAD_ACK_WAIT",
                                    "YouTube의 전송 완료 확인을 기다리고 있습니다.",
                                    true,
                                ));
                            }
                            unchanged += 1;
                            cancellable_wait(cancel, Duration::from_secs(2))?;
                            progress = self.authorized(&job.channel_id, |token| {
                                upload::query_session(
                                    self.transport.as_ref(),
                                    token,
                                    &session,
                                    bytes,
                                )
                            })?;
                            continue;
                        }
                        let current_metadata =
                            source.file.metadata().map_err(|_| CloudError::source())?;
                        if current_metadata.len() != bytes
                            || current_metadata.modified().ok() != metadata.modified().ok()
                        {
                            return Err(CloudError::source());
                        }
                        let length = (bytes - offset).min(upload::CHUNK_BYTES as u64) as usize;
                        let mut block = vec![0; length];
                        source
                            .file
                            .seek(SeekFrom::Start(offset))
                            .and_then(|_| source.file.read_exact(&mut block))
                            .map_err(|_| CloudError::source())?;
                        check(cancel)?;
                        progress = self.authorized(&job.channel_id, |token| {
                            upload::send_chunk(
                                self.transport.as_ref(),
                                token,
                                &session,
                                offset,
                                bytes,
                                block.clone(),
                            )
                        })?;
                        if let upload::Progress::Offset(next) = progress {
                            if next < offset || next > offset + length as u64 {
                                return Err(CloudError::protocol());
                            }
                            if next == offset {
                                unchanged += 1;
                                if unchanged >= 3 {
                                    return Err(CloudError::protocol());
                                }
                            } else {
                                unchanged = 0;
                            }
                        }
                    }
                }
            }
        }
        // Original retention is independent of this attempt's handle/lease.
        // The complete upload receipt is already durable before it is dropped.
        drop(source);
        self.check_processing(job, cancel)
    }
    fn check_processing(&self, job: Job, cancel: &AtomicBool) -> Result<(), CloudError> {
        check(cancel)?;
        if !job.has_upload_receipt() {
            return Err(CloudError::protocol());
        }
        let id = job
            .view
            .video_id
            .as_ref()
            .ok_or_else(CloudError::protocol)?;
        let ready = self.authorized(&job.channel_id, |token| {
            upload::verify_remote(
                self.transport.as_ref(),
                token,
                id,
                &job.channel_id,
                job.view.duration_seconds,
                job.view.visibility,
            )
        })?;
        // Serialize the cancel check with pause/shutdown publication so their
        // state cannot be overwritten by a late successful metadata response.
        let mut inner = self.lock()?;
        check(cancel)?;
        if inner.closing {
            return Err(CloudError::cancelled());
        }
        let mut candidate = inner.catalog.clone();
        let current = candidate
            .jobs
            .iter_mut()
            .find(|current| current.view.id == job.view.id)
            .ok_or_else(CloudError::source)?;
        let at = now();
        let started = current.processing_started_at.get_or_insert(at);
        // Query first: a completed video is valid even if the client was offline
        // beyond the waiting window. Only an actual pending response times out.
        if !ready && at.saturating_sub(*started) >= PROCESSING_WAIT_LIMIT_MS {
            return Err(CloudError::new(
                "CLOUD_PROCESSING_WAIT",
                "YouTube 영상 처리가 오래 걸리고 있습니다. 원본을 유지했으며 다시 확인할 수 있습니다.",
                true,
            ));
        }
        current.view.state = if ready { "ready" } else { "processing" }.into();
        current.view.error = None;
        current.view.retry_at = if ready {
            None
        } else {
            Some(at.saturating_add(PROCESSING_CHECK_DELAY_MS))
        };
        current.view.updated_at = at;
        current.attempts = 0;
        if ready {
            current.session_url = None;
        }
        self.publish(&mut inner, candidate)
    }
}
fn check(cancel: &AtomicBool) -> Result<(), CloudError> {
    if cancel.load(Ordering::Acquire) {
        Err(CloudError::cancelled())
    } else {
        Ok(())
    }
}
fn cancellable_wait(cancel: &AtomicBool, duration: Duration) -> Result<(), CloudError> {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        check(cancel)?;
        std::thread::sleep(
            Duration::from_millis(100).min(until.saturating_duration_since(Instant::now())),
        );
    }
    check(cancel)
}
fn hash_source(file: &mut File, cancel: &AtomicBool) -> Result<String, CloudError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| CloudError::source())?;
    let mut hash = Sha256::new();
    let mut block = [0; 64 * 1024];
    loop {
        check(cancel)?;
        let count = file.read(&mut block).map_err(|_| CloudError::source())?;
        if count == 0 {
            break;
        }
        hash.update(&block[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
