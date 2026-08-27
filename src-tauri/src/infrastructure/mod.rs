mod artifact_store;
mod artifact_thumbnail;
mod avif_decode;
mod fixture_search;
mod hitomi_live;
mod internal_duplicate_repository;
mod migrations;
mod sqlite_repository;
pub mod telemetry;
mod windows_folder_picker;

pub(crate) use artifact_store::normalized_webp_bytes;
pub use artifact_store::FilesystemArtifactStore;
pub use artifact_thumbnail::CompositeThumbnailResolver;
pub use fixture_search::FixtureSearchRepository;
pub use hitomi_live::{HitomiLiveAdapter, HitomiLiveConfig};
pub use migrations::{MigrationReport, MigrationRunner, MIGRATIONS};
pub use sqlite_repository::SqliteRepository;
pub use windows_folder_picker::WindowsFolderPicker;
