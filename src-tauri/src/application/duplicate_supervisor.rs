use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, Sender, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    domain::{
        ArtifactBundle, ArtifactSha256, DuplicateDecisionApplyOutcome, DuplicateDecisionRequest,
        DuplicatePageHash, DuplicateReview, DuplicateScanRun, DuplicateScanState,
        DuplicateSnapshot, ExternalRelationEvidence, GalleryId, HashProfile, SourcePageNumber,
    },
    thumbnail::CancellationToken,
};

use super::{
    download_overlap::normalized_artist_keys,
    duplicate_analyzer::{
        analyze_artifact_pair, compute_page_hash, gallery_ref, verified_scan_pages, HashedArtifact,
    },
    ApplicationError, ArtifactStore, DuplicateRelationProvider, DuplicateRepository,
    RepositoryError, StateRepository,
};

const PAIR_PROGRESS_EVENT_INTERVAL: u64 = 64;
const MAX_DUPLICATE_HASH_WORKERS: usize = 4;

#[derive(Debug, Default)]
pub struct DisabledDuplicateRelationProvider;

impl DuplicateRelationProvider for DisabledDuplicateRelationProvider {
    fn enabled(&self) -> bool {
        false
    }

    fn relation(
        &self,
        _parent_gallery_id: crate::domain::GalleryId,
        _candidate_gallery_id: crate::domain::GalleryId,
    ) -> Result<Option<ExternalRelationEvidence>, RepositoryError> {
        Ok(None)
    }
}

#[derive(Clone)]
pub struct DuplicateSupervisor {
    inner: Arc<DuplicateSupervisorInner>,
}

struct DuplicateSupervisorInner {
    repository: Arc<dyn DuplicateRepository>,
    settings: Arc<dyn StateRepository>,
    store: Arc<dyn ArtifactStore>,
    relations: Arc<dyn DuplicateRelationProvider>,
    events: Sender<DuplicateScanRun>,
    control: Mutex<()>,
    active: Mutex<Option<ActiveRun>>,
}

struct ActiveRun {
    run_id: String,
    cancellation: CancellationToken,
    worker: Option<JoinHandle<()>>,
}

pub(crate) struct PreparedDuplicateScan {
    root: PathBuf,
    bundles: Vec<ArtifactBundle>,
    candidate_pairs: Vec<CandidatePair>,
    total_artifacts: u32,
    total_pairs: u64,
    profile: HashProfile,
}

impl DuplicateSupervisor {
    pub fn new(
        repository: Arc<dyn DuplicateRepository>,
        settings: Arc<dyn StateRepository>,
        store: Arc<dyn ArtifactStore>,
        relations: Arc<dyn DuplicateRelationProvider>,
        events: Sender<DuplicateScanRun>,
    ) -> Self {
        Self {
            inner: Arc::new(DuplicateSupervisorInner {
                repository,
                settings,
                store,
                relations,
                events,
                control: Mutex::new(()),
                active: Mutex::new(None),
            }),
        }
    }

    pub fn recover_interrupted(&self) -> Result<usize, ApplicationError> {
        self.inner
            .repository
            .duplicate_recover_interrupted()
            .map_err(Into::into)
    }

    pub fn snapshot(&self) -> Result<DuplicateSnapshot, ApplicationError> {
        self.inner
            .repository
            .duplicate_snapshot()
            .map_err(Into::into)
    }

    pub fn review_get(&self, candidate_id: &str) -> Result<DuplicateReview, ApplicationError> {
        let candidate_id = normalized_candidate_id(candidate_id)?;
        self.inner
            .repository
            .duplicate_review_get(candidate_id)?
            .ok_or_else(|| ApplicationError::DuplicateCandidateNotFound(candidate_id.to_owned()))
    }

    pub fn decision_apply(
        &self,
        request: DuplicateDecisionRequest,
    ) -> Result<DuplicateReview, ApplicationError> {
        validate_decision_request(&request)?;
        match self.inner.repository.duplicate_decision_apply(&request)? {
            DuplicateDecisionApplyOutcome::Applied(review) => Ok(*review),
            DuplicateDecisionApplyOutcome::CandidateNotFound => Err(
                ApplicationError::DuplicateCandidateNotFound(request.candidate_id),
            ),
            DuplicateDecisionApplyOutcome::RevisionConflict { actual_revision } => {
                Err(ApplicationError::RevisionConflict {
                    resource: "duplicateCandidate",
                    expected: request.expected_revision,
                    actual: actual_revision,
                })
            }
        }
    }

    pub fn start(&self) -> Result<DuplicateScanRun, ApplicationError> {
        if let Some(run) = self.active_run_snapshot()? {
            return Ok(run);
        }
        let prepared = self.prepare_start()?;
        self.commit_start(prepared)
    }

    /// Load and validate the immutable scan input without holding the
    /// supervisor control lock. The caller may also keep a broader managed
    /// work gate free while artifact rows are read.
    pub(crate) fn prepare_start(&self) -> Result<PreparedDuplicateScan, ApplicationError> {
        let settings = self.inner.settings.settings_get()?;
        if settings.download_root.trim().is_empty() {
            return Err(super::DownloadPipelineError::root_required().into());
        }
        // Validate once at the scan boundary. Individual reads use a
        // canonical read-only resolver and therefore never create probes.
        let root = self
            .inner
            .store
            .validate_download_root(&PathBuf::from(settings.download_root))?;
        let bundles = select_scan_bundles(self.inner.repository.duplicate_artifact_bundles()?);
        let (bundles, candidate_pairs) = same_artist_scan_plan(bundles);
        let total_artifacts = u32::try_from(bundles.len()).unwrap_or(u32::MAX);
        let total_pairs = u64::try_from(candidate_pairs.len()).unwrap_or(u64::MAX);
        Ok(PreparedDuplicateScan {
            root,
            bundles,
            candidate_pairs,
            total_artifacts,
            total_pairs,
            profile: HashProfile::current(),
        })
    }

    /// Recheck the active slot and atomically commit only the short run-row /
    /// worker-start phase. Concurrent preparations therefore collapse to one
    /// active run instead of creating overlapping workers.
    pub(crate) fn commit_start(
        &self,
        prepared: PreparedDuplicateScan,
    ) -> Result<DuplicateScanRun, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        if let Some(run) = self.active_run()? {
            return Ok(run);
        }
        let PreparedDuplicateScan {
            root,
            bundles,
            candidate_pairs,
            total_artifacts,
            total_pairs,
            profile,
        } = prepared;
        let run = self.inner.repository.duplicate_scan_start(
            profile.profile_version,
            total_artifacts,
            total_pairs,
        )?;
        if run.state != DuplicateScanState::Running {
            return Ok(run);
        }

        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let inner = Arc::clone(&self.inner);
        let run_id = run.run_id.clone();
        let worker_run_id = run_id.clone();
        let worker = thread::Builder::new()
            .name(format!("atsumi-duplicate-{}", short_id(&run_id)))
            .spawn(move || {
                run_scan(
                    inner,
                    worker_run_id,
                    root,
                    bundles,
                    candidate_pairs,
                    profile,
                    worker_cancellation,
                );
            })
            .map_err(|error| {
                let _ = self.inner.repository.duplicate_scan_finish(
                    &run_id,
                    DuplicateScanState::Failed,
                    Some("DUPLICATE_WORKER_UNAVAILABLE"),
                    Some("The duplicate scan worker could not be started"),
                );
                RepositoryError::Other(format!("could not start duplicate scan worker: {error}"))
            })?;
        *self.active_lock()? = Some(ActiveRun {
            run_id,
            cancellation,
            worker: Some(worker),
        });
        let _ = self.inner.events.send(run.clone());
        Ok(run)
    }

    pub fn cancel(&self) -> Result<DuplicateScanRun, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        let run_id = {
            let active = self.active_lock()?;
            let active = active
                .as_ref()
                .ok_or(ApplicationError::DuplicateScanNotRunning)?;
            active.cancellation.cancel();
            active.run_id.clone()
        };
        let run = self
            .inner
            .repository
            .duplicate_scan_finish(
                &run_id,
                DuplicateScanState::Cancelled,
                Some("DUPLICATE_SCAN_CANCELLED"),
                Some("The duplicate scan was cancelled"),
            )?
            .ok_or(ApplicationError::DuplicateScanNotRunning)?;
        // Do not detach the cancelled worker: a replacement scan must never
        // overlap reads or writes from the previous run.
        if let Some(mut active) = self.active_lock()?.take() {
            if let Some(worker) = active.worker.take() {
                let _ = worker.join();
            }
        }
        let _ = self.inner.events.send(run.clone());
        Ok(run)
    }

    /// Read a stable active-run projection without exposing the worker handle
    /// or treating a finished-but-not-yet-reaped slot as active work.
    pub fn active_run_snapshot(&self) -> Result<Option<DuplicateScanRun>, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        self.active_run()
    }

    pub fn shutdown_and_wait(&self) {
        let _control = self.control_lock().ok();
        let active = self.active_lock().ok().and_then(|mut active| active.take());
        if let Some(mut active) = active {
            active.cancellation.cancel();
            let _ = self.inner.repository.duplicate_scan_finish(
                &active.run_id,
                DuplicateScanState::Cancelled,
                Some("DUPLICATE_SCAN_APP_EXIT"),
                Some("The application closed during duplicate scanning"),
            );
            if let Some(worker) = active.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn control_lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, ApplicationError> {
        self.inner.control.lock().map_err(|_| {
            RepositoryError::Other("duplicate supervisor control mutex was poisoned".into()).into()
        })
    }

    fn active_lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<ActiveRun>>, ApplicationError> {
        self.inner.active.lock().map_err(|_| {
            RepositoryError::Other("duplicate supervisor mutex was poisoned".into()).into()
        })
    }

    fn active_run(&self) -> Result<Option<DuplicateScanRun>, ApplicationError> {
        let run_id = self
            .active_lock()?
            .as_ref()
            .map(|active| active.run_id.clone());
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        Ok(self
            .inner
            .repository
            .duplicate_snapshot()?
            .run
            .filter(|run| run.run_id == run_id && run.state == DuplicateScanState::Running))
    }

    fn reap_finished_worker(&self) {
        let finished = self.active_lock().ok().and_then(|mut active| {
            active
                .as_ref()
                .and_then(|run| run.worker.as_ref())
                .is_some_and(JoinHandle::is_finished)
                .then(|| active.take())
                .flatten()
        });
        if let Some(mut finished) = finished {
            if let Some(worker) = finished.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn run_scan(
    inner: Arc<DuplicateSupervisorInner>,
    run_id: String,
    root: PathBuf,
    bundles: Vec<crate::domain::ArtifactBundle>,
    candidate_pairs: Vec<CandidatePair>,
    profile: HashProfile,
    cancellation: CancellationToken,
) {
    let result = scan_inner(
        &inner,
        &run_id,
        &root,
        &bundles,
        &candidate_pairs,
        &profile,
        &cancellation,
    );
    if let Err(error) = result {
        if !cancellation.is_cancelled()
            && inner
                .repository
                .duplicate_scan_is_running(&run_id)
                .unwrap_or(false)
        {
            if let Ok(Some(run)) = inner.repository.duplicate_scan_finish(
                &run_id,
                DuplicateScanState::Failed,
                Some("DUPLICATE_SCAN_FAILED"),
                Some(&stable_scan_error(&error)),
            ) {
                let _ = inner.events.send(run);
            }
        }
    }
}

fn scan_inner(
    inner: &DuplicateSupervisorInner,
    run_id: &str,
    root: &std::path::Path,
    bundles: &[crate::domain::ArtifactBundle],
    candidate_pairs: &[CandidatePair],
    profile: &HashProfile,
    cancellation: &CancellationToken,
) -> Result<(), RepositoryError> {
    let hash_pool = DuplicateHashPool::new(profile)?;
    let mut timing = DuplicateScanTiming::new(run_id, hash_pool.worker_count());
    let mut artifacts = Vec::with_capacity(bundles.len());
    for (artifact_index, bundle) in bundles.iter().enumerate() {
        if cancelled(inner, run_id, cancellation)? {
            timing.finish("cancelled");
            return Ok(());
        }
        let pages = verified_scan_pages(bundle).ok_or_else(|| {
            RepositoryError::Corrupt("artifact lost its verified scan eligibility".into())
        })?;
        let mut hashes = Vec::with_capacity(pages.len());
        let mut pending_hashes = 0_usize;
        let mut hash_pipeline_started = None;
        for page in pages {
            if cancelled(inner, run_id, cancellation)? {
                timing.finish("cancelled");
                return Ok(());
            }
            let sha = page
                .sha256
                .as_ref()
                .expect("verified_scan_pages guarantees SHA-256");
            let cached = measure(&mut timing.hash_cache_read, || {
                inner.repository.duplicate_page_hash_get(
                    bundle.artifact.entry_id.as_str(),
                    page.page_id.source_page_number,
                    profile.profile_version,
                    sha.as_str(),
                )
            })?;
            let hash = if let Some(hash) = cached {
                timing.hash_cache_hits = timing.hash_cache_hits.saturating_add(1);
                hash
            } else {
                timing.hash_cache_misses = timing.hash_cache_misses.saturating_add(1);
                hash_pipeline_started.get_or_insert_with(Instant::now);
                let bytes = measure(&mut timing.image_read, || {
                    inner
                        .store
                        .read_verified_page_bytes(root, page)
                        .map_err(|error| RepositoryError::Other(error.to_string()))
                })?;
                timing.image_bytes_read = timing
                    .image_bytes_read
                    .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                hash_pool.submit(DuplicateHashJob {
                    entry_id: bundle.artifact.entry_id.to_string(),
                    gallery_id: bundle.gallery.id,
                    source_page_number: page.page_id.source_page_number,
                    artifact_sha256: sha.clone(),
                    bytes,
                })?;
                pending_hashes = pending_hashes.saturating_add(1);
                continue;
            };
            hashes.push(hash);
        }

        let mut computed_hashes = Vec::with_capacity(pending_hashes);
        for _ in 0..pending_hashes {
            let completed = hash_pool.receive()?;
            timing.hash_compute = timing.hash_compute.saturating_add(completed.elapsed);
            let hash = completed.result?;
            timing.pages_hashed = timing.pages_hashed.saturating_add(1);
            computed_hashes.push(hash);
        }
        if let Some(started) = hash_pipeline_started {
            timing.hash_pipeline_wall = timing.hash_pipeline_wall.saturating_add(started.elapsed());
        }
        computed_hashes.sort_by_key(|hash| hash.source_page_number);
        for hash in &computed_hashes {
            measure(&mut timing.hash_cache_write, || {
                inner.repository.duplicate_page_hash_upsert(hash)
            })?;
        }
        hashes.extend(computed_hashes);
        hashes.sort_by_key(|hash| hash.source_page_number);
        artifacts.push(HashedArtifact {
            gallery: gallery_ref(bundle, hashes.len() as u32),
            pages: hashes,
        });
        let progress = measure(&mut timing.progress_write, || {
            inner.repository.duplicate_scan_progress(
                run_id,
                u32::try_from(artifact_index + 1).unwrap_or(u32::MAX),
                0,
            )
        })?;
        if let Some(run) = progress {
            let _ = inner.events.send(run);
        }
    }

    let mut compared_pairs = 0_u64;
    for candidate_pair in candidate_pairs {
        let parent_index = candidate_pair.parent_index;
        let candidate_index = candidate_pair.candidate_index;
        if cancelled(inner, run_id, cancellation)? {
            timing.finish("cancelled");
            return Ok(());
        }
        let parent = &artifacts[parent_index];
        let candidate = &artifacts[candidate_index];
        let preliminary = measure(&mut timing.hash_compare, || {
            analyze_artifact_pair(run_id, parent, candidate, profile, None)
        });
        let external = if inner.relations.enabled() {
            measure(&mut timing.relation_lookup, || {
                inner
                    .relations
                    .relation(parent.gallery.gallery_id, candidate.gallery.gallery_id)
            })?
        } else {
            None
        };
        let record = if external.is_some() {
            measure(&mut timing.hash_compare, || {
                analyze_artifact_pair(run_id, parent, candidate, profile, external)
            })
        } else {
            preliminary
        };
        if let Some(record) = record {
            let _ = measure(&mut timing.candidate_write, || {
                inner.repository.duplicate_candidate_replace(&record)
            })?;
            timing.candidates_written = timing.candidates_written.saturating_add(1);
        }
        compared_pairs += 1;
        timing.pairs_compared = compared_pairs;
        if compared_pairs.is_multiple_of(PAIR_PROGRESS_EVENT_INTERVAL) {
            let progress = measure(&mut timing.progress_write, || {
                inner.repository.duplicate_scan_progress(
                    run_id,
                    artifacts.len() as u32,
                    compared_pairs,
                )
            })?;
            if let Some(run) = progress {
                let _ = inner.events.send(run);
            }
        }
    }

    if cancelled(inner, run_id, cancellation)? {
        timing.finish("cancelled");
        return Ok(());
    }
    let progress = measure(&mut timing.progress_write, || {
        inner
            .repository
            .duplicate_scan_progress(run_id, artifacts.len() as u32, compared_pairs)
    })?;
    if let Some(run) = progress {
        let _ = inner.events.send(run);
    }
    let finished = measure(&mut timing.finish_write, || {
        inner
            .repository
            .duplicate_scan_finish(run_id, DuplicateScanState::Completed, None, None)
    })?;
    if let Some(run) = finished {
        let _ = inner.events.send(run);
    }
    timing.finish("completed");
    Ok(())
}

struct DuplicateHashJob {
    entry_id: String,
    gallery_id: GalleryId,
    source_page_number: SourcePageNumber,
    artifact_sha256: ArtifactSha256,
    bytes: Vec<u8>,
}

struct DuplicateHashResult {
    elapsed: Duration,
    result: Result<DuplicatePageHash, RepositoryError>,
}

struct DuplicateHashPool {
    sender: Option<SyncSender<DuplicateHashJob>>,
    results: Receiver<DuplicateHashResult>,
    workers: Vec<JoinHandle<()>>,
}

impl DuplicateHashPool {
    fn new(profile: &HashProfile) -> Result<Self, RepositoryError> {
        let worker_count = duplicate_hash_worker_count();
        let (sender, jobs) = mpsc::sync_channel::<DuplicateHashJob>(worker_count);
        let jobs = Arc::new(Mutex::new(jobs));
        let (result_sender, results) = mpsc::channel();
        let profile = Arc::new(profile.clone());
        let mut workers: Vec<JoinHandle<()>> = Vec::with_capacity(worker_count);

        for ordinal in 0..worker_count {
            let jobs = Arc::clone(&jobs);
            let result_sender = result_sender.clone();
            let profile = Arc::clone(&profile);
            let worker = match thread::Builder::new()
                .name(format!("atsumi-duplicate-hash-{}", ordinal + 1))
                .spawn(move || loop {
                    let job = match jobs.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else {
                        return;
                    };
                    let started = Instant::now();
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        compute_page_hash(
                            &job.entry_id,
                            job.gallery_id,
                            job.source_page_number,
                            job.artifact_sha256,
                            &job.bytes,
                            &profile,
                        )
                    }))
                    .unwrap_or_else(|_| {
                        Err(RepositoryError::Other(
                            "managed page hash worker stopped unexpectedly".into(),
                        ))
                    });
                    if result_sender
                        .send(DuplicateHashResult {
                            elapsed: started.elapsed(),
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                }) {
                Ok(worker) => worker,
                Err(error) => {
                    drop(sender);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(RepositoryError::Other(format!(
                        "could not start duplicate hash worker: {error}"
                    )));
                }
            };
            workers.push(worker);
        }
        drop(result_sender);

        Ok(Self {
            sender: Some(sender),
            results,
            workers,
        })
    }

    fn worker_count(&self) -> usize {
        self.workers.len()
    }

    fn submit(&self, job: DuplicateHashJob) -> Result<(), RepositoryError> {
        self.sender
            .as_ref()
            .ok_or_else(|| RepositoryError::Other("duplicate hash workers are closed".into()))?
            .send(job)
            .map_err(|_| RepositoryError::Other("duplicate hash worker stopped".into()))
    }

    fn receive(&self) -> Result<DuplicateHashResult, RepositoryError> {
        self.results
            .recv()
            .map_err(|_| RepositoryError::Other("duplicate hash result was unavailable".into()))
    }

    fn shutdown(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for DuplicateHashPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn duplicate_hash_worker_count() -> usize {
    thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, MAX_DUPLICATE_HASH_WORKERS)
}

fn measure<T>(total: &mut Duration, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    *total = total.saturating_add(started.elapsed());
    result
}

struct DuplicateScanTiming<'a> {
    run_id: &'a str,
    started: Instant,
    hash_cache_read: Duration,
    image_read: Duration,
    hash_pipeline_wall: Duration,
    hash_compute: Duration,
    hash_cache_write: Duration,
    hash_compare: Duration,
    relation_lookup: Duration,
    candidate_write: Duration,
    progress_write: Duration,
    finish_write: Duration,
    hash_cache_hits: u64,
    hash_cache_misses: u64,
    image_bytes_read: u64,
    pages_hashed: u64,
    pairs_compared: u64,
    candidates_written: u64,
    hash_worker_count: usize,
    logged: bool,
}

impl<'a> DuplicateScanTiming<'a> {
    fn new(run_id: &'a str, hash_worker_count: usize) -> Self {
        Self {
            run_id,
            started: Instant::now(),
            hash_cache_read: Duration::ZERO,
            image_read: Duration::ZERO,
            hash_pipeline_wall: Duration::ZERO,
            hash_compute: Duration::ZERO,
            hash_cache_write: Duration::ZERO,
            hash_compare: Duration::ZERO,
            relation_lookup: Duration::ZERO,
            candidate_write: Duration::ZERO,
            progress_write: Duration::ZERO,
            finish_write: Duration::ZERO,
            hash_cache_hits: 0,
            hash_cache_misses: 0,
            image_bytes_read: 0,
            pages_hashed: 0,
            pairs_compared: 0,
            candidates_written: 0,
            hash_worker_count,
            logged: false,
        }
    }

    fn database_write_time(&self) -> Duration {
        self.hash_cache_write
            .saturating_add(self.candidate_write)
            .saturating_add(self.progress_write)
            .saturating_add(self.finish_write)
    }

    fn image_pipeline_time(&self) -> Duration {
        self.hash_pipeline_wall
    }

    fn bottleneck(&self) -> &'static str {
        [
            ("image_load_and_hash", self.image_pipeline_time()),
            ("hash_compare", self.hash_compare),
            ("database_write", self.database_write_time()),
            ("hash_cache_read", self.hash_cache_read),
        ]
        .into_iter()
        .max_by_key(|(_, elapsed)| *elapsed)
        .map_or("none", |(stage, _)| stage)
    }

    fn finish(&mut self, outcome: &'static str) {
        self.log(outcome);
        self.logged = true;
    }

    fn log(&self, outcome: &'static str) {
        tracing::info!(
            run_id = self.run_id,
            outcome,
            bottleneck = self.bottleneck(),
            total_us = duration_micros(self.started.elapsed()),
            hash_cache_read_us = duration_micros(self.hash_cache_read),
            image_read_us = duration_micros(self.image_read),
            hash_pipeline_wall_us = duration_micros(self.hash_pipeline_wall),
            hash_compute_cpu_us = duration_micros(self.hash_compute),
            hash_cache_write_us = duration_micros(self.hash_cache_write),
            hash_compare_us = duration_micros(self.hash_compare),
            relation_lookup_us = duration_micros(self.relation_lookup),
            candidate_write_us = duration_micros(self.candidate_write),
            progress_write_us = duration_micros(self.progress_write),
            finish_write_us = duration_micros(self.finish_write),
            database_write_us = duration_micros(self.database_write_time()),
            hash_cache_hits = self.hash_cache_hits,
            hash_cache_misses = self.hash_cache_misses,
            image_bytes_read = self.image_bytes_read,
            pages_hashed = self.pages_hashed,
            pairs_compared = self.pairs_compared,
            candidates_written = self.candidates_written,
            hash_worker_count = self.hash_worker_count,
            "duplicate scan stage_profile"
        );
    }
}

impl Drop for DuplicateScanTiming<'_> {
    fn drop(&mut self) {
        if !self.logged {
            self.log("failed");
        }
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn cancelled(
    inner: &DuplicateSupervisorInner,
    run_id: &str,
    cancellation: &CancellationToken,
) -> Result<bool, RepositoryError> {
    Ok(cancellation.is_cancelled() || !inner.repository.duplicate_scan_is_running(run_id)?)
}

fn normalized_candidate_id(candidate_id: &str) -> Result<&str, ApplicationError> {
    let candidate_id = candidate_id.trim();
    if candidate_id.is_empty() || candidate_id.len() > 200 {
        return Err(crate::domain::ValidationError::new(
            "candidateId",
            "must be non-empty and at most 200 bytes",
        )
        .into());
    }
    Ok(candidate_id)
}

fn validate_decision_request(request: &DuplicateDecisionRequest) -> Result<(), ApplicationError> {
    normalized_candidate_id(&request.candidate_id)?;
    if request.series_group_id.as_ref().is_some_and(|value| {
        let value = value.trim();
        value.is_empty() || value.len() > 200
    }) {
        return Err(crate::domain::ValidationError::new(
            "seriesGroupId",
            "must be non-empty and at most 200 bytes",
        )
        .into());
    }
    if request.series_name.as_ref().is_some_and(|value| {
        let value = value.trim();
        value.is_empty() || value.len() > 200
    }) {
        return Err(crate::domain::ValidationError::new(
            "seriesName",
            "must be non-empty and at most 200 bytes",
        )
        .into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CandidatePair {
    parent_index: usize,
    candidate_index: usize,
}

/// Builds the complete deterministic worklist within each normalized artist
/// bucket. Albums without a shared artist are never hashed or compared by the
/// global duplicate scan. A set deduplicates pairs for multi-artist albums.
fn same_artist_scan_plan(
    bundles: Vec<crate::domain::ArtifactBundle>,
) -> (Vec<crate::domain::ArtifactBundle>, Vec<CandidatePair>) {
    let mut artist_buckets = BTreeMap::<String, Vec<usize>>::new();
    for (bundle_index, bundle) in bundles.iter().enumerate() {
        let mut artists = bundle.gallery.metadata.artists.clone();
        if let Some(primary_artist) = bundle.gallery.metadata.primary_artist.as_ref() {
            artists.push(primary_artist.clone());
        }
        for artist in normalized_artist_keys(&artists) {
            artist_buckets.entry(artist).or_default().push(bundle_index);
        }
    }

    let mut original_pairs = BTreeSet::<(usize, usize)>::new();
    for bucket in artist_buckets.values() {
        for left_position in 0..bucket.len() {
            for right_position in (left_position + 1)..bucket.len() {
                original_pairs.insert((bucket[left_position], bucket[right_position]));
            }
        }
    }

    let participating_indices = original_pairs
        .iter()
        .flat_map(|(left, right)| [*left, *right])
        .collect::<BTreeSet<_>>();
    let index_map = participating_indices
        .iter()
        .enumerate()
        .map(|(new_index, old_index)| (*old_index, new_index))
        .collect::<BTreeMap<_, _>>();
    let candidate_pairs = original_pairs
        .into_iter()
        .map(|(parent_index, candidate_index)| CandidatePair {
            parent_index: index_map[&parent_index],
            candidate_index: index_map[&candidate_index],
        })
        .collect();
    let bundles = bundles
        .into_iter()
        .enumerate()
        .filter_map(|(index, bundle)| participating_indices.contains(&index).then_some(bundle))
        .collect();
    (bundles, candidate_pairs)
}

fn select_scan_bundles(
    bundles: Vec<crate::domain::ArtifactBundle>,
) -> Vec<crate::domain::ArtifactBundle> {
    let mut bundles = bundles
        .into_iter()
        .filter(|bundle| verified_scan_pages(bundle).is_some())
        .collect::<Vec<_>>();
    bundles.sort_by(|left, right| {
        left.gallery
            .id
            .cmp(&right.gallery.id)
            .then_with(|| right.artifact.completed_at.cmp(&left.artifact.completed_at))
            .then_with(|| right.artifact.revision.cmp(&left.artifact.revision))
            .then_with(|| left.artifact.entry_id.cmp(&right.artifact.entry_id))
    });
    bundles.dedup_by(|left, right| left.gallery.id == right.gallery.id);
    bundles
}

fn stable_scan_error(error: &RepositoryError) -> String {
    match error {
        RepositoryError::Busy(_) => "The duplicate evidence database is busy; retry the scan",
        RepositoryError::Corrupt(_) => {
            "Verified artifact metadata changed during duplicate scanning; reconcile downloads"
        }
        RepositoryError::Source(_) => "Optional relation evidence could not be loaded",
        _ => "A verified artifact could not be analyzed; reconcile downloads and retry",
    }
    .into()
}

fn short_id(value: &str) -> &str {
    value.rsplit('-').next().unwrap_or("run")
}

#[cfg(test)]
mod tests {
    use std::{
        hint::black_box,
        sync::{mpsc, Arc},
        time::{Duration, Instant},
    };

    use crate::application::{ArtifactStore, DuplicateRepository, StateRepository};
    use crate::domain::{
        ArtifactBundle, ArtifactRelativePath, ArtifactSha256, ArtifactStorageFormat,
        DownloadArtifact, DownloadArtifactState, DownloadEntryId, DuplicateScanState, Gallery,
        GalleryId, GalleryMetadata, PageArtifact, PageArtifactState, SourcePageNumber,
    };
    use crate::infrastructure::{FilesystemArtifactStore, SqliteRepository};
    use crate::thumbnail::CancellationToken;
    use tempfile::tempdir;

    use super::{
        duplicate_hash_worker_count, same_artist_scan_plan, select_scan_bundles, ActiveRun,
        DisabledDuplicateRelationProvider, DuplicateHashJob, DuplicateHashPool,
        DuplicateScanTiming, DuplicateSupervisor, MAX_DUPLICATE_HASH_WORKERS,
    };

    fn bundle(
        gallery_id: i64,
        entry_id: &str,
        revision: u64,
        completed_at: &str,
    ) -> ArtifactBundle {
        let gallery_id = GalleryId::new(gallery_id).unwrap();
        let entry_id = DownloadEntryId::new(entry_id).unwrap();
        let directory = ArtifactRelativePath::new(format!("gallery-{entry_id}")).unwrap();
        let gallery = Gallery::new(
            gallery_id,
            revision,
            GalleryMetadata::new("Gallery", None, None, 1).unwrap(),
        );
        let artifact = DownloadArtifact::new(
            entry_id.clone(),
            gallery_id,
            revision,
            directory.clone(),
            1,
            DownloadArtifactState::Complete,
        )
        .unwrap()
        .with_manifest(
            ArtifactRelativePath::new(format!("{directory}/manifest.json")).unwrap(),
            1,
            "test",
            1,
            completed_at,
        )
        .unwrap();
        let page = PageArtifact::new(
            entry_id,
            gallery_id,
            SourcePageNumber::new(1).unwrap(),
            ArtifactRelativePath::new(format!("{directory}/page-1.webp")).unwrap(),
            PageArtifactState::Present,
            Some(10),
        )
        .unwrap()
        .with_verification(
            ArtifactSha256::new(format!("{:064x}", revision + 1)).unwrap(),
            ArtifactStorageFormat::Webp,
            "source",
            completed_at,
        )
        .unwrap();
        ArtifactBundle::new(gallery, artifact, vec![page]).unwrap()
    }

    #[test]
    fn latest_verified_artifact_per_gallery_is_selected_deterministically() {
        let older = bundle(1, "entry-z-older", 50, "2026-08-14T00:00:00.000Z");
        let latest_lower_revision = bundle(1, "entry-b-latest", 2, "2026-08-15T00:00:00.000Z");
        let latest_higher_revision = bundle(1, "entry-a-latest", 3, "2026-08-15T00:00:00.000Z");
        let other = bundle(2, "entry-other", 1, "2026-08-10T00:00:00.000Z");
        let selected = select_scan_bundles(vec![
            older,
            latest_lower_revision,
            other,
            latest_higher_revision,
        ]);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].artifact.entry_id.as_str(), "entry-a-latest");
        assert_eq!(selected[1].artifact.entry_id.as_str(), "entry-other");
    }

    #[test]
    fn candidate_generation_keeps_only_normalized_same_artist_pairs() {
        let mut first = bundle(1, "entry-first", 1, "2026-08-15T00:00:00.000Z");
        first.gallery.metadata.artists = vec!["Shared_Artist".into(), "Co Author".into()];
        let mut second = bundle(2, "entry-second", 1, "2026-08-15T00:00:00.000Z");
        second.gallery.metadata.artists = vec![" shared artist ".into()];
        let mut third = bundle(3, "entry-third", 1, "2026-08-15T00:00:00.000Z");
        third.gallery.metadata.artists = vec!["CO  AUTHOR".into()];
        let mut unrelated = bundle(4, "entry-unrelated", 1, "2026-08-15T00:00:00.000Z");
        unrelated.gallery.metadata.artists = vec!["Different Artist".into()];
        let missing = bundle(5, "entry-missing", 1, "2026-08-15T00:00:00.000Z");

        let (bundles, pairs) =
            same_artist_scan_plan(vec![first, unrelated, second, missing, third]);

        assert_eq!(
            bundles
                .iter()
                .map(|bundle| bundle.gallery.id.get())
                .collect::<Vec<_>>(),
            vec![1, 2, 3],
            "unrelated and missing-artist artifacts must not be hashed",
        );
        assert_eq!(
            pairs
                .iter()
                .map(|pair| (pair.parent_index, pair.candidate_index))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 2)],
            "multi-artist overlap is deduplicated and different artists are excluded",
        );
    }

    #[test]
    fn primary_artist_fallback_participates_in_same_artist_plan() {
        let mut first = bundle(1, "entry-first", 1, "2026-08-15T00:00:00.000Z");
        first.gallery.metadata.primary_artist = Some("Serein".into());
        let mut second = bundle(2, "entry-second", 1, "2026-08-15T00:00:00.000Z");
        second.gallery.metadata.primary_artist = Some("serein".into());

        let (bundles, pairs) = same_artist_scan_plan(vec![first, second]);

        assert_eq!(bundles.len(), 2);
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].parent_index, pairs[0].candidate_index), (0, 1));
    }

    #[test]
    fn stage_profile_identifies_the_largest_measured_cost_without_counting_total_wall_time() {
        let mut timing = DuplicateScanTiming::new("run-profile", 2);
        timing.image_read = Duration::from_millis(4);
        timing.hash_pipeline_wall = Duration::from_millis(11);
        timing.hash_compute = Duration::from_millis(7);
        timing.hash_compare = Duration::from_millis(19);
        timing.hash_cache_write = Duration::from_millis(3);
        timing.candidate_write = Duration::from_millis(2);
        timing.progress_write = Duration::from_millis(1);

        assert_eq!(timing.image_pipeline_time(), Duration::from_millis(11));
        assert_eq!(timing.database_write_time(), Duration::from_millis(6));
        assert_eq!(timing.bottleneck(), "hash_compare");
        timing.finish("test");
    }

    #[test]
    fn duplicate_hash_workers_are_bounded_and_preserve_page_identity() {
        let worker_count = duplicate_hash_worker_count();
        assert!((1..=MAX_DUPLICATE_HASH_WORKERS).contains(&worker_count));

        let bytes = {
            use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
            let mut rgba = vec![0_u8; 64 * 96 * 4];
            for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let value = (index % 251) as u8;
                pixel.copy_from_slice(&[value, value.rotate_left(2), 255 - value, 255]);
            }
            let mut encoded = Vec::new();
            PngEncoder::new(&mut encoded)
                .write_image(&rgba, 64, 96, ExtendedColorType::Rgba8)
                .unwrap();
            encoded
        };
        let pool = DuplicateHashPool::new(&crate::domain::HashProfile::current()).unwrap();
        for source_page in [4_u32, 1, 3, 2] {
            pool.submit(DuplicateHashJob {
                entry_id: "entry-parallel".into(),
                gallery_id: GalleryId::new(7).unwrap(),
                source_page_number: SourcePageNumber::new(source_page).unwrap(),
                artifact_sha256: ArtifactSha256::new(format!("{source_page:064x}")).unwrap(),
                bytes: bytes.clone(),
            })
            .unwrap();
        }
        let mut pages = (0..4)
            .map(|_| {
                pool.receive()
                    .unwrap()
                    .result
                    .unwrap()
                    .source_page_number
                    .get()
            })
            .collect::<Vec<_>>();
        pages.sort_unstable();
        assert_eq!(pages, vec![1, 2, 3, 4]);
    }

    #[test]
    #[ignore = "manual local bounded hash-worker throughput profile"]
    fn profile_bounded_duplicate_hash_workers() {
        use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

        const PAGE_COUNT: u32 = 96;
        let mut rgba = vec![0_u8; 640 * 960 * 4];
        for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let value = (index % 251) as u8;
            pixel.copy_from_slice(&[value, value.rotate_left(2), 255 - value, 255]);
        }
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&rgba, 640, 960, ExtendedColorType::Rgba8)
            .unwrap();
        let profile = crate::domain::HashProfile::current();

        let sequential_started = Instant::now();
        for source_page in 1..=PAGE_COUNT {
            let hash = super::compute_page_hash(
                "entry-sequential-profile",
                GalleryId::new(1).unwrap(),
                SourcePageNumber::new(source_page).unwrap(),
                ArtifactSha256::new(format!("{source_page:064x}")).unwrap(),
                &bytes,
                &profile,
            )
            .unwrap();
            black_box(hash);
        }
        let sequential = sequential_started.elapsed();

        let pool = DuplicateHashPool::new(&profile).unwrap();
        let parallel_started = Instant::now();
        for source_page in 1..=PAGE_COUNT {
            pool.submit(DuplicateHashJob {
                entry_id: "entry-parallel-profile".into(),
                gallery_id: GalleryId::new(2).unwrap(),
                source_page_number: SourcePageNumber::new(source_page).unwrap(),
                artifact_sha256: ArtifactSha256::new(format!("{source_page:064x}")).unwrap(),
                bytes: bytes.clone(),
            })
            .unwrap();
        }
        for _ in 0..PAGE_COUNT {
            black_box(pool.receive().unwrap().result.unwrap());
        }
        let parallel = parallel_started.elapsed();

        eprintln!(
            "duplicate bounded hash profile: pages={} workers={} sequential_us={} parallel_us={} speedup={:.2}",
            PAGE_COUNT,
            pool.worker_count(),
            sequential.as_micros(),
            parallel.as_micros(),
            sequential.as_secs_f64() / parallel.as_secs_f64(),
        );
    }

    #[test]
    fn active_run_snapshot_includes_running_and_excludes_terminal_runs() {
        let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let run = DuplicateRepository::duplicate_scan_start(repository.as_ref(), 1, 0, 0)
            .expect("seed running duplicate scan");
        let duplicate_repository: Arc<dyn DuplicateRepository> = repository.clone();
        let settings: Arc<dyn StateRepository> = repository.clone();
        let store: Arc<dyn ArtifactStore> = Arc::new(FilesystemArtifactStore::new());
        let (events, _receiver) = mpsc::channel();
        let supervisor = DuplicateSupervisor::new(
            duplicate_repository,
            settings,
            store,
            Arc::new(DisabledDuplicateRelationProvider),
            events,
        );
        *supervisor.active_lock().unwrap() = Some(ActiveRun {
            run_id: run.run_id.clone(),
            cancellation: CancellationToken::new(),
            worker: None,
        });

        let active = supervisor
            .active_run_snapshot()
            .expect("read active duplicate run")
            .expect("running scan should be included");
        assert_eq!(active.run_id, run.run_id);
        assert_eq!(active.state, DuplicateScanState::Running);

        DuplicateRepository::duplicate_scan_finish(
            repository.as_ref(),
            &run.run_id,
            DuplicateScanState::Completed,
            None,
            None,
        )
        .expect("finish duplicate scan");
        assert!(supervisor
            .active_run_snapshot()
            .expect("read terminal duplicate run")
            .is_none());
        *supervisor.active_lock().unwrap() = None;
    }

    #[test]
    fn commit_rechecks_for_a_run_started_during_unlocked_preflight() {
        let temporary = tempdir().expect("create duplicate scan root");
        let root = temporary.path().join("downloads");
        std::fs::create_dir_all(&root).expect("create download root");
        let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let mut settings = StateRepository::settings_get(repository.as_ref()).unwrap();
        let expected_revision = settings.revision;
        settings.revision += 1;
        settings.download_root = root.to_string_lossy().into_owned();
        assert!(StateRepository::settings_compare_and_set(
            repository.as_ref(),
            &settings,
            expected_revision,
        )
        .unwrap());
        let duplicate_repository: Arc<dyn DuplicateRepository> = repository.clone();
        let state_repository: Arc<dyn StateRepository> = repository.clone();
        let store: Arc<dyn ArtifactStore> = Arc::new(FilesystemArtifactStore::new());
        let (events, _receiver) = mpsc::channel();
        let supervisor = DuplicateSupervisor::new(
            duplicate_repository,
            state_repository,
            store,
            Arc::new(DisabledDuplicateRelationProvider),
            events,
        );

        let prepared = supervisor.prepare_start().expect("prepare scan input");
        let existing = DuplicateRepository::duplicate_scan_start(repository.as_ref(), 1, 0, 0)
            .expect("start competing duplicate scan");
        *supervisor.active_lock().unwrap() = Some(ActiveRun {
            run_id: existing.run_id.clone(),
            cancellation: CancellationToken::new(),
            worker: None,
        });

        let returned = supervisor
            .commit_start(prepared)
            .expect("reuse the run that won the commit race");
        assert_eq!(returned.run_id, existing.run_id);
        *supervisor.active_lock().unwrap() = None;
        DuplicateRepository::duplicate_scan_finish(
            repository.as_ref(),
            &existing.run_id,
            DuplicateScanState::Cancelled,
            None,
            None,
        )
        .expect("finish seeded duplicate run");
    }
}
