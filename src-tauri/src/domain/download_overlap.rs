use serde::{Deserialize, Serialize};

use super::{DownloadEntryId, DownloadJobDescriptor, DownloadJobProjection, GalleryId};

pub const DOWNLOAD_OVERLAP_POLICY_VERSION: u32 = 1;
pub const DOWNLOAD_OVERLAP_MAX_STORED_PAGE_PAIRS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapRelation {
    NearEquivalent,
    IncomingContainsExisting,
    ExistingContainsIncoming,
    PartialOverlap,
    TranslationEdition,
}

impl DownloadOverlapRelation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NearEquivalent => "near_equivalent",
            Self::IncomingContainsExisting => "incoming_contains_existing",
            Self::ExistingContainsIncoming => "existing_contains_incoming",
            Self::PartialOverlap => "partial_overlap",
            Self::TranslationEdition => "translation_edition",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "near_equivalent" => Some(Self::NearEquivalent),
            "incoming_contains_existing" => Some(Self::IncomingContainsExisting),
            "existing_contains_incoming" => Some(Self::ExistingContainsIncoming),
            "partial_overlap" => Some(Self::PartialOverlap),
            "translation_edition" => Some(Self::TranslationEdition),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapReviewState {
    Pending,
    Resolved,
    Cancelled,
    Stale,
}

impl DownloadOverlapReviewState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Cancelled => "cancelled",
            Self::Stale => "stale",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "resolved" => Some(Self::Resolved),
            "cancelled" => Some(Self::Cancelled),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapPairDecision {
    KeepBoth,
    FalsePositive,
    ExistingRemoved,
}

impl DownloadOverlapPairDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeepBoth => "keep_both",
            Self::FalsePositive => "false_positive",
            Self::ExistingRemoved => "existing_removed",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "keep_both" => Some(Self::KeepBoth),
            "false_positive" => Some(Self::FalsePositive),
            "existing_removed" => Some(Self::ExistingRemoved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapGalleryRef {
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub title: String,
    pub artists: Vec<String>,
    pub page_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapPagePair {
    pub incoming_source_page: u32,
    pub existing_source_page: u32,
    pub exact_sha256: bool,
    pub d_hash_distance: u32,
    pub p_hash_distance: u32,
    pub detail_hash_distance: u32,
    pub edge_similarity: f64,
    pub visual_similarity: f64,
    pub low_information: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapCandidate {
    pub candidate_id: String,
    pub existing: DownloadOverlapGalleryRef,
    pub existing_fingerprint: String,
    pub relation: DownloadOverlapRelation,
    pub confidence: f64,
    pub matched_pages: u32,
    pub exact_pages: u32,
    pub visual_pages: u32,
    pub existing_coverage: f64,
    pub incoming_coverage: f64,
    pub existing_unique_pages: u32,
    pub incoming_unique_pages: u32,
    pub longest_aligned_run: u32,
    pub rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<DownloadOverlapPairDecision>,
    pub page_pairs: Vec<DownloadOverlapPagePair>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapReview {
    pub review_id: String,
    pub entry_id: String,
    pub incoming: DownloadOverlapGalleryRef,
    pub revision: u64,
    pub state: DownloadOverlapReviewState,
    pub profile_version: u32,
    pub policy_version: u32,
    pub incoming_fingerprint: String,
    pub candidates: Vec<DownloadOverlapCandidate>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadOverlapReviewDraft {
    pub review_id: String,
    pub entry_id: DownloadEntryId,
    pub incoming: DownloadOverlapGalleryRef,
    pub profile_version: u32,
    pub policy_version: u32,
    pub incoming_fingerprint: String,
    pub candidates: Vec<DownloadOverlapCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapDecisionAction {
    #[serde(alias = "continue_keep_both")]
    KeepBothContinue,
    FalsePositiveContinue,
    RemoveExistingContinue,
    #[serde(alias = "cancel_incoming")]
    RemoveIncoming,
}

impl DownloadOverlapDecisionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeepBothContinue => "keep_both_continue",
            Self::FalsePositiveContinue => "false_positive_continue",
            Self::RemoveExistingContinue => "remove_existing_continue",
            Self::RemoveIncoming => "remove_incoming",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapDecisionActor {
    #[default]
    Human,
    Automation,
}

impl DownloadOverlapDecisionActor {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Automation => "automation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadOverlapDecisionRequest {
    pub review_id: String,
    pub expected_revision: u64,
    pub action: DownloadOverlapDecisionAction,
    #[serde(default)]
    pub candidate_id: Option<String>,
    #[serde(default)]
    pub actor: DownloadOverlapDecisionActor,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub rule_version: Option<u32>,
    #[serde(default)]
    pub feature_snapshot_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapDecisionResult {
    pub review: DownloadOverlapReview,
    pub resumed: bool,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadOverlapDecisionApplyOutcome {
    Applied(Box<DownloadOverlapDecisionApplied>),
    ReviewNotFound,
    RevisionConflict { actual_revision: u64 },
    InvalidCandidate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadOverlapDecisionApplied {
    pub result: DownloadOverlapDecisionResult,
    pub projection: Option<DownloadJobProjection>,
    pub removed_existing_projection: Option<DownloadJobProjection>,
    pub resume: Option<DownloadJobDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOverlapCandidateIdentity {
    pub entry_id: DownloadEntryId,
    pub artists: Vec<String>,
}
