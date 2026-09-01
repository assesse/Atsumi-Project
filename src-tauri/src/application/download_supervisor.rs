use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc, Condvar, Mutex, MutexGuard,
    },
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::{
    domain::{
        plan_artifact_relative_directory, ArtifactManifest, ArtifactRelativePath,
        ArtifactStorageFormat, DownloadArtifactState, DownloadEntry, DownloadEntryId,
        DownloadJobDescriptor, DownloadJobProjection, DownloadOverlapDecisionAction,
        DownloadOverlapDecisionActor, DownloadOverlapDecisionApplyOutcome,
        DownloadOverlapDecisionRequest, DownloadOverlapDecisionResult, DownloadOverlapReview,
        DownloadOverlapReviewDraft, DuplicateGalleryRef, HashProfile, JobRef, JobState,
        PageArtifactState, ARTIFACT_MANIFEST_SCHEMA_VERSION, DOWNLOAD_OVERLAP_POLICY_VERSION,
        HASH_PROFILE_VERSION,
    },
    source::{SourceCandidateDiagnostic, SourceContractError, SourceErrorCode},
    thumbnail::CancellationToken,
};

use super::{
    analyze_download_overlap_pair, duplicate_analyzer::compute_page_hash, hashed_artifact,
    normalized_artist_keys, overlap_artifact_fingerprint, overlap_artists_intersect,
    overlap_gallery_ref, verified_overlap_pages, ApplicationError, ArtifactLayout, ArtifactStore,
    DownloadArtifactPlan, DownloadOverlapRepository, DownloadPageAttempt,
    DownloadPageAttemptOutcome, DownloadPageAttemptResult, DownloadPipelineError,
    DownloadPipelineErrorCode, DownloadSourcePort, ExistingPageVerification, QuarantineSaga,
    QuarantineSagaState, ReconcileIssue, ReconcileReport, RepositoryError, StateRepository,
    StoredPage,
};

#[derive(Clone)]
pub struct DownloadSupervisor {
    inner: Arc<SupervisorInner>,
}

struct SupervisorInner {
    queue: Mutex<QueueState>,
    wake: Condvar,
    cancellations: Mutex<HashMap<String, ActiveCancellation>>,
    repository: Arc<dyn DownloadOverlapRepository>,
    settings: Arc<dyn StateRepository>,
    source: Arc<dyn DownloadSourcePort>,
    store: Arc<dyn ArtifactStore>,
    events: Sender<DownloadJobProjection>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    finalization_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    overlap_decisions: Mutex<()>,
    shutting_down: AtomicBool,
}

struct QueueState {
    pending: VecDeque<DownloadJobDescriptor>,
    known: HashSet<String>,
    closed: bool,
}

struct ActiveCancellation {
    entry_id: String,
    token: CancellationToken,
}

const MAX_FULL_IMAGE_MEMORY_SLOTS: usize = 2;

impl DownloadSupervisor {
    pub fn new(
        repository: Arc<dyn DownloadOverlapRepository>,
        settings: Arc<dyn StateRepository>,
        source: Arc<dyn DownloadSourcePort>,
        store: Arc<dyn ArtifactStore>,
        events: Sender<DownloadJobProjection>,
        gallery_worker_count: usize,
    ) -> Result<Self, DownloadPipelineError> {
        if !(1..=8).contains(&gallery_worker_count) {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::WorkerUnavailable,
                "The gallery worker count must be between 1 and 8",
                false,
            ));
        }
        let inner = Arc::new(SupervisorInner {
            queue: Mutex::new(QueueState {
                pending: VecDeque::new(),
                known: HashSet::new(),
                closed: false,
            }),
            wake: Condvar::new(),
            cancellations: Mutex::new(HashMap::new()),
            repository,
            settings,
            source,
            store,
            events,
            workers: Mutex::new(Vec::new()),
            finalization_locks: Mutex::new(HashMap::new()),
            overlap_decisions: Mutex::new(()),
            shutting_down: AtomicBool::new(false),
        });
        let supervisor = Self {
            inner: Arc::clone(&inner),
        };
        let mut workers = unpoison(inner.workers.lock());
        for index in 0..gallery_worker_count.min(MAX_FULL_IMAGE_MEMORY_SLOTS) {
            let worker_inner = Arc::clone(&inner);
            let handle = thread::Builder::new()
                .name(format!("atsumi-download-{index}"))
                .spawn(move || worker_loop(worker_inner))
                .map_err(|_| {
                    DownloadPipelineError::new(
                        DownloadPipelineErrorCode::WorkerUnavailable,
                        "A download worker thread could not be started",
                        true,
                    )
                })?;
            workers.push(handle);
        }
        drop(workers);
        Ok(supervisor)
    }

    pub fn enqueue(
        &self,
        descriptor: DownloadJobDescriptor,
    ) -> Result<bool, DownloadPipelineError> {
        let key = descriptor_key(&descriptor);
        let mut queue = unpoison(self.inner.queue.lock());
        if queue.closed || self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::WorkerUnavailable,
                "The download worker is shutting down",
                true,
            ));
        }
        if !queue.known.insert(key) {
            return Ok(false);
        }
        queue.pending.push_back(descriptor);
        self.inner.wake.notify_one();
        Ok(true)
    }

    pub fn enqueue_all(
        &self,
        descriptors: impl IntoIterator<Item = DownloadJobDescriptor>,
    ) -> Result<usize, DownloadPipelineError> {
        let mut added = 0;
        for descriptor in descriptors {
            added += usize::from(self.enqueue(descriptor)?);
        }
        Ok(added)
    }

    pub fn cancel_entries(&self, entry_ids: &[String]) -> usize {
        let entry_ids = entry_ids.iter().collect::<HashSet<_>>();
        let cancellations = unpoison(self.inner.cancellations.lock());
        let mut cancelled = 0;
        for active in cancellations.values() {
            if entry_ids.contains(&active.entry_id) {
                active.token.cancel();
                cancelled += 1;
            }
        }
        cancelled
    }

    pub fn resume_interrupted(&self) -> Result<usize, RepositoryError> {
        let jobs = self.inner.repository.pipeline_resume_interrupted()?;
        self.enqueue_all(jobs).map_err(|error| {
            RepositoryError::Other(format!("could not resume interrupted jobs: {error}"))
        })
    }

    pub fn enqueue_retries(&self, jobs: &[JobRef]) -> Result<usize, RepositoryError> {
        let descriptors = self.inner.repository.pipeline_descriptors_for_jobs(jobs)?;
        self.enqueue_all(descriptors).map_err(|error| {
            RepositoryError::Other(format!("could not launch retried jobs: {error}"))
        })
    }

    pub fn open_first(&self, entry_id: String) -> Result<(), ApplicationError> {
        let entry_id = DownloadEntryId::new(entry_id)?;
        let bundle = self
            .inner
            .repository
            .pipeline_artifact_bundle(&entry_id)?
            .ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ArtifactMissing,
                    "The completed download has no artifact record",
                    false,
                )
            })?;
        let root = self.inner.repository.pipeline_artifact_root(&entry_id)?;
        let path = self.inner.store.first_verified_page_path(&root, &bundle)?;
        self.inner.store.open_with_default_viewer(&path)?;
        Ok(())
    }

    pub fn open_folder(&self, entry_id: String) -> Result<(), ApplicationError> {
        let path = self.artifact_folder_path(&entry_id)?;
        self.inner.store.open_with_default_viewer(&path)?;
        Ok(())
    }

    fn artifact_folder_path(&self, entry_id: &str) -> Result<PathBuf, ApplicationError> {
        let entry_id = DownloadEntryId::new(entry_id.to_owned())?;
        let bundle = self
            .inner
            .repository
            .pipeline_artifact_bundle(&entry_id)?
            .ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ArtifactMissing,
                    "The gallery storage folder is not ready yet",
                    false,
                )
            })?;
        let root = self.inner.repository.pipeline_artifact_root(&entry_id)?;
        Ok(self
            .inner
            .store
            .artifact_directory_path(&root, &bundle.artifact.relative_directory)?)
    }

    pub fn quarantine_entries(
        &self,
        entry_ids: Vec<String>,
        reason: String,
    ) -> Result<Vec<DownloadEntry>, ApplicationError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 500 {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::QuarantineConflict,
                "A quarantine reason between 1 and 500 bytes is required",
                false,
            )
            .into());
        }
        let mut unique = BTreeMap::new();
        for raw in entry_ids {
            let entry_id = DownloadEntryId::new(raw)?;
            unique.insert(entry_id.to_string(), entry_id);
        }
        let mut entries = Vec::with_capacity(unique.len());
        for entry_id in unique.into_values() {
            let root = self.inner.store.validate_download_root(
                &self.inner.repository.pipeline_artifact_root(&entry_id)?,
            )?;
            let bundle = self
                .inner
                .repository
                .pipeline_artifact_bundle(&entry_id)?
                .ok_or_else(|| {
                    DownloadPipelineError::new(
                        DownloadPipelineErrorCode::ArtifactMissing,
                        "The download has no verified artifact to quarantine",
                        false,
                    )
                })?;
            if bundle.artifact.state != DownloadArtifactState::Complete {
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::QuarantineConflict,
                    "Only a verified complete artifact can be quarantined",
                    false,
                )
                .into());
            }
            let original_layout = layout_for_bundle(root.clone(), &bundle)?;
            let expected_manifest = ArtifactManifest::from_bundle(&bundle)?;
            match self.inner.store.read_manifest(&original_layout)? {
                Some(actual) if actual == expected_manifest => {}
                _ => {
                    return Err(DownloadPipelineError::new(
                        DownloadPipelineErrorCode::ManifestInvalid,
                        "The artifact manifest must be verified before quarantine",
                        false,
                    )
                    .into())
                }
            }
            let record_id = Uuid::new_v4().to_string();
            let quarantine_relative_path = ArtifactRelativePath::new(format!(
                ".atsumi-quarantine/{record_id}/{}",
                bundle.artifact.relative_directory.as_str()
            ))?;
            let saga = QuarantineSaga {
                record_id,
                entry_id: entry_id.clone(),
                original_relative_path: bundle.artifact.relative_directory.clone(),
                quarantine_relative_path,
                reason: reason.to_owned(),
                state: QuarantineSagaState::PendingQuarantine,
            };
            self.inner.repository.pipeline_quarantine_begin(&saga)?;
            self.inner.store.move_managed_directory(
                &root,
                &saga.original_relative_path,
                &saga.quarantine_relative_path,
            )?;
            write_quarantine_manifest(
                &self.inner,
                &root,
                &saga.quarantine_relative_path,
                &saga.quarantine_relative_path,
                expected_manifest,
                true,
            )?;
            let projection = self
                .inner
                .repository
                .pipeline_quarantine_complete(&saga.record_id)?;
            entries.push(download_entry_from_projection(&projection)?);
            emit(&self.inner, projection);
        }
        Ok(entries)
    }

    pub fn restore_entries(
        &self,
        entry_ids: Vec<String>,
    ) -> Result<Vec<DownloadEntry>, ApplicationError> {
        let mut unique = BTreeMap::new();
        for raw in entry_ids {
            let entry_id = DownloadEntryId::new(raw)?;
            unique.insert(entry_id.to_string(), entry_id);
        }
        let mut entries = Vec::with_capacity(unique.len());
        for entry_id in unique.into_values() {
            let saga = self.inner.repository.pipeline_restore_begin(&entry_id)?;
            let root = self.inner.store.validate_download_root(
                &self.inner.repository.pipeline_artifact_root(&entry_id)?,
            )?;
            let quarantine_layout =
                layout_for_directory(root.clone(), saga.quarantine_relative_path.clone())?;
            let manifest = self
                .inner
                .store
                .read_manifest(&quarantine_layout)?
                .ok_or_else(|| {
                    DownloadPipelineError::new(
                        DownloadPipelineErrorCode::ManifestInvalid,
                        "The quarantined artifact manifest is missing",
                        false,
                    )
                })?;
            write_quarantine_manifest(
                &self.inner,
                &root,
                &saga.quarantine_relative_path,
                &saga.original_relative_path,
                manifest,
                false,
            )?;
            self.inner.store.move_managed_directory(
                &root,
                &saga.quarantine_relative_path,
                &saga.original_relative_path,
            )?;
            let projection = self
                .inner
                .repository
                .pipeline_restore_complete(&saga.record_id)?;
            entries.push(download_entry_from_projection(&projection)?);
            emit(&self.inner, projection);
        }
        Ok(entries)
    }

    pub fn reconcile(&self) -> Result<ReconcileReport, ApplicationError> {
        let mut report = self.reconcile_without_resume()?;
        self.resume_after_reconcile(&mut report)?;
        Ok(report)
    }

    /// Performs filesystem/manifest reconciliation without changing an
    /// interrupted download into runnable work. Callers can do this long phase
    /// outside the app-exit gate and commit only `resume_after_reconcile` while
    /// holding the gate.
    pub fn reconcile_without_resume(&self) -> Result<ReconcileReport, ApplicationError> {
        let mut report = ReconcileReport {
            inspected_artifacts: 0,
            verified_artifacts: 0,
            resumed_jobs: 0,
            issues: Vec::new(),
        };
        self.reconcile_quarantine_sagas(&mut report)?;
        let bundles = self.inner.repository.pipeline_artifact_bundles()?;
        report.inspected_artifacts = u64::try_from(bundles.len()).unwrap_or(u64::MAX);
        for bundle in bundles {
            if bundle.artifact.state == DownloadArtifactState::Quarantined {
                continue;
            }
            let root = self.inner.store.validate_download_root(
                &self
                    .inner
                    .repository
                    .pipeline_artifact_root(&bundle.artifact.entry_id)?,
            )?;
            let mut artifact_issues = inspect_bundle(&self.inner, &root, &bundle);
            artifact_issues.sort_unstable();
            artifact_issues.dedup();
            if artifact_issues.is_empty() {
                if bundle.artifact.state == DownloadArtifactState::Complete {
                    report.verified_artifacts = report.verified_artifacts.saturating_add(1);
                }
                continue;
            }
            for (code, message) in artifact_issues {
                report.issues.push(ReconcileIssue {
                    entry_id: bundle.artifact.entry_id.to_string(),
                    code: code.to_owned(),
                    message: message.to_owned(),
                    recoverable: true,
                });
                if let Some(projection) = self.inner.repository.pipeline_mark_artifact_issue(
                    &bundle.artifact.entry_id,
                    &code,
                    &message,
                )? {
                    emit(&self.inner, projection);
                }
            }
        }
        Ok(report)
    }

    /// Performs only the recovery work that must happen before downloads can
    /// resume. Full artifact hash/decode verification stays behind the
    /// explicit `app_reconcile` command so opening the application does not
    /// scale with the user's completed library.
    pub fn recover_startup_state(&self) -> Result<ReconcileReport, ApplicationError> {
        let mut report = self.recover_startup_state_without_resume()?;
        self.resume_after_reconcile(&mut report)?;
        Ok(report)
    }

    /// Reconciles pending quarantine state but deliberately leaves interrupted
    /// downloads non-runnable until `resume_after_reconcile` is committed.
    pub fn recover_startup_state_without_resume(
        &self,
    ) -> Result<ReconcileReport, ApplicationError> {
        let mut report = ReconcileReport {
            inspected_artifacts: 0,
            verified_artifacts: 0,
            resumed_jobs: 0,
            issues: Vec::new(),
        };
        self.reconcile_quarantine_sagas(&mut report)?;
        Ok(report)
    }

    /// The short state transition + enqueue phase of reconcile/recovery.
    pub fn resume_after_reconcile(
        &self,
        report: &mut ReconcileReport,
    ) -> Result<(), ApplicationError> {
        report.resumed_jobs = u64::try_from(self.resume_interrupted()?).unwrap_or(u64::MAX);
        Ok(())
    }

    fn reconcile_quarantine_sagas(
        &self,
        report: &mut ReconcileReport,
    ) -> Result<(), ApplicationError> {
        for saga in self.inner.repository.pipeline_pending_quarantine_sagas()? {
            let root = self.inner.store.validate_download_root(
                &self
                    .inner
                    .repository
                    .pipeline_artifact_root(&saga.entry_id)?,
            )?;
            match self.reconcile_quarantine_saga(&root, &saga) {
                Ok(projection) => emit(&self.inner, projection),
                Err(error) => {
                    let (code, message) = stable_application_issue(&error);
                    report.issues.push(ReconcileIssue {
                        entry_id: saga.entry_id.to_string(),
                        code: code.to_owned(),
                        message: message.to_owned(),
                        recoverable: true,
                    });
                }
            }
        }
        Ok(())
    }

    fn reconcile_quarantine_saga(
        &self,
        root: &std::path::Path,
        saga: &QuarantineSaga,
    ) -> Result<DownloadJobProjection, ApplicationError> {
        let original_exists = self
            .inner
            .store
            .managed_path_exists(root, &saga.original_relative_path)?;
        let quarantine_exists = self
            .inner
            .store
            .managed_path_exists(root, &saga.quarantine_relative_path)?;
        match saga.state {
            QuarantineSagaState::PendingQuarantine => {
                match (original_exists, quarantine_exists) {
                    (true, false) => self.inner.store.move_managed_directory(
                        root,
                        &saga.original_relative_path,
                        &saga.quarantine_relative_path,
                    )?,
                    (false, true) => {}
                    _ => return Err(quarantine_path_conflict().into()),
                }
                let bundle = self
                    .inner
                    .repository
                    .pipeline_artifact_bundle(&saga.entry_id)?
                    .ok_or_else(|| {
                        DownloadPipelineError::new(
                            DownloadPipelineErrorCode::ArtifactMissing,
                            "The pending quarantine artifact no longer exists",
                            false,
                        )
                    })?;
                let manifest = ArtifactManifest::from_bundle(&bundle)?;
                write_quarantine_manifest(
                    &self.inner,
                    root,
                    &saga.quarantine_relative_path,
                    &saga.quarantine_relative_path,
                    manifest,
                    true,
                )?;
                Ok(self
                    .inner
                    .repository
                    .pipeline_quarantine_complete(&saga.record_id)?)
            }
            QuarantineSagaState::PendingRestore => {
                match (original_exists, quarantine_exists) {
                    (false, true) => {
                        let layout = layout_for_directory(
                            root.to_path_buf(),
                            saga.quarantine_relative_path.clone(),
                        )?;
                        let manifest =
                            self.inner.store.read_manifest(&layout)?.ok_or_else(|| {
                                DownloadPipelineError::new(
                                    DownloadPipelineErrorCode::ManifestInvalid,
                                    "The pending restore manifest is missing",
                                    false,
                                )
                            })?;
                        write_quarantine_manifest(
                            &self.inner,
                            root,
                            &saga.quarantine_relative_path,
                            &saga.original_relative_path,
                            manifest,
                            false,
                        )?;
                        self.inner.store.move_managed_directory(
                            root,
                            &saga.quarantine_relative_path,
                            &saga.original_relative_path,
                        )?;
                    }
                    (true, false) => {}
                    _ => return Err(quarantine_path_conflict().into()),
                }
                Ok(self
                    .inner
                    .repository
                    .pipeline_restore_complete(&saga.record_id)?)
            }
            QuarantineSagaState::Quarantined | QuarantineSagaState::Restored => {
                Err(quarantine_path_conflict().into())
            }
        }
    }

    pub fn overlap_review_get(
        &self,
        review_id: &str,
    ) -> Result<Option<DownloadOverlapReview>, ApplicationError> {
        let review_id = review_id.trim();
        if review_id.is_empty() || review_id.len() > 200 {
            return Err(crate::domain::ValidationError::new(
                "reviewId",
                "must contain between 1 and 200 bytes",
            )
            .into());
        }
        self.inner
            .repository
            .overlap_review_get(review_id)
            .map_err(Into::into)
    }

    pub fn overlap_decision_apply(
        &self,
        mut request: DownloadOverlapDecisionRequest,
    ) -> Result<DownloadOverlapDecisionResult, ApplicationError> {
        let _decision_guard = unpoison(self.inner.overlap_decisions.lock());
        request.review_id = request.review_id.trim().to_owned();
        if let Some(candidate_id) = request.candidate_id.as_mut() {
            *candidate_id = candidate_id.trim().to_owned();
            if candidate_id.is_empty() {
                request.candidate_id = None;
            }
        }
        normalize_overlap_decision_audit(&mut request)?;
        let review = self
            .overlap_review_get(&request.review_id)?
            .ok_or_else(|| {
                ApplicationError::DownloadOverlapReviewNotFound(request.review_id.clone())
            })?;
        if review.revision != request.expected_revision {
            return Err(ApplicationError::RevisionConflict {
                resource: "downloadOverlapReview",
                expected: request.expected_revision,
                actual: review.revision,
            });
        }
        if request.actor == DownloadOverlapDecisionActor::Automation {
            validate_strict_overlap_automatic_decision(&review, &request)?;
        }
        if request.action != DownloadOverlapDecisionAction::RemoveIncoming
            && request.candidate_id.is_none()
        {
            let unresolved = review
                .candidates
                .iter()
                .filter(|candidate| candidate.decision.is_none())
                .collect::<Vec<_>>();
            if let [candidate] = unresolved.as_slice() {
                request.candidate_id = Some(candidate.candidate_id.clone());
            }
        }
        let (incoming_fingerprint, existing_fingerprints) =
            self.verify_overlap_review_fingerprints(&review, &request)?;
        let fingerprints_changed = incoming_fingerprint != review.incoming_fingerprint
            || review
                .candidates
                .iter()
                .filter(|candidate| candidate.decision.is_none())
                .any(|candidate| {
                    existing_fingerprints
                        .iter()
                        .find(|(candidate_id, _)| candidate_id == &candidate.candidate_id)
                        .is_none_or(|(_, fingerprint)| {
                            fingerprint != &candidate.existing_fingerprint
                        })
                });
        let mut quarantined_existing = None;
        let outcome_result = if fingerprints_changed {
            self.inner
                .repository
                .overlap_review_requeue_stale(&review.review_id, review.revision)
        } else {
            if request.action == DownloadOverlapDecisionAction::RemoveExistingContinue {
                let unresolved = review
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.decision.is_none())
                    .collect::<Vec<_>>();
                let selected = match (request.candidate_id.as_deref(), unresolved.as_slice()) {
                    (Some(candidate_id), _) => unresolved
                        .iter()
                        .find(|candidate| candidate.candidate_id == candidate_id)
                        .copied(),
                    (None, [candidate]) => Some(*candidate),
                    _ => None,
                }
                .ok_or_else(|| {
                    ApplicationError::DownloadOverlapDecisionInvalid(
                        "Select one unresolved candidate before removing existing album A".into(),
                    )
                })?;
                let entry_id = selected.existing.entry_id.clone();
                let existing_bundle = self
                    .inner
                    .repository
                    .pipeline_artifact_bundle(&DownloadEntryId::new(entry_id.clone())?)?
                    .ok_or_else(|| {
                        ApplicationError::DownloadOverlapDecisionInvalid(
                            "기존 앨범 A의 파일 상태가 변경되었습니다. 목록을 새로고침한 뒤 다시 시도해 주세요."
                                .into(),
                        )
                    })?;
                match existing_bundle.artifact.state {
                    DownloadArtifactState::Complete => {
                        self.quarantine_entries(
                            vec![entry_id.clone()],
                            format!(
                                "Removed existing album A during overlap review {}",
                                review.review_id
                            ),
                        )?;
                        quarantined_existing = Some(entry_id);
                    }
                    DownloadArtifactState::Incomplete => {
                        // A chained overlap candidate is another fully verified staging
                        // download. The repository cancels only that staging review in
                        // the same transaction as the current candidate decision.
                    }
                    DownloadArtifactState::Quarantined => {
                        // A different overlap review may already have removed this
                        // candidate. The repository records the same terminal pair
                        // decision without trying to move the artifact again.
                    }
                    DownloadArtifactState::MissingArtifacts => {
                        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
                            "기존 앨범 A의 파일 상태가 변경되었습니다. 목록을 새로고침한 뒤 다시 시도해 주세요."
                                .into(),
                        ));
                    }
                }
            }
            self.inner.repository.overlap_decision_apply(
                &request,
                &incoming_fingerprint,
                &existing_fingerprints,
            )
        };
        let outcome = match outcome_result {
            Ok(outcome) => outcome,
            Err(error) => {
                if let Some(entry_id) = quarantined_existing.take() {
                    if let Err(restore_error) = self.restore_entries(vec![entry_id.clone()]) {
                        tracing::error!(
                            entry_id,
                            error = %restore_error,
                            "overlap decision failed after quarantine and automatic restore also failed"
                        );
                        return Err(restore_error);
                    }
                }
                return Err(error.into());
            }
        };
        if quarantined_existing.is_some()
            && !matches!(&outcome, DownloadOverlapDecisionApplyOutcome::Applied(_))
        {
            self.restore_entries(vec![quarantined_existing
                .take()
                .expect("checked existing quarantine")])?;
        }
        match outcome {
            DownloadOverlapDecisionApplyOutcome::Applied(applied) => {
                if let Some(projection) = applied.removed_existing_projection {
                    emit(&self.inner, projection);
                }
                if let Some(projection) = applied.projection {
                    emit(&self.inner, projection);
                }
                if let Some(descriptor) = applied.resume {
                    self.enqueue(descriptor)?;
                }
                Ok(applied.result)
            }
            DownloadOverlapDecisionApplyOutcome::ReviewNotFound => Err(
                ApplicationError::DownloadOverlapReviewNotFound(request.review_id),
            ),
            DownloadOverlapDecisionApplyOutcome::RevisionConflict { actual_revision } => {
                Err(ApplicationError::RevisionConflict {
                    resource: "downloadOverlapReview",
                    expected: request.expected_revision,
                    actual: actual_revision,
                })
            }
            DownloadOverlapDecisionApplyOutcome::InvalidCandidate => {
                Err(ApplicationError::DownloadOverlapDecisionInvalid(
                    "Select one unresolved candidate before applying a candidate decision".into(),
                ))
            }
        }
    }

    fn verify_overlap_review_fingerprints(
        &self,
        review: &DownloadOverlapReview,
        request: &DownloadOverlapDecisionRequest,
    ) -> Result<(String, Vec<(String, String)>), ApplicationError> {
        let profile = HashProfile::current();
        let incoming_entry = DownloadEntryId::new(review.entry_id.clone())?;
        let incoming = self
            .inner
            .repository
            .pipeline_artifact_bundle(&incoming_entry)?
            .ok_or_else(|| {
                ApplicationError::DownloadOverlapDecisionInvalid(
                    "The incoming staging artifact is no longer available".into(),
                )
            })?;
        let incoming_layout = overlap_layout(&self.inner, &incoming)?;
        verify_bundle_files(&self.inner, &incoming_layout, &incoming)
            .map_err(|_| ApplicationError::DownloadPipeline(overlap_check_failed()))?;
        let incoming_fingerprint = overlap_artifact_fingerprint(&incoming, profile.profile_version)
            .ok_or_else(|| {
                ApplicationError::DownloadOverlapDecisionInvalid(
                    "The incoming staging fingerprint could not be reproduced".into(),
                )
            })?;
        let mut existing = Vec::new();
        for candidate in review
            .candidates
            .iter()
            .filter(|candidate| candidate.decision.is_none())
        {
            let entry_id = DownloadEntryId::new(candidate.existing.entry_id.clone())?;
            let bundle = self
                .inner
                .repository
                .pipeline_artifact_bundle(&entry_id)?
                .ok_or_else(|| {
                    ApplicationError::DownloadOverlapDecisionInvalid(
                        "An existing comparison artifact is no longer available".into(),
                    )
                })?;
            if bundle.artifact.state == DownloadArtifactState::Quarantined {
                if request.candidate_id.as_deref() == Some(candidate.candidate_id.as_str())
                    && request.action != DownloadOverlapDecisionAction::RemoveExistingContinue
                {
                    return Err(ApplicationError::DownloadOverlapDecisionInvalid(
                        "기존 앨범 A가 이미 격리되었습니다. 목록을 새로고침한 뒤 다시 확인해 주세요."
                            .into(),
                    ));
                }
                existing.push((
                    candidate.candidate_id.clone(),
                    candidate.existing_fingerprint.clone(),
                ));
                continue;
            }
            let layout = overlap_layout(&self.inner, &bundle)?;
            verify_bundle_files(&self.inner, &layout, &bundle)
                .map_err(|_| ApplicationError::DownloadPipeline(overlap_check_failed()))?;
            let fingerprint = overlap_artifact_fingerprint(&bundle, profile.profile_version)
                .ok_or_else(|| {
                    ApplicationError::DownloadOverlapDecisionInvalid(
                        "An existing comparison fingerprint could not be reproduced".into(),
                    )
                })?;
            existing.push((candidate.candidate_id.clone(), fingerprint));
        }
        Ok((incoming_fingerprint, existing))
    }

    pub fn shutdown_and_wait(&self) {
        if self.inner.shutting_down.swap(true, Ordering::AcqRel) {
            return;
        }
        {
            let mut queue = unpoison(self.inner.queue.lock());
            queue.closed = true;
            queue.pending.clear();
        }
        {
            let cancellations = unpoison(self.inner.cancellations.lock());
            for active in cancellations.values() {
                active.token.cancel();
            }
        }
        self.inner.wake.notify_all();
        let workers = {
            let mut workers = unpoison(self.inner.workers.lock());
            std::mem::take(&mut *workers)
        };
        for worker in workers {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }

    #[cfg(test)]
    pub fn is_shutting_down(&self) -> bool {
        self.inner.shutting_down.load(Ordering::Acquire)
    }
}

fn layout_for_bundle(
    root: PathBuf,
    bundle: &crate::domain::ArtifactBundle,
) -> Result<ArtifactLayout, ApplicationError> {
    let manifest_relative_path =
        bundle
            .artifact
            .manifest_relative_path
            .clone()
            .unwrap_or(ArtifactRelativePath::new(format!(
                "{}/manifest.json",
                bundle.artifact.relative_directory.as_str()
            ))?);
    Ok(ArtifactLayout {
        root,
        relative_directory: bundle.artifact.relative_directory.clone(),
        manifest_relative_path,
    })
}

fn layout_for_directory(
    root: PathBuf,
    relative_directory: ArtifactRelativePath,
) -> Result<ArtifactLayout, ApplicationError> {
    let manifest_relative_path =
        ArtifactRelativePath::new(format!("{}/manifest.json", relative_directory.as_str()))?;
    Ok(ArtifactLayout {
        root,
        relative_directory,
        manifest_relative_path,
    })
}

fn write_quarantine_manifest(
    inner: &SupervisorInner,
    root: &std::path::Path,
    storage_directory: &ArtifactRelativePath,
    target_page_directory: &ArtifactRelativePath,
    mut manifest: ArtifactManifest,
    quarantined: bool,
) -> Result<(), ApplicationError> {
    for page in &mut manifest.pages {
        let file_name = std::path::Path::new(&page.relative_path)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "A manifest page path is invalid",
                    false,
                )
            })?;
        page.relative_path = format!("{}/{file_name}", target_page_directory.as_str());
        page.quarantined = quarantined;
    }
    let layout = layout_for_directory(root.to_path_buf(), storage_directory.clone())?;
    inner.store.write_manifest(&layout, &manifest)?;
    Ok(())
}

fn download_entry_from_projection(
    projection: &DownloadJobProjection,
) -> Result<DownloadEntry, ApplicationError> {
    Ok(DownloadEntry {
        entry_id: DownloadEntryId::new(projection.download.entry_id.clone())?,
        gallery_id: crate::domain::GalleryId::new(projection.download.gallery_id)?,
        revision: projection.download.revision,
        state: projection.download.state,
        progress: projection.download.progress,
        attempt: projection.download.attempt,
        error_code: projection.download.error_code.clone(),
        error_message: projection.download.error_message.clone(),
        error_retryable: None,
        review_kind: None,
        review_id: None,
        created_at: None,
        updated_at: None,
    })
}

fn inspect_bundle(
    inner: &SupervisorInner,
    root: &std::path::Path,
    bundle: &crate::domain::ArtifactBundle,
) -> Vec<(String, String)> {
    let layout = match layout_for_bundle(root.to_path_buf(), bundle) {
        Ok(layout) => layout,
        Err(error) => return vec![stable_application_issue_owned(&error)],
    };
    let mut issues = Vec::new();
    for page in &bundle.pages {
        let expected = page_verification(page);
        if page.state == PageArtifactState::Present && expected.is_none() {
            issues.push((
                "ARTIFACT_MANIFEST_INVALID".into(),
                "A present page is missing verification metadata".into(),
            ));
            continue;
        }
        let Some(expected) = expected.as_ref() else {
            continue;
        };
        match inner.store.verify_existing_page(
            &layout,
            page.page_id.source_page_number,
            &expected.source_revision,
            Some(expected),
        ) {
            Ok(ExistingPageVerification::Verified(_)) => {}
            Ok(ExistingPageVerification::Missing) => issues.push((
                "FILESYSTEM_MISSING".into(),
                "A verified page is missing from disk".into(),
            )),
            Ok(ExistingPageVerification::Invalid { .. }) => issues.push((
                "ARTIFACT_HASH_MISMATCH".into(),
                "A page no longer matches its verified digest".into(),
            )),
            Err(error) => issues.push((error.code.as_str().into(), error.message)),
        }
    }
    if bundle.artifact.state == DownloadArtifactState::Complete {
        match ArtifactManifest::from_bundle(bundle) {
            Ok(expected_manifest) => match inner.store.read_manifest(&layout) {
                Ok(Some(actual)) if actual == expected_manifest => {}
                Ok(Some(_)) => issues.push((
                    "ARTIFACT_MANIFEST_INVALID".into(),
                    "The artifact manifest does not match the database snapshot".into(),
                )),
                Ok(None) => issues.push((
                    "FILESYSTEM_MISSING".into(),
                    "The completed artifact manifest is missing".into(),
                )),
                Err(error) => issues.push((error.code.as_str().into(), error.message)),
            },
            Err(_) => issues.push((
                "ARTIFACT_MANIFEST_INVALID".into(),
                "The database artifact cannot produce a valid manifest".into(),
            )),
        }
    }
    issues
}

fn stable_application_issue(error: &ApplicationError) -> (&str, &str) {
    match error {
        ApplicationError::DownloadPipeline(error) => (error.code.as_str(), &error.message),
        ApplicationError::Repository(_) => (
            "DATABASE_ERROR",
            "The recovery state could not be updated safely",
        ),
        _ => (
            "QUARANTINE_CONFLICT",
            "The quarantine operation could not be reconciled safely",
        ),
    }
}

fn stable_application_issue_owned(error: &ApplicationError) -> (String, String) {
    let (code, message) = stable_application_issue(error);
    (code.to_owned(), message.to_owned())
}

fn quarantine_path_conflict() -> DownloadPipelineError {
    DownloadPipelineError::new(
        DownloadPipelineErrorCode::QuarantineConflict,
        "The original and quarantine paths are in an ambiguous state; no file was deleted",
        false,
    )
}

fn worker_loop(inner: Arc<SupervisorInner>) {
    loop {
        let descriptor = {
            let mut queue = unpoison(inner.queue.lock());
            loop {
                if let Some(descriptor) = queue.pending.pop_front() {
                    break Some(descriptor);
                }
                if queue.closed {
                    break None;
                }
                queue = unpoison(inner.wake.wait(queue));
            }
        };
        let Some(descriptor) = descriptor else {
            break;
        };
        let key = descriptor_key(&descriptor);
        let cancellation = CancellationToken::new();
        unpoison(inner.cancellations.lock()).insert(
            key.clone(),
            ActiveCancellation {
                entry_id: descriptor.entry_id.clone(),
                token: cancellation.clone(),
            },
        );
        if let Err(error) = run_download(&inner, &descriptor, &cancellation) {
            handle_download_error(&inner, &descriptor, &cancellation, error);
        }
        unpoison(inner.cancellations.lock()).remove(&key);
        unpoison(inner.queue.lock()).known.remove(&key);
    }
}

fn run_download(
    inner: &SupervisorInner,
    descriptor: &DownloadJobDescriptor,
    cancellation: &CancellationToken,
) -> Result<(), RunError> {
    emit(inner, inner.repository.pipeline_begin(descriptor)?);
    check_cancelled(cancellation)?;
    let settings = inner.settings.settings_get()?;
    if settings.download_root.trim().is_empty() {
        return Err(DownloadPipelineError::root_required().into());
    }

    let snapshot = inner
        .source
        .gallery_snapshot(descriptor.gallery_id, cancellation)?;
    check_cancelled(cancellation)?;
    let root = PathBuf::from(settings.download_root);
    let planned_relative_directory =
        plan_artifact_relative_directory(&settings.folder_name_template, &snapshot.gallery)
            .map_err(|error| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::PathOutsideRoot,
                    format!("The configured artifact folder template is invalid: {error}"),
                    false,
                )
            })?;
    let planned_manifest_relative_path = ArtifactRelativePath::new(format!(
        "{}/manifest.json",
        planned_relative_directory.as_str()
    ))
    .map_err(|error| {
        DownloadPipelineError::new(
            DownloadPipelineErrorCode::PathOutsideRoot,
            format!("The planned manifest path is invalid: {error}"),
            false,
        )
    })?;
    let prepared = inner.repository.pipeline_prepare(&DownloadArtifactPlan {
        descriptor: descriptor.clone(),
        gallery: snapshot.gallery,
        source_revision: snapshot.source_revision,
        root_snapshot: root.clone(),
        relative_directory: planned_relative_directory,
        manifest_relative_path: planned_manifest_relative_path,
        source_pages: snapshot.pages.clone(),
    })?;
    // A pre-existing row is the durable DB reservation for this immutable destination.
    // Files inside it are still verified against checkpoints or moved to recovery review.
    let allow_existing_directory = !prepared.artifact_created;
    let layout = inner.store.prepare_layout(
        &prepared.root_snapshot,
        &prepared.relative_directory,
        allow_existing_directory,
    )?;
    if layout.manifest_relative_path != prepared.manifest_relative_path {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::ManifestInvalid,
            "The persisted artifact manifest path does not match its immutable directory",
            false,
        )
        .into());
    }
    emit(inner, prepared.projection);
    let checkpoints = prepared
        .checkpoints
        .into_iter()
        .map(|checkpoint| (checkpoint.page.source_page_number, checkpoint))
        .collect::<BTreeMap<_, _>>();

    for source_page in &snapshot.pages {
        check_cancelled(cancellation)?;
        let checkpoint = checkpoints.get(&source_page.source_page_number);
        let existing = inner.store.verify_existing_page(
            &layout,
            source_page.source_page_number,
            &source_page.source_revision,
            checkpoint.map(|checkpoint| &checkpoint.page),
        )?;
        match existing {
            ExistingPageVerification::Verified(page) => {
                if checkpoint.is_none() {
                    emit(
                        inner,
                        inner.repository.pipeline_page_verified(descriptor, &page)?,
                    );
                }
                continue;
            }
            ExistingPageVerification::Invalid { .. } => {
                if let Some(projection) = inner.repository.pipeline_mark_artifact_issue(
                    &DownloadEntryId::new(descriptor.entry_id.clone()).map_err(|_| {
                        DownloadPipelineError::new(
                            DownloadPipelineErrorCode::ManifestInvalid,
                            "The download entry identity is invalid",
                            false,
                        )
                    })?,
                    "RECOVERY_CONFLICT",
                    "Ambiguous page files were moved aside for review",
                )? {
                    emit(inner, projection);
                }
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::HashMismatch,
                    "A stored page does not match its verified checkpoint",
                    false,
                )
                .into());
            }
            ExistingPageVerification::Missing => {}
        }

        let payload = match inner.source.download_page(
            descriptor.gallery_id,
            source_page.source_page_number,
            cancellation,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                let diagnostics = if error.candidate_diagnostics.is_empty() {
                    vec![SourceCandidateDiagnostic {
                        candidate_index: 0,
                        format: "unknown".into(),
                        http_status: error.http_status,
                        content_type: None,
                        bytes_received: None,
                        error_code: Some(error.code),
                        retryable: error.retryable,
                    }]
                } else {
                    error.candidate_diagnostics.clone()
                };
                persist_candidate_diagnostics(
                    inner,
                    descriptor,
                    source_page.source_page_number,
                    &diagnostics,
                )?;
                return Err(error.into());
            }
        };
        let diagnostics = if payload.candidate_diagnostics.is_empty() {
            vec![SourceCandidateDiagnostic {
                candidate_index: payload.candidate_index,
                format: payload.source_format.as_str().to_owned(),
                http_status: None,
                content_type: None,
                bytes_received: u64::try_from(payload.bytes.len()).ok(),
                error_code: None,
                retryable: false,
            }]
        } else {
            payload.candidate_diagnostics.clone()
        };
        persist_candidate_diagnostics(
            inner,
            descriptor,
            source_page.source_page_number,
            &diagnostics,
        )?;
        if payload.source_page_number != source_page.source_page_number
            || payload.source_revision != source_page.source_revision
        {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ManifestInvalid,
                "The downloaded page identity does not match the immutable source mapping",
                false,
            )
            .into());
        }
        let stored = inner.store.store_page(&layout, &payload, cancellation)?;
        emit(
            inner,
            inner
                .repository
                .pipeline_page_verified(descriptor, &stored)?,
        );
    }

    emit(
        inner,
        inner
            .repository
            .pipeline_stage(descriptor, JobState::Hashing, "Rechecking page hashes")?,
    );
    check_cancelled(cancellation)?;
    let mut bundle = inner
        .repository
        .pipeline_artifact_bundle(
            &DownloadEntryId::new(descriptor.entry_id.clone())
                .map_err(|error| RepositoryError::Other(error.to_string()))?,
        )?
        .ok_or_else(|| RepositoryError::Corrupt("prepared artifact is missing".into()))?;
    verify_bundle_files(inner, &layout, &bundle)?;

    let artist_keys = normalized_artist_keys(&bundle.gallery.metadata.artists);
    let finalization_locks = {
        let mut locks = unpoison(inner.finalization_locks.lock());
        artist_keys
            .iter()
            .map(|artist| {
                Arc::clone(
                    locks
                        .entry(artist.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(()))),
                )
            })
            .collect::<Vec<_>>()
    };
    let _finalization_guards = finalization_locks
        .iter()
        .map(|lock| unpoison(lock.lock()))
        .collect::<Vec<_>>();
    check_cancelled(cancellation)?;
    if let Some(projection) =
        run_overlap_review_gate(inner, descriptor, &layout, &bundle, cancellation)?
    {
        emit(inner, projection);
        return Ok(());
    }

    emit(
        inner,
        inner.repository.pipeline_stage(
            descriptor,
            JobState::Verifying,
            "Writing and verifying the artifact manifest",
        )?,
    );
    check_cancelled(cancellation)?;
    let completed_at = now_unix_ms();
    bundle.artifact.state = DownloadArtifactState::Complete;
    bundle.artifact = bundle
        .artifact
        .with_manifest(
            layout.manifest_relative_path.clone(),
            ARTIFACT_MANIFEST_SCHEMA_VERSION,
            env!("CARGO_PKG_VERSION"),
            HASH_PROFILE_VERSION,
            completed_at,
        )
        .map_err(|error| RepositoryError::Other(error.to_string()))?;
    let manifest = ArtifactManifest::from_bundle(&bundle)
        .map_err(|error| RepositoryError::Other(error.to_string()))?;
    inner.store.write_manifest(&layout, &manifest)?;
    let persisted = inner.store.read_manifest(&layout)?.ok_or_else(|| {
        DownloadPipelineError::new(
            DownloadPipelineErrorCode::ManifestInvalid,
            "The artifact manifest disappeared before completion",
            false,
        )
    })?;
    if persisted != manifest {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::ManifestInvalid,
            "The persisted artifact manifest does not match the verified snapshot",
            false,
        )
        .into());
    }
    emit(
        inner,
        inner.repository.pipeline_complete(
            descriptor,
            &manifest,
            &layout.manifest_relative_path,
        )?,
    );
    Ok(())
}

fn run_overlap_review_gate(
    inner: &SupervisorInner,
    descriptor: &DownloadJobDescriptor,
    incoming_layout: &ArtifactLayout,
    incoming_bundle: &crate::domain::ArtifactBundle,
    cancellation: &CancellationToken,
) -> Result<Option<DownloadJobProjection>, DownloadPipelineError> {
    let incoming_artists = &incoming_bundle.gallery.metadata.artists;
    if normalized_artist_keys(incoming_artists).is_empty() {
        tracing::info!(
            entry_id = descriptor.entry_id,
            gallery_id = descriptor.gallery_id.get(),
            "download overlap gate skipped because the incoming gallery has no artist metadata"
        );
        return Ok(None);
    }
    verify_bundle_files(inner, incoming_layout, incoming_bundle)
        .map_err(|_| overlap_check_failed())?;
    let profile = HashProfile::current();
    let incoming_fingerprint =
        overlap_artifact_fingerprint(incoming_bundle, profile.profile_version)
            .ok_or_else(overlap_check_failed)?;
    let incoming_hashed = prepare_overlap_hashes(inner, incoming_bundle, &profile, cancellation)?;
    let identities = inner
        .repository
        .overlap_candidate_identities(&incoming_bundle.artifact.entry_id)
        .map_err(|_| overlap_check_failed())?;
    let review_id = format!("download-overlap-{}", Uuid::new_v4());
    let mut candidates = Vec::new();

    for identity in identities {
        check_cancelled(cancellation).map_err(|_| DownloadPipelineError::cancelled())?;
        if !overlap_artists_intersect(incoming_artists, &identity.artists) {
            continue;
        }
        let existing_bundle = inner
            .repository
            .pipeline_artifact_bundle(&identity.entry_id)
            .map_err(|_| overlap_check_failed())?
            .ok_or_else(overlap_check_failed)?;
        if verified_overlap_pages(&existing_bundle).is_none() {
            return Err(overlap_check_failed());
        }
        let existing_layout = overlap_layout(inner, &existing_bundle)?;
        verify_bundle_files(inner, &existing_layout, &existing_bundle)
            .map_err(|_| overlap_check_failed())?;
        let existing_fingerprint =
            overlap_artifact_fingerprint(&existing_bundle, profile.profile_version)
                .ok_or_else(overlap_check_failed)?;
        if inner
            .repository
            .overlap_pair_policy_exists(
                &incoming_fingerprint,
                &existing_fingerprint,
                profile.profile_version,
                DOWNLOAD_OVERLAP_POLICY_VERSION,
            )
            .map_err(|_| overlap_check_failed())?
        {
            continue;
        }
        let existing_hashed =
            prepare_overlap_hashes(inner, &existing_bundle, &profile, cancellation)?;
        if let Some(mut candidate) = analyze_download_overlap_pair(
            &review_id,
            &incoming_hashed,
            &existing_hashed,
            existing_fingerprint,
            &profile,
        ) {
            candidate.existing = overlap_gallery_ref(&existing_bundle);
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        tracing::info!(
            entry_id = descriptor.entry_id,
            gallery_id = descriptor.gallery_id.get(),
            "download overlap gate completed without a blocking candidate"
        );
        return Ok(None);
    }
    candidates.sort_by(|left, right| {
        overlap_relation_severity(left.relation)
            .cmp(&overlap_relation_severity(right.relation))
            .then_with(|| {
                right
                    .existing_coverage
                    .max(right.incoming_coverage)
                    .total_cmp(&left.existing_coverage.max(left.incoming_coverage))
            })
            .then_with(|| right.matched_pages.cmp(&left.matched_pages))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.existing.gallery_id.cmp(&right.existing.gallery_id))
    });
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
    }
    let draft = DownloadOverlapReviewDraft {
        review_id: review_id.clone(),
        entry_id: incoming_bundle.artifact.entry_id.clone(),
        incoming: overlap_gallery_ref(incoming_bundle),
        profile_version: profile.profile_version,
        policy_version: DOWNLOAD_OVERLAP_POLICY_VERSION,
        incoming_fingerprint,
        candidates,
    };
    let projection = inner
        .repository
        .overlap_review_pause(descriptor, &draft)
        .map_err(|_| overlap_check_failed())?;
    tracing::info!(
        entry_id = descriptor.entry_id,
        gallery_id = descriptor.gallery_id.get(),
        review_id,
        candidate_count = draft.candidates.len(),
        "download paused before manifest creation for edition overlap review"
    );
    Ok(Some(projection))
}

fn overlap_layout(
    inner: &SupervisorInner,
    bundle: &crate::domain::ArtifactBundle,
) -> Result<ArtifactLayout, DownloadPipelineError> {
    let root = inner
        .repository
        .pipeline_artifact_root(&bundle.artifact.entry_id)
        .map_err(|_| overlap_check_failed())?;
    let layout = inner
        .store
        .prepare_layout(&root, &bundle.artifact.relative_directory, true)?;
    if bundle
        .artifact
        .manifest_relative_path
        .as_ref()
        .is_some_and(|path| path != &layout.manifest_relative_path)
    {
        return Err(overlap_check_failed());
    }
    Ok(layout)
}

fn prepare_overlap_hashes(
    inner: &SupervisorInner,
    bundle: &crate::domain::ArtifactBundle,
    profile: &HashProfile,
    cancellation: &CancellationToken,
) -> Result<super::duplicate_analyzer::HashedArtifact, DownloadPipelineError> {
    let pages = verified_overlap_pages(bundle).ok_or_else(overlap_check_failed)?;
    let root = inner
        .repository
        .pipeline_artifact_root(&bundle.artifact.entry_id)
        .map_err(|_| overlap_check_failed())?;
    let mut hashes = Vec::with_capacity(pages.len());
    for page in pages {
        check_cancelled(cancellation).map_err(|_| DownloadPipelineError::cancelled())?;
        let sha = page.sha256.as_ref().ok_or_else(overlap_check_failed)?;
        if let Some(cached) = inner
            .repository
            .overlap_page_hash_get(
                bundle.artifact.entry_id.as_str(),
                page.page_id.source_page_number,
                profile.profile_version,
                sha.as_str(),
            )
            .map_err(|_| overlap_check_failed())?
        {
            hashes.push(cached);
            continue;
        }
        let bytes = inner
            .store
            .read_verified_page_bytes(&root, page)
            .map_err(|_| overlap_check_failed())?;
        let hash = compute_page_hash(
            bundle.artifact.entry_id.as_str(),
            bundle.gallery.id,
            page.page_id.source_page_number,
            sha.clone(),
            &bytes,
            profile,
        )
        .map_err(|_| overlap_check_failed())?;
        inner
            .repository
            .overlap_page_hash_upsert(&hash)
            .map_err(|_| overlap_check_failed())?;
        hashes.push(hash);
    }
    Ok(hashed_artifact(
        DuplicateGalleryRef {
            gallery_id: bundle.gallery.id,
            entry_id: bundle.artifact.entry_id.to_string(),
            title: bundle.gallery.metadata.title.clone(),
            artist: bundle.gallery.metadata.primary_artist.clone(),
            group: bundle.gallery.metadata.primary_group.clone(),
            page_count: u32::try_from(hashes.len()).unwrap_or(u32::MAX),
        },
        hashes,
    ))
}

fn overlap_relation_severity(relation: crate::domain::DownloadOverlapRelation) -> u8 {
    match relation {
        crate::domain::DownloadOverlapRelation::NearEquivalent => 0,
        crate::domain::DownloadOverlapRelation::IncomingContainsExisting
        | crate::domain::DownloadOverlapRelation::ExistingContainsIncoming => 1,
        crate::domain::DownloadOverlapRelation::TranslationEdition => 2,
        crate::domain::DownloadOverlapRelation::PartialOverlap => 3,
    }
}

fn normalize_overlap_decision_audit(
    request: &mut DownloadOverlapDecisionRequest,
) -> Result<(), ApplicationError> {
    if request.actor == DownloadOverlapDecisionActor::Human {
        request.reason_code = None;
        request.rule_version = None;
        request.feature_snapshot_json = None;
        return Ok(());
    }

    if !matches!(
        request.action,
        DownloadOverlapDecisionAction::RemoveExistingContinue
            | DownloadOverlapDecisionAction::RemoveIncoming
    ) {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "Automatic overlap decisions may only quarantine one side of a verified overlap match"
                .into(),
        ));
    }
    if request.candidate_id.is_none() {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "Automatic overlap decisions must identify the exact compared candidate".into(),
        ));
    }

    let reason = request
        .reason_code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApplicationError::DownloadOverlapDecisionInvalid(
                "Automatic overlap decisions require a reason code".into(),
            )
        })?;
    if reason != "balanced_overlap_v2" {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "The requested automatic overlap rule is not supported by this build".into(),
        ));
    }
    request.reason_code = Some(reason.to_owned());
    if request.rule_version != Some(2) {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "The requested automatic overlap rule version is not supported by this build".into(),
        ));
    }

    let snapshot = request
        .feature_snapshot_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApplicationError::DownloadOverlapDecisionInvalid(
                "Automatic overlap decisions require an auditable feature snapshot".into(),
            )
        })?;
    if snapshot.len() > 65_536 {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "The automatic overlap feature snapshot is too large".into(),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(snapshot).map_err(|_| {
        ApplicationError::DownloadOverlapDecisionInvalid(
            "The automatic overlap feature snapshot is not valid JSON".into(),
        )
    })?;
    if !parsed.is_object() {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "The automatic overlap feature snapshot must be a JSON object".into(),
        ));
    }
    let expected_candidate = request.candidate_id.as_deref().unwrap_or_default();
    if parsed.get("rule").and_then(serde_json::Value::as_str) != Some("balanced_overlap_v2")
        || parsed
            .get("ruleVersion")
            .and_then(serde_json::Value::as_u64)
            != Some(2)
        || parsed.get("reviewId").and_then(serde_json::Value::as_str)
            != Some(request.review_id.as_str())
        || parsed
            .get("reviewRevision")
            .and_then(serde_json::Value::as_u64)
            != Some(request.expected_revision)
        || parsed
            .get("candidateId")
            .and_then(serde_json::Value::as_str)
            != Some(expected_candidate)
    {
        return Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "The automatic overlap feature snapshot does not identify this review revision and candidate"
                .into(),
        ));
    }
    request.feature_snapshot_json = Some(serde_json::to_string(&parsed).map_err(|_| {
        ApplicationError::DownloadOverlapDecisionInvalid(
            "The automatic overlap feature snapshot could not be normalized".into(),
        )
    })?);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StrictOverlapWinner {
    Incoming,
    Existing,
}

fn overlap_edition_preference(title: &str) -> i8 {
    let normalized = title
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if [
        "uncensored",
        "decensored",
        "uncen",
        "無修正",
        "无修正",
        "無碼",
        "无码",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        1
    } else if ["censored", "mosaic", "モザイク", "修正版"]
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        -1
    } else {
        0
    }
}

fn strict_overlap_winner(
    incoming: &crate::domain::DownloadOverlapGalleryRef,
    candidate: &crate::domain::DownloadOverlapCandidate,
) -> Option<StrictOverlapWinner> {
    let incoming_page_count = incoming.page_count;
    let existing_page_count = candidate.existing.page_count;
    let incoming_preference = overlap_edition_preference(&incoming.title);
    let existing_preference = overlap_edition_preference(&candidate.existing.title);
    let (winner, loser_coverage) = match candidate.relation {
        crate::domain::DownloadOverlapRelation::IncomingContainsExisting => {
            if incoming_preference < existing_preference {
                return None;
            }
            (StrictOverlapWinner::Incoming, candidate.existing_coverage)
        }
        crate::domain::DownloadOverlapRelation::ExistingContainsIncoming => {
            if existing_preference < incoming_preference {
                return None;
            }
            (StrictOverlapWinner::Existing, candidate.incoming_coverage)
        }
        crate::domain::DownloadOverlapRelation::NearEquivalent => {
            let winner = if incoming_preference != existing_preference {
                if incoming_preference > existing_preference {
                    StrictOverlapWinner::Incoming
                } else {
                    StrictOverlapWinner::Existing
                }
            } else if incoming_page_count != existing_page_count {
                if incoming_page_count > existing_page_count {
                    StrictOverlapWinner::Incoming
                } else {
                    StrictOverlapWinner::Existing
                }
            } else {
                StrictOverlapWinner::Existing
            };
            (
                winner,
                candidate.existing_coverage.min(candidate.incoming_coverage),
            )
        }
        _ => return None,
    };
    if candidate.matched_pages == 0 {
        return None;
    }
    let matched = f64::from(candidate.matched_pages);
    let aligned_run_ratio = f64::from(candidate.longest_aligned_run) / matched;
    let informative_matches = candidate
        .page_pairs
        .iter()
        .filter(|pair| !pair.low_information)
        .count();
    let informative_match_ratio = informative_matches as f64 / matched;
    let smaller_page_count = incoming_page_count.min(existing_page_count);
    let required_matched_pages = 4_u32.min(smaller_page_count);
    (loser_coverage >= 0.95
        && incoming_page_count.abs_diff(existing_page_count) <= 5
        && candidate.matched_pages >= required_matched_pages
        && candidate.confidence >= 0.9
        && aligned_run_ratio >= 0.75
        && informative_match_ratio >= 0.75
        && (smaller_page_count > 3 || candidate.exact_pages == candidate.matched_pages))
        .then_some(winner)
}

fn validate_strict_overlap_automatic_decision(
    review: &DownloadOverlapReview,
    request: &DownloadOverlapDecisionRequest,
) -> Result<(), ApplicationError> {
    let pending = review
        .candidates
        .iter()
        .filter(|candidate| candidate.decision.is_none())
        .collect::<Vec<_>>();
    let candidate_id = request.candidate_id.as_deref().unwrap_or_default();
    let selected = pending
        .iter()
        .find(|candidate| candidate.candidate_id == candidate_id)
        .copied()
        .ok_or_else(|| {
            ApplicationError::DownloadOverlapDecisionInvalid(
                "The automatic overlap candidate is no longer pending".into(),
            )
        })?;
    let selected_winner = strict_overlap_winner(&review.incoming, selected);
    let safe = match request.action {
        DownloadOverlapDecisionAction::RemoveExistingContinue => {
            selected_winner == Some(StrictOverlapWinner::Incoming)
                && pending.iter().all(|candidate| {
                    strict_overlap_winner(&review.incoming, candidate)
                        == Some(StrictOverlapWinner::Incoming)
                })
        }
        DownloadOverlapDecisionAction::RemoveIncoming => {
            pending.len() == 1 && selected_winner == Some(StrictOverlapWinner::Existing)
        }
        _ => false,
    };
    if safe {
        Ok(())
    } else {
        Err(ApplicationError::DownloadOverlapDecisionInvalid(
            "This review does not satisfy the 95 percent overlap and five-page safety rule; manual review is required"
                .into(),
        ))
    }
}

fn overlap_check_failed() -> DownloadPipelineError {
    DownloadPipelineError::new(
        DownloadPipelineErrorCode::OverlapCheckFailed,
        "The verified download could not be compared safely with owned editions",
        true,
    )
}

fn verify_bundle_files(
    inner: &SupervisorInner,
    layout: &ArtifactLayout,
    bundle: &crate::domain::ArtifactBundle,
) -> Result<(), RunError> {
    if bundle.pages.len() != bundle.artifact.expected_page_count as usize {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::ManifestInvalid,
            "The database page map does not match the expected page count",
            false,
        )
        .into());
    }
    for page in &bundle.pages {
        if page.state != PageArtifactState::Present || page.excluded {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "A required artifact page is not present",
                false,
            )
            .into());
        }
        let expected = StoredPage {
            source_page_number: page.page_id.source_page_number,
            relative_path: page.relative_path.clone(),
            byte_length: page.byte_length.ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "A page checkpoint is missing its byte length",
                    false,
                )
            })?,
            sha256: page.sha256.clone().ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "A page checkpoint is missing its SHA-256 digest",
                    false,
                )
            })?,
            storage_format: page.storage_format.unwrap_or(ArtifactStorageFormat::Webp),
            source_revision: page.source_revision.clone().ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "A page checkpoint is missing its source revision",
                    false,
                )
            })?,
            verified_at: page.verified_at.clone().unwrap_or_default(),
        };
        match inner.store.verify_existing_page(
            layout,
            page.page_id.source_page_number,
            &expected.source_revision,
            Some(&expected),
        )? {
            ExistingPageVerification::Verified(_) => {}
            ExistingPageVerification::Missing => {
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ArtifactMissing,
                    "A verified artifact page is missing from disk",
                    false,
                )
                .into())
            }
            ExistingPageVerification::Invalid { .. } => {
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::HashMismatch,
                    "A verified artifact page no longer matches its digest",
                    false,
                )
                .into())
            }
        }
    }
    Ok(())
}

fn page_verification(page: &crate::domain::PageArtifact) -> Option<StoredPage> {
    Some(StoredPage {
        source_page_number: page.page_id.source_page_number,
        relative_path: page.relative_path.clone(),
        byte_length: page.byte_length?,
        sha256: page.sha256.clone()?,
        storage_format: page.storage_format?,
        source_revision: page.source_revision.clone()?,
        verified_at: page.verified_at.clone()?,
    })
}

fn handle_download_error(
    inner: &SupervisorInner,
    descriptor: &DownloadJobDescriptor,
    cancellation: &CancellationToken,
    error: RunError,
) {
    if cancellation.is_cancelled() {
        tracing::info!(
            job_id = descriptor.job_id,
            worker_attempt = descriptor.worker_attempt,
            "download worker stopped after cancellation"
        );
        return;
    }
    let (code, message, retryable) = error.stable();
    tracing::error!(
        job_id = descriptor.job_id,
        gallery_id = descriptor.gallery_id.get(),
        worker_attempt = descriptor.worker_attempt,
        error_code = code,
        retryable,
        "download worker stopped before verification"
    );
    match inner
        .repository
        .pipeline_fail(descriptor, code, message, retryable)
    {
        Ok(Some(projection)) => emit(inner, projection),
        Ok(None) => {}
        Err(repository_error) => tracing::error!(
            job_id = descriptor.job_id,
            error_code = repository_error.stable_code(),
            "download failure state could not be persisted"
        ),
    }
}

fn emit(inner: &SupervisorInner, projection: DownloadJobProjection) {
    if inner.events.send(projection).is_err() {
        tracing::warn!("download event receiver is no longer available");
    }
}

fn descriptor_key(descriptor: &DownloadJobDescriptor) -> String {
    format!("{}:{}", descriptor.job_id, descriptor.worker_attempt)
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), RunError> {
    if cancellation.is_cancelled() {
        Err(DownloadPipelineError::cancelled().into())
    } else {
        Ok(())
    }
}

enum RunError {
    Repository(RepositoryError),
    Source(SourceContractError),
    Pipeline(DownloadPipelineError),
}

impl RunError {
    fn stable(&self) -> (&str, &str, bool) {
        match self {
            Self::Repository(error) => {
                let busy = matches!(error, RepositoryError::Busy(_));
                (
                    if busy {
                        "DATABASE_BUSY"
                    } else {
                        "DATABASE_ERROR"
                    },
                    "The download state could not be updated safely",
                    busy,
                )
            }
            Self::Source(error) => {
                let (code, message) = stable_source_failure(error);
                (code, message, error.retryable)
            }
            Self::Pipeline(error) => (error.code.as_str(), &error.message, error.retryable),
        }
    }
}

impl From<RepositoryError> for RunError {
    fn from(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

impl From<SourceContractError> for RunError {
    fn from(error: SourceContractError) -> Self {
        Self::Source(error)
    }
}

impl From<DownloadPipelineError> for RunError {
    fn from(error: DownloadPipelineError) -> Self {
        Self::Pipeline(error)
    }
}

fn persist_candidate_diagnostics(
    inner: &SupervisorInner,
    descriptor: &DownloadJobDescriptor,
    source_page_number: crate::domain::SourcePageNumber,
    diagnostics: &[SourceCandidateDiagnostic],
) -> Result<(), RepositoryError> {
    for diagnostic in diagnostics {
        let attempt = DownloadPageAttempt {
            descriptor: descriptor.clone(),
            source_page_number,
            candidate_index: diagnostic.candidate_index,
            candidate_format: diagnostic.format.clone(),
        };
        inner.repository.pipeline_page_attempt_start(&attempt)?;
        let outcome = match diagnostic.error_code {
            None => DownloadPageAttemptOutcome::Succeeded,
            Some(SourceErrorCode::Cancelled) => DownloadPageAttemptOutcome::Cancelled,
            Some(_) => DownloadPageAttemptOutcome::Failed,
        };
        inner
            .repository
            .pipeline_page_attempt_finish(&DownloadPageAttemptResult {
                attempt,
                outcome,
                bytes_received: diagnostic.bytes_received,
                http_status: diagnostic.http_status,
                content_type: diagnostic.content_type.clone(),
                error_code: diagnostic.error_code.map(|code| code.as_str().to_owned()),
                error_message: None,
                retryable: diagnostic.retryable,
            })?;
    }
    Ok(())
}

fn stable_source_failure(error: &SourceContractError) -> (&'static str, &'static str) {
    match error.code {
        SourceErrorCode::Cancelled => ("REQUEST_CANCELLED", "The source request was cancelled"),
        SourceErrorCode::Validation => (
            "SOURCE_VALIDATION",
            "The source request did not pass validation",
        ),
        SourceErrorCode::NotFound => ("SOURCE_NOT_FOUND", "A required source page was not found"),
        SourceErrorCode::Protocol => (
            "SOURCE_PROTOCOL",
            "The source response did not match the supported protocol",
        ),
        SourceErrorCode::InvalidData => (
            "SOURCE_INVALID_DATA",
            "The source returned metadata that could not be read safely",
        ),
        SourceErrorCode::RateLimited => (
            "SOURCE_RATE_LIMITED",
            "The source is rate limiting page downloads",
        ),
        SourceErrorCode::TemporarilyUnavailable => (
            "SOURCE_TEMPORARILY_UNAVAILABLE",
            "The source is temporarily unavailable",
        ),
        SourceErrorCode::Timeout => (
            "SOURCE_TIMEOUT",
            "The source did not return the page in time",
        ),
        SourceErrorCode::Unauthorized => (
            "SOURCE_UNAUTHORIZED",
            "The source rejected the page request",
        ),
        SourceErrorCode::Transport => (
            "NETWORK_OFFLINE",
            "A connection to the source could not be established",
        ),
        SourceErrorCode::ImageCandidatesExhausted => (
            "IMAGE_CANDIDATES_EXHAUSTED",
            "All supported page image candidates were exhausted",
        ),
        SourceErrorCode::ImageResponseInvalid => (
            "IMAGE_RESPONSE_INVALID",
            "The source returned a response that is not a supported image",
        ),
        SourceErrorCode::ImageDecodeFailed => (
            "IMAGE_DECODE_FAILED",
            "The downloaded page could not be decoded safely",
        ),
        SourceErrorCode::ImageFormatUnsupported => (
            "IMAGE_FORMAT_UNSUPPORTED",
            "The downloaded page format is not supported safely",
        ),
    }
}

fn now_unix_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn unpoison<T>(result: std::sync::LockResult<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    result.unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use crate::application::download_pipeline::DownloadPipelineRepository;
    use std::{
        io::Cursor,
        sync::{mpsc, Mutex},
        time::{Duration, Instant},
    };

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use tempfile::tempdir;

    use super::*;
    use crate::{
        application::{
            ApplicationService, AutomationRepository, DownloadGallerySnapshot, DownloadPagePayload,
            DownloadSourceImageFormat, DownloadSourcePage,
        },
        domain::{
            DownloadListRequest, DownloadOverlapDecisionAction, DownloadOverlapDecisionRequest,
            DownloadOverlapPairDecision, DownloadOverlapRelation, DownloadOverlapReviewState,
            DownloadReviewKind, Gallery, GalleryId, GalleryMetadata, SettingsPatch,
            SourcePageNumber,
        },
        infrastructure::{FilesystemArtifactStore, SqliteRepository},
    };

    fn strict_automatic_review() -> DownloadOverlapReview {
        let page_pairs = (1..=20)
            .map(|page| crate::domain::DownloadOverlapPagePair {
                incoming_source_page: page,
                existing_source_page: page,
                exact_sha256: true,
                d_hash_distance: 0,
                p_hash_distance: 0,
                detail_hash_distance: 0,
                edge_similarity: 1.0,
                visual_similarity: 1.0,
                low_information: false,
            })
            .collect();
        DownloadOverlapReview {
            review_id: "strict-review".to_owned(),
            entry_id: "incoming-entry".to_owned(),
            incoming: crate::domain::DownloadOverlapGalleryRef {
                entry_id: "incoming-entry".to_owned(),
                gallery_id: GalleryId::new(200).unwrap(),
                title: "Incoming B".to_owned(),
                artists: vec!["artist".to_owned()],
                page_count: 25,
            },
            revision: 4,
            state: DownloadOverlapReviewState::Pending,
            profile_version: 1,
            policy_version: 1,
            incoming_fingerprint: "b".repeat(64),
            candidates: vec![crate::domain::DownloadOverlapCandidate {
                candidate_id: "strict-candidate".to_owned(),
                existing: crate::domain::DownloadOverlapGalleryRef {
                    entry_id: "existing-entry".to_owned(),
                    gallery_id: GalleryId::new(100).unwrap(),
                    title: "Existing A".to_owned(),
                    artists: vec!["artist".to_owned()],
                    page_count: 20,
                },
                existing_fingerprint: "a".repeat(64),
                relation: DownloadOverlapRelation::IncomingContainsExisting,
                confidence: 0.99,
                matched_pages: 20,
                exact_pages: 20,
                visual_pages: 0,
                existing_coverage: 1.0,
                incoming_coverage: 0.8,
                existing_unique_pages: 0,
                incoming_unique_pages: 5,
                longest_aligned_run: 20,
                rank: 1,
                decision: None,
                page_pairs,
            }],
            created_at: "2026-08-31T00:00:00Z".to_owned(),
            updated_at: "2026-08-31T00:00:00Z".to_owned(),
            resolved_at: None,
        }
    }

    fn strict_automatic_request() -> DownloadOverlapDecisionRequest {
        DownloadOverlapDecisionRequest {
            review_id: "strict-review".to_owned(),
            expected_revision: 4,
            action: DownloadOverlapDecisionAction::RemoveExistingContinue,
            candidate_id: Some("strict-candidate".to_owned()),
            actor: DownloadOverlapDecisionActor::Automation,
            reason_code: Some("balanced_overlap_v2".to_owned()),
            rule_version: Some(2),
            feature_snapshot_json: Some(
                serde_json::json!({
                    "rule": "balanced_overlap_v2",
                    "ruleVersion": 2,
                    "reviewId": "strict-review",
                    "reviewRevision": 4,
                    "candidateId": "strict-candidate"
                })
                .to_string(),
            ),
        }
    }

    #[test]
    fn strict_overlap_automation_is_rechecked_in_the_supervisor() {
        let review = strict_automatic_review();
        let mut request = strict_automatic_request();
        normalize_overlap_decision_audit(&mut request).expect("valid audit snapshot");
        validate_strict_overlap_automatic_decision(&review, &request)
            .expect("strict containment should be eligible");

        let mut near_equivalent = review.clone();
        near_equivalent.candidates[0].relation = DownloadOverlapRelation::NearEquivalent;
        near_equivalent.candidates[0].incoming_coverage = 0.95;
        near_equivalent.candidates[0].existing_coverage = 1.0;
        validate_strict_overlap_automatic_decision(&near_equivalent, &request)
            .expect("near-equivalent larger incoming should be eligible");

        let mut equal_unknown = near_equivalent.clone();
        equal_unknown.incoming.page_count = 20;
        equal_unknown.candidates[0].incoming_coverage = 1.0;
        assert!(validate_strict_overlap_automatic_decision(&equal_unknown, &request).is_err());
        let mut keep_existing = request.clone();
        keep_existing.action = DownloadOverlapDecisionAction::RemoveIncoming;
        validate_strict_overlap_automatic_decision(&equal_unknown, &keep_existing)
            .expect("equal unknown editions should keep the stable existing artifact");

        let mut uncensored_incoming = equal_unknown.clone();
        uncensored_incoming.incoming.title = "Edition [ＵＮＣＥＮＳＯＲＥＤ]".to_owned();
        uncensored_incoming.candidates[0].existing.title = "Edition [Censored]".to_owned();
        validate_strict_overlap_automatic_decision(&uncensored_incoming, &request)
            .expect("the NFKC-normalized uncensored marker should take priority");

        let mut weak_evidence = review.clone();
        weak_evidence.candidates[0].confidence = 0.89;
        assert!(validate_strict_overlap_automatic_decision(&weak_evidence, &request).is_err());

        let mut excessive_page_gap = review.clone();
        excessive_page_gap.incoming.page_count = 26;
        assert!(validate_strict_overlap_automatic_decision(&excessive_page_gap, &request).is_err());

        let mut mismatched_snapshot = strict_automatic_request();
        mismatched_snapshot.feature_snapshot_json = Some(
            serde_json::json!({
                "rule": "balanced_overlap_v2",
                "ruleVersion": 2,
                "reviewId": "strict-review",
                "reviewRevision": 3,
                "candidateId": "strict-candidate"
            })
            .to_string(),
        );
        assert!(normalize_overlap_decision_audit(&mut mismatched_snapshot).is_err());
    }

    struct FakeDownloadSource {
        pages: u32,
        block_page: Option<u32>,
        gallery_revision: u64,
        calls: Mutex<Vec<u32>>,
    }

    impl FakeDownloadSource {
        fn new(pages: u32, block_page: Option<u32>) -> Self {
            Self {
                pages,
                block_page,
                gallery_revision: 1,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_gallery_revision(mut self, gallery_revision: u64) -> Self {
            self.gallery_revision = gallery_revision;
            self
        }

        fn calls(&self) -> Vec<u32> {
            unpoison(self.calls.lock()).clone()
        }
    }

    impl DownloadSourcePort for FakeDownloadSource {
        fn gallery_snapshot(
            &self,
            gallery_id: GalleryId,
            _cancellation: &CancellationToken,
        ) -> Result<DownloadGallerySnapshot, SourceContractError> {
            let metadata = GalleryMetadata::new(
                "Synthetic download fixture",
                Some("fixture artist".into()),
                Some("fixture group".into()),
                self.pages,
            )
            .unwrap();
            let pages = (1..=self.pages)
                .map(|number| DownloadSourcePage {
                    source_page_number: SourcePageNumber::new(number).unwrap(),
                    source_revision: format!("fixture-page-v1:{number}"),
                })
                .collect();
            Ok(DownloadGallerySnapshot {
                gallery: Gallery::new(gallery_id, self.gallery_revision, metadata),
                source_revision: format!("fixture-gallery:{:016x}", self.gallery_revision),
                pages,
            })
        }

        fn download_page(
            &self,
            _gallery_id: GalleryId,
            source_page_number: SourcePageNumber,
            cancellation: &CancellationToken,
        ) -> Result<DownloadPagePayload, SourceContractError> {
            unpoison(self.calls.lock()).push(source_page_number.get());
            if self.block_page == Some(source_page_number.get()) {
                while !cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                return Err(SourceContractError::cancelled());
            }
            let color = u8::try_from(source_page_number.get()).unwrap_or(u8::MAX);
            let image =
                DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([color, 20, 30, 255])));
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, ImageFormat::Png).unwrap();
            Ok(DownloadPagePayload {
                source_page_number,
                bytes: bytes.into_inner(),
                source_revision: format!("fixture-page-v1:{}", source_page_number.get()),
                source_format: DownloadSourceImageFormat::Png,
                width: 2,
                height: 2,
                candidate_index: 0,
                candidate_diagnostics: Vec::new(),
            })
        }
    }

    fn configured_repository(
        directory: &std::path::Path,
    ) -> (Arc<SqliteRepository>, ApplicationService) {
        let repository = Arc::new(SqliteRepository::open(directory.join("state.sqlite3")).unwrap());
        let service = ApplicationService::new(repository.clone())
            .with_download_repository(repository.clone());
        let settings = service.settings_get().unwrap();
        service
            .settings_update(
                SettingsPatch {
                    download_root: Some(directory.join("downloads").to_string_lossy().into_owned()),
                    ..SettingsPatch::default()
                },
                settings.revision,
            )
            .unwrap();
        (repository, service)
    }

    fn launch(
        repository: &Arc<SqliteRepository>,
        source: Arc<dyn DownloadSourcePort>,
    ) -> (DownloadSupervisor, mpsc::Receiver<DownloadJobProjection>) {
        let (events, receiver) = mpsc::channel();
        let pipeline_repository: Arc<dyn DownloadOverlapRepository> = repository.clone();
        let settings_repository: Arc<dyn StateRepository> = repository.clone();
        let store: Arc<dyn ArtifactStore> = Arc::new(FilesystemArtifactStore::new());
        (
            DownloadSupervisor::new(
                pipeline_repository,
                settings_repository,
                source,
                store,
                events,
                1,
            )
            .unwrap(),
            receiver,
        )
    }

    fn wait_for_state(
        service: &ApplicationService,
        entry_id: &str,
        expected: JobState,
        minimum_progress: f64,
    ) -> crate::domain::DownloadEntry {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let page = service
                .download_entries_list(DownloadListRequest {
                    state: None,
                    query: None,
                    page: 1,
                    page_size: 20,
                })
                .unwrap();
            if let Some(entry) = page.entries.into_iter().find(|entry| {
                entry.entry_id.as_str() == entry_id
                    && entry.state == expected
                    && entry.progress.unwrap_or_default() >= minimum_progress
            }) {
                return entry;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("download {entry_id} did not reach {expected}");
    }

    fn wait_for_stored_state(database_path: &std::path::Path, entry_id: &str, expected: JobState) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let expected = expected.to_string();
        while Instant::now() < deadline {
            let stored = rusqlite::Connection::open(database_path)
                .and_then(|connection| {
                    connection.query_row(
                        "SELECT state FROM download_entries WHERE entry_id = ?1",
                        [entry_id],
                        |row| row.get::<_, String>(0),
                    )
                })
                .ok();
            if stored.as_deref() == Some(expected.as_str()) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("stored download {entry_id} did not reach {expected}");
    }

    #[test]
    fn source_failures_are_redacted_and_stable() {
        let error = SourceContractError::image_response_invalid(
            "https://private.invalid/image?token=secret returned HTML",
        );
        let (code, message) = stable_source_failure(&error);
        assert_eq!(code, "IMAGE_RESPONSE_INVALID");
        assert!(!message.contains("private"));
        assert!(!message.contains("secret"));
    }

    #[test]
    fn retryability_is_persisted_for_job_attempt_and_list_projection() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let queued = service
            .download_queue_add(vec![41], "retryability-persistence".into())
            .unwrap();
        let descriptor = queued.jobs.into_iter().next().unwrap();
        repository.pipeline_begin(&descriptor).unwrap();
        repository
            .pipeline_fail(&descriptor, "NETWORK_OFFLINE", "Source unavailable", true)
            .unwrap();

        let entries = service
            .download_entries_list(crate::domain::DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(entries.entries[0].error_retryable, Some(true));
        let stored: i64 = rusqlite::Connection::open(root.join("state.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT error_retryable FROM download_attempts WHERE job_id = ?1 AND attempt = ?2",
                rusqlite::params![descriptor.job_id, descriptor.worker_attempt],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, 1);
    }

    #[test]
    fn real_pipeline_writes_verified_webp_manifest_before_completed() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(2, None));
        let (supervisor, _events) = launch(&repository, source.clone());
        let queued = service
            .download_queue_add(vec![42], "pipeline-complete".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        let pending_error = supervisor.artifact_folder_path(&entry_id).unwrap_err();
        assert!(matches!(
            pending_error,
            ApplicationError::DownloadPipeline(DownloadPipelineError {
                code: DownloadPipelineErrorCode::ArtifactMissing,
                ..
            })
        ));
        supervisor.enqueue_all(queued.jobs).unwrap();

        let completed = wait_for_state(&service, &entry_id, JobState::Completed, 100.0);
        assert_eq!(completed.attempt, Some(1));
        supervisor.shutdown_and_wait();

        let diagnostics = rusqlite::Connection::open(root.join("state.sqlite3"))
            .unwrap()
            .query_row(
                r#"
                    SELECT COUNT(*), MIN(candidate_format), SUM(retryable),
                           SUM(CASE WHEN finished_at IS NOT NULL THEN 1 ELSE 0 END)
                    FROM download_page_attempts
                "#,
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(diagnostics, (2, "png".into(), 0, 2));

        let bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(entry_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        let artifact_directory = supervisor.artifact_folder_path(&entry_id).unwrap();
        assert_eq!(
            artifact_directory,
            root.join("downloads")
                .join(bundle.artifact.relative_directory.as_str())
                .canonicalize()
                .unwrap()
        );
        assert_eq!(bundle.artifact.state, DownloadArtifactState::Complete);
        assert_eq!(bundle.pages.len(), 2);
        assert!(bundle.pages.iter().all(|page| {
            page.state == PageArtifactState::Present
                && page.sha256.is_some()
                && page.storage_format == Some(ArtifactStorageFormat::Webp)
        }));
        let manifest_path = root.join("downloads").join(
            bundle
                .artifact
                .manifest_relative_path
                .as_ref()
                .unwrap()
                .as_str(),
        );
        let manifest: ArtifactManifest =
            serde_json::from_reader(std::fs::File::open(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.schema_version, ARTIFACT_MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.pages.len(), 2);
        let first_page = FilesystemArtifactStore::new()
            .first_verified_page_path(&root.join("downloads"), &bundle)
            .unwrap();
        assert_eq!(
            first_page.file_name().and_then(|name| name.to_str()),
            Some("0001.webp")
        );
        assert_eq!(source.calls(), vec![1, 2]);
        let part_files = std::fs::read_dir(
            root.join("downloads")
                .join(bundle.artifact.relative_directory.as_str()),
        )
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
        .count();
        assert_eq!(part_files, 0);
    }

    #[test]
    fn strong_same_artist_overlap_pauses_before_manifest_and_resumes_without_redownload() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(8, None));
        let (supervisor, events) = launch(&repository, source.clone());

        let first = service
            .download_queue_add(vec![501], "overlap-existing".into())
            .unwrap();
        supervisor.enqueue_all(first.jobs).unwrap();
        wait_for_state(
            &service,
            first.entries[0].entry_id.as_str(),
            JobState::Completed,
            100.0,
        );

        let removed = rusqlite::Connection::open(root.join("state.sqlite3"))
            .unwrap()
            .execute(
                "DELETE FROM owned_gallery_artists WHERE gallery_id = ?1",
                [501_i64],
            )
            .unwrap();
        assert_eq!(
            removed, 1,
            "fixture must reproduce a legacy primary-artist-only artifact"
        );

        let incoming = service
            .download_queue_add(vec![502], "overlap-incoming".into())
            .unwrap();
        let incoming_id = incoming.entries[0].entry_id.to_string();
        supervisor.enqueue_all(incoming.jobs).unwrap();
        let paused = wait_for_state(&service, &incoming_id, JobState::ReviewRequired, 100.0);
        assert_eq!(
            paused.review_kind,
            Some(DownloadReviewKind::GalleryDuplicate)
        );
        let review_id = paused.review_id.expect("review id");
        let review_event = events
            .try_iter()
            .find(|projection| {
                projection.download.entry_id == incoming_id
                    && projection.download.state == JobState::ReviewRequired
            })
            .expect("review-required event");
        assert_eq!(
            review_event.download.review_kind,
            Some(DownloadReviewKind::GalleryDuplicate)
        );
        assert_eq!(
            review_event.download.review_id.as_deref(),
            Some(review_id.as_str())
        );
        let review = supervisor
            .overlap_review_get(&review_id)
            .unwrap()
            .expect("stored overlap review");
        assert_eq!(review.state, DownloadOverlapReviewState::Pending);
        assert_eq!(review.candidates.len(), 1);
        assert_eq!(
            review.candidates[0].relation,
            DownloadOverlapRelation::NearEquivalent
        );

        let paused_bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(incoming_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            paused_bundle.artifact.state,
            DownloadArtifactState::Incomplete
        );
        let reserved_manifest = paused_bundle
            .artifact
            .manifest_relative_path
            .as_ref()
            .expect("manifest destination is reserved before completion");
        assert!(!root
            .join("downloads")
            .join(reserved_manifest.as_str())
            .exists());
        let calls_before_decision = source.calls();

        let candidate_id = review.candidates[0].candidate_id.clone();
        let decided = supervisor
            .overlap_decision_apply(DownloadOverlapDecisionRequest {
                review_id,
                expected_revision: review.revision,
                action: DownloadOverlapDecisionAction::KeepBothContinue,
                candidate_id: Some(candidate_id),
                actor: Default::default(),
                reason_code: None,
                rule_version: None,
                feature_snapshot_json: None,
            })
            .unwrap();
        assert!(decided.resumed);
        assert!(!decided.cancelled);
        wait_for_state(&service, &incoming_id, JobState::Completed, 100.0);
        assert_eq!(source.calls(), calls_before_decision);
        let completed_bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(incoming_id).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            completed_bundle.artifact.state,
            DownloadArtifactState::Complete
        );
        assert!(completed_bundle.artifact.manifest_relative_path.is_some());

        let approved = service
            .download_queue_add(vec![503], "overlap-approved-pair".into())
            .unwrap();
        let approved_id = approved.entries[0].entry_id.to_string();
        supervisor.enqueue_all(approved.jobs).unwrap();
        let approved_entry = wait_for_state(&service, &approved_id, JobState::Completed, 100.0);
        assert_eq!(approved_entry.review_kind, None);
        assert_eq!(approved_entry.review_id, None);
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn removing_incoming_overlap_keeps_audit_row_but_omits_it_from_downloads() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let database_path = root.join("state.sqlite3");
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(8, None));
        let (supervisor, _events) = launch(&repository, source);

        let existing = service
            .download_queue_add(vec![551], "overlap-remove-incoming-existing".into())
            .unwrap();
        let existing_id = existing.entries[0].entry_id.to_string();
        supervisor.enqueue_all(existing.jobs).unwrap();
        wait_for_state(&service, &existing_id, JobState::Completed, 100.0);

        let incoming = service
            .download_queue_add(vec![552], "overlap-remove-incoming".into())
            .unwrap();
        let incoming_id = incoming.entries[0].entry_id.to_string();
        supervisor.enqueue_all(incoming.jobs).unwrap();
        let paused = wait_for_state(&service, &incoming_id, JobState::ReviewRequired, 100.0);
        let review_id = paused.review_id.expect("overlap review id");
        let review = supervisor
            .overlap_review_get(&review_id)
            .unwrap()
            .expect("overlap review");

        let decided = supervisor
            .overlap_decision_apply(DownloadOverlapDecisionRequest {
                review_id,
                expected_revision: review.revision,
                action: DownloadOverlapDecisionAction::RemoveIncoming,
                candidate_id: None,
                actor: Default::default(),
                reason_code: None,
                rule_version: None,
                feature_snapshot_json: None,
            })
            .unwrap();
        assert!(decided.cancelled);
        assert!(!decided.resumed);
        wait_for_stored_state(&database_path, &incoming_id, JobState::Cancelled);

        let visible = service
            .download_entries_list(DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert!(visible
            .entries
            .iter()
            .any(|entry| entry.entry_id.as_str() == existing_id));
        assert!(!visible
            .entries
            .iter()
            .any(|entry| entry.entry_id.as_str() == incoming_id));
        assert_eq!(visible.total_items, 1);

        let cancelled = service
            .download_entries_list(DownloadListRequest {
                state: Some(JobState::Cancelled),
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert!(cancelled.entries.is_empty());
        assert_eq!(cancelled.total_items, 0);
        let exclusions = repository
            .exploration_exclusions_list()
            .expect("list overlap removal exclusions");
        assert!(exclusions.iter().any(|exclusion| {
            exclusion.gallery_id.get() == 552
                && exclusion.reasons.iter().any(|reason| {
                    reason.kind == crate::domain::ExplorationExclusionKind::DuplicateHidden
                })
        }));
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn removing_existing_overlap_candidate_quarantines_it_and_resumes_incoming() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(8, None));
        let (supervisor, _events) = launch(&repository, source);

        let existing = service
            .download_queue_add(vec![601], "overlap-remove-existing".into())
            .unwrap();
        let existing_id = existing.entries[0].entry_id.to_string();
        supervisor.enqueue_all(existing.jobs).unwrap();
        wait_for_state(&service, &existing_id, JobState::Completed, 100.0);
        let existing_bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(existing_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        let existing_relative_directory = existing_bundle.artifact.relative_directory.clone();

        let incoming = service
            .download_queue_add(vec![602], "overlap-replace-existing".into())
            .unwrap();
        let incoming_id = incoming.entries[0].entry_id.to_string();
        supervisor.enqueue_all(incoming.jobs).unwrap();
        let paused = wait_for_state(&service, &incoming_id, JobState::ReviewRequired, 100.0);
        let review_id = paused.review_id.expect("overlap review id");
        let review = supervisor
            .overlap_review_get(&review_id)
            .unwrap()
            .expect("overlap review");
        let candidate_id = review.candidates[0].candidate_id.clone();

        let decided = supervisor
            .overlap_decision_apply(DownloadOverlapDecisionRequest {
                review_id,
                expected_revision: review.revision,
                action: DownloadOverlapDecisionAction::RemoveExistingContinue,
                candidate_id: Some(candidate_id),
                actor: Default::default(),
                reason_code: None,
                rule_version: None,
                feature_snapshot_json: None,
            })
            .unwrap();
        assert!(decided.resumed);
        assert!(!decided.cancelled);
        assert_eq!(
            decided.review.candidates[0].decision,
            Some(DownloadOverlapPairDecision::ExistingRemoved)
        );
        wait_for_stored_state(
            &root.join("state.sqlite3"),
            &existing_id,
            JobState::Quarantined,
        );
        wait_for_state(&service, &incoming_id, JobState::Completed, 100.0);
        assert!(!root
            .join("downloads")
            .join(existing_relative_directory.as_str())
            .exists());
        assert!(root.join("downloads/.atsumi-quarantine").is_dir());

        let restored = supervisor
            .restore_entries(vec![existing_id.clone()])
            .unwrap();
        assert_eq!(restored[0].state, JobState::Completed);
        assert!(root
            .join("downloads")
            .join(existing_relative_directory.as_str())
            .join("manifest.json")
            .is_file());
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn removing_a_chained_staging_candidate_cancels_only_its_own_review() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(8, None));
        let (supervisor, events) = launch(&repository, source);

        let first = service
            .download_queue_add(vec![701], "overlap-chain-first".into())
            .unwrap();
        let first_id = first.entries[0].entry_id.to_string();
        supervisor.enqueue_all(first.jobs).unwrap();
        wait_for_state(&service, &first_id, JobState::Completed, 100.0);

        let second = service
            .download_queue_add(vec![702], "overlap-chain-second".into())
            .unwrap();
        let second_id = second.entries[0].entry_id.to_string();
        supervisor.enqueue_all(second.jobs).unwrap();
        let second_paused = wait_for_state(&service, &second_id, JobState::ReviewRequired, 100.0);
        let second_review_id = second_paused.review_id.expect("second overlap review id");
        let second_bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(second_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        let second_directory = root
            .join("downloads")
            .join(second_bundle.artifact.relative_directory.as_str());
        assert_eq!(
            second_bundle.artifact.state,
            DownloadArtifactState::Incomplete
        );
        assert!(second_directory.is_dir());

        let third = service
            .download_queue_add(vec![703], "overlap-chain-third".into())
            .unwrap();
        let third_id = third.entries[0].entry_id.to_string();
        supervisor.enqueue_all(third.jobs).unwrap();
        let third_paused = wait_for_state(&service, &third_id, JobState::ReviewRequired, 100.0);
        let third_review_id = third_paused.review_id.expect("third overlap review id");
        let third_review = supervisor
            .overlap_review_get(&third_review_id)
            .unwrap()
            .expect("third overlap review");
        assert_eq!(third_review.candidates.len(), 2);
        let staged_candidate = third_review
            .candidates
            .iter()
            .find(|candidate| candidate.existing.entry_id == second_id)
            .expect("second staging must be a chained candidate");

        let decided = supervisor
            .overlap_decision_apply(DownloadOverlapDecisionRequest {
                review_id: third_review_id.clone(),
                expected_revision: third_review.revision,
                action: DownloadOverlapDecisionAction::RemoveExistingContinue,
                candidate_id: Some(staged_candidate.candidate_id.clone()),
                actor: Default::default(),
                reason_code: None,
                rule_version: None,
                feature_snapshot_json: None,
            })
            .unwrap();
        assert!(
            !decided.resumed,
            "the completed first candidate remains pending"
        );
        assert!(
            !decided.cancelled,
            "the current incoming review remains active"
        );
        wait_for_stored_state(&root.join("state.sqlite3"), &second_id, JobState::Cancelled);
        let visible_after_removal = service
            .download_entries_list(DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert!(!visible_after_removal
            .entries
            .iter()
            .any(|entry| entry.entry_id.as_str() == second_id));
        wait_for_state(&service, &first_id, JobState::Completed, 100.0);
        assert!(
            second_directory.is_dir(),
            "verified staging files are preserved"
        );
        assert_eq!(
            repository
                .pipeline_artifact_bundle(&DownloadEntryId::new(second_id.clone()).unwrap())
                .unwrap()
                .unwrap()
                .artifact
                .state,
            DownloadArtifactState::Incomplete
        );
        assert_eq!(
            supervisor
                .overlap_review_get(&second_review_id)
                .unwrap()
                .expect("cancelled chained review")
                .state,
            DownloadOverlapReviewState::Cancelled
        );
        assert!(events.try_iter().any(|projection| {
            projection.download.entry_id == second_id
                && projection.download.state == JobState::Cancelled
        }));

        let current = supervisor
            .overlap_review_get(&third_review_id)
            .unwrap()
            .expect("current overlap review");
        assert_eq!(current.state, DownloadOverlapReviewState::Pending);
        assert_eq!(
            current
                .candidates
                .iter()
                .find(|candidate| candidate.existing.entry_id == second_id)
                .and_then(|candidate| candidate.decision),
            Some(DownloadOverlapPairDecision::ExistingRemoved)
        );
        let remaining = current
            .candidates
            .iter()
            .find(|candidate| candidate.decision.is_none())
            .expect("completed first candidate remains");
        let finished = supervisor
            .overlap_decision_apply(DownloadOverlapDecisionRequest {
                review_id: third_review_id,
                expected_revision: current.revision,
                action: DownloadOverlapDecisionAction::KeepBothContinue,
                candidate_id: Some(remaining.candidate_id.clone()),
                actor: Default::default(),
                reason_code: None,
                rule_version: None,
                feature_snapshot_json: None,
            })
            .unwrap();
        assert!(finished.resumed);
        wait_for_state(&service, &third_id, JobState::Completed, 100.0);
        wait_for_state(&service, &first_id, JobState::Completed, 100.0);
        wait_for_stored_state(&root.join("state.sqlite3"), &second_id, JobState::Cancelled);
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn removing_failed_candidate_ignores_another_candidate_already_quarantined_elsewhere() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let database_path = root.join("state.sqlite3");
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(8, None));
        let (supervisor, _events) = launch(&repository, source);

        let first = service
            .download_queue_add(vec![801], "overlap-stale-first".into())
            .unwrap();
        let first_id = first.entries[0].entry_id.to_string();
        supervisor.enqueue_all(first.jobs).unwrap();
        wait_for_state(&service, &first_id, JobState::Completed, 100.0);

        let second = service
            .download_queue_add(vec![802], "overlap-stale-second".into())
            .unwrap();
        let second_id = second.entries[0].entry_id.to_string();
        supervisor.enqueue_all(second.jobs).unwrap();
        let second_paused = wait_for_state(&service, &second_id, JobState::ReviewRequired, 100.0);
        let second_review_id = second_paused.review_id.expect("second overlap review id");
        let second_bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(second_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        let second_directory = root
            .join("downloads")
            .join(second_bundle.artifact.relative_directory.as_str());

        let third = service
            .download_queue_add(vec![803], "overlap-stale-third".into())
            .unwrap();
        let third_id = third.entries[0].entry_id.to_string();
        supervisor.enqueue_all(third.jobs).unwrap();
        let third_paused = wait_for_state(&service, &third_id, JobState::ReviewRequired, 100.0);
        let third_review_id = third_paused.review_id.expect("third overlap review id");
        let third_review = supervisor
            .overlap_review_get(&third_review_id)
            .unwrap()
            .expect("third overlap review");
        assert_eq!(third_review.candidates.len(), 2);
        let failed_candidate = third_review
            .candidates
            .iter()
            .find(|candidate| candidate.existing.entry_id == second_id)
            .expect("second staging must be a candidate")
            .candidate_id
            .clone();

        supervisor
            .quarantine_entries(
                vec![first_id.clone()],
                "removed by an earlier overlap review".into(),
            )
            .unwrap();
        wait_for_state(&service, &first_id, JobState::Quarantined, 100.0);

        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .execute(
                "UPDATE download_overlap_reviews SET revision=revision+1, state='resolved', resolved_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE review_id=?1",
                [&second_review_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE download_jobs SET revision=revision+1, state='failed', last_error_code='OVERLAP_CHECK_FAILED', last_error_message='fixture failure', finished_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE entry_id=?1",
                [&second_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE download_entries SET revision=revision+1, state='failed', review_kind=NULL, review_id=NULL WHERE entry_id=?1",
                [&second_id],
            )
            .unwrap();
        drop(connection);
        wait_for_state(&service, &second_id, JobState::Failed, 100.0);

        let decided = supervisor
            .overlap_decision_apply(DownloadOverlapDecisionRequest {
                review_id: third_review_id.clone(),
                expected_revision: third_review.revision,
                action: DownloadOverlapDecisionAction::RemoveExistingContinue,
                candidate_id: Some(failed_candidate),
                actor: Default::default(),
                reason_code: None,
                rule_version: None,
                feature_snapshot_json: None,
            })
            .unwrap();
        assert!(decided.resumed);
        assert!(!decided.cancelled);
        assert_eq!(decided.review.state, DownloadOverlapReviewState::Resolved);
        assert!(decided.review.candidates.iter().all(|candidate| {
            candidate.decision == Some(DownloadOverlapPairDecision::ExistingRemoved)
        }));
        wait_for_stored_state(&database_path, &second_id, JobState::Cancelled);
        wait_for_state(&service, &third_id, JobState::Completed, 100.0);
        wait_for_state(&service, &first_id, JobState::Quarantined, 100.0);
        assert!(
            second_directory.is_dir(),
            "failed staging files are preserved"
        );

        let visible = service
            .download_entries_list(DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert!(!visible
            .entries
            .iter()
            .any(|entry| entry.entry_id.as_str() == second_id));
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn unsigned_source_revision_never_overflows_sqlite_gallery_revision() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(1, None).with_gallery_revision(u64::MAX));
        let (supervisor, _events) = launch(&repository, source);
        let queued = service
            .download_queue_add(vec![4_113_714], "unsigned-source-revision".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        supervisor.enqueue_all(queued.jobs).unwrap();
        wait_for_state(&service, &entry_id, JobState::Completed, 100.0);
        supervisor.shutdown_and_wait();

        let stored: (i64, String) = rusqlite::Connection::open(root.join("state.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT revision, source_revision FROM galleries WHERE gallery_id=4113714",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored.0, 0);
        assert_eq!(stored.1, "fixture-gallery:ffffffffffffffff");
    }

    #[test]
    fn interrupted_pipeline_resumes_from_verified_page_checkpoint() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let blocking_source = Arc::new(FakeDownloadSource::new(2, Some(2)));
        let (first_supervisor, _events) = launch(&repository, blocking_source.clone());
        let queued = service
            .download_queue_add(vec![77], "pipeline-resume".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        first_supervisor.enqueue_all(queued.jobs).unwrap();
        wait_for_state(&service, &entry_id, JobState::Downloading, 50.0);
        first_supervisor.shutdown_and_wait();
        assert_eq!(blocking_source.calls(), vec![1, 2]);

        let entry_key = DownloadEntryId::new(entry_id.clone()).unwrap();
        let reserved_directory = repository
            .pipeline_artifact_bundle(&entry_key)
            .unwrap()
            .unwrap()
            .artifact
            .relative_directory;
        let settings = service.settings_get().unwrap();
        service
            .settings_update(
                SettingsPatch {
                    folder_name_template: Some("{id} renamed".into()),
                    ..SettingsPatch::default()
                },
                settings.revision,
            )
            .unwrap();

        assert_eq!(service.download_recover_interrupted().unwrap(), 1);
        let resumed_source = Arc::new(FakeDownloadSource::new(2, None));
        let (second_supervisor, _events) = launch(&repository, resumed_source.clone());
        let mut prepared = second_supervisor
            .recover_startup_state_without_resume()
            .expect("prepare startup recovery without starting work");
        assert_eq!(prepared.resumed_jobs, 0);
        assert_eq!(service.download_active_count().unwrap(), 0);
        second_supervisor
            .resume_after_reconcile(&mut prepared)
            .expect("commit interrupted download resume");
        assert_eq!(prepared.resumed_jobs, 1);
        let completed = wait_for_state(&service, &entry_id, JobState::Completed, 100.0);
        second_supervisor.shutdown_and_wait();

        assert_eq!(completed.attempt, Some(2));
        assert_eq!(resumed_source.calls(), vec![2]);
        let bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(entry_id).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(bundle.artifact.relative_directory, reserved_directory);
        assert!(!root.join("downloads/77 renamed").exists());
        assert_eq!(bundle.pages.len(), 2);
        assert_eq!(bundle.pages[0].page_id.source_page_number.get(), 1);
        assert_eq!(bundle.pages[1].page_id.source_page_number.get(), 2);
    }

    #[test]
    fn ambiguous_resume_file_is_preserved_and_reported_as_a_listable_failure() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let blocking_source = Arc::new(FakeDownloadSource::new(2, Some(2)));
        let (first_supervisor, _events) = launch(&repository, blocking_source);
        let queued = service
            .download_queue_add(vec![4_136_275], "pipeline-recovery-conflict".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        first_supervisor.enqueue_all(queued.jobs).unwrap();
        wait_for_state(&service, &entry_id, JobState::Downloading, 50.0);
        first_supervisor.shutdown_and_wait();

        let entry_key = DownloadEntryId::new(entry_id.clone()).unwrap();
        let relative_directory = repository
            .pipeline_artifact_bundle(&entry_key)
            .unwrap()
            .unwrap()
            .artifact
            .relative_directory;
        let part_path = root
            .join("downloads")
            .join(relative_directory.as_str())
            .join(".0002.webp.part");
        let ambiguous_bytes = b"preserve ambiguous staging bytes";
        std::fs::write(&part_path, ambiguous_bytes).unwrap();

        assert_eq!(service.download_recover_interrupted().unwrap(), 1);
        let resumed_source = Arc::new(FakeDownloadSource::new(2, None));
        let (second_supervisor, _events) = launch(&repository, resumed_source.clone());
        assert_eq!(second_supervisor.resume_interrupted().unwrap(), 1);
        let failed = wait_for_state(&service, &entry_id, JobState::Failed, 50.0);
        assert_eq!(second_supervisor.resume_interrupted().unwrap(), 0);
        second_supervisor.shutdown_and_wait();

        assert_eq!(failed.error_code.as_deref(), Some("RECOVERY_CONFLICT"));
        assert_eq!(failed.error_retryable, Some(false));
        assert_eq!(failed.review_kind, None);
        assert_eq!(failed.review_id, None);
        assert!(resumed_source.calls().is_empty());
        assert!(!part_path.exists());
        let conflict_root = root.join("downloads/.atsumi-recovery/conflicts");
        let recovered_part = std::fs::read_dir(&conflict_root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join(".0002.webp.part"))
            .find(|candidate| candidate.is_file())
            .expect("ambiguous staging file remains in unique recovery storage");
        assert_eq!(std::fs::read(recovered_part).unwrap(), ambiguous_bytes);
        assert_eq!(service.download_recover_interrupted().unwrap(), 0);
        let listed = service
            .download_entries_list(DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(listed.total_items, 1);
        assert_eq!(listed.entries[0].state, JobState::Failed);
        let attempt: (String, String, i64) = rusqlite::Connection::open(root.join("state.sqlite3"))
            .unwrap()
            .query_row(
                r#"
                        SELECT outcome_state, error_code, error_retryable
                        FROM download_attempts
                        WHERE job_id = (
                            SELECT job_id FROM download_jobs WHERE entry_id = ?1
                        ) AND attempt = 2
                    "#,
                [&entry_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(attempt, ("failed".into(), "RECOVERY_CONFLICT".into(), 0));

        let retry = service.download_retry(vec![entry_id.clone()]).unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].worker_attempt, 3);
        let queued = service
            .download_entries_list(DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(queued.entries[0].state, JobState::Queued);
        assert_eq!(queued.entries[0].error_code, None);
        assert_eq!(queued.entries[0].error_message, None);
        assert_eq!(queued.entries[0].error_retryable, None);
    }

    #[test]
    fn occupied_first_destination_stays_typed_and_unmodified_across_retry() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let occupied = root
            .join("downloads")
            .join("[fixture artist] Synthetic download fixture [fixture group] 66");
        std::fs::create_dir_all(&occupied).unwrap();
        std::fs::write(occupied.join("user-owned.txt"), b"keep").unwrap();
        let source = Arc::new(FakeDownloadSource::new(1, None));
        let (supervisor, _events) = launch(&repository, source.clone());
        let queued = service
            .download_queue_add(vec![66], "pipeline-collision".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        supervisor.enqueue_all(queued.jobs).unwrap();

        let first = wait_for_state(&service, &entry_id, JobState::Failed, 0.0);
        assert_eq!(
            first.error_code.as_deref(),
            Some("ARTIFACT_DESTINATION_OCCUPIED")
        );
        let retry = service.download_retry(vec![entry_id.clone()]).unwrap();
        supervisor.enqueue_retries(&retry).unwrap();
        let second = wait_for_state(&service, &entry_id, JobState::Failed, 0.0);
        assert_eq!(second.attempt, Some(2));
        assert_eq!(
            second.error_code.as_deref(),
            Some("ARTIFACT_DESTINATION_OCCUPIED")
        );
        assert_eq!(
            std::fs::read(occupied.join("user-owned.txt")).unwrap(),
            b"keep"
        );
        assert!(source.calls().is_empty());
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn verified_artifact_quarantine_is_recoverable_and_never_purged_automatically() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(1, None));
        let (supervisor, _events) = launch(&repository, source);
        let queued = service
            .download_queue_add(vec![88], "pipeline-quarantine".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        supervisor.enqueue_all(queued.jobs).unwrap();
        wait_for_state(&service, &entry_id, JobState::Completed, 100.0);

        let replacement_root = root.join("replacement-downloads");
        std::fs::create_dir(&replacement_root).unwrap();
        let settings = service.settings_get().unwrap();
        service
            .settings_update(
                crate::domain::SettingsPatch {
                    download_root: Some(replacement_root.to_string_lossy().into_owned()),
                    ..crate::domain::SettingsPatch::default()
                },
                settings.revision,
            )
            .unwrap();

        let quarantined = supervisor
            .quarantine_entries(vec![entry_id.clone()], "integration test quarantine".into())
            .unwrap();
        assert_eq!(quarantined[0].state, JobState::Quarantined);
        let bundle = repository
            .pipeline_artifact_bundle(&DownloadEntryId::new(entry_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        let relative_directory = bundle.artifact.relative_directory.as_str();
        assert!(!root.join("downloads").join(relative_directory).exists());
        let record_directory = std::fs::read_dir(root.join("downloads/.atsumi-quarantine"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let quarantine_directory = record_directory.join(relative_directory);
        assert!(quarantine_directory.join("0001.webp").is_file());
        let manifest: ArtifactManifest = serde_json::from_reader(
            std::fs::File::open(quarantine_directory.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert!(manifest.pages.iter().all(|page| page.quarantined));
        assert!(manifest
            .pages
            .iter()
            .all(|page| page.relative_path.starts_with(".atsumi-quarantine/")));

        let restored = supervisor.restore_entries(vec![entry_id.clone()]).unwrap();
        assert_eq!(restored[0].state, JobState::Completed);
        assert!(root
            .join("downloads")
            .join(relative_directory)
            .join("0001.webp")
            .is_file());
        assert!(!quarantine_directory.exists());
        let restored_manifest: ArtifactManifest = serde_json::from_reader(
            std::fs::File::open(
                root.join("downloads")
                    .join(relative_directory)
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(restored_manifest.pages.iter().all(|page| !page.quarantined));
        assert!(restored_manifest
            .pages
            .iter()
            .all(|page| page.relative_path.starts_with(relative_directory)));
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn startup_recovery_finishes_a_quarantine_move_without_scanning_completed_artifacts() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let (repository, service) = configured_repository(&root);
        let source = Arc::new(FakeDownloadSource::new(1, None));
        let (supervisor, _events) = launch(&repository, source);
        let queued = service
            .download_queue_add(vec![99], "pipeline-quarantine-crash".into())
            .unwrap();
        let entry_id = queued.entries[0].entry_id.to_string();
        supervisor.enqueue_all(queued.jobs).unwrap();
        wait_for_state(&service, &entry_id, JobState::Completed, 100.0);

        let entry = DownloadEntryId::new(entry_id.clone()).unwrap();
        let bundle = repository
            .pipeline_artifact_bundle(&entry)
            .unwrap()
            .unwrap();
        let saga = QuarantineSaga {
            record_id: "crash-window-record".into(),
            entry_id: entry,
            original_relative_path: bundle.artifact.relative_directory.clone(),
            quarantine_relative_path: ArtifactRelativePath::new(
                ".atsumi-quarantine/crash-window-record/gallery-99",
            )
            .unwrap(),
            reason: "fault injection".into(),
            state: QuarantineSagaState::PendingQuarantine,
        };
        repository.pipeline_quarantine_begin(&saga).unwrap();
        let store = FilesystemArtifactStore::new();
        store
            .move_managed_directory(
                &root.join("downloads"),
                &saga.original_relative_path,
                &saga.quarantine_relative_path,
            )
            .unwrap();

        let report = supervisor.recover_startup_state().unwrap();
        assert_eq!(report.inspected_artifacts, 0);
        assert_eq!(report.verified_artifacts, 0);
        assert!(
            report.issues.iter().all(|issue| issue.entry_id != entry_id),
            "{:?}",
            report.issues
        );
        let quarantined = wait_for_state(&service, &entry_id, JobState::Quarantined, 100.0);
        assert_eq!(quarantined.state, JobState::Quarantined);
        assert!(root
            .join("downloads/.atsumi-quarantine/crash-window-record/gallery-99/manifest.json")
            .is_file());
        supervisor.shutdown_and_wait();
    }
}
