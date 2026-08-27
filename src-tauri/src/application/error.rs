use thiserror::Error;

use crate::domain::{DownloadEntryId, GalleryId, JobState, ValidationError};
use crate::source::SourceContractError;

use super::DownloadPipelineError;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database is busy: {0}")]
    Busy(String),
    #[error("database is corrupt: {0}")]
    Corrupt(String),
    #[error(
        "database schema version {found} is newer than the latest supported version {latest_supported}"
    )]
    UnsupportedSchema { found: i64, latest_supported: i64 },
    #[error("database backup failed: {0}")]
    MigrationBackup(String),
    #[error("database operation failed: {0}")]
    Other(String),
    #[error("operation is active: {0}")]
    OperationActive(String),
    #[error(transparent)]
    Source(#[from] SourceContractError),
}

impl RepositoryError {
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::Busy(_) => "DATABASE_BUSY",
            Self::Corrupt(_) => "DATABASE_CORRUPT",
            Self::UnsupportedSchema { .. } => "DATABASE_SCHEMA_NEWER",
            Self::MigrationBackup(_) => "DATABASE_BACKUP_FAILED",
            Self::Other(_) => "DATABASE_ERROR",
            Self::OperationActive(_) => "OPERATION_ACTIVE",
            Self::Source(error) => error.code.as_str(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("{resource} revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict {
        resource: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("search query {0:?} is not available")]
    QueryNotFound(String),
    #[error("gallery {0} was not found")]
    GalleryNotFound(GalleryId),
    #[error("request ID {request_id:?} was already used for a different gallery batch")]
    IdempotencyConflict { request_id: String },
    #[error("download entry {0} was not found")]
    DownloadEntryNotFound(DownloadEntryId),
    #[error("download entry {entry_id} cannot {operation} from {state}")]
    InvalidDownloadState {
        entry_id: DownloadEntryId,
        state: JobState,
        operation: &'static str,
    },
    #[error("no Auto Find refresh is currently running")]
    AutoFindNotRunning,
    #[error("no duplicate scan is currently running")]
    DuplicateScanNotRunning,
    #[error("duplicate candidate {0:?} was not found")]
    DuplicateCandidateNotFound(String),
    #[error("download overlap review {0:?} was not found")]
    DownloadOverlapReviewNotFound(String),
    #[error("download overlap decision is invalid: {0}")]
    DownloadOverlapDecisionInvalid(String),
    #[error("no internal duplicate scan is currently running")]
    InternalDuplicateScanNotRunning,
    #[error("internal duplicate entry {0:?} was not found")]
    InternalDuplicateEntryNotFound(String),
    #[error("internal removal plan is invalid: {0}")]
    InternalRemovalPlanInvalid(String),
    #[error("the application is already shutting down")]
    AppQuitInProgress,
    #[error(transparent)]
    DownloadPipeline(#[from] DownloadPipelineError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}
