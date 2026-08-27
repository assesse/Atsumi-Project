use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A stable identity for source artwork. `source_page` is the immutable,
/// one-based page number from the upstream gallery, never a UI list index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ThumbnailKey {
    #[serde(rename = "galleryCover")]
    GalleryCover {
        #[serde(rename = "galleryId")]
        gallery_id: i64,
    },
    #[serde(rename = "galleryPage")]
    GalleryPage {
        #[serde(rename = "galleryId")]
        gallery_id: i64,
        #[serde(rename = "sourcePage")]
        source_page: u32,
    },
    #[serde(rename = "artifactPage")]
    ArtifactPage {
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(rename = "sourcePage")]
        source_page: u32,
    },
}

impl ThumbnailKey {
    pub fn gallery_cover(gallery_id: i64) -> Result<Self, ThumbnailKeyError> {
        let key = Self::GalleryCover { gallery_id };
        key.validate()?;
        Ok(key)
    }

    pub fn gallery_page(gallery_id: i64, source_page: u32) -> Result<Self, ThumbnailKeyError> {
        let key = Self::GalleryPage {
            gallery_id,
            source_page,
        };
        key.validate()?;
        Ok(key)
    }

    pub fn artifact_page(
        entry_id: impl Into<String>,
        source_page: u32,
    ) -> Result<Self, ThumbnailKeyError> {
        let key = Self::ArtifactPage {
            entry_id: entry_id.into(),
            source_page,
        };
        key.validate()?;
        Ok(key)
    }

    pub fn gallery_id(&self) -> Option<i64> {
        match self {
            Self::GalleryCover { gallery_id } | Self::GalleryPage { gallery_id, .. } => {
                Some(*gallery_id)
            }
            Self::ArtifactPage { .. } => None,
        }
    }

    pub fn source_page(&self) -> Option<u32> {
        match self {
            Self::GalleryCover { .. } => None,
            Self::GalleryPage { source_page, .. } | Self::ArtifactPage { source_page, .. } => {
                Some(*source_page)
            }
        }
    }

    pub fn validate(&self) -> Result<(), ThumbnailKeyError> {
        if let Some(gallery_id) = self.gallery_id() {
            if gallery_id <= 0 {
                return Err(ThumbnailKeyError::InvalidGalleryId(gallery_id));
            }
        }
        if let Self::ArtifactPage { entry_id, .. } = self {
            if entry_id.trim().is_empty() || entry_id.len() > 200 {
                return Err(ThumbnailKeyError::InvalidEntryId);
            }
        }
        if matches!(self.source_page(), Some(0)) {
            return Err(ThumbnailKeyError::InvalidSourcePage);
        }
        Ok(())
    }

    pub fn cache_id(&self) -> String {
        match self {
            Self::GalleryCover { gallery_id } => format!("gallery:{gallery_id}:cover"),
            Self::GalleryPage {
                gallery_id,
                source_page,
            } => format!("gallery:{gallery_id}:source-page:{source_page}"),
            Self::ArtifactPage {
                entry_id,
                source_page,
            } => format!(
                "artifact:{}:{entry_id}:source-page:{source_page}",
                entry_id.len()
            ),
        }
    }
}

impl fmt::Display for ThumbnailKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.cache_id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ThumbnailKeyError {
    #[error("gallery ID must be positive, got {0}")]
    InvalidGalleryId(i64),
    #[error("artifact entry ID must be non-empty and at most 200 bytes")]
    InvalidEntryId,
    #[error("source page must be one-based")]
    InvalidSourcePage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailConsumer {
    Explore,
    Downloads,
    Detail,
    Review,
}

/// Explicit scheduling priority supplied by the caller. A consumer does not
/// imply a priority: e.g. a visible Downloads row and a Downloads prefetch can
/// intentionally use different priorities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailPriority {
    Critical,
    Visible,
    Prefetch,
}

impl ThumbnailPriority {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Critical => 3,
            Self::Visible => 2,
            Self::Prefetch => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThumbnailRequestDto {
    pub key: ThumbnailKey,
    pub consumer: ThumbnailConsumer,
    pub priority: ThumbnailPriority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedThumbnail {
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source_revision: Option<String>,
}

impl ResolvedThumbnail {
    pub fn validate(&self) -> Result<(), String> {
        if !self.content_type.starts_with("image/") {
            return Err("content type must be an image MIME type".into());
        }
        if self.bytes.is_empty() {
            return Err("thumbnail payload must not be empty".into());
        }
        if self.width == 0 || self.height == 0 {
            return Err("thumbnail dimensions must be positive".into());
        }
        Ok(())
    }

    pub(crate) fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailCacheStatus {
    Resolved,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailDeliveryDto {
    pub key: ThumbnailKey,
    pub thumbnail: ResolvedThumbnail,
    pub cache_status: ThumbnailCacheStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailFailureCode {
    Cancelled,
    NotFound,
    CandidatesExhausted,
    ResponseInvalid,
    DecodeFailed,
    TemporarilyUnavailable,
    Unauthorized,
    InvalidData,
    Resolver,
    CoordinatorClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailFailureDto {
    pub key: ThumbnailKey,
    pub code: ThumbnailFailureCode,
    pub message: String,
    pub retryable: bool,
    pub negative_cache_hit: bool,
}

impl ThumbnailFailureDto {
    pub(crate) fn cancelled(key: ThumbnailKey) -> Self {
        Self {
            key,
            code: ThumbnailFailureCode::Cancelled,
            message: "thumbnail request was cancelled".into(),
            retryable: true,
            negative_cache_hit: false,
        }
    }

    pub(crate) fn coordinator_closed(key: ThumbnailKey) -> Self {
        Self {
            key,
            code: ThumbnailFailureCode::CoordinatorClosed,
            message: "thumbnail coordinator is closed".into(),
            retryable: true,
            negative_cache_hit: false,
        }
    }
}

pub type ThumbnailResult = Result<ThumbnailDeliveryDto, ThumbnailFailureDto>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailRequestTokenDto {
    pub request_id: String,
    pub key: ThumbnailKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ThumbnailCompletionOutcomeDto {
    Ready { delivery: ThumbnailDeliveryDto },
    Failed { failure: ThumbnailFailureDto },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailCompletionEventDto {
    pub request_id: String,
    pub key: ThumbnailKey,
    pub outcome: ThumbnailCompletionOutcomeDto,
}

impl ThumbnailCompletionEventDto {
    pub fn from_result(token: ThumbnailRequestTokenDto, result: ThumbnailResult) -> Self {
        let outcome = match result {
            Ok(delivery) => ThumbnailCompletionOutcomeDto::Ready { delivery },
            Err(failure) => ThumbnailCompletionOutcomeDto::Failed { failure },
        };
        Self {
            request_id: token.request_id,
            key: token.key,
            outcome,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailRuntimeConfigDto {
    pub concurrent_image_requests: u32,
    pub request_start_interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailInvalidationDto {
    pub key: ThumbnailKey,
    pub success_cache_removed: bool,
    pub negative_cache_removed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailCacheClearDto {
    pub success_entries_removed: usize,
    pub success_bytes_removed: usize,
    pub negative_entries_removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailWorkerStatsDto {
    pub worker_count: usize,
    pub concurrency_limit: usize,
    pub request_start_interval_ms: u64,
    pub active_workers: usize,
    pub queued_keys: usize,
    pub in_flight_keys: usize,
    pub subscriber_count: usize,
    pub success_cache_entries: usize,
    pub success_cache_bytes: usize,
    pub negative_cache_entries: usize,
    pub requests_total: u64,
    pub success_cache_hits: u64,
    pub negative_cache_hits: u64,
    pub joined_in_flight: u64,
    pub resolved_success: u64,
    pub resolved_failure: u64,
    pub cancelled_subscribers: u64,
    pub cancelled_work: u64,
}
