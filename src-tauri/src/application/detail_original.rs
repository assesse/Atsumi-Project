use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{
        ArtifactBundle, DownloadArtifactState, DownloadEntryId, GalleryId, PageArtifact,
        SourcePageNumber,
    },
    infrastructure::normalized_webp_bytes,
    thumbnail::CancellationToken,
};

use super::{
    ArtifactStore, DownloadPipelineRepository, DownloadSourceImageFormat, DownloadSourcePort,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailOriginalPrepareRequest {
    pub request_id: String,
    pub gallery_id: i64,
    pub source_page: u32,
    #[serde(default)]
    pub entry_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailOriginalPrepared {
    pub request_id: String,
    pub gallery_id: i64,
    pub source_page: u32,
    /// Opaque app-owned custom-protocol URL. A filesystem path never crosses IPC.
    pub media_url: String,
    pub content_type: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DetailOriginalError {
    #[error("detail original request is invalid")]
    InvalidRequest,
    #[error("detail original request was cancelled")]
    Cancelled,
    #[error("the source page could not be prepared")]
    SourceFailed { source_code: String },
    #[error("the source image could not be converted for display")]
    ConversionFailed,
    #[error("the temporary original file could not be written")]
    WriteFailed,
    #[error("the prepared original file could not be finalized")]
    Unavailable,
    #[error("the verified local artifact page is unavailable")]
    ArtifactUnavailable,
}

impl DetailOriginalError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::Unavailable | Self::ArtifactUnavailable => {
                "DETAIL_ORIGINAL_UNAVAILABLE"
            }
            Self::Cancelled => "DETAIL_ORIGINAL_CANCELLED",
            Self::SourceFailed { .. } => "DETAIL_ORIGINAL_SOURCE_FAILED",
            Self::ConversionFailed => "DETAIL_ORIGINAL_CONVERSION_FAILED",
            Self::WriteFailed => "DETAIL_ORIGINAL_WRITE_FAILED",
        }
    }
}

#[derive(Clone)]
struct StoredOriginal {
    path: PathBuf,
    content_type: String,
}
struct ActiveOriginal {
    generation: u64,
    token: CancellationToken,
    stored: Option<StoredOriginal>,
}
#[derive(Default)]
struct DetailOriginalState {
    active: HashMap<String, ActiveOriginal>,
    next_generation: u64,
}

struct OriginalMediaPayload {
    bytes: Vec<u8>,
    content_type: String,
    extension: &'static str,
    width: u32,
    height: u32,
}

trait DetailOriginalArtifactSource: Send + Sync {
    fn load(
        &self,
        entry_id: &DownloadEntryId,
        gallery_id: GalleryId,
        source_page: SourcePageNumber,
        cancellation: &CancellationToken,
    ) -> Result<OriginalMediaPayload, DetailOriginalError>;
}

struct ManagedDetailOriginalArtifactSource {
    repository: Arc<dyn DownloadPipelineRepository>,
    store: Arc<dyn ArtifactStore>,
}

impl DetailOriginalArtifactSource for ManagedDetailOriginalArtifactSource {
    fn load(
        &self,
        entry_id: &DownloadEntryId,
        gallery_id: GalleryId,
        source_page: SourcePageNumber,
        cancellation: &CancellationToken,
    ) -> Result<OriginalMediaPayload, DetailOriginalError> {
        if cancellation.is_cancelled() {
            return Err(DetailOriginalError::Cancelled);
        }
        let bundle = self
            .repository
            .pipeline_artifact_bundle(entry_id)
            .map_err(|_| DetailOriginalError::ArtifactUnavailable)?
            .ok_or(DetailOriginalError::ArtifactUnavailable)?;
        let page = completed_artifact_page(&bundle, entry_id, gallery_id, source_page)
            .ok_or(DetailOriginalError::ArtifactUnavailable)?;
        let root = self
            .repository
            .pipeline_artifact_root(entry_id)
            .map_err(|_| DetailOriginalError::ArtifactUnavailable)?;
        let bytes = self
            .store
            .read_verified_page_bytes(&root, page)
            .map_err(|_| DetailOriginalError::ArtifactUnavailable)?;
        if cancellation.is_cancelled() {
            return Err(DetailOriginalError::Cancelled);
        }
        let (width, height) =
            ImageReader::with_format(Cursor::new(bytes.as_slice()), ImageFormat::WebP)
                .into_dimensions()
                .map_err(|_| DetailOriginalError::ArtifactUnavailable)?;
        Ok(OriginalMediaPayload {
            bytes,
            content_type: "image/webp".into(),
            extension: "webp",
            width,
            height,
        })
    }
}

fn completed_artifact_page<'a>(
    bundle: &'a ArtifactBundle,
    entry_id: &DownloadEntryId,
    gallery_id: GalleryId,
    source_page: SourcePageNumber,
) -> Option<&'a PageArtifact> {
    if bundle.artifact.state != DownloadArtifactState::Complete
        || &bundle.artifact.entry_id != entry_id
        || bundle.artifact.gallery_id != gallery_id
        || bundle.gallery.id != gallery_id
    {
        return None;
    }
    bundle.pages.iter().find(|page| {
        &page.entry_id == entry_id
            && page.page_id.gallery_id == gallery_id
            && page.page_id.source_page_number == source_page
    })
}

/// Original media bypasses the thumbnail cache and is visible only through
/// request-ID custom-protocol URLs. Independent request slots let the retained
/// detail hero and a page-preview dialog remain valid at the same time.
#[derive(Clone)]
pub struct DetailOriginalSupervisor {
    source: Arc<dyn DownloadSourcePort>,
    artifact_source: Option<Arc<dyn DetailOriginalArtifactSource>>,
    root: PathBuf,
    state: Arc<Mutex<DetailOriginalState>>,
}

impl DetailOriginalSupervisor {
    pub fn new(source: Arc<dyn DownloadSourcePort>, data_dir: &Path) -> std::io::Result<Self> {
        Self::new_with_artifact_source(source, None, data_dir)
    }

    pub fn new_with_artifacts(
        source: Arc<dyn DownloadSourcePort>,
        repository: Arc<dyn DownloadPipelineRepository>,
        store: Arc<dyn ArtifactStore>,
        data_dir: &Path,
    ) -> std::io::Result<Self> {
        Self::new_with_artifact_source(
            source,
            Some(Arc::new(ManagedDetailOriginalArtifactSource {
                repository,
                store,
            })),
            data_dir,
        )
    }

    fn new_with_artifact_source(
        source: Arc<dyn DownloadSourcePort>,
        artifact_source: Option<Arc<dyn DetailOriginalArtifactSource>>,
        data_dir: &Path,
    ) -> std::io::Result<Self> {
        let root = data_dir.join("detail-original");
        fs::create_dir_all(&root)?;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(Self {
            source,
            artifact_source,
            root,
            state: Arc::new(Mutex::new(DetailOriginalState::default())),
        })
    }

    pub fn prepare(
        &self,
        request: DetailOriginalPrepareRequest,
    ) -> Result<DetailOriginalPrepared, DetailOriginalError> {
        let request_id = canonical_request_id(&request.request_id)?;
        let gallery_id =
            GalleryId::new(request.gallery_id).map_err(|_| DetailOriginalError::InvalidRequest)?;
        let source_page = SourcePageNumber::new(request.source_page)
            .map_err(|_| DetailOriginalError::InvalidRequest)?;
        let entry_id = request
            .entry_id
            .as_ref()
            .map(|entry_id| DownloadEntryId::new(entry_id.clone()))
            .transpose()
            .map_err(|_| DetailOriginalError::InvalidRequest)?;
        if entry_id.is_none() && source_page.get() != 1 {
            return Err(DetailOriginalError::InvalidRequest);
        }

        let cancellation = CancellationToken::new();
        let (generation, replaced) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.next_generation = state.next_generation.wrapping_add(1).max(1);
            let generation = state.next_generation;
            let replaced = state.active.insert(
                request_id.clone(),
                ActiveOriginal {
                    generation,
                    token: cancellation.clone(),
                    stored: None,
                },
            );
            (generation, replaced)
        };
        if let Some(replaced) = replaced {
            cleanup_active_original(replaced);
        }
        tracing::info!(request_id = %request_id, gallery_id = request.gallery_id, source_page = request.source_page, local_artifact = entry_id.is_some(), "detail original prepare_started");
        let result = self.prepare_registered(
            &request_id,
            generation,
            gallery_id,
            source_page,
            entry_id.as_ref(),
            &cancellation,
        );
        if let Err(error) = &result {
            tracing::warn!(request_id = %request_id, gallery_id = request.gallery_id, source_page = request.source_page, local_artifact = entry_id.is_some(), code = error.code(), "detail original prepare_failed");
            self.dispose_generation(&request_id, generation);
        }
        result
    }

    pub fn dispose(&self, request_id: &str) -> bool {
        let Ok(request_id) = canonical_request_id(request_id) else {
            return false;
        };
        let active = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.active.remove(&request_id)
        };
        if let Some(active) = active {
            cleanup_active_original(active);
            tracing::info!(request_id = %request_id, "detail original disposed");
        }
        true // repeated React cleanup is deliberately idempotent
    }

    /// Returns only an app-owned path copied while holding no lock. The async
    /// protocol handler performs the potentially large file read afterwards.
    pub fn media_file(&self, request_id: &str) -> Option<(PathBuf, String)> {
        let request_id = canonical_request_id(request_id).ok()?;
        let stored = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .get(&request_id)
            .filter(|active| !active.token.is_cancelled())?
            .stored
            .clone()?;
        let root = fs::canonicalize(&self.root).ok()?;
        let path = fs::canonicalize(&stored.path).ok()?;
        path.starts_with(root)
            .then_some((path, stored.content_type))
    }

    fn dispose_generation(&self, request_id: &str, generation: u64) {
        let active = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .active
                .get(request_id)
                .is_some_and(|active| active.generation == generation)
            {
                state.active.remove(request_id)
            } else {
                None
            }
        };
        if let Some(active) = active {
            cleanup_active_original(active);
        }
    }

    fn prepare_registered(
        &self,
        request_id: &str,
        generation: u64,
        gallery_id: GalleryId,
        source_page: SourcePageNumber,
        entry_id: Option<&DownloadEntryId>,
        cancellation: &CancellationToken,
    ) -> Result<DetailOriginalPrepared, DetailOriginalError> {
        let payload = if let Some(entry_id) = entry_id {
            self.artifact_source
                .as_ref()
                .ok_or(DetailOriginalError::ArtifactUnavailable)?
                .load(entry_id, gallery_id, source_page, cancellation)?
        } else {
            let payload = self
                .source
                .download_page(gallery_id, source_page, cancellation)
                .map_err(|error| DetailOriginalError::SourceFailed {
                    source_code: error.code.as_str().into(),
                })?;
            let width = payload.width;
            let height = payload.height;
            let (bytes, content_type, extension) = match payload.source_format {
                DownloadSourceImageFormat::Avif => normalized_webp_bytes(&payload)
                    .map(|bytes| (bytes.into_owned(), "image/webp".to_owned(), "webp"))
                    .map_err(|_| DetailOriginalError::ConversionFailed)?,
                DownloadSourceImageFormat::Webp => (payload.bytes, "image/webp".to_owned(), "webp"),
                DownloadSourceImageFormat::Jpeg => (payload.bytes, "image/jpeg".to_owned(), "jpg"),
                DownloadSourceImageFormat::Png => (payload.bytes, "image/png".to_owned(), "png"),
            };
            OriginalMediaPayload {
                bytes,
                content_type,
                extension,
                width,
                height,
            }
        };
        if cancellation.is_cancelled() {
            return Err(DetailOriginalError::Cancelled);
        }
        tracing::info!(request_id = %request_id, gallery_id = gallery_id.get(), source_page = source_page.get(), local_artifact = entry_id.is_some(), "detail original source_resolved");
        let temporary_path = self.root.join(format!("{request_id}.{generation}.part"));
        let final_path = self
            .root
            .join(format!("{request_id}.{generation}.{}", payload.extension));
        if fs::write(&temporary_path, payload.bytes).is_err() {
            let _ = fs::remove_file(&temporary_path);
            return Err(DetailOriginalError::WriteFailed);
        }
        if cancellation.is_cancelled() {
            let _ = fs::remove_file(&temporary_path);
            return Err(DetailOriginalError::Cancelled);
        }
        fs::rename(&temporary_path, &final_path).map_err(|_| {
            let _ = fs::remove_file(&temporary_path);
            DetailOriginalError::Unavailable
        })?;
        let accepted = match self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .get_mut(request_id)
        {
            Some(active) if active.generation == generation && !active.token.is_cancelled() => {
                active.stored = Some(StoredOriginal {
                    path: final_path.clone(),
                    content_type: payload.content_type.clone(),
                });
                true
            }
            _ => false,
        };
        if !accepted {
            let _ = fs::remove_file(final_path);
            return Err(DetailOriginalError::Cancelled);
        }
        tracing::info!(request_id = %request_id, gallery_id = gallery_id.get(), source_page = source_page.get(), local_artifact = entry_id.is_some(), "detail original file_prepared");
        Ok(DetailOriginalPrepared {
            request_id: request_id.into(),
            gallery_id: gallery_id.get(),
            source_page: source_page.get(),
            media_url: detail_original_media_url(request_id),
            content_type: payload.content_type,
            width: payload.width,
            height: payload.height,
        })
    }
}

fn cleanup_active_original(active: ActiveOriginal) {
    active.token.cancel();
    if let Some(stored) = active.stored {
        let _ = fs::remove_file(stored.path);
    }
}

pub(crate) fn canonical_request_id(value: &str) -> Result<String, DetailOriginalError> {
    let parsed = Uuid::parse_str(value).map_err(|_| DetailOriginalError::InvalidRequest)?;
    let canonical = parsed.to_string();
    (canonical == value)
        .then_some(canonical)
        .ok_or(DetailOriginalError::InvalidRequest)
}

pub(crate) fn detail_original_media_url(request_id: &str) -> String {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        format!("http://detail-original.localhost/{request_id}")
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        format!("detail-original://localhost/{request_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        thread,
        time::Duration,
    };

    use crate::{
        application::{DownloadGallerySnapshot, DownloadPagePayload},
        domain::{
            ArtifactRelativePath, ArtifactSha256, ArtifactStorageFormat, DownloadArtifact, Gallery,
            GalleryMetadata, PageArtifactState,
        },
        source::SourceContractError,
    };

    #[derive(Clone, Copy)]
    enum FixtureMode {
        Png,
        SourceFailure,
        WaitForCancellation,
    }

    struct FixtureSource {
        mode: FixtureMode,
        started: Arc<AtomicBool>,
    }

    struct FixtureArtifactSource {
        calls: Arc<Mutex<Vec<(String, i64, u32)>>>,
    }

    impl DetailOriginalArtifactSource for FixtureArtifactSource {
        fn load(
            &self,
            entry_id: &DownloadEntryId,
            gallery_id: GalleryId,
            source_page: SourcePageNumber,
            cancellation: &CancellationToken,
        ) -> Result<OriginalMediaPayload, DetailOriginalError> {
            if cancellation.is_cancelled() {
                return Err(DetailOriginalError::Cancelled);
            }
            self.calls
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((
                    entry_id.as_str().into(),
                    gallery_id.get(),
                    source_page.get(),
                ));
            Ok(OriginalMediaPayload {
                bytes: b"verified-local-webp".to_vec(),
                content_type: "image/webp".into(),
                extension: "webp",
                width: 1_600,
                height: 900,
            })
        }
    }

    impl DownloadSourcePort for FixtureSource {
        fn gallery_snapshot(
            &self,
            _gallery_id: GalleryId,
            _cancellation: &CancellationToken,
        ) -> Result<DownloadGallerySnapshot, SourceContractError> {
            Err(SourceContractError::protocol("not used by detail original"))
        }

        fn download_page(
            &self,
            _gallery_id: GalleryId,
            source_page_number: SourcePageNumber,
            cancellation: &CancellationToken,
        ) -> Result<DownloadPagePayload, SourceContractError> {
            self.started.store(true, Ordering::Release);
            match self.mode {
                FixtureMode::SourceFailure => {
                    Err(SourceContractError::protocol("fixture source failure"))
                }
                FixtureMode::WaitForCancellation => {
                    while !cancellation.is_cancelled() {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(SourceContractError::cancelled())
                }
                FixtureMode::Png => Ok(DownloadPagePayload {
                    source_page_number,
                    // The supervisor delegates image validation to the source port;
                    // it only validates lifecycle and transient-file containment.
                    bytes: vec![0x89, b'P', b'N', b'G'],
                    source_revision: "fixture-v1".into(),
                    source_format: DownloadSourceImageFormat::Png,
                    width: 2,
                    height: 3,
                    candidate_index: 0,
                    candidate_diagnostics: Vec::new(),
                }),
            }
        }
    }

    fn fixture_supervisor(
        mode: FixtureMode,
        directory: &Path,
    ) -> (DetailOriginalSupervisor, Arc<AtomicBool>) {
        let started = Arc::new(AtomicBool::new(false));
        let supervisor = DetailOriginalSupervisor::new(
            Arc::new(FixtureSource {
                mode,
                started: started.clone(),
            }),
            directory,
        )
        .unwrap();
        (supervisor, started)
    }

    fn fixture_request(request_id: &str) -> DetailOriginalPrepareRequest {
        DetailOriginalPrepareRequest {
            request_id: request_id.into(),
            gallery_id: 4_133_977,
            source_page: 1,
            entry_id: None,
        }
    }

    fn fixture_artifact_bundle(state: DownloadArtifactState) -> ArtifactBundle {
        let entry_id = DownloadEntryId::new("entry-local-original").unwrap();
        let gallery_id = GalleryId::new(4_133_977).unwrap();
        let gallery = Gallery::new(
            gallery_id,
            0,
            GalleryMetadata::new("Local original", Some("artist".into()), None, 1).unwrap(),
        );
        let directory = ArtifactRelativePath::new("local-original").unwrap();
        let mut artifact =
            DownloadArtifact::new(entry_id.clone(), gallery_id, 0, directory.clone(), 1, state)
                .unwrap();
        let page = PageArtifact::new(
            entry_id,
            gallery_id,
            SourcePageNumber::new(1).unwrap(),
            ArtifactRelativePath::new("local-original/0001.webp").unwrap(),
            PageArtifactState::Present,
            Some(4),
        )
        .unwrap()
        .with_verification(
            ArtifactSha256::new("0".repeat(64)).unwrap(),
            ArtifactStorageFormat::Webp,
            "source-v1",
            "2026-08-31T00:00:00Z",
        )
        .unwrap();
        if state == DownloadArtifactState::Complete {
            artifact = artifact
                .with_manifest(
                    ArtifactRelativePath::new("local-original/manifest.json").unwrap(),
                    1,
                    "test-writer",
                    1,
                    "2026-08-31T00:00:01Z",
                )
                .unwrap();
        }
        ArtifactBundle::new(gallery, artifact, vec![page]).unwrap()
    }

    #[test]
    fn validates_canonical_uuid_and_platform_url() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(canonical_request_id(id).unwrap(), id);
        assert!(canonical_request_id("../outside").is_err());
        assert!(canonical_request_id("550E8400-E29B-41D4-A716-446655440000").is_err());
        #[cfg(target_os = "windows")]
        assert_eq!(
            detail_original_media_url(id),
            format!("http://detail-original.localhost/{id}")
        );
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            detail_original_media_url(id),
            format!("detail-original://localhost/{id}")
        );
    }

    #[test]
    fn startup_removes_only_stale_app_owned_originals_and_dispose_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let stale_root = temp.path().join("detail-original");
        fs::create_dir_all(&stale_root).unwrap();
        fs::write(stale_root.join("stale.webp"), b"old").unwrap();
        let (supervisor, _) = fixture_supervisor(FixtureMode::Png, temp.path());
        assert!(!stale_root.join("stale.webp").exists());
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert!(supervisor.dispose(id));
        assert!(supervisor.dispose(id));
        assert!(supervisor.media_file("../outside").is_none());
    }

    #[test]
    fn prepare_returns_terminal_source_failure_and_leaves_no_active_media() {
        let temp = tempfile::tempdir().unwrap();
        let (supervisor, _) = fixture_supervisor(FixtureMode::SourceFailure, temp.path());
        let request_id = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            supervisor.prepare(fixture_request(request_id)),
            Err(DetailOriginalError::SourceFailed {
                source_code: "protocol".into()
            })
        );
        assert!(supervisor.media_file(request_id).is_none());
        assert!(supervisor.dispose(request_id));
    }

    #[test]
    fn successful_prepare_uses_an_app_owned_file_until_idempotent_dispose() {
        let temp = tempfile::tempdir().unwrap();
        let (supervisor, _) = fixture_supervisor(FixtureMode::Png, temp.path());
        let request_id = "550e8400-e29b-41d4-a716-446655440000";
        let prepared = supervisor.prepare(fixture_request(request_id)).unwrap();
        assert_eq!(prepared.request_id, request_id);
        assert_eq!(prepared.source_page, 1);
        assert_eq!(prepared.content_type, "image/png");
        let (path, content_type) = supervisor.media_file(request_id).unwrap();
        assert_eq!(content_type, "image/png");
        assert_eq!(fs::read(&path).unwrap(), vec![0x89, b'P', b'N', b'G']);
        assert!(path.starts_with(fs::canonicalize(temp.path().join("detail-original")).unwrap()));
        assert!(supervisor.dispose(request_id));
        assert!(!path.exists());
        assert!(supervisor.dispose(request_id));
        assert!(supervisor.media_file(request_id).is_none());
    }

    #[test]
    fn completed_artifact_selection_requires_exact_entry_gallery_and_page() {
        let complete = fixture_artifact_bundle(DownloadArtifactState::Complete);
        let entry_id = DownloadEntryId::new("entry-local-original").unwrap();
        let gallery_id = GalleryId::new(4_133_977).unwrap();
        assert!(completed_artifact_page(
            &complete,
            &entry_id,
            gallery_id,
            SourcePageNumber::new(1).unwrap(),
        )
        .is_some());
        assert!(completed_artifact_page(
            &complete,
            &entry_id,
            GalleryId::new(4_133_978).unwrap(),
            SourcePageNumber::new(1).unwrap(),
        )
        .is_none());
        assert!(completed_artifact_page(
            &complete,
            &entry_id,
            gallery_id,
            SourcePageNumber::new(2).unwrap(),
        )
        .is_none());

        let incomplete = fixture_artifact_bundle(DownloadArtifactState::Incomplete);
        assert!(completed_artifact_page(
            &incomplete,
            &entry_id,
            gallery_id,
            SourcePageNumber::new(1).unwrap(),
        )
        .is_none());
    }

    #[test]
    fn local_page_and_remote_hero_keep_independent_request_lifecycles() {
        let temp = tempfile::tempdir().unwrap();
        let started = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let supervisor = DetailOriginalSupervisor::new_with_artifact_source(
            Arc::new(FixtureSource {
                mode: FixtureMode::Png,
                started: started.clone(),
            }),
            Some(Arc::new(FixtureArtifactSource {
                calls: calls.clone(),
            })),
            temp.path(),
        )
        .unwrap();
        let hero_id = "550e8400-e29b-41d4-a716-446655440000";
        let page_id = "550e8400-e29b-41d4-a716-446655440001";

        let hero = supervisor.prepare(fixture_request(hero_id)).unwrap();
        let page = supervisor
            .prepare(DetailOriginalPrepareRequest {
                request_id: page_id.into(),
                gallery_id: 4_133_977,
                source_page: 7,
                entry_id: Some("entry-local-original".into()),
            })
            .unwrap();

        assert!(started.load(Ordering::Acquire));
        assert_eq!(hero.source_page, 1);
        assert_eq!(hero.content_type, "image/png");
        assert_eq!(page.source_page, 7);
        assert_eq!(page.content_type, "image/webp");
        assert_eq!(page.width, 1_600);
        assert_eq!(page.height, 900);
        assert_eq!(
            calls
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            [("entry-local-original".into(), 4_133_977, 7)]
        );
        assert!(supervisor.media_file(hero_id).is_some());
        let (page_path, _) = supervisor.media_file(page_id).unwrap();
        assert_eq!(fs::read(page_path).unwrap(), b"verified-local-webp");

        assert!(supervisor.dispose(page_id));
        assert!(supervisor.media_file(page_id).is_none());
        assert!(supervisor.media_file(hero_id).is_some());
        assert!(supervisor.dispose(hero_id));
    }

    #[test]
    fn remote_original_remains_page_one_only() {
        let temp = tempfile::tempdir().unwrap();
        let (supervisor, started) = fixture_supervisor(FixtureMode::Png, temp.path());
        let mut request = fixture_request("550e8400-e29b-41d4-a716-446655440000");
        request.source_page = 2;

        assert_eq!(
            supervisor.prepare(request),
            Err(DetailOriginalError::InvalidRequest)
        );
        assert!(!started.load(Ordering::Acquire));
    }

    #[test]
    fn dispose_cancels_an_inflight_prepare_without_leaving_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let (supervisor, started) =
            fixture_supervisor(FixtureMode::WaitForCancellation, temp.path());
        let request_id = "550e8400-e29b-41d4-a716-446655440000";
        let worker = {
            let supervisor = supervisor.clone();
            thread::spawn(move || supervisor.prepare(fixture_request(request_id)))
        };
        for _ in 0..100 {
            if started.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(started.load(Ordering::Acquire));
        assert!(supervisor.dispose(request_id));
        assert_eq!(
            worker.join().unwrap(),
            Err(DetailOriginalError::SourceFailed {
                source_code: "cancelled".into()
            })
        );
        assert!(supervisor.media_file(request_id).is_none());
        assert!(fs::read_dir(temp.path().join("detail-original"))
            .unwrap()
            .all(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|extension| extension != "part")));
    }

    #[test]
    #[ignore = "opt-in live Floating Detail original regression smoke"]
    fn live_page_one_prepares_platform_media_and_disposes_cleanly() {
        assert_eq!(
            std::env::var("ATSUMI_ALLOW_LIVE_SMOKE").as_deref(),
            Ok("1"),
            "live network access requires ATSUMI_ALLOW_LIVE_SMOKE=1"
        );
        let temp = tempfile::tempdir().unwrap();
        let source: Arc<dyn DownloadSourcePort> = Arc::new(
            crate::infrastructure::HitomiLiveAdapter::new(
                crate::infrastructure::HitomiLiveConfig::default(),
            )
            .unwrap(),
        );
        let supervisor = DetailOriginalSupervisor::new(source, temp.path()).unwrap();
        let request_id = "550e8400-e29b-41d4-a716-446655440000";
        let prepared = supervisor.prepare(fixture_request(request_id)).unwrap();
        assert_eq!(prepared.source_page, 1);
        assert!(matches!(
            prepared.content_type.as_str(),
            "image/webp" | "image/jpeg" | "image/png"
        ));
        assert_eq!(prepared.media_url, detail_original_media_url(request_id));
        assert!(supervisor.media_file(request_id).is_some());
        assert!(supervisor.dispose(request_id));
        assert!(supervisor.media_file(request_id).is_none());
    }
}
