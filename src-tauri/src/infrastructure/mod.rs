mod artifact_store;
mod artifact_thumbnail;
mod avif_decode;
mod excluded_artifacts;
mod fixture_search;
mod gallery_summary_cache;
mod hitomi_live;
mod internal_duplicate_repository;
mod migrations;
mod overlap_merge;
mod preview_repository;
mod sqlite_repository;
pub mod telemetry;
mod thumbnail_disk_cache;
mod windows_folder_picker;

pub(crate) use artifact_store::normalized_webp_bytes;
pub use artifact_store::FilesystemArtifactStore;
pub use artifact_thumbnail::CompositeThumbnailResolver;
pub use excluded_artifacts::{ExcludedArtifactReport, ExcludedArtifactService};
pub use fixture_search::FixtureSearchRepository;
pub use hitomi_live::{HitomiLiveAdapter, HitomiLiveConfig};
pub use migrations::{MigrationReport, MigrationRunner, MIGRATIONS};
pub use overlap_merge::{
    DownloadOverlapMergeRequest, DownloadOverlapMergeResult, DownloadOverlapMergeSide,
    OverlapMergeService,
};
pub use sqlite_repository::SqliteRepository;
pub use thumbnail_disk_cache::ThumbnailDiskCache;
pub use windows_folder_picker::WindowsFolderPicker;
