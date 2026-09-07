use std::{io::Cursor, path::PathBuf, sync::Arc};

use image::{
    codecs::webp::WebPEncoder, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat,
    ImageReader, Limits,
};

use crate::{
    application::{verified_overlap_pages, ArtifactRepository, ArtifactStore, StateRepository},
    domain::{
        ArtifactBundle, DownloadArtifactState, DownloadEntryId, PageArtifact, PageArtifactState,
    },
    thumbnail::{
        CancellationToken, ResolvedThumbnail, ThumbnailFailureCode, ThumbnailKey,
        ThumbnailPriority, ThumbnailResolveError, ThumbnailResolver,
    },
};

use super::thumbnail_disk_cache::ThumbnailDiskCache;

const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_DECODE_ALLOC: u64 = 256 * 1024 * 1024;
const LOCAL_THUMBNAIL_RECIPE: &str = "artifact-webp-lossless-1024-v1";
const REMOTE_THUMBNAIL_RECIPE: &str = "gallery-artwork-v1";

/// Routes remote gallery artwork to the source resolver and verified local
/// artifact pages to the managed artifact store.  Artifact requests never
/// fall back to the network, so Review displays exactly the bytes that were
/// hashed by duplicate analysis.
pub struct CompositeThumbnailResolver {
    remote: Arc<dyn ThumbnailResolver>,
    repository: Arc<dyn ArtifactRepository>,
    settings: Arc<dyn StateRepository>,
    store: Arc<dyn ArtifactStore>,
    disk_cache: Option<Arc<ThumbnailDiskCache>>,
}

impl CompositeThumbnailResolver {
    pub fn new(
        remote: Arc<dyn ThumbnailResolver>,
        repository: Arc<dyn ArtifactRepository>,
        settings: Arc<dyn StateRepository>,
        store: Arc<dyn ArtifactStore>,
    ) -> Self {
        Self {
            remote,
            repository,
            settings,
            store,
            disk_cache: None,
        }
    }

    pub fn with_disk_cache(mut self, disk_cache: Arc<ThumbnailDiskCache>) -> Self {
        self.disk_cache = Some(disk_cache);
        self
    }

    fn resolve_artifact(
        &self,
        entry_id: &str,
        source_page: u32,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        if cancellation.is_cancelled() {
            return Err(ThumbnailResolveError::cancelled());
        }
        let ticket = self.disk_cache.as_ref().map(|cache| cache.ticket());
        let entry_id = DownloadEntryId::new(entry_id).map_err(|_| {
            ThumbnailResolveError::new(
                ThumbnailFailureCode::InvalidData,
                "artifact thumbnail entry ID is invalid",
                false,
            )
        })?;
        let bundle = self
            .repository
            .artifact_bundle_get(&entry_id)
            .map_err(repository_error)?
            .ok_or_else(|| not_found("verified artifact was not found"))?;
        if !artifact_thumbnail_bundle_is_verified(&bundle) {
            return Err(not_found(
                "artifact is not a verified complete or overlap-review bundle",
            ));
        }
        let page = bundle
            .pages
            .iter()
            .find(|page| {
                page.page_id.source_page_number.get() == source_page
                    && artifact_thumbnail_page_is_verified(page)
            })
            .ok_or_else(|| not_found("verified artifact page was not found"))?;
        let settings = self.settings.settings_get().map_err(repository_error)?;
        if settings.download_root.trim().is_empty() {
            return Err(not_found("download root is not configured"));
        }
        if cancellation.is_cancelled() {
            return Err(ThumbnailResolveError::cancelled());
        }
        let key = ThumbnailKey::ArtifactPage {
            entry_id: entry_id.to_string(),
            source_page,
        };
        let profile = format!(
            "{LOCAL_THUMBNAIL_RECIPE}:hash-profile:{}:{}",
            bundle.artifact.hash_profile_version,
            page.sha256.as_ref().unwrap(),
        );
        if let (Some(cache), Some(ticket)) = (&self.disk_cache, ticket) {
            if let Some(thumbnail) = cache.get(&key, &profile, ticket) {
                if cancellation.is_cancelled() {
                    return Err(ThumbnailResolveError::cancelled());
                }
                return Ok(thumbnail);
            }
        }
        let bytes = self
            .store
            .read_verified_page_bytes(&PathBuf::from(settings.download_root), page)
            .map_err(|error| {
                ThumbnailResolveError::new(
                    match error.code {
                        crate::application::DownloadPipelineErrorCode::ArtifactMissing => {
                            ThumbnailFailureCode::NotFound
                        }
                        crate::application::DownloadPipelineErrorCode::HashMismatch
                        | crate::application::DownloadPipelineErrorCode::ManifestInvalid => {
                            ThumbnailFailureCode::InvalidData
                        }
                        _ => ThumbnailFailureCode::DecodeFailed,
                    },
                    error.message,
                    error.retryable,
                )
            })?;
        if cancellation.is_cancelled() {
            return Err(ThumbnailResolveError::cancelled());
        }
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::WebP);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
        limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
        limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC);
        reader.limits(limits);
        let image = reader.decode().map_err(|_| {
            ThumbnailResolveError::new(
                ThumbnailFailureCode::DecodeFailed,
                "verified artifact page could not be decoded",
                false,
            )
        })?;
        let image = image.thumbnail(1_024, 1_024);
        let (width, height) = image.dimensions();
        let rgba = image.to_rgba8();
        let mut preview_bytes = Vec::new();
        WebPEncoder::new_lossless(&mut preview_bytes)
            .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
            .map_err(|_| {
                ThumbnailResolveError::new(
                    ThumbnailFailureCode::DecodeFailed,
                    "verified artifact preview could not be encoded",
                    false,
                )
            })?;
        if cancellation.is_cancelled() {
            return Err(ThumbnailResolveError::cancelled());
        }
        let thumbnail = ResolvedThumbnail {
            content_type: "image/webp".into(),
            bytes: preview_bytes,
            width,
            height,
            source_revision: page.sha256.as_ref().map(ToString::to_string),
        };
        // A page can be excluded or replaced while the codec is running. Read
        // current eligibility again before publishing those old bytes.
        let current = self
            .repository
            .artifact_bundle_get(&entry_id)
            .map_err(repository_error)?
            .ok_or_else(|| not_found("verified artifact was removed"))?;
        if !artifact_thumbnail_bundle_is_verified(&current)
            || !current.pages.iter().any(|current_page| {
                current_page.page_id == page.page_id
                    && artifact_thumbnail_page_is_verified(current_page)
                    && current_page.sha256 == page.sha256
            })
        {
            return Err(not_found(
                "verified artifact page changed during thumbnail resolution",
            ));
        }
        if cancellation.is_cancelled() {
            return Err(ThumbnailResolveError::cancelled());
        }
        if let (Some(cache), Some(ticket)) = (&self.disk_cache, ticket) {
            let _ = cache.put(&key, &profile, ticket, &thumbnail, cancellation);
        }
        Ok(thumbnail)
    }
}

fn artifact_thumbnail_page_is_verified(page: &PageArtifact) -> bool {
    page.state == PageArtifactState::Present
        && !page.excluded
        && page.sha256.is_some()
        && page.verified_at.is_some()
        && page.storage_format.is_some()
        && page.source_revision.is_some()
        && page.byte_length.is_some()
}

fn artifact_thumbnail_bundle_is_verified(bundle: &ArtifactBundle) -> bool {
    match bundle.artifact.state {
        DownloadArtifactState::Complete => true,
        DownloadArtifactState::Incomplete => verified_overlap_pages(bundle).is_some(),
        DownloadArtifactState::MissingArtifacts | DownloadArtifactState::Quarantined => false,
    }
}

impl ThumbnailResolver for CompositeThumbnailResolver {
    fn resolve(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        self.resolve_with_priority(key, cancellation, ThumbnailPriority::Visible)
    }

    fn resolve_with_priority(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
        priority: ThumbnailPriority,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        match key {
            ThumbnailKey::ArtifactPage {
                entry_id,
                source_page,
            } => self.resolve_artifact(entry_id, *source_page, cancellation),
            ThumbnailKey::GalleryCover { .. } | ThumbnailKey::GalleryPage { .. } => {
                if cancellation.is_cancelled() {
                    return Err(ThumbnailResolveError::cancelled());
                }
                let ticket = self.disk_cache.as_ref().map(|cache| cache.ticket());
                if let (Some(cache), Some(ticket)) = (&self.disk_cache, ticket) {
                    if let Some(thumbnail) = cache.get(key, REMOTE_THUMBNAIL_RECIPE, ticket) {
                        if cancellation.is_cancelled() {
                            return Err(ThumbnailResolveError::cancelled());
                        }
                        return Ok(thumbnail);
                    }
                }
                let thumbnail = self
                    .remote
                    .resolve_with_priority(key, cancellation, priority)?;
                if cancellation.is_cancelled() {
                    return Err(ThumbnailResolveError::cancelled());
                }
                if let (Some(cache), Some(ticket)) = (&self.disk_cache, ticket) {
                    let _ = cache.put(
                        key,
                        REMOTE_THUMBNAIL_RECIPE,
                        ticket,
                        &thumbnail,
                        cancellation,
                    );
                }
                Ok(thumbnail)
            }
        }
    }
}

fn repository_error(_error: crate::application::RepositoryError) -> ThumbnailResolveError {
    ThumbnailResolveError::new(
        ThumbnailFailureCode::TemporarilyUnavailable,
        "verified artifact metadata is temporarily unavailable",
        true,
    )
}

fn not_found(message: &'static str) -> ThumbnailResolveError {
    ThumbnailResolveError::new(ThumbnailFailureCode::NotFound, message, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ArtifactBundle, ArtifactRelativePath, ArtifactSha256, ArtifactStorageFormat,
        DownloadArtifact, DownloadArtifactState, DownloadEntryId, Gallery, GalleryId,
        GalleryMetadata, PageArtifact, PageArtifactState, SettingsSnapshot, SourcePageNumber,
        WindowPlacementSnapshot,
    };
    use crate::{
        application::RepositoryError, infrastructure::FilesystemArtifactStore,
        thumbnail::FixtureThumbnailResolver,
    };
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };

    fn artifact_bundle(state: DownloadArtifactState, verified: bool) -> ArtifactBundle {
        let entry_id = DownloadEntryId::new("entry-thumbnail-review").unwrap();
        let gallery_id = GalleryId::new(2_406_818).unwrap();
        let gallery = Gallery::new(
            gallery_id,
            0,
            GalleryMetadata::new("Review gallery", Some("artist".into()), None, 1).unwrap(),
        );
        let directory = ArtifactRelativePath::new("review-gallery").unwrap();
        let mut artifact =
            DownloadArtifact::new(entry_id.clone(), gallery_id, 0, directory, 1, state).unwrap();
        let mut page = PageArtifact::new(
            entry_id,
            gallery_id,
            SourcePageNumber::new(1).unwrap(),
            ArtifactRelativePath::new("review-gallery/0001.webp").unwrap(),
            PageArtifactState::Present,
            Some(4),
        )
        .unwrap();
        if verified {
            page = page
                .with_verification(
                    ArtifactSha256::new("0".repeat(64)).unwrap(),
                    ArtifactStorageFormat::Webp,
                    "source-revision-1",
                    "2026-08-27T00:00:00Z",
                )
                .unwrap();
        }
        if state == DownloadArtifactState::Complete {
            artifact = artifact
                .with_manifest(
                    ArtifactRelativePath::new("review-gallery/manifest.json").unwrap(),
                    1,
                    "test-writer",
                    1,
                    "2026-08-27T00:00:01Z",
                )
                .unwrap();
        }
        ArtifactBundle::new(gallery, artifact, vec![page]).unwrap()
    }

    #[test]
    fn verified_incomplete_overlap_bundle_is_thumbnail_eligible() {
        let bundle = artifact_bundle(DownloadArtifactState::Incomplete, true);

        assert!(artifact_thumbnail_bundle_is_verified(&bundle));
    }

    #[test]
    fn partial_incomplete_bundle_remains_thumbnail_ineligible() {
        let bundle = artifact_bundle(DownloadArtifactState::Incomplete, false);

        assert!(!artifact_thumbnail_bundle_is_verified(&bundle));
    }

    #[test]
    fn complete_bundle_remains_thumbnail_eligible() {
        let bundle = artifact_bundle(DownloadArtifactState::Complete, true);

        assert!(artifact_thumbnail_bundle_is_verified(&bundle));
    }

    struct TestRepository {
        bundle: Mutex<Option<ArtifactBundle>>,
        settings: SettingsSnapshot,
    }

    impl ArtifactRepository for TestRepository {
        fn artifact_bundle_replace(&self, bundle: &ArtifactBundle) -> Result<(), RepositoryError> {
            *self.bundle.lock().unwrap() = Some(bundle.clone());
            Ok(())
        }

        fn artifact_bundle_get(
            &self,
            entry: &DownloadEntryId,
        ) -> Result<Option<ArtifactBundle>, RepositoryError> {
            Ok(self
                .bundle
                .lock()
                .unwrap()
                .clone()
                .filter(|bundle| bundle.artifact.entry_id == *entry))
        }
    }

    impl StateRepository for TestRepository {
        fn settings_get(&self) -> Result<SettingsSnapshot, RepositoryError> {
            Ok(self.settings.clone())
        }
        fn settings_compare_and_set(
            &self,
            _: &SettingsSnapshot,
            _: u64,
        ) -> Result<bool, RepositoryError> {
            unreachable!("thumbnail resolution does not write settings")
        }
        fn window_placement_get(&self) -> Result<WindowPlacementSnapshot, RepositoryError> {
            unreachable!("thumbnail resolution does not read window state")
        }
        fn window_placement_compare_and_set(
            &self,
            _: &WindowPlacementSnapshot,
            _: u64,
        ) -> Result<bool, RepositoryError> {
            unreachable!("thumbnail resolution does not write window state")
        }
    }

    #[derive(Default)]
    struct CountingRemote {
        calls: AtomicUsize,
        fail: bool,
        cancel: bool,
    }

    impl ThumbnailResolver for CountingRemote {
        fn resolve(
            &self,
            key: &ThumbnailKey,
            token: &CancellationToken,
        ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err(ThumbnailResolveError::temporarily_unavailable(
                    "test failure",
                ));
            }
            let thumbnail = FixtureThumbnailResolver::new().resolve(key, token)?;
            if self.cancel {
                token.cancel();
            }
            Ok(thumbnail)
        }
    }

    struct Fixture {
        temp: tempfile::TempDir,
        repository: Arc<TestRepository>,
        remote: Arc<CountingRemote>,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let repository = Arc::new(TestRepository {
                bundle: Mutex::new(Some(artifact_bundle(DownloadArtifactState::Complete, true))),
                settings: SettingsSnapshot {
                    download_root: temp.path().to_string_lossy().into_owned(),
                    ..SettingsSnapshot::default()
                },
            });
            let fixture = Self {
                temp,
                repository,
                remote: Arc::new(CountingRemote::default()),
            };
            fixture.write_source([20, 80, 160, 255]);
            fixture
        }

        fn source_path(&self) -> PathBuf {
            self.temp.path().join("review-gallery/0001.webp")
        }

        fn write_source(&self, color: [u8; 4]) {
            fs::create_dir_all(self.source_path().parent().unwrap()).unwrap();
            let image = image::RgbaImage::from_pixel(48, 64, image::Rgba(color));
            let mut bytes = Vec::new();
            WebPEncoder::new_lossless(&mut bytes)
                .write_image(&image, 48, 64, ExtendedColorType::Rgba8)
                .unwrap();
            fs::write(self.source_path(), &bytes).unwrap();
            let mut bundle = self.repository.bundle.lock().unwrap();
            let page = &mut bundle.as_mut().unwrap().pages[0];
            page.sha256 =
                Some(ArtifactSha256::new(format!("{:x}", Sha256::digest(&bytes))).unwrap());
            page.byte_length = Some(bytes.len() as u64);
        }

        fn cache(&self) -> Arc<ThumbnailDiskCache> {
            Arc::new(ThumbnailDiskCache::new(
                self.temp.path().join("thumbnail-cache"),
                1024 * 1024,
            ))
        }

        fn resolver(&self, cache: Arc<ThumbnailDiskCache>) -> CompositeThumbnailResolver {
            CompositeThumbnailResolver::new(
                self.remote.clone(),
                self.repository.clone(),
                self.repository.clone(),
                Arc::new(FilesystemArtifactStore::new()),
            )
            .with_disk_cache(cache)
        }

        fn key(&self) -> ThumbnailKey {
            ThumbnailKey::artifact_page("entry-thumbnail-review", 1).unwrap()
        }
    }

    #[test]
    fn artifact_restart_cache_hit_does_not_read_hash_or_decode_original() {
        let fixture = Fixture::new();
        let first = fixture
            .resolver(fixture.cache())
            .resolve(&fixture.key(), &CancellationToken::new())
            .unwrap();
        // Any original read/hash/decode now fails. A new cache instance has only
        // persisted metadata and bytes, with no resolver or memory-cache state.
        fs::write(fixture.source_path(), b"original no longer decodable").unwrap();
        let second = fixture
            .resolver(fixture.cache())
            .resolve(&fixture.key(), &CancellationToken::new())
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(fixture.remote.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn artifact_sha_change_and_corrupt_cache_regenerate_from_verified_source() {
        let fixture = Fixture::new();
        let cache = fixture.cache();
        let resolver = fixture.resolver(cache.clone());
        let first = resolver
            .resolve(&fixture.key(), &CancellationToken::new())
            .unwrap();
        fixture.write_source([190, 10, 30, 255]);
        let second = resolver
            .resolve(&fixture.key(), &CancellationToken::new())
            .unwrap();
        assert_ne!(first.bytes, second.bytes);
        assert_ne!(first.source_revision, second.source_revision);
        for entry in fs::read_dir(fixture.temp.path().join("thumbnail-cache")).unwrap() {
            fs::write(entry.unwrap().path(), b"truncated").unwrap();
        }
        let repaired = resolver
            .resolve(&fixture.key(), &CancellationToken::new())
            .unwrap();
        assert_eq!(repaired, second);
    }

    #[test]
    fn cached_artifact_is_rejected_after_exclusion_quarantine_or_removal() {
        let fixture = Fixture::new();
        let resolver = fixture.resolver(fixture.cache());
        resolver
            .resolve(&fixture.key(), &CancellationToken::new())
            .unwrap();
        let original = fixture.repository.bundle.lock().unwrap().clone().unwrap();
        let mut excluded = original.clone();
        excluded.pages[0].excluded = true;
        let mut page_quarantined = original.clone();
        page_quarantined.pages[0].state = PageArtifactState::Quarantined;
        let mut bundle_quarantined = original.clone();
        bundle_quarantined.artifact.state = DownloadArtifactState::Quarantined;
        let mut removed = original;
        removed.pages.clear();
        for state in [
            Some(excluded),
            Some(page_quarantined),
            Some(bundle_quarantined),
            Some(removed),
            None,
        ] {
            *fixture.repository.bundle.lock().unwrap() = state;
            let error = resolver
                .resolve(&fixture.key(), &CancellationToken::new())
                .unwrap_err();
            assert_eq!(error.code, ThumbnailFailureCode::NotFound);
        }
        assert_eq!(fixture.remote.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn remote_cover_and_page_restart_hits_do_not_call_source_resolver() {
        let fixture = Fixture::new();
        let keys = [
            ThumbnailKey::gallery_cover(10).unwrap(),
            ThumbnailKey::gallery_page(10, 7).unwrap(),
        ];
        let resolver = fixture.resolver(fixture.cache());
        let expected = keys
            .iter()
            .map(|key| resolver.resolve(key, &CancellationToken::new()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(fixture.remote.calls.load(Ordering::Relaxed), 2);
        drop(resolver);
        let resolver = fixture.resolver(fixture.cache());
        for (key, expected) in keys.iter().zip(expected) {
            assert_eq!(
                resolver.resolve(key, &CancellationToken::new()).unwrap(),
                expected
            );
        }
        assert_eq!(fixture.remote.calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn unavailable_cache_falls_back_and_failures_or_cancellation_are_never_persisted() {
        let fixture = Fixture::new();
        let occupied = fixture.temp.path().join("occupied");
        fs::write(&occupied, b"preserve").unwrap();
        let cache = Arc::new(ThumbnailDiskCache::new(&occupied, 8192));
        assert!(fixture
            .resolver(cache.clone())
            .resolve(&fixture.key(), &CancellationToken::new())
            .is_ok());
        let key = ThumbnailKey::gallery_cover(1).unwrap();
        assert!(fixture
            .resolver(cache)
            .resolve(&key, &CancellationToken::new())
            .is_ok());
        for (fail, cancel, expected) in [
            (true, false, ThumbnailFailureCode::TemporarilyUnavailable),
            (false, true, ThumbnailFailureCode::Cancelled),
        ] {
            let cache = fixture.cache();
            let remote = Arc::new(CountingRemote {
                calls: AtomicUsize::new(0),
                fail,
                cancel,
            });
            let resolver = CompositeThumbnailResolver::new(
                remote.clone(),
                fixture.repository.clone(),
                fixture.repository.clone(),
                Arc::new(FilesystemArtifactStore::new()),
            )
            .with_disk_cache(cache.clone());
            for _ in 0..2 {
                assert_eq!(
                    resolver
                        .resolve(&key, &CancellationToken::new())
                        .unwrap_err()
                        .code,
                    expected
                );
            }
            assert_eq!(remote.calls.load(Ordering::Relaxed), 2);
            assert_eq!(cache.usage().entries, 0);
        }
        assert_eq!(fs::read(occupied).unwrap(), b"preserve");
    }
}
