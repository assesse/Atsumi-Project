use serde::{Deserialize, Serialize};

use super::{
    DownloadEntryId, DownloadJobDescriptor, DownloadJobProjection, GalleryId, ValidationError,
};

pub const DOWNLOAD_OVERLAP_POLICY_VERSION: u32 = 2;
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DownloadOverlapDecisionAudit>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "continue_keep_both" | "keep_both_continue" => Some(Self::KeepBothContinue),
            "false_positive_continue" => Some(Self::FalsePositiveContinue),
            "remove_existing_continue" => Some(Self::RemoveExistingContinue),
            "cancel_incoming" | "remove_incoming" => Some(Self::RemoveIncoming),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadOverlapAutomationHistoryListRequest {
    pub page: u32,
    pub page_size: u32,
}

impl DownloadOverlapAutomationHistoryListRequest {
    pub fn normalized(self) -> Result<Self, ValidationError> {
        if self.page == 0 {
            return Err(ValidationError::new("page", "must be one-based"));
        }
        if !(1..=200).contains(&self.page_size) {
            return Err(ValidationError::new(
                "pageSize",
                "must be between 1 and 200",
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapAutomationHistoryItem {
    pub review_id: String,
    pub incoming_gallery_id: GalleryId,
    pub title: String,
    pub occurred_at: String,
    pub review_state: DownloadOverlapReviewState,
    pub remove_incoming_count: u64,
    pub remove_existing_count: u64,
    pub removed_gallery_ids: Vec<GalleryId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapAutomationHistoryPage {
    pub total_items: u64,
    pub unacknowledged_items: u64,
    pub page: u32,
    pub page_size: u32,
    pub items: Vec<DownloadOverlapAutomationHistoryItem>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "automation" => Some(Self::Automation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapDecisionAudit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    pub action: DownloadOverlapDecisionAction,
    pub actor: DownloadOverlapDecisionActor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_snapshot_json: Option<String>,
    pub created_at: String,
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
    pub gallery_id: GalleryId,
    pub artists: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_database_values_accept_current_and_legacy_actions() {
        for (stored, expected) in [
            (
                "continue_keep_both",
                DownloadOverlapDecisionAction::KeepBothContinue,
            ),
            (
                "keep_both_continue",
                DownloadOverlapDecisionAction::KeepBothContinue,
            ),
            (
                "false_positive_continue",
                DownloadOverlapDecisionAction::FalsePositiveContinue,
            ),
            (
                "remove_existing_continue",
                DownloadOverlapDecisionAction::RemoveExistingContinue,
            ),
            (
                "cancel_incoming",
                DownloadOverlapDecisionAction::RemoveIncoming,
            ),
            (
                "remove_incoming",
                DownloadOverlapDecisionAction::RemoveIncoming,
            ),
        ] {
            assert_eq!(
                DownloadOverlapDecisionAction::from_database(stored),
                Some(expected)
            );
        }
        assert_eq!(
            DownloadOverlapDecisionAction::from_database("unknown"),
            None
        );
        assert_eq!(
            DownloadOverlapDecisionActor::from_database("human"),
            Some(DownloadOverlapDecisionActor::Human)
        );
        assert_eq!(
            DownloadOverlapDecisionActor::from_database("automation"),
            Some(DownloadOverlapDecisionActor::Automation)
        );
        assert_eq!(DownloadOverlapDecisionActor::from_database("unknown"), None);
    }

    #[test]
    fn decision_audit_serializes_with_the_review_contract_names() {
        let audit = DownloadOverlapDecisionAudit {
            candidate_id: Some("candidate-a".to_owned()),
            action: DownloadOverlapDecisionAction::RemoveExistingContinue,
            actor: DownloadOverlapDecisionActor::Automation,
            reason_code: Some("balanced_overlap_v3".to_owned()),
            rule_version: Some(3),
            feature_snapshot_json: Some("{\"winner\":\"incoming\"}".to_owned()),
            created_at: "2026-09-05T00:00:00Z".to_owned(),
        };
        let value = serde_json::to_value(&audit).expect("serialize decision audit");

        assert_eq!(value["candidateId"], "candidate-a");
        assert_eq!(value["action"], "remove_existing_continue");
        assert_eq!(value["actor"], "automation");
        assert_eq!(value["reasonCode"], "balanced_overlap_v3");
        assert_eq!(value["ruleVersion"], 3);
        assert_eq!(value["featureSnapshotJson"], "{\"winner\":\"incoming\"}");
        assert_eq!(value["createdAt"], "2026-09-05T00:00:00Z");

        let review = DownloadOverlapReview {
            review_id: "review-a".to_owned(),
            entry_id: "entry-a".to_owned(),
            incoming: DownloadOverlapGalleryRef {
                entry_id: "entry-a".to_owned(),
                gallery_id: GalleryId::new(1).expect("valid gallery ID"),
                title: "Incoming".to_owned(),
                artists: Vec::new(),
                page_count: 1,
            },
            revision: 0,
            state: DownloadOverlapReviewState::Pending,
            profile_version: 1,
            policy_version: 1,
            incoming_fingerprint: "a".repeat(64),
            candidates: Vec::new(),
            decisions: Vec::new(),
            created_at: "2026-09-05T00:00:00Z".to_owned(),
            updated_at: "2026-09-05T00:00:00Z".to_owned(),
            resolved_at: None,
        };
        let review_value = serde_json::to_value(review).expect("serialize empty review audit");
        assert!(review_value.get("decisions").is_none());
    }
}
