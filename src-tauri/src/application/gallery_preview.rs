use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::domain::{
    ArtifactBundle, DuplicatePageHash, GalleryId, HashProfile, PageArtifactState, ValidationError,
};

use super::{
    duplicate_analyzer::compute_page_hash, ApplicationError, ArtifactRepository, ArtifactStore,
    DuplicateRepository, RepositoryError, StateRepository,
};

pub const GALLERY_PREVIEW_ALGORITHM_VERSION: u32 = 1;
const ANALYSIS_PAGE_LIMIT: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryPreview {
    pub gallery_id: GalleryId,
    pub mode: String,
    pub source_page: Option<u32>,
    pub manual_source_page: Option<u32>,
    pub entry_id: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub candidates: Vec<u32>,
    pub algorithm_version: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPreview {
    pub artist: String,
    pub gallery_ids: Vec<GalleryId>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GalleryPreviewSetRequest {
    #[serde(deserialize_with = "deserialize_gallery_id")]
    pub gallery_id: GalleryId,
    pub source_page: Option<u32>,
}

fn deserialize_gallery_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<GalleryId, D::Error> {
    GalleryId::new(i64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCandidate {
    pub source_page: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

pub struct GalleryPreviewUpdate {
    pub preview: Option<GalleryPreview>,
    pub artists: Vec<ArtistPreview>,
}

pub trait GalleryPreviewRepository: ArtifactRepository {
    fn gallery_preview_list(
        &self,
        gallery_ids: &[GalleryId],
    ) -> Result<Vec<GalleryPreview>, RepositoryError>;
    fn gallery_preview_set(
        &self,
        request: &GalleryPreviewSetRequest,
    ) -> Result<GalleryPreview, ApplicationError>;
    fn gallery_preview_pending(&self) -> Result<Vec<GalleryId>, RepositoryError>;
    fn gallery_preview_bundle(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Option<ArtifactBundle>, RepositoryError>;
    fn gallery_preview_root(&self, entry_id: &str) -> Result<Option<PathBuf>, RepositoryError>;
    fn gallery_preview_store_automatic(
        &self,
        gallery_id: GalleryId,
        entry_id: &str,
        candidates: &[PreviewCandidate],
        analyzed: bool,
    ) -> Result<(), RepositoryError>;
    fn artist_preview_list(
        &self,
        artists: &[String],
    ) -> Result<Vec<ArtistPreview>, RepositoryError>;
    fn artist_preview_refresh(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Vec<ArtistPreview>, RepositoryError>;
}

#[derive(Clone)]
pub struct GalleryPreviewService {
    inner: Arc<PreviewInner>,
    work: mpsc::Sender<PreviewWork>,
}

struct PreviewInner {
    repository: Arc<dyn GalleryPreviewRepository>,
    hashes: Arc<dyn DuplicateRepository>,
    settings: Arc<dyn StateRepository>,
    store: Arc<dyn ArtifactStore>,
    events: mpsc::Sender<GalleryPreviewUpdate>,
}

enum PreviewWork {
    Backfill,
    Gallery(GalleryId),
}

impl GalleryPreviewService {
    pub fn new(
        repository: Arc<dyn GalleryPreviewRepository>,
        hashes: Arc<dyn DuplicateRepository>,
        settings: Arc<dyn StateRepository>,
        store: Arc<dyn ArtifactStore>,
        events: mpsc::Sender<GalleryPreviewUpdate>,
    ) -> Result<Self, RepositoryError> {
        let inner = Arc::new(PreviewInner {
            repository,
            hashes,
            settings,
            store,
            events,
        });
        let (work, receiver) = mpsc::channel();
        let worker = Arc::clone(&inner);
        thread::Builder::new().name("atsumi-gallery-previews".into()).spawn(move || {
            let mut pending = VecDeque::new();
            loop {
                let item = if pending.is_empty() {
                    match receiver.recv() { Ok(work) => work, Err(_) => break }
                } else {
                    // New downloads have priority over the one-time legacy backfill.
                    match receiver.recv_timeout(Duration::from_millis(80)) {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => PreviewWork::Gallery(pending.pop_front().unwrap()),
                    }
                };
                match item {
                    PreviewWork::Backfill => match worker.repository.gallery_preview_pending() {
                        Ok(ids) => pending.extend(ids),
                        Err(error) => tracing::warn!(error = %error, "could not load gallery preview backfill"),
                    },
                    PreviewWork::Gallery(gallery_id) => {
                        pending.retain(|id| *id != gallery_id);
                        if let Err(error) = refresh_gallery(&worker, gallery_id) {
                            tracing::warn!(gallery_id = gallery_id.get(), error = %error, "gallery preview refresh was deferred");
                        }
                    }
                }
            }
        }).map_err(|error| RepositoryError::Other(format!("could not start gallery preview worker: {error}")))?;
        Ok(Self { inner, work })
    }

    pub fn start_backfill(&self) {
        let _ = self.work.send(PreviewWork::Backfill);
    }
    pub fn enqueue(&self, gallery_id: GalleryId) {
        let _ = self.work.send(PreviewWork::Gallery(gallery_id));
    }

    pub fn list(&self, gallery_ids: &[GalleryId]) -> Result<Vec<GalleryPreview>, ApplicationError> {
        if gallery_ids.len() > 500 {
            return Err(ValidationError::new("galleryIds", "must contain at most 500 IDs").into());
        }
        self.inner
            .repository
            .gallery_preview_list(gallery_ids)
            .map_err(Into::into)
    }

    pub fn artist_list(&self, artists: &[String]) -> Result<Vec<ArtistPreview>, ApplicationError> {
        if artists.len() > 500
            || artists
                .iter()
                .any(|artist| artist.trim().is_empty() || artist.len() > 800)
        {
            return Err(ValidationError::new(
                "artists",
                "must contain at most 500 nonempty artist names",
            )
            .into());
        }
        self.inner
            .repository
            .artist_preview_list(artists)
            .map_err(Into::into)
    }

    pub fn set(
        &self,
        request: GalleryPreviewSetRequest,
    ) -> Result<GalleryPreview, ApplicationError> {
        if request.source_page == Some(0) {
            return Err(ValidationError::new("sourcePage", "must be positive").into());
        }
        let preview = self.inner.repository.gallery_preview_set(&request)?;
        let _ = self.inner.events.send(GalleryPreviewUpdate {
            preview: Some(preview.clone()),
            artists: vec![],
        });
        Ok(preview)
    }

    /// Publish the persisted choice with current page availability; no image I/O.
    pub fn publish_saved(
        &self,
        gallery_ids: &[GalleryId],
        refresh_artists: bool,
    ) -> Result<(), ApplicationError> {
        for gallery_id in gallery_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            let artists = if refresh_artists {
                self.inner.repository.artist_preview_refresh(gallery_id)?
            } else {
                vec![]
            };
            let preview = self
                .inner
                .repository
                .gallery_preview_list(&[gallery_id])?
                .into_iter()
                .next();
            let _ = self
                .inner
                .events
                .send(GalleryPreviewUpdate { preview, artists });
        }
        Ok(())
    }
}

fn refresh_gallery(inner: &PreviewInner, gallery_id: GalleryId) -> Result<(), ApplicationError> {
    let artists = inner.repository.artist_preview_refresh(gallery_id)?;
    let Some(bundle) = inner.repository.gallery_preview_bundle(gallery_id)? else {
        let preview = inner
            .repository
            .gallery_preview_list(&[gallery_id])?
            .into_iter()
            .next();
        let _ = inner.events.send(GalleryPreviewUpdate { preview, artists });
        return Ok(());
    };
    let pages = bundle
        .pages
        .iter()
        .filter(|page| {
            !page.excluded
                && page.state == PageArtifactState::Present
                && page.verified_at.is_some()
                && page.sha256.is_some()
                && page.byte_length.is_some()
                && page.storage_format.is_some()
                && page.source_revision.is_some()
        })
        .take(ANALYSIS_PAGE_LIMIT)
        .collect::<Vec<_>>();
    let entry_id = bundle.artifact.entry_id.as_str();
    let fallback = pages
        .iter()
        .map(|page| PreviewCandidate {
            source_page: page.page_id.source_page_number.get(),
            width: None,
            height: None,
        })
        .collect::<Vec<_>>();
    // Save even a first-page fallback before decoding: interrupted or unreadable
    // legacy albums are not repeatedly analyzed at every application launch.
    inner
        .repository
        .gallery_preview_store_automatic(gallery_id, entry_id, &fallback, false)?;
    let root = inner
        .repository
        .gallery_preview_root(entry_id)?
        .unwrap_or(PathBuf::from(inner.settings.settings_get()?.download_root));
    let profile = HashProfile::current();
    let mut hashes = Vec::new();
    for page in &pages {
        let sha256 = page.sha256.as_ref().unwrap();
        if let Some(hash) = inner.hashes.duplicate_page_hash_get(
            entry_id,
            page.page_id.source_page_number,
            profile.profile_version,
            sha256.as_str(),
        )? {
            hashes.push(hash);
            continue;
        }
        if root.as_os_str().is_empty() {
            continue;
        }
        let computed = inner
            .store
            .read_verified_page_bytes(&root, page)
            .ok()
            .and_then(|bytes| {
                compute_page_hash(
                    entry_id,
                    gallery_id,
                    page.page_id.source_page_number,
                    sha256.clone(),
                    &bytes,
                    &profile,
                )
                .ok()
            });
        if let Some(hash) = computed {
            inner.hashes.duplicate_page_hash_upsert(&hash)?;
            hashes.push(hash);
        }
    }
    let candidates = ordered_candidates(&fallback, &hashes);
    inner
        .repository
        .gallery_preview_store_automatic(gallery_id, entry_id, &candidates, true)?;
    if let Some(preview) = inner
        .repository
        .gallery_preview_list(&[gallery_id])?
        .into_iter()
        .next()
    {
        let _ = inner.events.send(GalleryPreviewUpdate {
            preview: Some(preview),
            artists,
        });
    }
    Ok(())
}

fn likely_front_matter(hash: &DuplicatePageHash) -> bool {
    hash.low_information || (hash.mean_luma < 65.0 && hash.std_dev < 35.0)
}

fn ordered_candidates(
    fallback: &[PreviewCandidate],
    hashes: &[DuplicatePageHash],
) -> Vec<PreviewCandidate> {
    let first = fallback.first().and_then(|page| {
        hashes
            .iter()
            .find(|hash| hash.source_page_number.get() == page.source_page)
    });
    // Preserve a normal first page. Only move past front-matter separators when
    // the first page itself is uninformative/dark, and inspect just the first 4.
    let skip_through = first
        .filter(|hash| likely_front_matter(hash))
        .map(|_| {
            hashes
                .iter()
                .filter(|hash| hash.source_page_number.get() <= 4 && likely_front_matter(hash))
                .map(|hash| hash.source_page_number.get())
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let preferred = hashes
        .iter()
        .find(|hash| hash.source_page_number.get() > skip_through && !likely_front_matter(hash))
        .map(|hash| hash.source_page_number.get());
    let mut result = fallback
        .iter()
        .map(|page| {
            let hash = hashes
                .iter()
                .find(|hash| hash.source_page_number.get() == page.source_page);
            PreviewCandidate {
                source_page: page.source_page,
                width: hash.map(|hash| hash.width),
                height: hash.map(|hash| hash.height),
            }
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|page| {
        (
            if Some(page.source_page) == preferred {
                0
            } else if hashes.iter().any(|hash| {
                hash.source_page_number.get() == page.source_page && !likely_front_matter(hash)
            }) {
                1
            } else {
                2
            },
            page.source_page,
        )
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactSha256, SourcePageNumber};

    fn hash(page: u32, mean: f64, std_dev: f64, low_information: bool) -> DuplicatePageHash {
        DuplicatePageHash {
            entry_id: "entry".into(),
            gallery_id: GalleryId::new(1).unwrap(),
            source_page_number: SourcePageNumber::new(page).unwrap(),
            profile_version: 1,
            artifact_sha256: ArtifactSha256::new("a".repeat(64)).unwrap(),
            coarse_d_hash: 0,
            detail_d_hash_hex: "0".repeat(256),
            p_hash: 0,
            mean_luma: mean,
            std_dev,
            non_uniform_ratio: 0.5,
            edge_density: 0.2,
            width: 800,
            height: 1200,
            low_information,
        }
    }

    #[test]
    fn skips_dark_title_cover_and_blank_separator_but_preserves_regular_first_page() {
        for (hashes, expected) in [
            (
                vec![
                    hash(1, 43.0, 15.0, false),
                    hash(2, 255.0, 0.0, true),
                    hash(3, 162.0, 51.0, false),
                ],
                3,
            ),
            (
                vec![
                    hash(1, 12.0, 20.0, false),
                    hash(2, 188.0, 52.0, false),
                    hash(3, 255.0, 0.0, true),
                    hash(4, 152.0, 60.0, false),
                ],
                4,
            ),
            (
                vec![
                    hash(1, 160.0, 50.0, false),
                    hash(2, 255.0, 0.0, true),
                    hash(3, 170.0, 52.0, false),
                ],
                1,
            ),
        ] {
            let fallback = hashes
                .iter()
                .map(|hash| PreviewCandidate {
                    source_page: hash.source_page_number.get(),
                    width: None,
                    height: None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                ordered_candidates(&fallback, &hashes)[0].source_page,
                expected
            );
        }
    }
}
