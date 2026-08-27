use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::{
    domain::{
        ArtifactBundle, ArtifactManifest, ArtifactRelativePath, ArtifactSha256,
        ArtifactStorageFormat, DownloadEntryId, DownloadJobDescriptor, DownloadJobProjection,
        DownloadOverlapCandidateIdentity, DownloadOverlapDecisionApplyOutcome,
        DownloadOverlapDecisionRequest, DownloadOverlapReview, DownloadOverlapReviewDraft,
        DuplicatePageHash, Gallery, GalleryId, JobRef, JobState, PageArtifact, SourcePageNumber,
    },
    source::{SourceCandidateDiagnostic, SourceContractError},
    thumbnail::CancellationToken,
};

use super::RepositoryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadSourceImageFormat {
    Webp,
    Jpeg,
    Png,
    Avif,
}

impl DownloadSourceImageFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Avif => "avif",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSourcePage {
    pub source_page_number: SourcePageNumber,
    pub source_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadGallerySnapshot {
    pub gallery: Gallery,
    pub source_revision: String,
    pub pages: Vec<DownloadSourcePage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPagePayload {
    pub source_page_number: SourcePageNumber,
    pub bytes: Vec<u8>,
    pub source_revision: String,
    pub source_format: DownloadSourceImageFormat,
    pub width: u32,
    pub height: u32,
    pub candidate_index: u32,
    pub candidate_diagnostics: Vec<SourceCandidateDiagnostic>,
}

pub trait DownloadSourcePort: Send + Sync {
    fn gallery_snapshot(
        &self,
        gallery_id: GalleryId,
        cancellation: &CancellationToken,
    ) -> Result<DownloadGallerySnapshot, SourceContractError>;

    fn download_page(
        &self,
        gallery_id: GalleryId,
        source_page_number: SourcePageNumber,
        cancellation: &CancellationToken,
    ) -> Result<DownloadPagePayload, SourceContractError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactLayout {
    pub root: PathBuf,
    pub relative_directory: ArtifactRelativePath,
    pub manifest_relative_path: ArtifactRelativePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPage {
    pub source_page_number: SourcePageNumber,
    pub relative_path: ArtifactRelativePath,
    pub byte_length: u64,
    pub sha256: ArtifactSha256,
    pub storage_format: ArtifactStorageFormat,
    pub source_revision: String,
    pub verified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistingPageVerification {
    Missing,
    Verified(StoredPage),
    Invalid {
        relative_path: ArtifactRelativePath,
        reason: &'static str,
    },
}

pub trait ArtifactStore: Send + Sync {
    fn validate_download_root(&self, root: &Path) -> Result<PathBuf, DownloadPipelineError>;

    fn prepare_layout(
        &self,
        root: &Path,
        relative_directory: &ArtifactRelativePath,
        allow_existing: bool,
    ) -> Result<ArtifactLayout, DownloadPipelineError>;

    fn verify_existing_page(
        &self,
        layout: &ArtifactLayout,
        source_page_number: SourcePageNumber,
        source_revision: &str,
        expected: Option<&StoredPage>,
    ) -> Result<ExistingPageVerification, DownloadPipelineError>;

    fn store_page(
        &self,
        layout: &ArtifactLayout,
        page: &DownloadPagePayload,
        cancellation: &CancellationToken,
    ) -> Result<StoredPage, DownloadPipelineError>;

    fn write_manifest(
        &self,
        layout: &ArtifactLayout,
        manifest: &ArtifactManifest,
    ) -> Result<(), DownloadPipelineError>;

    fn read_manifest(
        &self,
        layout: &ArtifactLayout,
    ) -> Result<Option<ArtifactManifest>, DownloadPipelineError>;

    fn first_verified_page_path(
        &self,
        root: &Path,
        bundle: &ArtifactBundle,
    ) -> Result<PathBuf, DownloadPipelineError>;

    fn artifact_directory_path(
        &self,
        root: &Path,
        relative_directory: &ArtifactRelativePath,
    ) -> Result<PathBuf, DownloadPipelineError>;

    fn open_with_default_viewer(&self, path: &Path) -> Result<(), DownloadPipelineError>;

    fn move_managed_directory(
        &self,
        root: &Path,
        source: &ArtifactRelativePath,
        destination: &ArtifactRelativePath,
    ) -> Result<(), DownloadPipelineError>;

    fn move_managed_file(
        &self,
        root: &Path,
        source: &ArtifactRelativePath,
        destination: &ArtifactRelativePath,
    ) -> Result<(), DownloadPipelineError>;

    fn managed_path_exists(
        &self,
        root: &Path,
        relative_path: &ArtifactRelativePath,
    ) -> Result<bool, DownloadPipelineError>;

    fn read_verified_page_bytes(
        &self,
        root: &Path,
        page: &PageArtifact,
    ) -> Result<Vec<u8>, DownloadPipelineError>;
}

pub trait DownloadRootPicker: Send + Sync {
    fn pick_download_root(&self) -> Result<Option<PathBuf>, DownloadPipelineError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadArtifactPlan {
    pub descriptor: DownloadJobDescriptor,
    pub gallery: Gallery,
    pub source_revision: String,
    pub root_snapshot: PathBuf,
    pub relative_directory: ArtifactRelativePath,
    pub manifest_relative_path: ArtifactRelativePath,
    pub source_pages: Vec<DownloadSourcePage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadCheckpoint {
    pub page: StoredPage,
    pub excluded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadPrepared {
    pub projection: DownloadJobProjection,
    pub checkpoints: Vec<DownloadCheckpoint>,
    pub relative_directory: ArtifactRelativePath,
    pub manifest_relative_path: ArtifactRelativePath,
    pub root_snapshot: PathBuf,
    pub artifact_created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPageAttempt {
    pub descriptor: DownloadJobDescriptor,
    pub source_page_number: SourcePageNumber,
    pub candidate_index: u32,
    pub candidate_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPageAttemptResult {
    pub attempt: DownloadPageAttempt,
    pub outcome: DownloadPageAttemptOutcome,
    pub bytes_received: Option<u64>,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPageAttemptOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineSagaState {
    PendingQuarantine,
    Quarantined,
    PendingRestore,
    Restored,
}

impl QuarantineSagaState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingQuarantine => "pending_quarantine",
            Self::Quarantined => "quarantined",
            Self::PendingRestore => "pending_restore",
            Self::Restored => "restored",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineSaga {
    pub record_id: String,
    pub entry_id: DownloadEntryId,
    pub original_relative_path: ArtifactRelativePath,
    pub quarantine_relative_path: ArtifactRelativePath,
    pub reason: String,
    pub state: QuarantineSagaState,
}

impl DownloadPageAttemptOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

pub trait DownloadPipelineRepository: Send + Sync {
    fn pipeline_begin(
        &self,
        descriptor: &DownloadJobDescriptor,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn pipeline_prepare(
        &self,
        plan: &DownloadArtifactPlan,
    ) -> Result<DownloadPrepared, RepositoryError>;

    fn pipeline_page_attempt_start(
        &self,
        attempt: &DownloadPageAttempt,
    ) -> Result<(), RepositoryError>;

    fn pipeline_page_attempt_finish(
        &self,
        result: &DownloadPageAttemptResult,
    ) -> Result<(), RepositoryError>;

    fn pipeline_page_verified(
        &self,
        descriptor: &DownloadJobDescriptor,
        page: &StoredPage,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn pipeline_stage(
        &self,
        descriptor: &DownloadJobDescriptor,
        state: JobState,
        message: &'static str,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn pipeline_complete(
        &self,
        descriptor: &DownloadJobDescriptor,
        manifest: &ArtifactManifest,
        manifest_relative_path: &ArtifactRelativePath,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn pipeline_fail(
        &self,
        descriptor: &DownloadJobDescriptor,
        code: &str,
        message: &str,
        retryable: bool,
    ) -> Result<Option<DownloadJobProjection>, RepositoryError>;

    fn pipeline_resume_interrupted(&self) -> Result<Vec<DownloadJobDescriptor>, RepositoryError>;

    fn pipeline_descriptors_for_jobs(
        &self,
        jobs: &[JobRef],
    ) -> Result<Vec<DownloadJobDescriptor>, RepositoryError>;

    fn pipeline_mark_artifact_issue(
        &self,
        entry_id: &DownloadEntryId,
        code: &str,
        message: &str,
    ) -> Result<Option<DownloadJobProjection>, RepositoryError>;

    fn pipeline_artifact_bundle(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<Option<ArtifactBundle>, RepositoryError>;

    fn pipeline_artifact_root(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<PathBuf, RepositoryError>;

    fn pipeline_artifact_bundles(&self) -> Result<Vec<ArtifactBundle>, RepositoryError>;

    fn pipeline_quarantine_begin(&self, saga: &QuarantineSaga) -> Result<(), RepositoryError>;

    fn pipeline_quarantine_complete(
        &self,
        record_id: &str,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn pipeline_restore_begin(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<QuarantineSaga, RepositoryError>;

    fn pipeline_restore_complete(
        &self,
        record_id: &str,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn pipeline_pending_quarantine_sagas(&self) -> Result<Vec<QuarantineSaga>, RepositoryError>;
}

pub trait DownloadOverlapRepository: DownloadPipelineRepository {
    fn overlap_candidate_identities(
        &self,
        incoming_entry_id: &DownloadEntryId,
    ) -> Result<Vec<DownloadOverlapCandidateIdentity>, RepositoryError>;

    fn overlap_page_hash_get(
        &self,
        entry_id: &str,
        source_page_number: SourcePageNumber,
        profile_version: u32,
        artifact_sha256: &str,
    ) -> Result<Option<DuplicatePageHash>, RepositoryError>;

    fn overlap_page_hash_upsert(&self, hash: &DuplicatePageHash) -> Result<(), RepositoryError>;

    fn overlap_pair_policy_exists(
        &self,
        incoming_fingerprint: &str,
        existing_fingerprint: &str,
        profile_version: u32,
        policy_version: u32,
    ) -> Result<bool, RepositoryError>;

    fn overlap_review_pause(
        &self,
        descriptor: &DownloadJobDescriptor,
        draft: &DownloadOverlapReviewDraft,
    ) -> Result<DownloadJobProjection, RepositoryError>;

    fn overlap_review_get(
        &self,
        review_id: &str,
    ) -> Result<Option<DownloadOverlapReview>, RepositoryError>;

    fn overlap_decision_apply(
        &self,
        request: &DownloadOverlapDecisionRequest,
        verified_incoming_fingerprint: &str,
        verified_existing_fingerprints: &[(String, String)],
    ) -> Result<DownloadOverlapDecisionApplyOutcome, RepositoryError>;

    fn overlap_review_requeue_stale(
        &self,
        review_id: &str,
        expected_revision: u64,
    ) -> Result<DownloadOverlapDecisionApplyOutcome, RepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileIssue {
    pub entry_id: String,
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileReport {
    pub inspected_artifacts: u64,
    pub verified_artifacts: u64,
    pub resumed_jobs: u64,
    pub issues: Vec<ReconcileIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPipelineErrorCode {
    RootRequired,
    RootSelectionCancelled,
    RootUnavailable,
    PathOutsideRoot,
    Filesystem,
    ImageDecodeFailed,
    ImageEncodeFailed,
    ManifestInvalid,
    HashMismatch,
    ArtifactMissing,
    Cancelled,
    WorkerUnavailable,
    QuarantineConflict,
    DestinationOccupied,
    OverlapCheckFailed,
}

impl DownloadPipelineErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RootRequired => "DOWNLOAD_ROOT_REQUIRED",
            Self::RootSelectionCancelled => "DOWNLOAD_ROOT_SELECTION_CANCELLED",
            Self::RootUnavailable => "DOWNLOAD_ROOT_UNAVAILABLE",
            Self::PathOutsideRoot => "FILESYSTEM_PATH_OUTSIDE_ROOT",
            Self::Filesystem => "FILESYSTEM_ERROR",
            Self::ImageDecodeFailed => "IMAGE_DECODE_FAILED",
            Self::ImageEncodeFailed => "IMAGE_ENCODE_FAILED",
            Self::ManifestInvalid => "ARTIFACT_MANIFEST_INVALID",
            Self::HashMismatch => "ARTIFACT_HASH_MISMATCH",
            Self::ArtifactMissing => "FILESYSTEM_MISSING",
            Self::Cancelled => "REQUEST_CANCELLED",
            Self::WorkerUnavailable => "DOWNLOAD_WORKER_UNAVAILABLE",
            Self::QuarantineConflict => "QUARANTINE_CONFLICT",
            Self::DestinationOccupied => "ARTIFACT_DESTINATION_OCCUPIED",
            Self::OverlapCheckFailed => "DOWNLOAD_OVERLAP_CHECK_FAILED",
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct DownloadPipelineError {
    pub code: DownloadPipelineErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl DownloadPipelineError {
    pub fn new(
        code: DownloadPipelineErrorCode,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    pub fn root_required() -> Self {
        Self::new(
            DownloadPipelineErrorCode::RootRequired,
            "Choose a download folder before starting a download",
            false,
        )
    }

    pub fn cancelled() -> Self {
        Self::new(
            DownloadPipelineErrorCode::Cancelled,
            "The download was cancelled",
            false,
        )
    }
}
