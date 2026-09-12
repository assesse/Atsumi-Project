mod auto_find_supervisor;
mod detail_original;
mod download_overlap;
mod download_pipeline;
mod download_supervisor;
mod download_tuning;
mod duplicate_analyzer;
mod duplicate_supervisor;
mod error;
mod gallery_preview;
pub(crate) mod image_work_budget;
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
pub use download_tuning::{DownloadTuningProfile, DownloadTuningStore};
pub use duplicate_supervisor::{DisabledDuplicateRelationProvider, DuplicateSupervisor};
pub use error::{ApplicationError, RepositoryError};
pub use gallery_preview::{
    ArtistPreview, GalleryPreview, GalleryPreviewRepository, GalleryPreviewService,
    GalleryPreviewSetRequest, GalleryPreviewUpdate, PreviewCandidate,
    GALLERY_PREVIEW_ALGORITHM_VERSION,
};
pub use internal_duplicate_supervisor::InternalDuplicateSupervisor;
pub use ports::{
    ArtifactRepository, AutoFindCheckpointStage, AutoFindIncrementalCheckpoint, AutoFindSource,
    AutoFindSourceRequest, AutoFindSourceResult, AutomationRepository, DownloadMutationOutcome,
    DownloadQueueAddOutcome, DownloadQueueRecord, DownloadRepository, DuplicateRelationProvider,
    DuplicateRepository, GallerySummaryCache, InternalDuplicateRepository,
    InternalPlanPrepareOutcome, SearchRepository, StateRepository, TagCatalogRepository,
    TagCatalogSource,
};
pub use service::{ApplicationService, DownloadQueueLaunch};
