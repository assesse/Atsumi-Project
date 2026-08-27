use crate::domain::{
    ArtifactBundle, AutoFindCandidateRecord, AutoFindCutoffEvidence, AutoFindExclusionResult,
    AutoFindHistoryMode, AutoFindRun, AutoFindRunState, AutoFindSnapshot, AutoFindTruncation,
    DownloadEntry, DownloadEntryId, DownloadJobDescriptor, DownloadJobProjection,
    DownloadListRequest, DownloadPage, DuplicateCandidateRecord, DuplicateDecisionApplyOutcome,
    DuplicateDecisionRequest, DuplicatePageHash, DuplicateReview, DuplicateScanRun,
    DuplicateScanState, DuplicateSnapshot, ExplorationDataResetResult, ExplorationExclusion,
    ExplorationExclusionRestoreResult, ExternalRelationEvidence, FavoriteKey,
    FavoriteMutationResult, FavoriteRecord, FixtureDownloadJobStep, GalleryDetail, GalleryId,
    GalleryPage, InternalDuplicateReview, InternalDuplicateSnapshot, InternalGroupRecord,
    InternalRemovalPlan, InternalRemovalSelection, InternalScanRun, InternalScanState, JobRef,
    JobState, PageQuarantineRecord, PageQuarantineSaga, SearchHistoryEntry, SearchRequest,
    SearchSubmission, SettingsSnapshot, SourcePageNumber, TagCatalogEntry, TagCatalogStatus,
    TagSuggestion, TagSuggestionRequest, WindowPlacementSnapshot,
};

use super::RepositoryError;

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadQueueRecord {
    pub entries: Vec<DownloadEntry>,
    pub jobs: Vec<DownloadJobDescriptor>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadQueueAddOutcome {
    Added(DownloadQueueRecord),
    IdempotencyConflict,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadMutationOutcome<T> {
    Applied(T),
    EntryNotFound(DownloadEntryId),
    InvalidState {
        entry_id: DownloadEntryId,
        state: JobState,
    },
}

pub trait StateRepository: Send + Sync {
    fn settings_get(&self) -> Result<SettingsSnapshot, RepositoryError>;

    fn settings_compare_and_set(
        &self,
        next: &SettingsSnapshot,
        expected_revision: u64,
    ) -> Result<bool, RepositoryError>;

    fn window_placement_get(&self) -> Result<WindowPlacementSnapshot, RepositoryError>;

    fn window_placement_compare_and_set(
        &self,
        next: &WindowPlacementSnapshot,
        expected_revision: u64,
    ) -> Result<bool, RepositoryError>;
}

pub trait ArtifactRepository: Send + Sync {
    fn artifact_bundle_replace(&self, bundle: &ArtifactBundle) -> Result<(), RepositoryError>;

    fn artifact_bundle_get(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<Option<ArtifactBundle>, RepositoryError>;
}

pub trait SearchRepository: Send + Sync {
    fn search_submit(&self, request: &SearchRequest) -> Result<SearchSubmission, RepositoryError>;

    fn search_page_get(
        &self,
        query_id: &str,
        page: u32,
    ) -> Result<Option<GalleryPage>, RepositoryError>;

    fn search_page_get_cancellable(
        &self,
        query_id: &str,
        page: u32,
        cancellation: &crate::thumbnail::CancellationToken,
    ) -> Result<Option<GalleryPage>, RepositoryError> {
        if cancellation.is_cancelled() {
            return Err(RepositoryError::Source(
                crate::source::SourceContractError::cancelled(),
            ));
        }
        let result = self.search_page_get(query_id, page)?;
        if cancellation.is_cancelled() {
            return Err(RepositoryError::Source(
                crate::source::SourceContractError::cancelled(),
            ));
        }
        Ok(result)
    }

    fn gallery_detail_get(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Option<GalleryDetail>, RepositoryError>;
}

/// A fixed-source catalog. Implementations never accept caller-provided URLs.
pub trait TagCatalogSource: Send + Sync {
    fn tag_catalog_fetch_all(&self) -> Result<Vec<TagCatalogEntry>, RepositoryError>;
}

pub trait TagCatalogRepository: Send + Sync {
    fn tag_catalog_status(&self) -> Result<TagCatalogStatus, RepositoryError>;
    fn tag_catalog_record_attempt(&self) -> Result<(), RepositoryError>;
    fn tag_catalog_replace(
        &self,
        entries: &[TagCatalogEntry],
    ) -> Result<TagCatalogStatus, RepositoryError>;
    fn tag_catalog_record_failure(&self, code: &str, message: &str) -> Result<(), RepositoryError>;
    fn tag_suggestions_search(
        &self,
        request: &TagSuggestionRequest,
    ) -> Result<Vec<TagSuggestion>, RepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFindSourceRequest {
    pub artist: String,
    pub languages: Vec<crate::domain::Language>,
    pub newer_than_gallery_id: Option<crate::domain::GalleryId>,
    pub candidate_limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFindSourceResult {
    pub candidate_ids: Vec<crate::domain::GalleryId>,
    pub eligible_count: u32,
    pub limit: u32,
    pub truncated_reason: Option<String>,
}

/// Source-specific artist discovery. This intentionally bypasses the user
/// search cap so Auto Find can apply its persisted history cutoff before any
/// gallery metadata is fetched.
pub trait AutoFindSource: Send + Sync {
    fn auto_find_artist_plan(
        &self,
        request: &AutoFindSourceRequest,
        cancellation: &crate::thumbnail::CancellationToken,
    ) -> Result<AutoFindSourceResult, RepositoryError>;

    fn auto_find_gallery_summary(
        &self,
        gallery_id: crate::domain::GalleryId,
        cancellation: &crate::thumbnail::CancellationToken,
    ) -> Result<Option<crate::domain::GallerySummary>, RepositoryError>;
}

pub trait AutomationRepository: Send + Sync {
    fn favorites_list(&self) -> Result<Vec<FavoriteRecord>, RepositoryError>;

    fn favorite_set(
        &self,
        key: &FavoriteKey,
        enabled: bool,
    ) -> Result<FavoriteMutationResult, RepositoryError>;

    fn search_history_record(
        &self,
        request: &SearchRequest,
    ) -> Result<SearchHistoryEntry, RepositoryError>;

    fn search_history_list(&self, limit: u32) -> Result<Vec<SearchHistoryEntry>, RepositoryError>;

    fn auto_find_recover_interrupted(&self) -> Result<usize, RepositoryError>;

    fn auto_find_owned_cutoffs(
        &self,
        artists: &[String],
    ) -> Result<Vec<AutoFindCutoffEvidence>, RepositoryError>;

    fn auto_find_start(
        &self,
        total_favorites: u32,
        history_mode: AutoFindHistoryMode,
        cutoff_evidence: &[AutoFindCutoffEvidence],
    ) -> Result<AutoFindRun, RepositoryError>;

    fn auto_find_truncation_add(
        &self,
        run_id: &str,
        truncation: &AutoFindTruncation,
    ) -> Result<(), RepositoryError>;

    fn auto_find_candidate_add(
        &self,
        candidate: &AutoFindCandidateRecord,
    ) -> Result<Option<AutoFindRun>, RepositoryError>;

    fn auto_find_progress(
        &self,
        run_id: &str,
        completed_favorites: u32,
    ) -> Result<Option<AutoFindRun>, RepositoryError>;

    fn auto_find_finish(
        &self,
        run_id: &str,
        state: AutoFindRunState,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<Option<AutoFindRun>, RepositoryError>;

    fn auto_find_is_running(&self, run_id: &str) -> Result<bool, RepositoryError>;

    fn auto_find_snapshot(&self) -> Result<AutoFindSnapshot, RepositoryError>;

    fn auto_find_exclude(
        &self,
        gallery_ids: &[GalleryId],
        reason: &str,
    ) -> Result<AutoFindExclusionResult, RepositoryError>;

    fn exploration_exclusions_list(&self) -> Result<Vec<ExplorationExclusion>, RepositoryError>;

    fn exploration_exclusions_restore(
        &self,
        gallery_ids: &[GalleryId],
    ) -> Result<ExplorationExclusionRestoreResult, RepositoryError>;

    fn exploration_data_reset(&self) -> Result<ExplorationDataResetResult, RepositoryError>;
}

pub trait DuplicateRepository: Send + Sync {
    fn duplicate_artifact_bundles(&self) -> Result<Vec<ArtifactBundle>, RepositoryError>;

    fn duplicate_page_hash_get(
        &self,
        entry_id: &str,
        source_page_number: SourcePageNumber,
        profile_version: u32,
        artifact_sha256: &str,
    ) -> Result<Option<DuplicatePageHash>, RepositoryError>;

    fn duplicate_page_hash_upsert(&self, hash: &DuplicatePageHash) -> Result<(), RepositoryError>;

    fn duplicate_recover_interrupted(&self) -> Result<usize, RepositoryError>;

    fn duplicate_scan_start(
        &self,
        profile_version: u32,
        total_artifacts: u32,
        total_pairs: u64,
    ) -> Result<DuplicateScanRun, RepositoryError>;

    fn duplicate_scan_progress(
        &self,
        run_id: &str,
        hashed_artifacts: u32,
        compared_pairs: u64,
    ) -> Result<Option<DuplicateScanRun>, RepositoryError>;

    fn duplicate_candidate_replace(
        &self,
        record: &DuplicateCandidateRecord,
    ) -> Result<Option<DuplicateScanRun>, RepositoryError>;

    fn duplicate_scan_finish(
        &self,
        run_id: &str,
        state: DuplicateScanState,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<Option<DuplicateScanRun>, RepositoryError>;

    fn duplicate_scan_is_running(&self, run_id: &str) -> Result<bool, RepositoryError>;

    fn duplicate_snapshot(&self) -> Result<DuplicateSnapshot, RepositoryError>;

    fn duplicate_review_get(
        &self,
        candidate_id: &str,
    ) -> Result<Option<DuplicateReview>, RepositoryError>;

    fn duplicate_decision_apply(
        &self,
        request: &DuplicateDecisionRequest,
    ) -> Result<DuplicateDecisionApplyOutcome, RepositoryError>;
}

pub trait DuplicateRelationProvider: Send + Sync {
    fn enabled(&self) -> bool;

    fn relation(
        &self,
        parent_gallery_id: GalleryId,
        candidate_gallery_id: GalleryId,
    ) -> Result<Option<ExternalRelationEvidence>, RepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalPlanPrepareOutcome {
    Prepared(InternalRemovalPlan),
    EntryNotFound,
    RevisionConflict {
        group_id: String,
        actual_revision: u64,
    },
    InvalidSelection(String),
}

pub trait InternalDuplicateRepository: Send + Sync {
    fn internal_recover_interrupted(&self) -> Result<usize, RepositoryError>;

    fn internal_scan_start(
        &self,
        profile_version: u32,
        algorithm_version: u32,
        total_artifacts: u32,
        total_pages: u32,
        skips: &[crate::domain::InternalScanSkip],
    ) -> Result<InternalScanRun, RepositoryError>;

    fn internal_scan_progress(
        &self,
        run_id: &str,
        scanned_artifacts: u32,
        compared_pairs: u64,
    ) -> Result<Option<InternalScanRun>, RepositoryError>;

    fn internal_group_replace(
        &self,
        record: &InternalGroupRecord,
    ) -> Result<Option<InternalScanRun>, RepositoryError>;

    fn internal_scan_finish(
        &self,
        run_id: &str,
        state: InternalScanState,
        error_code: Option<&str>,
        error_message: Option<&str>,
        completed_gallery_ids: &[GalleryId],
    ) -> Result<Option<InternalScanRun>, RepositoryError>;

    fn internal_scan_is_running(&self, run_id: &str) -> Result<bool, RepositoryError>;
    fn internal_snapshot(&self) -> Result<InternalDuplicateSnapshot, RepositoryError>;
    fn internal_review_get(
        &self,
        entry_id: &str,
    ) -> Result<Option<InternalDuplicateReview>, RepositoryError>;

    fn internal_plan_prepare(
        &self,
        plan: &InternalRemovalPlan,
    ) -> Result<InternalPlanPrepareOutcome, RepositoryError>;

    fn internal_removal_begin(
        &self,
        plan_id: &str,
        reason: &str,
    ) -> Result<Vec<PageQuarantineSaga>, RepositoryError>;
    fn internal_removal_complete(
        &self,
        plan_id: &str,
    ) -> Result<Vec<PageQuarantineRecord>, RepositoryError>;
    fn internal_restore_begin(
        &self,
        record_ids: &[String],
    ) -> Result<Vec<PageQuarantineSaga>, RepositoryError>;
    fn internal_restore_complete(
        &self,
        record_ids: &[String],
    ) -> Result<Vec<PageQuarantineRecord>, RepositoryError>;
    fn internal_pending_page_sagas(&self) -> Result<Vec<PageQuarantineSaga>, RepositoryError>;
    fn internal_plan_selections(
        &self,
        plan_id: &str,
    ) -> Result<Vec<InternalRemovalSelection>, RepositoryError>;
}

pub trait DownloadRepository: Send + Sync {
    fn download_recover_interrupted(&self) -> Result<usize, RepositoryError>;

    fn download_queue_add(
        &self,
        request_id: &str,
        galleries: &[GalleryId],
    ) -> Result<DownloadQueueAddOutcome, RepositoryError>;

    fn download_entries_list(
        &self,
        request: &DownloadListRequest,
    ) -> Result<DownloadPage, RepositoryError>;

    fn download_active_count(&self) -> Result<u64, RepositoryError>;

    /// Canonical identities for work that would be interrupted by app exit.
    /// The ordering must be stable so callers can derive a progress-insensitive
    /// work-set fingerprint.
    fn download_active_entry_ids(&self) -> Result<Vec<DownloadEntryId>, RepositoryError>;

    fn download_retry(
        &self,
        entry_ids: &[DownloadEntryId],
    ) -> Result<DownloadMutationOutcome<Vec<JobRef>>, RepositoryError>;

    fn download_cancel(
        &self,
        entry_ids: &[DownloadEntryId],
    ) -> Result<DownloadMutationOutcome<Vec<DownloadEntry>>, RepositoryError>;

    fn fixture_download_job_advance(
        &self,
        job_id: &str,
        worker_attempt: u64,
        step: FixtureDownloadJobStep,
    ) -> Result<DownloadJobProjection, RepositoryError>;
}
