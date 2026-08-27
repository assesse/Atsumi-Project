use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use super::{ResolvedThumbnail, ThumbnailFailureCode, ThumbnailKey, ThumbnailPriority};

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailResolveError {
    pub code: ThumbnailFailureCode,
    pub message: String,
    pub retryable: bool,
}

impl ThumbnailResolveError {
    pub fn new(code: ThumbnailFailureCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    pub fn cancelled() -> Self {
        Self::new(
            ThumbnailFailureCode::Cancelled,
            "thumbnail resolution was cancelled",
            true,
        )
    }

    pub fn temporarily_unavailable(message: impl Into<String>) -> Self {
        Self::new(ThumbnailFailureCode::TemporarilyUnavailable, message, true)
    }
}

/// Port implemented by fixture, HTTP, disk-cache, or composite resolvers.
///
/// Implementations should check `cancellation` between blocking operations.
/// If the last subscriber disappears the coordinator cancels this token; a
/// resolver which cannot abort its current I/O may finish, but its result is
/// discarded and never enters the success cache.
pub trait ThumbnailResolver: Send + Sync + 'static {
    fn resolve(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError>;

    fn resolve_with_priority(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
        _priority: ThumbnailPriority,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        self.resolve(key, cancellation)
    }
}

/// Deterministic, dependency-free resolver for local development and tests.
/// It emits a valid square SVG whose colors and label derive only from the key.
pub struct FixtureThumbnailResolver {
    latency: Duration,
    failures: HashMap<ThumbnailKey, ThumbnailResolveError>,
}

impl Default for FixtureThumbnailResolver {
    fn default() -> Self {
        Self {
            latency: Duration::ZERO,
            failures: HashMap::new(),
        }
    }
}

impl FixtureThumbnailResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    pub fn with_failure(mut self, key: ThumbnailKey, failure: ThumbnailResolveError) -> Self {
        self.failures.insert(key, failure);
        self
    }

    fn wait_cooperatively(&self, cancellation: &CancellationToken) -> bool {
        let deadline = Instant::now() + self.latency;
        while Instant::now() < deadline {
            if cancellation.is_cancelled() {
                return false;
            }
            thread::sleep((deadline - Instant::now()).min(Duration::from_millis(5)));
        }
        !cancellation.is_cancelled()
    }
}

impl ThumbnailResolver for FixtureThumbnailResolver {
    fn resolve(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        if !self.wait_cooperatively(cancellation) {
            return Err(ThumbnailResolveError::cancelled());
        }
        if let Some(failure) = self.failures.get(key) {
            return Err(failure.clone());
        }

        let hash = fnv1a(key.cache_id().as_bytes());
        let hue = hash % 360;
        let accent_hue = (hue + 42) % 360;
        let label = match key {
            ThumbnailKey::GalleryCover { gallery_id } => format!("G{gallery_id} · COVER"),
            ThumbnailKey::GalleryPage {
                gallery_id,
                source_page,
            } => format!("G{gallery_id} · PAGE {source_page}"),
            ThumbnailKey::ArtifactPage {
                entry_id,
                source_page,
            } => format!("{entry_id} · PAGE {source_page}"),
        };
        let svg = format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">"#,
                r#"<rect width="512" height="512" fill="hsl({hue} 24% 30%)"/>"#,
                r#"<circle cx="394" cy="118" r="176" fill="hsl({accent_hue} 45% 55% / .42)"/>"#,
                r#"<path d="M0 366 172 214l98 92 74-68 168 170v104H0z" fill="hsl({hue} 22% 16% / .72)"/>"#,
                r#"<text x="32" y="468" fill="white" font-family="Segoe UI,sans-serif" font-size="25" font-weight="600">{label}</text>"#,
                "</svg>"
            ),
            hue = hue,
            accent_hue = accent_hue,
            label = label,
        );

        Ok(ResolvedThumbnail {
            content_type: "image/svg+xml".into(),
            bytes: svg.into_bytes(),
            width: 512,
            height: 512,
            source_revision: Some(format!("fixture-v1-{hash:016x}")),
        })
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
