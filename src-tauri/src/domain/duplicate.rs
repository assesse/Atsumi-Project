use serde::{Deserialize, Serialize};

use super::{ArtifactSha256, GalleryId, SourcePageNumber};

pub const DUPLICATE_HASH_PROFILE_VERSION: u32 = 1;
pub const DUPLICATE_HASH_ALGORITHM_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HashProfile {
    pub profile_version: u32,
    pub algorithm_version: u32,
    pub d_hash_bits: u32,
    pub p_hash_bits: u32,
    pub visual_match_threshold: f64,
    pub low_information_std_dev_threshold: f64,
}

impl HashProfile {
    pub const fn current() -> Self {
        Self {
            profile_version: DUPLICATE_HASH_PROFILE_VERSION,
            algorithm_version: DUPLICATE_HASH_ALGORITHM_VERSION,
            d_hash_bits: 1_024,
            p_hash_bits: 64,
            visual_match_threshold: 0.80,
            low_information_std_dev_threshold: 10.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateScanState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl DuplicateScanState {
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
pub struct DuplicateScanRun {
    pub run_id: String,
    pub revision: u64,
    pub state: DuplicateScanState,
    pub total_artifacts: u32,
    pub hashed_artifacts: u32,
    pub total_pairs: u64,
    pub compared_pairs: u64,
    pub candidates_found: u32,
    pub started_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateRelation {
    Exact,
    Contains,
    Partial,
    TranslationVisual,
}

impl DuplicateRelation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Contains => "contains",
            Self::Partial => "partial",
            Self::TranslationVisual => "translation_visual",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "exact" => Some(Self::Exact),
            "contains" => Some(Self::Contains),
            "partial" => Some(Self::Partial),
            "translation_visual" => Some(Self::TranslationVisual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGalleryRef {
    pub gallery_id: GalleryId,
    pub entry_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub page_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateCandidate {
    pub candidate_id: String,
    pub revision: u64,
    pub parent: DuplicateGalleryRef,
    pub candidate: DuplicateGalleryRef,
    pub relation: DuplicateRelation,
    pub confidence: f64,
    pub matched_pages: u32,
    pub parent_coverage: f64,
    pub candidate_coverage: f64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateEvidenceKind {
    ExactSha256,
    VisualHash,
    SequenceAlignment,
    EHentaiRelation,
}

impl DuplicateEvidenceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactSha256 => "exact_sha256",
            Self::VisualHash => "visual_hash",
            Self::SequenceAlignment => "sequence_alignment",
            Self::EHentaiRelation => "e_hentai_relation",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "exact_sha256" => Some(Self::ExactSha256),
            "visual_hash" => Some(Self::VisualHash),
            "sequence_alignment" => Some(Self::SequenceAlignment),
            "e_hentai_relation" => Some(Self::EHentaiRelation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateEvidence {
    pub evidence_id: String,
    pub kind: DuplicateEvidenceKind,
    pub confidence: f64,
    pub matched_pages: u32,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePagePair {
    pub parent_source_page: u32,
    pub candidate_source_page: u32,
    pub exact_sha256: bool,
    pub d_hash_distance: u32,
    pub p_hash_distance: u32,
    pub detail_hash_distance: u32,
    pub edge_similarity: f64,
    pub visual_similarity: f64,
    pub low_information: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateDecisionAction {
    HideParent,
    HideCandidate,
    SeriesLink,
    SeriesUnlink,
    ExcludePair,
}

impl DuplicateDecisionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HideParent => "hide_parent",
            Self::HideCandidate => "hide_candidate",
            Self::SeriesLink => "series_link",
            Self::SeriesUnlink => "series_unlink",
            Self::ExcludePair => "exclude_pair",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "hide_parent" => Some(Self::HideParent),
            "hide_candidate" => Some(Self::HideCandidate),
            "series_link" => Some(Self::SeriesLink),
            "series_unlink" => Some(Self::SeriesUnlink),
            "exclude_pair" => Some(Self::ExcludePair),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DuplicateDecisionRequest {
    pub candidate_id: String,
    pub expected_revision: u64,
    pub action: DuplicateDecisionAction,
    pub target_gallery_id: Option<i64>,
    pub series_group_id: Option<String>,
    pub series_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDecisionHistory {
    pub decision_id: String,
    pub candidate_id: String,
    pub candidate_revision: u64,
    pub action: DuplicateDecisionAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_gallery_id: Option<GalleryId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_group_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesGroup {
    pub series_group_id: String,
    pub name: String,
    pub revision: u64,
    pub members: Vec<DuplicateGalleryRef>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateReview {
    pub candidate: DuplicateCandidate,
    pub evidence: Vec<DuplicateEvidence>,
    pub page_pairs: Vec<DuplicatePagePair>,
    pub decisions: Vec<DuplicateDecisionHistory>,
    pub series_groups: Vec<SeriesGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSnapshot {
    pub profile: HashProfile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<DuplicateScanRun>,
    pub candidates: Vec<DuplicateCandidate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DuplicatePageHash {
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub source_page_number: SourcePageNumber,
    pub profile_version: u32,
    pub artifact_sha256: ArtifactSha256,
    pub coarse_d_hash: u64,
    pub detail_d_hash_hex: String,
    pub p_hash: u64,
    pub mean_luma: f64,
    pub std_dev: f64,
    pub non_uniform_ratio: f64,
    pub edge_density: f64,
    pub width: u32,
    pub height: u32,
    pub low_information: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateCandidateRecord {
    pub run_id: String,
    pub candidate: DuplicateCandidate,
    pub evidence: Vec<DuplicateEvidence>,
    pub page_pairs: Vec<DuplicatePagePair>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DuplicateDecisionApplyOutcome {
    Applied(Box<DuplicateReview>),
    CandidateNotFound,
    RevisionConflict { actual_revision: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalRelationEvidence {
    pub confidence: f64,
    pub description: String,
}
