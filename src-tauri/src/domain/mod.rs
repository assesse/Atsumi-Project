mod artifact;
mod artifact_path;
mod auto_find;
mod download;
mod download_overlap;
mod duplicate;
mod gallery;
mod internal_duplicate;
mod job;
mod maintenance;
mod search;
mod settings;
mod tag_catalog;
mod window_placement;

pub use artifact::{
    ArtifactBundle, ArtifactConversionPolicy, ArtifactManifest, ArtifactManifestGallery,
    ArtifactManifestPage, ArtifactRelativePath, ArtifactSha256, ArtifactStorageFormat,
    DownloadArtifact, DownloadArtifactState, DownloadEntryId, PageArtifact, PageArtifactState,
    ARTIFACT_MANIFEST_SCHEMA_VERSION, HASH_PROFILE_VERSION,
};
pub use artifact_path::{
    plan_artifact_relative_directory, validate_folder_name_template, DEFAULT_FOLDER_NAME_TEMPLATE,
    MAX_FOLDER_COMPONENT_UTF16, MAX_MANAGED_ABSOLUTE_PATH_UTF16,
};
pub use auto_find::{
    AutoFindCandidate, AutoFindCandidateRecord, AutoFindCutoffEvidence, AutoFindExclusionResult,
    AutoFindRun, AutoFindRunState, AutoFindSnapshot, AutoFindTruncation, ExplorationExclusion,
    ExplorationExclusionKind, ExplorationExclusionReason, ExplorationExclusionRestoreResult,
    FavoriteKey, FavoriteMutationResult, FavoriteNamespace, FavoriteRecord, SearchHistoryEntry,
};
pub use download::{DownloadEntry, DownloadListRequest, DownloadPage, DownloadReviewKind};
pub use download_overlap::{
    DownloadOverlapCandidate, DownloadOverlapCandidateIdentity, DownloadOverlapDecisionAction,
    DownloadOverlapDecisionApplied, DownloadOverlapDecisionApplyOutcome,
    DownloadOverlapDecisionRequest, DownloadOverlapDecisionResult, DownloadOverlapGalleryRef,
    DownloadOverlapPagePair, DownloadOverlapPairDecision, DownloadOverlapRelation,
    DownloadOverlapReview, DownloadOverlapReviewDraft, DownloadOverlapReviewState,
    DOWNLOAD_OVERLAP_MAX_STORED_PAGE_PAIRS, DOWNLOAD_OVERLAP_POLICY_VERSION,
};
pub use duplicate::{
    DuplicateCandidate, DuplicateCandidateRecord, DuplicateDecisionAction,
    DuplicateDecisionApplyOutcome, DuplicateDecisionHistory, DuplicateDecisionRequest,
    DuplicateEvidence, DuplicateEvidenceKind, DuplicateGalleryRef, DuplicatePageHash,
    DuplicatePagePair, DuplicateRelation, DuplicateReview, DuplicateScanRun, DuplicateScanState,
    DuplicateSnapshot, ExternalRelationEvidence, HashProfile, SeriesGroup,
    DUPLICATE_HASH_ALGORITHM_VERSION, DUPLICATE_HASH_PROFILE_VERSION,
};
pub use gallery::{Gallery, GalleryId, GalleryMetadata, GalleryPageId, SourcePageNumber};
pub use internal_duplicate::{
    InternalArtifactScanProgress, InternalArtifactScanStage, InternalDuplicateGroup,
    InternalDuplicateReview, InternalDuplicateSnapshot, InternalGroupRecord, InternalMatchKind,
    InternalPageEvidence, InternalRemovalApplyRequest, InternalRemovalPlan,
    InternalRemovalPlanRequest, InternalRemovalResult, InternalRemovalSelection,
    InternalRemovalUndoRequest, InternalScanRequest, InternalScanRun, InternalScanSkip,
    InternalScanState, PageQuarantineRecord, PageQuarantineSaga, PageQuarantineState,
    INTERNAL_DUPLICATE_ALGORITHM_VERSION,
};
pub use job::{
    DownloadChangedEvent, DownloadJobDescriptor, DownloadJobProjection, FixtureDownloadJobStep,
    JobEvent, JobRef, JobState,
};
pub use maintenance::{
    ExplorationDataResetRequest, ExplorationDataResetResult, MaintenanceAction, MaintenancePreview,
    MaintenanceResult, EXPLORATION_DATA_RESET_CONFIRMATION, FACTORY_RESET_CONFIRMATION,
};
pub(crate) use search::normalize_search_tags;
pub use search::{
    GalleryDetail, GalleryPage, GalleryPageDimension, GallerySummary, Language, SearchRequest,
    SearchSort, SearchSubmission,
};
pub use settings::{
    download_root_for_display, gallery_preview_preset_widths, is_gallery_preview_width,
    normalize_collapsed_group_keys, normalize_gallery_preview_width, windows_path_for_display,
    AutoFindHistoryMode, GalleryGroupingMode, SettingsPatch, SettingsSnapshot,
};
pub use tag_catalog::{
    canonical_tag_token, normalize_tag_name, TagCatalogEntry, TagCatalogStatus, TagNamespace,
    TagSuggestion, TagSuggestionRequest,
};
pub use window_placement::{WindowPlacement, WindowPlacementSnapshot};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {field}: {message}")]
pub struct ValidationError {
    pub field: &'static str,
    pub message: &'static str,
}

impl ValidationError {
    pub const fn new(field: &'static str, message: &'static str) -> Self {
        Self { field, message }
    }
}
