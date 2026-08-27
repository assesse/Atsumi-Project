use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{GalleryId, SourcePageNumber},
    infrastructure::normalized_webp_bytes,
    thumbnail::CancellationToken,
};

use super::{DownloadSourceImageFormat, DownloadSourcePort};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailOriginalPrepareRequest {
    pub request_id: String,
    pub gallery_id: i64,
    pub source_page: u32,
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
}

impl DetailOriginalError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::Unavailable => "DETAIL_ORIGINAL_UNAVAILABLE",
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
    request_id: String,
    token: CancellationToken,
    stored: Option<StoredOriginal>,
}
#[derive(Default)]
struct DetailOriginalState {
    active: Option<ActiveOriginal>,
}

/// One app-wide detail original at a time. It bypasses the thumbnail cache and
/// is visible only through a request-ID custom-protocol URL.
#[derive(Clone)]
pub struct DetailOriginalSupervisor {
    source: Arc<dyn DownloadSourcePort>,
    root: PathBuf,
    state: Arc<Mutex<DetailOriginalState>>,
}

impl DetailOriginalSupervisor {
    pub fn new(source: Arc<dyn DownloadSourcePort>, data_dir: &Path) -> std::io::Result<Self> {
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
        if source_page.get() != 1 {
            return Err(DetailOriginalError::InvalidRequest);
        }

        self.dispose_active();
        let cancellation = CancellationToken::new();
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active = Some(ActiveOriginal {
            request_id: request_id.clone(),
            token: cancellation.clone(),
            stored: None,
        });
        tracing::info!(request_id = %request_id, gallery_id = request.gallery_id, source_page = 1, "detail original prepare_started");
        let result = self.prepare_registered(&request_id, gallery_id, source_page, &cancellation);
        if let Err(error) = &result {
            tracing::warn!(request_id = %request_id, gallery_id = request.gallery_id, source_page = 1, code = error.code(), "detail original prepare_failed");
            self.dispose(&request_id);
        }
        result
    }

    pub fn dispose(&self, request_id: &str) -> bool {
        let Ok(request_id) = canonical_request_id(request_id) else {
            return false;
        };
        let active = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.request_id == request_id)
            {
                state.active.take()
            } else {
                None
            }
        };
        if let Some(active) = active {
            active.token.cancel();
            if let Some(stored) = active.stored {
                let _ = fs::remove_file(stored.path);
            }
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
            .as_ref()
            .filter(|active| active.request_id == request_id && !active.token.is_cancelled())?
            .stored
            .clone()?;
        let root = fs::canonicalize(&self.root).ok()?;
        let path = fs::canonicalize(&stored.path).ok()?;
        path.starts_with(root)
            .then_some((path, stored.content_type))
    }

    fn dispose_active(&self) {
        let request_id = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .as_ref()
            .map(|active| active.request_id.clone());
        if let Some(request_id) = request_id {
            self.dispose(&request_id);
        }
    }

    fn prepare_registered(
        &self,
        request_id: &str,
        gallery_id: GalleryId,
        source_page: SourcePageNumber,
        cancellation: &CancellationToken,
    ) -> Result<DetailOriginalPrepared, DetailOriginalError> {
        let payload = self
            .source
            .download_page(gallery_id, source_page, cancellation)
            .map_err(|error| DetailOriginalError::SourceFailed {
                source_code: error.code.as_str().into(),
            })?;
        if cancellation.is_cancelled() {
            return Err(DetailOriginalError::Cancelled);
        }
        tracing::info!(request_id = %request_id, gallery_id = gallery_id.get(), source_page = 1, "detail original source_resolved");
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
        if cancellation.is_cancelled() {
            return Err(DetailOriginalError::Cancelled);
        }
        let temporary_path = self.root.join(format!("{request_id}.part"));
        let final_path = self.root.join(format!("{request_id}.{extension}"));
        if fs::write(&temporary_path, bytes).is_err() {
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
            .as_mut()
        {
            Some(active) if active.request_id == request_id && !active.token.is_cancelled() => {
                active.stored = Some(StoredOriginal {
                    path: final_path.clone(),
                    content_type: content_type.clone(),
                });
                true
            }
            _ => false,
        };
        if !accepted {
            let _ = fs::remove_file(final_path);
            return Err(DetailOriginalError::Cancelled);
        }
        tracing::info!(request_id = %request_id, gallery_id = gallery_id.get(), source_page = 1, "detail original file_prepared");
        Ok(DetailOriginalPrepared {
            request_id: request_id.into(),
            gallery_id: gallery_id.get(),
            source_page: source_page.get(),
            media_url: detail_original_media_url(request_id),
            content_type,
            width,
            height,
        })
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
        }
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
        assert!(!temp
            .path()
            .join("detail-original")
            .join(format!("{request_id}.part"))
            .exists());
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
