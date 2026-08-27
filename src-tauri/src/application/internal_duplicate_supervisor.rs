use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

use uuid::Uuid;

use crate::{
    domain::{
        ArtifactBundle, ArtifactManifest, ArtifactRelativePath, DownloadEntryId, HashProfile,
        InternalArtifactScanProgress, InternalArtifactScanStage, InternalDuplicateReview,
        InternalDuplicateSnapshot, InternalRemovalApplyRequest, InternalRemovalPlan,
        InternalRemovalPlanRequest, InternalRemovalResult, InternalRemovalUndoRequest,
        InternalScanRequest, InternalScanRun, InternalScanSkip, InternalScanState,
        PageArtifactState, PageQuarantineSaga, PageQuarantineState,
        INTERNAL_DUPLICATE_ALGORITHM_VERSION,
    },
    thumbnail::CancellationToken,
};

use super::{
    duplicate_analyzer::{compute_page_hash, gallery_ref, verified_scan_pages, HashedArtifact},
    internal_duplicate_analyzer::detect_internal_groups_with_progress,
    ApplicationError, ArtifactLayout, ArtifactRepository, ArtifactStore, DuplicateRepository,
    InternalDuplicateRepository, InternalPlanPrepareOutcome, RepositoryError, StateRepository,
};

const PLAN_LIFETIME_MS: u128 = 15 * 60 * 1_000;
/// Deliberately exclusive: artifacts with 500 original pages are not scanned.
pub(crate) const INTERNAL_DUPLICATE_PAGE_LIMIT_EXCLUSIVE: usize = 500;

#[derive(Clone)]
pub struct InternalDuplicateSupervisor {
    inner: Arc<InternalDuplicateSupervisorInner>,
}

struct InternalDuplicateSupervisorInner {
    repository: Arc<dyn InternalDuplicateRepository>,
    duplicate_repository: Arc<dyn DuplicateRepository>,
    artifact_repository: Arc<dyn ArtifactRepository>,
    settings: Arc<dyn StateRepository>,
    store: Arc<dyn ArtifactStore>,
    events: Sender<InternalScanRun>,
    progress_events: Sender<InternalArtifactScanProgress>,
    control: Mutex<()>,
    active: Mutex<Option<ActiveRun>>,
}

struct ActiveRun {
    run_id: String,
    entry_ids: Option<BTreeSet<DownloadEntryId>>,
    cancellation: CancellationToken,
    worker: Option<JoinHandle<()>>,
    progress: Arc<ActiveProgress>,
}

#[derive(Default)]
struct ActiveProgress {
    current: Mutex<Option<InternalArtifactScanProgress>>,
    next_sequence: AtomicU64,
}

pub(crate) struct PreparedInternalScan {
    requested_entry_ids: Option<BTreeSet<DownloadEntryId>>,
    root: PathBuf,
    bundles: Vec<ArtifactBundle>,
    skips: Vec<InternalScanSkip>,
    total_artifacts: u32,
    total_pages: u32,
    profile: HashProfile,
}

impl InternalDuplicateSupervisor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository: Arc<dyn InternalDuplicateRepository>,
        duplicate_repository: Arc<dyn DuplicateRepository>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        settings: Arc<dyn StateRepository>,
        store: Arc<dyn ArtifactStore>,
        events: Sender<InternalScanRun>,
    ) -> Self {
        let (progress_events, progress_receiver) = mpsc::channel();
        drop(progress_receiver);
        Self::new_with_progress_events(
            repository,
            duplicate_repository,
            artifact_repository,
            settings,
            store,
            events,
            progress_events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_progress_events(
        repository: Arc<dyn InternalDuplicateRepository>,
        duplicate_repository: Arc<dyn DuplicateRepository>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        settings: Arc<dyn StateRepository>,
        store: Arc<dyn ArtifactStore>,
        events: Sender<InternalScanRun>,
        progress_events: Sender<InternalArtifactScanProgress>,
    ) -> Self {
        Self {
            inner: Arc::new(InternalDuplicateSupervisorInner {
                repository,
                duplicate_repository,
                artifact_repository,
                settings,
                store,
                events,
                progress_events,
                control: Mutex::new(()),
                active: Mutex::new(None),
            }),
        }
    }

    pub fn recover_interrupted(&self) -> Result<usize, ApplicationError> {
        self.inner
            .repository
            .internal_recover_interrupted()
            .map_err(Into::into)
    }

    pub fn reconcile_pending_page_moves(&self) -> Result<usize, ApplicationError> {
        let _control = self.control_lock()?;
        let root = self.download_root()?;
        let pending = self.inner.repository.internal_pending_page_sagas()?;
        let mut quarantine_plans = BTreeMap::<String, Vec<PageQuarantineSaga>>::new();
        let mut restore_records = BTreeMap::<String, Vec<PageQuarantineSaga>>::new();
        for saga in pending {
            match saga.state {
                PageQuarantineState::PendingQuarantine => quarantine_plans
                    .entry(saga.plan_id.clone())
                    .or_default()
                    .push(saga),
                PageQuarantineState::PendingRestore => restore_records
                    .entry(saga.entry_id.clone())
                    .or_default()
                    .push(saga),
                _ => {}
            }
        }
        let mut repaired = 0_usize;
        for (plan_id, sagas) in quarantine_plans {
            self.finish_quarantine(&root, &plan_id, &sagas)?;
            repaired = repaired.saturating_add(sagas.len());
        }
        for sagas in restore_records.into_values() {
            let record_ids = sagas
                .iter()
                .map(|saga| saga.record_id.clone())
                .collect::<Vec<_>>();
            self.finish_restore(&root, &record_ids, &sagas)?;
            repaired = repaired.saturating_add(sagas.len());
        }
        Ok(repaired)
    }

    pub fn snapshot(&self) -> Result<InternalDuplicateSnapshot, ApplicationError> {
        self.inner
            .repository
            .internal_snapshot()
            .map_err(Into::into)
    }

    pub fn review_get(&self, entry_id: &str) -> Result<InternalDuplicateReview, ApplicationError> {
        let entry_id = normalized_entry_id(entry_id)?;
        self.inner
            .repository
            .internal_review_get(entry_id.as_str())?
            .ok_or_else(|| ApplicationError::InternalDuplicateEntryNotFound(entry_id.to_string()))
    }

    pub fn start(&self, request: InternalScanRequest) -> Result<InternalScanRun, ApplicationError> {
        let entry_ids = normalize_scan_entry_ids(request.entry_ids)?;
        let requested_entry_ids = entry_ids.iter().cloned().collect::<BTreeSet<_>>();
        if let Some(run) = self.active_for_scope(Some(&requested_entry_ids))? {
            return Ok(run);
        }
        let prepared = self.prepare_start_scoped(Some(entry_ids))?;
        self.commit_start(prepared)
    }

    /// Full-library analysis is reserved for the explicit maintenance rebuild.
    /// User-facing scans always call `start` with selected entry IDs.
    pub fn start_all(&self) -> Result<InternalScanRun, ApplicationError> {
        if let Some(run) = self.active_for_scope(None)? {
            return Ok(run);
        }
        let prepared = self.prepare_start_all()?;
        self.commit_start(prepared)
    }

    pub(crate) fn prepare_start(
        &self,
        request: InternalScanRequest,
    ) -> Result<PreparedInternalScan, ApplicationError> {
        let entry_ids = normalize_scan_entry_ids(request.entry_ids)?;
        self.prepare_start_scoped(Some(entry_ids))
    }

    pub(crate) fn prepare_start_all(&self) -> Result<PreparedInternalScan, ApplicationError> {
        self.prepare_start_scoped(None)
    }

    fn prepare_start_scoped(
        &self,
        entry_ids: Option<Vec<DownloadEntryId>>,
    ) -> Result<PreparedInternalScan, ApplicationError> {
        let requested_entry_ids = entry_ids
            .as_ref()
            .map(|values| values.iter().cloned().collect::<BTreeSet<_>>());
        let root = self.download_root()?;
        let candidates = if let Some(entry_ids) = entry_ids {
            let mut gallery_ids = BTreeSet::new();
            let mut bundles = Vec::with_capacity(entry_ids.len());
            for entry_id in entry_ids {
                let bundle = self
                    .inner
                    .artifact_repository
                    .artifact_bundle_get(&entry_id)?
                    .filter(|bundle| verified_scan_pages(bundle).is_some())
                    .ok_or_else(|| {
                        ApplicationError::InternalDuplicateEntryNotFound(entry_id.to_string())
                    })?;
                if !gallery_ids.insert(bundle.gallery.id) {
                    return Err(crate::domain::ValidationError::new(
                        "entryIds",
                        "must not contain multiple artifacts for the same gallery",
                    )
                    .into());
                }
                bundles.push(bundle);
            }
            sort_scan_bundles(&mut bundles);
            bundles
        } else {
            select_scan_bundles(
                self.inner
                    .duplicate_repository
                    .duplicate_artifact_bundles()?,
            )
        };
        let (bundles, skips) = split_scan_bundles(candidates);
        let total_artifacts = u32::try_from(bundles.len()).unwrap_or(u32::MAX);
        let total_pages = bundles
            .iter()
            .filter_map(verified_scan_pages)
            .map(|pages| u32::try_from(pages.len()).unwrap_or(u32::MAX))
            .fold(0_u32, u32::saturating_add);
        Ok(PreparedInternalScan {
            requested_entry_ids,
            root,
            bundles,
            skips,
            total_artifacts,
            total_pages,
            profile: HashProfile::current(),
        })
    }

    pub(crate) fn commit_start(
        &self,
        prepared: PreparedInternalScan,
    ) -> Result<InternalScanRun, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        if let Some(run) = self.active_run()? {
            let same_scope = self.active_lock()?.as_ref().is_some_and(|active| {
                active.entry_ids.as_ref() == prepared.requested_entry_ids.as_ref()
            });
            return if same_scope {
                Ok(run)
            } else {
                Err(RepositoryError::OperationActive(
                    "internal duplicate scan for another selection".into(),
                )
                .into())
            };
        }
        let PreparedInternalScan {
            requested_entry_ids,
            root,
            bundles,
            skips,
            total_artifacts,
            total_pages,
            profile,
        } = prepared;
        let run = self.inner.repository.internal_scan_start(
            profile.profile_version,
            INTERNAL_DUPLICATE_ALGORITHM_VERSION,
            total_artifacts,
            total_pages,
            &skips,
        )?;
        if run.state != InternalScanState::Running {
            return Ok(run);
        }

        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let progress = Arc::new(ActiveProgress::default());
        let worker_progress = Arc::clone(&progress);
        let inner = Arc::clone(&self.inner);
        let run_id = run.run_id.clone();
        let worker_run_id = run_id.clone();
        let worker = thread::Builder::new()
            .name(format!("atsumi-internal-duplicate-{}", short_id(&run_id)))
            .spawn(move || {
                run_scan(
                    inner,
                    worker_run_id,
                    root,
                    bundles,
                    profile,
                    worker_cancellation,
                    worker_progress,
                );
            })
            .map_err(|error| {
                let _ = self.inner.repository.internal_scan_finish(
                    &run_id,
                    InternalScanState::Failed,
                    Some("INTERNAL_SCAN_WORKER_UNAVAILABLE"),
                    Some("The internal duplicate worker could not be started"),
                    &[],
                );
                RepositoryError::Other(format!(
                    "could not start internal duplicate worker: {error}"
                ))
            })?;
        *self.active_lock()? = Some(ActiveRun {
            run_id,
            entry_ids: requested_entry_ids,
            cancellation,
            worker: Some(worker),
            progress,
        });
        let _ = self.inner.events.send(run.clone());
        Ok(run)
    }

    fn active_for_scope(
        &self,
        requested_entry_ids: Option<&BTreeSet<DownloadEntryId>>,
    ) -> Result<Option<InternalScanRun>, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        let Some(run) = self.active_run()? else {
            return Ok(None);
        };
        let same_scope = self
            .active_lock()?
            .as_ref()
            .is_some_and(|active| active.entry_ids.as_ref() == requested_entry_ids);
        if same_scope {
            Ok(Some(run))
        } else {
            Err(RepositoryError::OperationActive(
                "internal duplicate scan for another selection".into(),
            )
            .into())
        }
    }

    pub fn cancel(&self) -> Result<InternalScanRun, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        let run_id = {
            let active = self.active_lock()?;
            let active = active
                .as_ref()
                .ok_or(ApplicationError::InternalDuplicateScanNotRunning)?;
            active.cancellation.cancel();
            active.run_id.clone()
        };
        let run = self
            .inner
            .repository
            .internal_scan_finish(
                &run_id,
                InternalScanState::Cancelled,
                Some("INTERNAL_SCAN_CANCELLED"),
                Some("The internal duplicate scan was cancelled"),
                &[],
            )?
            .ok_or(ApplicationError::InternalDuplicateScanNotRunning)?;
        if let Some(mut active) = self.active_lock()?.take() {
            if let Some(worker) = active.worker.take() {
                let _ = worker.join();
            }
            clear_progress(&active.progress);
        }
        let _ = self.inner.events.send(run.clone());
        Ok(run)
    }

    pub fn removal_plan(
        &self,
        request: InternalRemovalPlanRequest,
    ) -> Result<InternalRemovalPlan, ApplicationError> {
        let entry_id = normalized_entry_id(&request.entry_id)?;
        validate_selections(&request.selections)?;
        let bundle = self
            .inner
            .artifact_repository
            .artifact_bundle_get(&entry_id)?
            .ok_or_else(|| {
                ApplicationError::InternalDuplicateEntryNotFound(entry_id.to_string())
            })?;
        let removals = request
            .selections
            .iter()
            .flat_map(|selection| selection.remove_source_pages.iter().copied())
            .collect::<BTreeSet<_>>();
        let bytes_to_quarantine = bundle
            .pages
            .iter()
            .filter(|page| removals.contains(&page.page_id.source_page_number.get()))
            .map(|page| page.byte_length.unwrap_or(0))
            .fold(0_u64, u64::saturating_add);
        let plan = InternalRemovalPlan {
            plan_id: format!("internal-plan-{}", Uuid::new_v4()),
            entry_id: entry_id.to_string(),
            selections: request.selections,
            files_to_quarantine: u32::try_from(removals.len()).unwrap_or(u32::MAX),
            bytes_to_quarantine,
            expires_at: unix_ms().saturating_add(PLAN_LIFETIME_MS).to_string(),
        };
        match self.inner.repository.internal_plan_prepare(&plan)? {
            InternalPlanPrepareOutcome::Prepared(plan) => Ok(plan),
            InternalPlanPrepareOutcome::EntryNotFound => Err(
                ApplicationError::InternalDuplicateEntryNotFound(entry_id.to_string()),
            ),
            InternalPlanPrepareOutcome::RevisionConflict {
                group_id,
                actual_revision,
            } => {
                let expected = plan
                    .selections
                    .iter()
                    .find(|selection| selection.group_id == group_id)
                    .map_or(0, |selection| selection.expected_revision);
                Err(ApplicationError::RevisionConflict {
                    resource: "internalDuplicateGroup",
                    expected,
                    actual: actual_revision,
                })
            }
            InternalPlanPrepareOutcome::InvalidSelection(message) => {
                Err(ApplicationError::InternalRemovalPlanInvalid(message))
            }
        }
    }

    pub fn removal_apply(
        &self,
        request: InternalRemovalApplyRequest,
    ) -> Result<InternalRemovalResult, ApplicationError> {
        let _control = self.control_lock()?;
        let root = self.download_root()?;
        let reason = request.reason.trim();
        if reason.is_empty() || reason.len() > 500 {
            return Err(
                crate::domain::ValidationError::new("reason", "must be 1..=500 bytes").into(),
            );
        }
        let sagas = self
            .inner
            .repository
            .internal_removal_begin(&request.plan.plan_id, reason)?;
        self.finish_quarantine(&root, &request.plan.plan_id, &sagas)?;
        let review = self.review_get(&request.plan.entry_id)?;
        let records = review
            .quarantine_records
            .iter()
            .filter(|record| record.plan_id == request.plan.plan_id)
            .cloned()
            .collect();
        Ok(InternalRemovalResult { review, records })
    }

    pub fn removal_undo(
        &self,
        request: InternalRemovalUndoRequest,
    ) -> Result<InternalRemovalResult, ApplicationError> {
        let _control = self.control_lock()?;
        if request.record_ids.is_empty() {
            return Err(
                crate::domain::ValidationError::new("recordIds", "must not be empty").into(),
            );
        }
        let root = self.download_root()?;
        let sagas = self
            .inner
            .repository
            .internal_restore_begin(&request.record_ids)?;
        let entry_ids = sagas
            .iter()
            .map(|saga| saga.entry_id.as_str())
            .collect::<BTreeSet<_>>();
        if entry_ids.len() != 1 {
            return Err(ApplicationError::InternalRemovalPlanInvalid(
                "Undo must contain pages from one download entry".into(),
            ));
        }
        let entry_id = sagas
            .first()
            .map(|saga| saga.entry_id.clone())
            .ok_or_else(|| {
                ApplicationError::InternalRemovalPlanInvalid(
                    "No restorable quarantine records were selected".into(),
                )
            })?;
        self.finish_restore(&root, &request.record_ids, &sagas)?;
        let review = self.review_get(&entry_id)?;
        let records = review
            .quarantine_records
            .iter()
            .filter(|record| request.record_ids.contains(&record.record_id))
            .cloned()
            .collect();
        Ok(InternalRemovalResult { review, records })
    }

    pub fn shutdown_and_wait(&self) {
        let _control = self.control_lock().ok();
        let active = self.active_lock().ok().and_then(|mut active| active.take());
        if let Some(mut active) = active {
            active.cancellation.cancel();
            let _ = self.inner.repository.internal_scan_finish(
                &active.run_id,
                InternalScanState::Cancelled,
                Some("INTERNAL_SCAN_APP_EXIT"),
                Some("The application closed during internal duplicate scanning"),
                &[],
            );
            if let Some(worker) = active.worker.take() {
                let _ = worker.join();
            }
            clear_progress(&active.progress);
        }
    }

    /// Read a stable active-run projection without leaking selection details
    /// or retaining a completed worker as active work.
    pub fn active_run_snapshot(&self) -> Result<Option<InternalScanRun>, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        self.active_run()
    }

    /// Returns only ephemeral worker progress; no repository state or file I/O
    /// is performed while the progress mutex is held.
    pub fn active_artifact_progress(
        &self,
    ) -> Result<Option<InternalArtifactScanProgress>, ApplicationError> {
        self.reap_finished_worker();
        let progress = self
            .active_lock()?
            .as_ref()
            .map(|active| Arc::clone(&active.progress));
        let Some(progress) = progress else {
            return Ok(None);
        };
        progress
            .current
            .lock()
            .map(|current| current.clone())
            .map_err(|_| {
                RepositoryError::Other("internal duplicate progress mutex was poisoned".into())
                    .into()
            })
    }

    fn finish_quarantine(
        &self,
        root: &Path,
        plan_id: &str,
        sagas: &[PageQuarantineSaga],
    ) -> Result<(), ApplicationError> {
        let entry_id = sagas
            .first()
            .map(|saga| DownloadEntryId::new(&saga.entry_id))
            .transpose()?
            .ok_or_else(|| {
                ApplicationError::InternalRemovalPlanInvalid(
                    "Removal plan contains no pages".into(),
                )
            })?;
        for saga in sagas {
            self.ensure_move(
                root,
                &saga.original_relative_path,
                &saga.quarantine_relative_path,
            )?;
        }
        let mut bundle = self
            .inner
            .artifact_repository
            .artifact_bundle_get(&entry_id)?
            .ok_or_else(|| {
                ApplicationError::InternalDuplicateEntryNotFound(entry_id.to_string())
            })?;
        for saga in sagas {
            let page = bundle
                .pages
                .iter_mut()
                .find(|page| page.page_id.source_page_number == saga.source_page)
                .ok_or_else(|| {
                    ApplicationError::InternalRemovalPlanInvalid(
                        "The artifact no longer contains a selected source page".into(),
                    )
                })?;
            page.relative_path = ArtifactRelativePath::new(&saga.quarantine_relative_path)?;
            page.state = PageArtifactState::Quarantined;
            page.excluded = true;
        }
        self.write_bundle_manifest(root, &bundle)?;
        let _ = self.inner.repository.internal_removal_complete(plan_id)?;
        Ok(())
    }

    fn finish_restore(
        &self,
        root: &Path,
        record_ids: &[String],
        sagas: &[PageQuarantineSaga],
    ) -> Result<(), ApplicationError> {
        let entry_id = sagas
            .first()
            .map(|saga| DownloadEntryId::new(&saga.entry_id))
            .transpose()?
            .ok_or_else(|| {
                ApplicationError::InternalRemovalPlanInvalid("Undo contains no pages".into())
            })?;
        for saga in sagas {
            self.ensure_move(
                root,
                &saga.quarantine_relative_path,
                &saga.original_relative_path,
            )?;
        }
        let mut bundle = self
            .inner
            .artifact_repository
            .artifact_bundle_get(&entry_id)?
            .ok_or_else(|| {
                ApplicationError::InternalDuplicateEntryNotFound(entry_id.to_string())
            })?;
        for saga in sagas {
            let page = bundle
                .pages
                .iter_mut()
                .find(|page| page.page_id.source_page_number == saga.source_page)
                .ok_or_else(|| {
                    ApplicationError::InternalRemovalPlanInvalid(
                        "The artifact no longer contains a quarantined source page".into(),
                    )
                })?;
            page.relative_path = ArtifactRelativePath::new(&saga.original_relative_path)?;
            page.state = PageArtifactState::Present;
            page.excluded = false;
        }
        self.write_bundle_manifest(root, &bundle)?;
        let _ = self
            .inner
            .repository
            .internal_restore_complete(record_ids)?;
        Ok(())
    }

    fn ensure_move(
        &self,
        root: &Path,
        source: &str,
        destination: &str,
    ) -> Result<(), ApplicationError> {
        let source = ArtifactRelativePath::new(source)?;
        let destination = ArtifactRelativePath::new(destination)?;
        let source_exists = self.inner.store.managed_path_exists(root, &source)?;
        let destination_exists = self.inner.store.managed_path_exists(root, &destination)?;
        match (source_exists, destination_exists) {
            (true, false) => self
                .inner
                .store
                .move_managed_file(root, &source, &destination)
                .map_err(Into::into),
            (false, true) => Ok(()),
            (true, true) => Err(ApplicationError::InternalRemovalPlanInvalid(
                "Both source and quarantine files exist; no file was overwritten".into(),
            )),
            (false, false) => Err(ApplicationError::InternalRemovalPlanInvalid(
                "Neither source nor quarantine file exists; reconcile the artifact first".into(),
            )),
        }
    }

    fn write_bundle_manifest(
        &self,
        root: &Path,
        bundle: &crate::domain::ArtifactBundle,
    ) -> Result<(), ApplicationError> {
        let manifest_relative_path =
            bundle
                .artifact
                .manifest_relative_path
                .clone()
                .ok_or_else(|| {
                    RepositoryError::Corrupt("complete artifact has no manifest path".into())
                })?;
        let layout = ArtifactLayout {
            root: root.to_path_buf(),
            relative_directory: bundle.artifact.relative_directory.clone(),
            manifest_relative_path,
        };
        let manifest = ArtifactManifest::from_bundle(bundle)?;
        self.inner.store.write_manifest(&layout, &manifest)?;
        Ok(())
    }

    fn download_root(&self) -> Result<PathBuf, ApplicationError> {
        let settings = self.inner.settings.settings_get()?;
        if settings.download_root.trim().is_empty() {
            return Err(super::DownloadPipelineError::root_required().into());
        }
        self.inner
            .store
            .validate_download_root(&PathBuf::from(settings.download_root))
            .map_err(Into::into)
    }

    fn control_lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, ApplicationError> {
        self.inner.control.lock().map_err(|_| {
            RepositoryError::Other("internal duplicate control mutex was poisoned".into()).into()
        })
    }

    fn active_lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<ActiveRun>>, ApplicationError> {
        self.inner.active.lock().map_err(|_| {
            RepositoryError::Other("internal duplicate active mutex was poisoned".into()).into()
        })
    }

    fn active_run(&self) -> Result<Option<InternalScanRun>, ApplicationError> {
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
            .internal_snapshot()?
            .run
            .filter(|run| run.run_id == run_id && run.state == InternalScanState::Running))
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
    inner: Arc<InternalDuplicateSupervisorInner>,
    run_id: String,
    root: PathBuf,
    bundles: Vec<crate::domain::ArtifactBundle>,
    profile: HashProfile,
    cancellation: CancellationToken,
    progress: Arc<ActiveProgress>,
) {
    let result = scan_inner(
        &inner,
        &run_id,
        &root,
        &bundles,
        &profile,
        &cancellation,
        &progress,
    );
    clear_progress(&progress);
    if let Err(error) = result {
        if !cancellation.is_cancelled()
            && inner
                .repository
                .internal_scan_is_running(&run_id)
                .unwrap_or(false)
        {
            if let Ok(Some(run)) = inner.repository.internal_scan_finish(
                &run_id,
                InternalScanState::Failed,
                Some("INTERNAL_SCAN_FAILED"),
                Some(&stable_scan_error(&error)),
                &[],
            ) {
                let _ = inner.events.send(run);
            }
        }
    }
}

struct ProgressUpdate<'a> {
    run_id: &'a str,
    bundle: &'a ArtifactBundle,
    artifact_index: u32,
    total_artifacts: u32,
    processed_pages: u32,
    total_pages: u32,
    compared_pairs: u64,
    total_pairs: u64,
    stage: InternalArtifactScanStage,
}

fn publish_progress(
    inner: &InternalDuplicateSupervisorInner,
    state: &ActiveProgress,
    update: ProgressUpdate<'_>,
) -> Result<(), RepositoryError> {
    let sequence = state.next_sequence.fetch_add(1, Ordering::Relaxed) + 1;
    let progress = InternalArtifactScanProgress {
        run_id: update.run_id.to_owned(),
        sequence,
        entry_id: update.bundle.artifact.entry_id.to_string(),
        gallery_id: update.bundle.gallery.id,
        artifact_index: update.artifact_index,
        total_artifacts: update.total_artifacts,
        processed_pages: update.processed_pages.min(update.total_pages),
        total_pages: update.total_pages,
        compared_pairs: update.compared_pairs.min(update.total_pairs),
        total_pairs: update.total_pairs,
        progress_percent: artifact_progress_percent(
            update.processed_pages,
            update.total_pages,
            update.compared_pairs,
            update.total_pairs,
            update.stage,
        ),
        stage: update.stage,
    };
    *state.current.lock().map_err(|_| {
        RepositoryError::Other("internal duplicate progress mutex was poisoned".into())
    })? = Some(progress.clone());
    let _ = inner.progress_events.send(progress);
    Ok(())
}

fn clear_progress(state: &ActiveProgress) {
    if let Ok(mut current) = state.current.lock() {
        *current = None;
    }
}

fn artifact_progress_percent(
    processed_pages: u32,
    total_pages: u32,
    compared_pairs: u64,
    total_pairs: u64,
    stage: InternalArtifactScanStage,
) -> u32 {
    const HASHING_PERCENT: u128 = 35;
    const COMPARING_PERCENT: u128 = 60;
    match stage {
        InternalArtifactScanStage::Hashing => {
            if total_pages == 0 {
                return 0;
            }
            u32::try_from(
                u128::from(processed_pages.min(total_pages)).saturating_mul(HASHING_PERCENT)
                    / u128::from(total_pages),
            )
            .unwrap_or(HASHING_PERCENT as u32)
            .min(HASHING_PERCENT as u32)
        }
        InternalArtifactScanStage::Comparing => {
            if total_pairs == 0 {
                return (HASHING_PERCENT + COMPARING_PERCENT) as u32;
            }
            let pair_percent = u128::from(compared_pairs.min(total_pairs))
                .saturating_mul(COMPARING_PERCENT)
                / u128::from(total_pairs);
            u32::try_from(HASHING_PERCENT.saturating_add(pair_percent))
                .unwrap_or((HASHING_PERCENT + COMPARING_PERCENT) as u32)
                .min((HASHING_PERCENT + COMPARING_PERCENT) as u32)
        }
        InternalArtifactScanStage::Finalizing => 99,
    }
}

fn progress_report_interval(total: u64) -> u64 {
    total.div_ceil(100).max(1)
}

fn should_report_progress_step(completed: u64, total: u64, next_report: &mut u64) -> bool {
    if completed < *next_report && completed != total {
        return false;
    }
    *next_report = completed.saturating_add(progress_report_interval(total));
    true
}

fn scan_inner(
    inner: &InternalDuplicateSupervisorInner,
    run_id: &str,
    root: &Path,
    bundles: &[crate::domain::ArtifactBundle],
    profile: &HashProfile,
    cancellation: &CancellationToken,
    progress: &Arc<ActiveProgress>,
) -> Result<(), RepositoryError> {
    let mut compared_pairs = 0_u64;
    for (artifact_index, bundle) in bundles.iter().enumerate() {
        if cancelled(inner, run_id, cancellation)? {
            return Ok(());
        }
        let pages = verified_scan_pages(bundle).ok_or_else(|| {
            RepositoryError::Corrupt("artifact lost internal scan eligibility".into())
        })?;
        let artifact_index = u32::try_from(artifact_index + 1).unwrap_or(u32::MAX);
        let total_artifacts = u32::try_from(bundles.len()).unwrap_or(u32::MAX);
        let total_pages = u32::try_from(pages.len()).unwrap_or(u32::MAX);
        let page_count = u64::from(total_pages);
        let total_pairs = page_count.saturating_mul(page_count.saturating_sub(1)) / 2;
        let mut next_hash_report = progress_report_interval(page_count);
        publish_progress(
            inner,
            progress,
            ProgressUpdate {
                run_id,
                bundle,
                artifact_index,
                total_artifacts,
                processed_pages: 0,
                total_pages,
                compared_pairs: 0,
                total_pairs,
                stage: InternalArtifactScanStage::Hashing,
            },
        )?;
        let mut hashes = Vec::with_capacity(pages.len());
        for (page_index, page) in pages.iter().enumerate() {
            if cancelled(inner, run_id, cancellation)? {
                return Ok(());
            }
            let sha = page
                .sha256
                .as_ref()
                .expect("verified pages always contain SHA-256");
            let hash = if let Some(hash) = inner.duplicate_repository.duplicate_page_hash_get(
                bundle.artifact.entry_id.as_str(),
                page.page_id.source_page_number,
                profile.profile_version,
                sha.as_str(),
            )? {
                hash
            } else {
                let bytes = inner
                    .store
                    .read_verified_page_bytes(root, page)
                    .map_err(|error| RepositoryError::Other(error.to_string()))?;
                let hash = compute_page_hash(
                    bundle.artifact.entry_id.as_str(),
                    bundle.gallery.id,
                    page.page_id.source_page_number,
                    sha.clone(),
                    &bytes,
                    profile,
                )?;
                inner
                    .duplicate_repository
                    .duplicate_page_hash_upsert(&hash)?;
                hash
            };
            hashes.push(hash);
            let processed_pages = u32::try_from(page_index + 1).unwrap_or(u32::MAX);
            if should_report_progress_step(
                u64::from(processed_pages),
                page_count,
                &mut next_hash_report,
            ) {
                publish_progress(
                    inner,
                    progress,
                    ProgressUpdate {
                        run_id,
                        bundle,
                        artifact_index,
                        total_artifacts,
                        processed_pages,
                        total_pages,
                        compared_pairs: 0,
                        total_pairs,
                        stage: InternalArtifactScanStage::Hashing,
                    },
                )?;
            }
        }
        hashes.sort_by_key(|hash| hash.source_page_number);
        let artifact = HashedArtifact {
            gallery: gallery_ref(bundle, hashes.len() as u32),
            pages: hashes,
        };
        publish_progress(
            inner,
            progress,
            ProgressUpdate {
                run_id,
                bundle,
                artifact_index,
                total_artifacts,
                processed_pages: total_pages,
                total_pages,
                compared_pairs: 0,
                total_pairs,
                stage: InternalArtifactScanStage::Comparing,
            },
        )?;
        let mut progress_error = None;
        let detection = detect_internal_groups_with_progress(
            run_id,
            &artifact,
            profile,
            |artifact_compared_pairs, reported_total_pairs| {
                if progress_error.is_some() {
                    return;
                }
                progress_error = publish_progress(
                    inner,
                    progress,
                    ProgressUpdate {
                        run_id,
                        bundle,
                        artifact_index,
                        total_artifacts,
                        processed_pages: total_pages,
                        total_pages,
                        compared_pairs: artifact_compared_pairs,
                        total_pairs: reported_total_pairs,
                        stage: InternalArtifactScanStage::Comparing,
                    },
                )
                .err();
            },
        );
        if let Some(error) = progress_error {
            return Err(error);
        }
        compared_pairs = compared_pairs.saturating_add(detection.compared_pairs);
        publish_progress(
            inner,
            progress,
            ProgressUpdate {
                run_id,
                bundle,
                artifact_index,
                total_artifacts,
                processed_pages: total_pages,
                total_pages,
                compared_pairs: detection.compared_pairs,
                total_pairs,
                stage: InternalArtifactScanStage::Finalizing,
            },
        )?;
        for record in detection.groups {
            if cancelled(inner, run_id, cancellation)? {
                return Ok(());
            }
            let _ = inner.repository.internal_group_replace(&record)?;
        }
        if let Some(run) =
            inner
                .repository
                .internal_scan_progress(run_id, artifact_index, compared_pairs)?
        {
            let _ = inner.events.send(run);
        }
    }
    if cancelled(inner, run_id, cancellation)? {
        return Ok(());
    }
    let completed_gallery_ids = bundles
        .iter()
        .map(|bundle| bundle.gallery.id)
        .collect::<Vec<_>>();
    clear_progress(progress);
    if let Some(run) = inner.repository.internal_scan_finish(
        run_id,
        InternalScanState::Completed,
        None,
        None,
        &completed_gallery_ids,
    )? {
        let _ = inner.events.send(run);
    }
    Ok(())
}

fn cancelled(
    inner: &InternalDuplicateSupervisorInner,
    run_id: &str,
    cancellation: &CancellationToken,
) -> Result<bool, RepositoryError> {
    Ok(cancellation.is_cancelled() || !inner.repository.internal_scan_is_running(run_id)?)
}

fn select_scan_bundles(
    bundles: Vec<crate::domain::ArtifactBundle>,
) -> Vec<crate::domain::ArtifactBundle> {
    let mut bundles = bundles
        .into_iter()
        .filter(|bundle| verified_scan_pages(bundle).is_some())
        .collect::<Vec<_>>();
    sort_scan_bundles(&mut bundles);
    bundles.dedup_by(|left, right| left.gallery.id == right.gallery.id);
    bundles
}

fn sort_scan_bundles(bundles: &mut [crate::domain::ArtifactBundle]) {
    bundles.sort_by(|left, right| {
        left.gallery.id.cmp(&right.gallery.id).then_with(|| {
            right
                .artifact
                .completed_at
                .cmp(&left.artifact.completed_at)
                .then_with(|| right.artifact.revision.cmp(&left.artifact.revision))
                .then_with(|| left.artifact.entry_id.cmp(&right.artifact.entry_id))
        })
    });
}

fn canonical_page_count(bundle: &crate::domain::ArtifactBundle) -> usize {
    let recorded = bundle.pages.len();
    let artifact = bundle.artifact.expected_page_count as usize;
    let gallery = bundle.gallery.metadata.source_page_count as usize;
    recorded.max(artifact).max(gallery)
}

fn split_scan_bundles(
    bundles: Vec<crate::domain::ArtifactBundle>,
) -> (Vec<crate::domain::ArtifactBundle>, Vec<InternalScanSkip>) {
    let mut eligible = Vec::new();
    let mut skips = Vec::new();
    for bundle in bundles {
        let page_count = canonical_page_count(&bundle);
        if page_count >= INTERNAL_DUPLICATE_PAGE_LIMIT_EXCLUSIVE {
            skips.push(InternalScanSkip {
                entry_id: bundle.artifact.entry_id.to_string(),
                gallery_id: bundle.gallery.id,
                title: bundle.gallery.metadata.title.clone(),
                page_count: u32::try_from(page_count).unwrap_or(u32::MAX),
                reason: "page_limit".into(),
            });
        } else {
            eligible.push(bundle);
        }
    }
    (eligible, skips)
}

fn validate_selections(
    selections: &[crate::domain::InternalRemovalSelection],
) -> Result<(), ApplicationError> {
    if selections.is_empty() {
        return Err(crate::domain::ValidationError::new("selections", "must not be empty").into());
    }
    let mut groups = BTreeSet::new();
    for selection in selections {
        if selection.group_id.trim().is_empty()
            || !groups.insert(selection.group_id.trim().to_owned())
            || selection.keep_source_page == 0
            || selection.remove_source_pages.is_empty()
            || selection.remove_source_pages.contains(&0)
        {
            return Err(crate::domain::ValidationError::new(
                "selections",
                "must contain unique groups and positive keep/remove source pages",
            )
            .into());
        }
    }
    Ok(())
}

fn normalize_scan_entry_ids(values: Vec<String>) -> Result<Vec<DownloadEntryId>, ApplicationError> {
    if values.is_empty() {
        return Err(crate::domain::ValidationError::new("entryIds", "must not be empty").into());
    }
    if values.len() > 200 {
        return Err(crate::domain::ValidationError::new(
            "entryIds",
            "must contain at most 200 entries",
        )
        .into());
    }
    let mut unique = BTreeSet::new();
    for value in values {
        unique.insert(DownloadEntryId::new(value)?);
    }
    Ok(unique.into_iter().collect())
}

fn normalized_entry_id(value: &str) -> Result<DownloadEntryId, ApplicationError> {
    DownloadEntryId::new(value).map_err(Into::into)
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn stable_scan_error(error: &RepositoryError) -> String {
    match error {
        RepositoryError::Busy(_) => "The internal duplicate database is busy; retry the scan",
        RepositoryError::Corrupt(_) => {
            "Verified artifact metadata changed; reconcile downloads and retry"
        }
        _ => "A verified artifact page could not be analyzed safely",
    }
    .into()
}

fn short_id(value: &str) -> &str {
    value.rsplit('-').next().unwrap_or("run")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        io::Cursor,
        sync::{mpsc, Arc},
        time::Duration,
    };

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    use crate::{
        application::{
            ApplicationService, ArtifactRepository, ArtifactStore, InternalDuplicateRepository,
            StateRepository,
        },
        domain::{
            ArtifactBundle, ArtifactManifest, ArtifactRelativePath, ArtifactSha256,
            ArtifactStorageFormat, DownloadArtifact, DownloadArtifactState, Gallery, GalleryId,
            GalleryMetadata, InternalRemovalApplyRequest, InternalRemovalPlanRequest,
            InternalRemovalSelection, InternalRemovalUndoRequest, InternalScanRequest,
            InternalScanState, PageArtifact, PageArtifactState, PageQuarantineState,
            SourcePageNumber, INTERNAL_DUPLICATE_ALGORITHM_VERSION,
        },
        infrastructure::{FilesystemArtifactStore, SqliteRepository},
        thumbnail::CancellationToken,
    };

    use super::{
        should_report_progress_step, ActiveProgress, ActiveRun, ApplicationError, ArtifactLayout,
        InternalDuplicateSupervisor, RepositoryError,
    };

    fn webp_fixture(seed: u8) -> Vec<u8> {
        let mut image = RgbaImage::new(24, 24);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if seed != 2 {
                Rgba([
                    seed.saturating_mul(24),
                    40_u8.saturating_add(seed),
                    180,
                    255,
                ])
            } else if (x + y) % 3 == 0 {
                Rgba([20, 81, 112, 255])
            } else {
                Rgba([224, 229, 219, 255])
            };
        }
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut cursor, ImageFormat::WebP)
            .unwrap();
        cursor.into_inner()
    }

    #[test]
    fn hashing_progress_is_bounded_to_roughly_one_percent_updates() {
        let total = 499_u64;
        let mut next_report = super::progress_report_interval(total);
        let updates = (1..=total)
            .filter(|completed| should_report_progress_step(*completed, total, &mut next_report))
            .collect::<Vec<_>>();

        // The separate initial hashing event plus these step events remains
        // bounded to 101 emissions per artifact.
        assert!(updates.len() <= 100, "updates={}", updates.len() + 1);
        assert_eq!(updates.last(), Some(&total));
        assert!(updates.windows(2).all(|window| window[0] < window[1]));
    }

    #[test]
    fn artifact_progress_percent_reserves_visible_ranges_for_each_stage() {
        use crate::domain::InternalArtifactScanStage;

        assert_eq!(
            super::artifact_progress_percent(0, 211, 0, 22_155, InternalArtifactScanStage::Hashing),
            0
        );
        assert_eq!(
            super::artifact_progress_percent(
                105,
                211,
                0,
                22_155,
                InternalArtifactScanStage::Hashing
            ),
            17
        );
        assert_eq!(
            super::artifact_progress_percent(
                211,
                211,
                0,
                22_155,
                InternalArtifactScanStage::Hashing
            ),
            35
        );
        assert_eq!(
            super::artifact_progress_percent(
                211,
                211,
                0,
                22_155,
                InternalArtifactScanStage::Comparing
            ),
            35
        );
        assert_eq!(
            super::artifact_progress_percent(
                211,
                211,
                22_155,
                22_155,
                InternalArtifactScanStage::Comparing
            ),
            95
        );
        assert_eq!(
            super::artifact_progress_percent(1, 1, 0, 0, InternalArtifactScanStage::Comparing),
            95
        );
        assert_eq!(
            super::artifact_progress_percent(
                211,
                211,
                22_155,
                22_155,
                InternalArtifactScanStage::Finalizing
            ),
            99
        );
    }

    #[test]
    fn active_run_snapshot_includes_running_and_excludes_terminal_runs() {
        let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let run = InternalDuplicateRepository::internal_scan_start(
            repository.as_ref(),
            1,
            INTERNAL_DUPLICATE_ALGORITHM_VERSION,
            0,
            0,
            &[],
        )
        .expect("seed running internal duplicate scan");
        let store: Arc<dyn ArtifactStore> = Arc::new(FilesystemArtifactStore::new());
        let (events, _receiver) = mpsc::channel();
        let supervisor = InternalDuplicateSupervisor::new(
            repository.clone(),
            repository.clone(),
            repository.clone(),
            repository.clone(),
            store,
            events,
        );
        *supervisor.active_lock().unwrap() = Some(ActiveRun {
            run_id: run.run_id.clone(),
            entry_ids: None,
            cancellation: CancellationToken::new(),
            worker: None,
            progress: Arc::new(ActiveProgress::default()),
        });

        let active = supervisor
            .active_run_snapshot()
            .expect("read active internal duplicate run")
            .expect("running scan should be included");
        assert_eq!(active.run_id, run.run_id);
        assert_eq!(active.state, InternalScanState::Running);

        InternalDuplicateRepository::internal_scan_finish(
            repository.as_ref(),
            &run.run_id,
            InternalScanState::Completed,
            None,
            None,
            &[],
        )
        .expect("finish internal duplicate scan");
        assert!(supervisor
            .active_run_snapshot()
            .expect("read terminal internal duplicate run")
            .is_none());
        *supervisor.active_lock().unwrap() = None;
    }

    #[test]
    fn commit_rechecks_for_a_same_scope_run_started_during_unlocked_preflight() {
        let temporary = tempdir().expect("create internal scan root");
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
        let store: Arc<dyn ArtifactStore> = Arc::new(FilesystemArtifactStore::new());
        let (events, _receiver) = mpsc::channel();
        let supervisor = InternalDuplicateSupervisor::new(
            repository.clone(),
            repository.clone(),
            repository.clone(),
            repository.clone(),
            store,
            events,
        );

        let prepared = supervisor
            .prepare_start_all()
            .expect("prepare full-library scan input");
        let conflicting_prepared = supervisor
            .prepare_start_all()
            .expect("prepare a second full-library scan input");
        let existing = InternalDuplicateRepository::internal_scan_start(
            repository.as_ref(),
            1,
            INTERNAL_DUPLICATE_ALGORITHM_VERSION,
            0,
            0,
            &[],
        )
        .expect("start competing internal duplicate scan");
        *supervisor.active_lock().unwrap() = Some(ActiveRun {
            run_id: existing.run_id.clone(),
            entry_ids: None,
            cancellation: CancellationToken::new(),
            worker: None,
            progress: Arc::new(ActiveProgress::default()),
        });

        let returned = supervisor
            .commit_start(prepared)
            .expect("reuse the same-scope run that won the commit race");
        assert_eq!(returned.run_id, existing.run_id);
        supervisor
            .active_lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .entry_ids = Some(BTreeSet::from([crate::domain::DownloadEntryId::new(
            "selected-entry",
        )
        .unwrap()]));
        assert!(matches!(
            supervisor.commit_start(conflicting_prepared),
            Err(ApplicationError::Repository(
                RepositoryError::OperationActive(_)
            ))
        ));
        *supervisor.active_lock().unwrap() = None;
        InternalDuplicateRepository::internal_scan_finish(
            repository.as_ref(),
            &existing.run_id,
            InternalScanState::Cancelled,
            None,
            None,
            &[],
        )
        .expect("finish seeded internal duplicate run");
    }

    #[test]
    fn exact_internal_pages_are_planned_quarantined_and_undoable_without_renumbering() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("downloads");
        std::fs::create_dir_all(&root).unwrap();
        let repository = std::sync::Arc::new(SqliteRepository::open_in_memory().unwrap());
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

        let service = ApplicationService::new(repository.clone())
            .with_download_repository(repository.clone());
        let launch = service
            .download_queue_add(vec![101], "internal-duplicate-test".into())
            .unwrap();
        let entry_id = launch.entries[0].entry_id.clone();
        {
            let connection = repository.connection().unwrap();
            connection
                .execute(
                    "UPDATE download_entries SET state = 'completed', progress = 100 WHERE entry_id = ?1",
                    [entry_id.as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE download_jobs SET state = 'completed', completed_units = total_units WHERE entry_id = ?1",
                    [entry_id.as_str()],
                )
                .unwrap();
        }

        let gallery_id = GalleryId::new(101).unwrap();
        let relative_directory = ArtifactRelativePath::new("gallery-101").unwrap();
        let manifest_relative_path =
            ArtifactRelativePath::new("gallery-101/manifest.json").unwrap();
        std::fs::create_dir_all(root.join(relative_directory.as_str())).unwrap();
        let repeated_bytes = webp_fixture(2);
        let pages = (1_u32..=8)
            .map(|source_page| {
                let bytes = if matches!(source_page, 2 | 8) {
                    repeated_bytes.clone()
                } else {
                    webp_fixture(source_page as u8)
                };
                let sha = ArtifactSha256::new(format!("{:x}", Sha256::digest(&bytes))).unwrap();
                let relative_path =
                    ArtifactRelativePath::new(format!("gallery-101/{source_page:04}.webp"))
                        .unwrap();
                std::fs::write(root.join(relative_path.as_str()), &bytes).unwrap();
                PageArtifact::new(
                    entry_id.clone(),
                    gallery_id,
                    SourcePageNumber::new(source_page).unwrap(),
                    relative_path,
                    PageArtifactState::Present,
                    Some(bytes.len() as u64),
                )
                .unwrap()
                .with_verification(
                    sha,
                    ArtifactStorageFormat::Webp,
                    "fixture-source",
                    "2026-08-16T00:00:00.000Z",
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let gallery = Gallery::new(
            gallery_id,
            0,
            GalleryMetadata::new("Internal repeat", Some("artist".into()), None, 8).unwrap(),
        );
        let artifact = DownloadArtifact::new(
            entry_id.clone(),
            gallery_id,
            0,
            relative_directory.clone(),
            8,
            DownloadArtifactState::Complete,
        )
        .unwrap()
        .with_manifest(
            manifest_relative_path.clone(),
            1,
            "test",
            1,
            "2026-08-16T00:00:00.000Z",
        )
        .unwrap();
        let bundle = ArtifactBundle::new(gallery, artifact, pages).unwrap();
        ArtifactRepository::artifact_bundle_replace(repository.as_ref(), &bundle).unwrap();
        let store: std::sync::Arc<dyn ArtifactStore> =
            std::sync::Arc::new(FilesystemArtifactStore::new());
        store
            .write_manifest(
                &ArtifactLayout {
                    root: root.clone(),
                    relative_directory,
                    manifest_relative_path,
                },
                &ArtifactManifest::from_bundle(&bundle).unwrap(),
            )
            .unwrap();

        let (events, _receiver) = mpsc::channel();
        let (progress_events, progress_receiver) = mpsc::channel();
        let supervisor = InternalDuplicateSupervisor::new_with_progress_events(
            repository.clone(),
            repository.clone(),
            repository.clone(),
            repository.clone(),
            std::sync::Arc::clone(&store),
            events,
            progress_events,
        );
        assert!(supervisor
            .start(InternalScanRequest { entry_ids: vec![] })
            .is_err());
        assert!(supervisor
            .start(InternalScanRequest {
                entry_ids: vec!["missing-entry".into()],
            })
            .is_err());
        supervisor
            .start(InternalScanRequest {
                entry_ids: vec![entry_id.to_string()],
            })
            .unwrap();
        let snapshot = (0..100)
            .find_map(|_| {
                let snapshot = supervisor.snapshot().unwrap();
                if snapshot
                    .run
                    .as_ref()
                    .is_some_and(|run| run.state.as_str() == "completed")
                {
                    Some(snapshot)
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("internal scan completes");
        let progress_updates = progress_receiver.try_iter().collect::<Vec<_>>();
        assert!(progress_updates
            .iter()
            .any(|progress| progress.stage == crate::domain::InternalArtifactScanStage::Hashing));
        assert!(progress_updates.iter().any(|progress| {
            progress.stage == crate::domain::InternalArtifactScanStage::Comparing
        }));
        let finalizing = progress_updates
            .iter()
            .find(|progress| progress.stage == crate::domain::InternalArtifactScanStage::Finalizing)
            .expect("finalizing progress is emitted");
        assert_eq!(finalizing.artifact_index, 1);
        assert_eq!(finalizing.total_artifacts, 1);
        assert_eq!(finalizing.processed_pages, 8);
        assert_eq!(finalizing.total_pages, 8);
        assert_eq!(finalizing.compared_pairs, 28);
        assert_eq!(finalizing.total_pairs, 28);
        assert_eq!(finalizing.progress_percent, 99);
        assert!(progress_updates
            .windows(2)
            .all(|window| window[0].sequence < window[1].sequence));
        assert!(progress_updates
            .iter()
            .all(|progress| progress.run_id == snapshot.run.as_ref().unwrap().run_id));
        assert!(supervisor
            .active_artifact_progress()
            .expect("read terminal progress")
            .is_none());
        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(
            snapshot.groups[0]
                .pages
                .iter()
                .map(|page| page.source_page)
                .collect::<Vec<_>>(),
            vec![2, 8]
        );

        let group = &snapshot.groups[0];
        let plan = supervisor
            .removal_plan(InternalRemovalPlanRequest {
                entry_id: entry_id.to_string(),
                selections: vec![InternalRemovalSelection {
                    group_id: group.group_id.clone(),
                    expected_revision: group.revision,
                    keep_source_page: 2,
                    remove_source_pages: vec![8],
                }],
            })
            .unwrap();
        let applied = supervisor
            .removal_apply(InternalRemovalApplyRequest {
                plan,
                reason: "integration review".into(),
            })
            .unwrap();
        assert_eq!(applied.records.len(), 1);
        assert_eq!(applied.records[0].state, PageQuarantineState::Quarantined);
        assert!(root.join("gallery-101/0002.webp").is_file());
        assert!(!root.join("gallery-101/0008.webp").exists());
        assert!(root
            .join(&applied.records[0].quarantine_relative_path)
            .is_file());
        let stored = ArtifactRepository::artifact_bundle_get(repository.as_ref(), &entry_id)
            .unwrap()
            .unwrap();
        let quarantined = stored
            .pages
            .iter()
            .find(|page| page.page_id.source_page_number.get() == 8)
            .unwrap();
        assert_eq!(quarantined.state, PageArtifactState::Quarantined);
        assert!(quarantined.excluded);

        let restored = supervisor
            .removal_undo(InternalRemovalUndoRequest {
                record_ids: applied
                    .records
                    .iter()
                    .map(|record| record.record_id.clone())
                    .collect(),
            })
            .unwrap();
        assert_eq!(restored.records[0].state, PageQuarantineState::Restored);
        assert!(root.join("gallery-101/0008.webp").is_file());
        assert!(!root
            .join(&applied.records[0].quarantine_relative_path)
            .exists());
        assert_eq!(restored.review.groups[0].pages[1].source_page, 8);

        // Simulate a crash after the durable intent and file move, but before
        // the manifest/DB completion. Startup reconciliation must finish the
        // same plan without deleting or renumbering the source page.
        let reopened = &restored.review.groups[0];
        let recovery_plan = supervisor
            .removal_plan(InternalRemovalPlanRequest {
                entry_id: entry_id.to_string(),
                selections: vec![InternalRemovalSelection {
                    group_id: reopened.group_id.clone(),
                    expected_revision: reopened.revision,
                    keep_source_page: 2,
                    remove_source_pages: vec![8],
                }],
            })
            .unwrap();
        let pending = InternalDuplicateRepository::internal_removal_begin(
            repository.as_ref(),
            &recovery_plan.plan_id,
            "crash recovery test",
        )
        .unwrap();
        let source = ArtifactRelativePath::new(&pending[0].original_relative_path).unwrap();
        let destination = ArtifactRelativePath::new(&pending[0].quarantine_relative_path).unwrap();
        store
            .move_managed_file(&root, &source, &destination)
            .unwrap();
        assert_eq!(supervisor.reconcile_pending_page_moves().unwrap(), 1);
        let recovered = supervisor.review_get(entry_id.as_str()).unwrap();
        let recovered_record = recovered
            .quarantine_records
            .iter()
            .find(|record| record.plan_id == recovery_plan.plan_id)
            .unwrap();
        assert_eq!(recovered_record.state, PageQuarantineState::Quarantined);
        assert_eq!(recovered_record.source_page, 8);
        let final_restore = supervisor
            .removal_undo(InternalRemovalUndoRequest {
                record_ids: vec![recovered_record.record_id.clone()],
            })
            .unwrap();
        assert_eq!(
            final_restore.records[0].state,
            PageQuarantineState::Restored
        );
        assert!(root.join("gallery-101/0008.webp").is_file());
    }
}
