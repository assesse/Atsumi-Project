mod auto_find_supervisor;
mod detail_original;
mod download_overlap;
mod download_pipeline;
mod download_supervisor;
mod duplicate_analyzer;
mod duplicate_supervisor;
mod error;
mod internal_duplicate_analyzer;
#[cfg(test)]
mod internal_duplicate_corpus;
mod internal_duplicate_supervisor;
mod ports;
mod service;

pub use auto_find_supervisor::AutoFindSupervisor;
pub(crate) use detail_original::canonical_request_id;
pub use detail_original::{
    DetailOriginalError, DetailOriginalPrepareRequest, DetailOriginalPrepared,
    DetailOriginalSupervisor,
};
pub(crate) use download_overlap::{
    analyze_download_overlap_pair, hashed_artifact, normalized_artist_keys,
    overlap_artifact_fingerprint, overlap_artists_intersect, overlap_gallery_ref,
    verified_overlap_pages,
};
pub use download_pipeline::{
    ArtifactLayout, ArtifactStore, DownloadArtifactPlan, DownloadCheckpoint,
    DownloadGallerySnapshot, DownloadOverlapRepository, DownloadPageAttempt,
    DownloadPageAttemptOutcome, DownloadPageAttemptResult, DownloadPagePayload,
    DownloadPipelineError, DownloadPipelineErrorCode, DownloadPipelineRepository, DownloadPrepared,
    DownloadRootPicker, DownloadSourceImageFormat, DownloadSourcePage, DownloadSourcePort,
    ExistingPageVerification, QuarantineSaga, QuarantineSagaState, ReconcileIssue, ReconcileReport,
    StoredPage,
};
pub use download_supervisor::DownloadSupervisor;
pub use duplicate_supervisor::{DisabledDuplicateRelationProvider, DuplicateSupervisor};
pub use error::{ApplicationError, RepositoryError};
pub use internal_duplicate_supervisor::InternalDuplicateSupervisor;
pub use ports::{
    ArtifactRepository, AutoFindSource, AutoFindSourceRequest, AutoFindSourceResult,
    AutomationRepository, DownloadMutationOutcome, DownloadQueueAddOutcome, DownloadQueueRecord,
    DownloadRepository, DuplicateRelationProvider, DuplicateRepository,
    InternalDuplicateRepository, InternalPlanPrepareOutcome, SearchRepository, StateRepository,
    TagCatalogRepository, TagCatalogSource,
};
pub use service::{ApplicationService, DownloadQueueLaunch};
