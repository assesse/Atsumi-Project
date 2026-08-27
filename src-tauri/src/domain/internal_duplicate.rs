use serde::{Deserialize, Serialize};

use super::{GalleryId, SourcePageNumber};

/// Scene rows and edition tracks are derived from existing HashProfile 1 page
/// hashes. Bump this only when the grouping/result meaning changes.
pub const INTERNAL_DUPLICATE_ALGORITHM_VERSION: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InternalScanState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InternalArtifactScanStage {
    Hashing,
    Comparing,
    Finalizing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalArtifactScanProgress {
    pub run_id: String,
    pub sequence: u64,
    pub entry_id: String,
    pub gallery_id: GalleryId,
    /// One-based position in the current scan's deterministic artifact order.
    pub artifact_index: u32,
    pub total_artifacts: u32,
    pub processed_pages: u32,
    pub total_pages: u32,
    pub compared_pairs: u64,
    pub total_pairs: u64,
    pub progress_percent: u32,
    pub stage: InternalArtifactScanStage,
}

impl InternalScanState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalScanRun {
    pub run_id: String,
    pub revision: u64,
    pub state: InternalScanState,
    pub total_artifacts: u32,
    pub scanned_artifacts: u32,
    pub total_pages: u32,
    pub compared_pairs: u64,
    pub groups_found: u32,
    /// Version of the scene-clustering algorithm. HashProfile remains separate.
    pub algorithm_version: u32,
    pub skipped_artifacts: u32,
    pub skipped_pages: u32,
    pub started_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalScanSkip {
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub title: String,
    pub page_count: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalScanRequest {
    pub entry_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InternalMatchKind {
    Exact,
    TranslationVisual,
}

impl InternalMatchKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::TranslationVisual => "translation_visual",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "exact" => Some(Self::Exact),
            "translation_visual" => Some(Self::TranslationVisual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalPageEvidence {
    pub source_page: u32,
    pub exact_sha256: bool,
    pub visual_similarity: f64,
    pub detail_hash_distance: u32,
    pub low_information: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition_track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition_track_ordinal: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalDuplicateGroup {
    pub group_id: String,
    pub block_id: String,
    pub sequence_index: u32,
    pub revision: u64,
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub relation: InternalMatchKind,
    pub confidence: f64,
    pub recommended_keep_source_page: u32,
    pub pages: Vec<InternalPageEvidence>,
    pub resolved: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageQuarantineState {
    PendingQuarantine,
    Quarantined,
    PendingRestore,
    Restored,
}

impl PageQuarantineState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingQuarantine => "pending_quarantine",
            Self::Quarantined => "quarantined",
            Self::PendingRestore => "pending_restore",
            Self::Restored => "restored",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "pending_quarantine" => Some(Self::PendingQuarantine),
            "quarantined" => Some(Self::Quarantined),
            "pending_restore" => Some(Self::PendingRestore),
            "restored" => Some(Self::Restored),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageQuarantineRecord {
    pub record_id: String,
    pub plan_id: String,
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub source_page: u32,
    pub original_relative_path: String,
    pub quarantine_relative_path: String,
    pub reason: String,
    pub state: PageQuarantineState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalDuplicateSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<InternalScanRun>,
    pub groups: Vec<InternalDuplicateGroup>,
    pub quarantine_records: Vec<PageQuarantineRecord>,
    pub skips: Vec<InternalScanSkip>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalDuplicateReview {
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub title: String,
    pub groups: Vec<InternalDuplicateGroup>,
    pub quarantine_records: Vec<PageQuarantineRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalRemovalSelection {
    pub group_id: String,
    pub expected_revision: u64,
    pub keep_source_page: u32,
    pub remove_source_pages: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalRemovalPlanRequest {
    pub entry_id: String,
    pub selections: Vec<InternalRemovalSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalRemovalPlan {
    pub plan_id: String,
    pub entry_id: String,
    pub selections: Vec<InternalRemovalSelection>,
    pub files_to_quarantine: u32,
    pub bytes_to_quarantine: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalRemovalApplyRequest {
    pub plan: InternalRemovalPlan,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalRemovalUndoRequest {
    pub record_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalRemovalResult {
    pub review: InternalDuplicateReview,
    pub records: Vec<PageQuarantineRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InternalGroupRecord {
    pub run_id: String,
    pub group: InternalDuplicateGroup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageQuarantineSaga {
    pub record_id: String,
    pub plan_id: String,
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub source_page: SourcePageNumber,
    pub original_relative_path: String,
    pub quarantine_relative_path: String,
    pub reason: String,
    pub state: PageQuarantineState,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{InternalArtifactScanProgress, InternalArtifactScanStage};
    use crate::domain::GalleryId;

    #[test]
    fn artifact_scan_progress_uses_the_frontend_wire_contract() {
        let value = serde_json::to_value(InternalArtifactScanProgress {
            run_id: "run-1".into(),
            sequence: 7,
            entry_id: "entry-1".into(),
            gallery_id: GalleryId::new(4136275).unwrap(),
            artifact_index: 2,
            total_artifacts: 5,
            processed_pages: 33,
            total_pages: 77,
            compared_pairs: 528,
            total_pairs: 2_926,
            progress_percent: 18,
            stage: InternalArtifactScanStage::Comparing,
        })
        .unwrap();

        assert_eq!(
            value,
            json!({
                "runId": "run-1",
                "sequence": 7,
                "entryId": "entry-1",
                "galleryId": 4136275,
                "artifactIndex": 2,
                "totalArtifacts": 5,
                "processedPages": 33,
                "totalPages": 77,
                "comparedPairs": 528,
                "totalPairs": 2926,
                "progressPercent": 18,
                "stage": "comparing"
            })
        );
    }
}
