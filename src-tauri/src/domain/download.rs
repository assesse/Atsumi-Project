use serde::{Deserialize, Serialize};

use super::{DownloadEntryId, GalleryId, JobState, Language, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadReviewKind {
    GalleryDuplicate,
    InternalPages,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEntry {
    pub entry_id: DownloadEntryId,
    pub gallery_id: GalleryId,
    pub revision: u64,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_kind: Option<DownloadReviewKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadListRequest {
    #[serde(default)]
    pub state: Option<JobState>,
    #[serde(default)]
    pub query: Option<String>,
    pub page: u32,
    pub page_size: u32,
}

impl DownloadListRequest {
    pub fn normalized(mut self) -> Result<Self, ValidationError> {
        if self.page == 0 {
            return Err(ValidationError::new("page", "must be one-based"));
        }
        if !(1..=200).contains(&self.page_size) {
            return Err(ValidationError::new(
                "pageSize",
                "must be between 1 and 200",
            ));
        }
        self.query = self.query.and_then(|query| {
            let query = query.trim().to_lowercase();
            (!query.is_empty()).then_some(query)
        });
        if self.query.as_ref().is_some_and(|query| query.len() > 500) {
            return Err(ValidationError::new("query", "must be at most 500 bytes"));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPage {
    pub page: u32,
    pub total_items: u64,
    pub entries: Vec<DownloadEntry>,
}

/// Locally persisted gallery fields sufficient to paint the Downloads list
/// without issuing one live gallery-detail request per entry. Optional fields
/// are expected for queued jobs and databases created before the presentation
/// metadata migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLibraryGallery {
    pub id: GalleryId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_rank: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLibraryItem {
    pub gallery: DownloadLibraryGallery,
    pub download: DownloadEntry,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLibraryPage {
    pub page: u32,
    pub total_items: u64,
    pub items: Vec<DownloadLibraryItem>,
}
