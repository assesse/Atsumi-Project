use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSourceRef {
    pub provider: String,
    pub recording_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<String>,
}
impl CloudSourceRef {
    pub(crate) fn valid(&self) -> bool {
        matches!(self.provider.as_str(), "chzzk" | "youtube" | "soop")
            && !self.recording_id.is_empty()
            && self.recording_id.len() <= 128
            && self
                .recording_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            && self.part_index.is_none_or(|part| part < 100_000)
            && match (&self.plan_id, &self.unit_id) {
                (None, None) => true,
                (Some(plan), Some(unit)) => {
                    self.part_index.is_none()
                        && uuid::Uuid::parse_str(plan).is_ok()
                        && uuid::Uuid::parse_str(unit).is_ok()
                }
                _ => false,
            }
    }
}

/// Native-owned handle and lease. Neither paths nor bytes are accepted from IPC.
pub struct CloudSource {
    pub file: File,
    pub source: CloudSourceRef,
    pub title: String,
    pub duration_seconds: f64,
    pub media_offset_seconds: f64,
    pub lease: Box<dyn Send + Sync>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourcePreparationProgress {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_seconds: Option<f64>,
}
impl SourcePreparationProgress {
    pub(crate) fn valid(&self) -> bool {
        matches!(self.phase.as_str(), "checking" | "preparing" | "verifying")
            && self
                .completed_bytes
                .zip(self.total_bytes)
                .is_none_or(|(done, total)| done <= total)
            && self
                .completed_seconds
                .is_none_or(|v| v.is_finite() && v >= 0.0)
            && self.total_seconds.is_none_or(|v| v.is_finite() && v > 0.0)
            && self
                .completed_seconds
                .zip(self.total_seconds)
                .is_none_or(|(done, total)| done <= total)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
impl CloudError {
    pub fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
        }
    }
    pub(crate) fn storage() -> Self {
        Self::new(
            "CLOUD_STORAGE",
            "업로드 상태를 안전하게 저장하지 못했습니다. 원본은 유지됩니다.",
            true,
        )
    }
    pub(crate) fn auth() -> Self {
        Self::new(
            "CLOUD_AUTH_REQUIRED",
            "YouTube 업로드 계정을 연결해 주세요.",
            true,
        )
    }
    pub(crate) fn cancelled() -> Self {
        Self::new(
            "CLOUD_PAUSED",
            "업로드를 일시정지했습니다. 원본과 전송 상태는 유지됩니다.",
            true,
        )
    }
    pub(crate) fn source() -> Self {
        Self::new(
            "CLOUD_SOURCE_CHANGED",
            "녹화 원본을 확인하지 못했거나 변경됐습니다. 원본 상태를 확인해 주세요.",
            false,
        )
    }
    /// Native preparation code emits only these controlled categories. Never
    /// forward arbitrary filesystem, helper, provider or path-bearing text.
    pub(crate) fn source_failure(raw: &str, planning: bool, cancelled: bool) -> Self {
        if cancelled {
            return Self::cancelled();
        }
        if raw.starts_with("CLOUD_SOURCE_SPACE:") {
            Self::new("CLOUD_SOURCE_SPACE", "업로드 준비 공간이 부족합니다. 저장공간을 확보한 뒤 다시 시도해 주세요. 원본은 유지됩니다.", true)
        } else if raw.starts_with("CLOUD_SOURCE_IO:") {
            Self::new("CLOUD_SOURCE_IO", "업로드 준비 파일을 읽거나 저장하지 못했습니다. 저장소 연결과 여유 공간을 확인한 뒤 다시 시도해 주세요.", true)
        } else if raw.starts_with("CLOUD_SOURCE_BUSY:") {
            Self::new("CLOUD_SOURCE_BUSY", "녹화 파일 또는 준비 도구가 사용 중입니다. 잠시 후 다시 시도해 주세요. 원본은 유지됩니다.", true)
        } else if raw.starts_with("CLOUD_SOURCE_COVERAGE:") {
            Self::new("CLOUD_SOURCE_COVERAGE", "녹화 원본의 전체 구간을 확인하지 못했습니다. 원본 처리가 끝난 뒤 다시 확인해 주세요.", planning)
        } else {
            Self::source()
        }
    }
    pub(crate) fn protocol() -> Self {
        Self::new(
            "CLOUD_RESPONSE",
            "YouTube 응답을 확인하지 못했습니다. 원본은 유지됩니다.",
            true,
        )
    }
}
impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CloudError {}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAccountSnapshot {
    pub configured: bool,
    pub connected: bool,
    pub authorizing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CloudError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudLivePlan {
    pub title: String,
    pub scheduled_start_ms: u64,
    pub made_for_kids: bool,
}
impl CloudLivePlan {
    pub(crate) fn valid(&self) -> bool {
        !self.title.trim().is_empty() && self.title.chars().count() <= 100
            && !self.title.chars().any(|c| c.is_control() || matches!(c, '<' | '>'))
            && i64::try_from(self.scheduled_start_ms).ok()
                .and_then(chrono::DateTime::from_timestamp_millis).is_some()
    }
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudLiveAccountSnapshot {
    pub configured: bool,
    pub connected: bool,
    pub authorizing: bool,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub generation: u64,
    pub error: Option<CloudError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudLiveOperation {
    pub receipt_persisted: bool,
    pub selection_id: String,
    pub revision: u64,
    pub provider: String,
    pub channel_id: String,
    pub broadcast_key: Option<String>,
    pub state: String,
    pub preparation_stage: Option<String>,
    pub destination_channel_id: Option<String>,
    pub stream_id: Option<String>,
    pub broadcast_id: Option<String>,
    pub outcome_uncertain: bool,
    pub run_id: Option<String>,
    pub stop_requested: bool,
    pub interrupted: bool,
    pub segment_acks: u64,
    pub playlist_acks: u64,
    pub end_list_acked: bool,
    pub manager_joined: bool,
    pub worker_joined: bool,
    pub sidecar_joined: bool,
    pub sidecar_text_state: Option<String>,
    pub assets_incomplete: bool,
    pub timing_incomplete: bool,
    pub clock_quality: Option<String>,
    pub archive_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_verification: Option<super::live_archive::ArchiveSnapshot>,
    pub error: Option<CloudError>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudLiveSnapshot {
    pub account: CloudLiveAccountSnapshot,
    pub operations: Vec<CloudLiveOperation>,
}
/// Missing values belong to uploads created before link-access archives existed.
/// New admissions default to Private; recovery never changes persisted intent.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CloudVisibility {
    #[default]
    Private,
    Unlisted,
}
impl CloudVisibility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Unlisted => "unlisted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudJobSnapshot {
    pub id: String,
    pub source: CloudSourceRef,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub visibility: CloudVisibility,
    #[serde(default)]
    pub recording_plan: bool,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    /// Opaque derived identity for replay lifetime; never upload authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_cohort_id: Option<String>,
    pub duration_seconds: f64,
    pub media_offset_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CloudError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<SourcePreparationProgress>,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSnapshot {
    pub account: CloudAccountSnapshot,
    pub jobs: Vec<CloudJobSnapshot>,
    pub automatic: super::automatic::CloudAutomaticSnapshot,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
pub(crate) fn video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
pub(crate) fn channel_id(value: &str) -> bool {
    value.len() == 24
        && value.starts_with("UC")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
pub(crate) fn job_state(value: &str) -> bool {
    matches!(
        value,
        "queued"
            | "validating"
            | "uploading"
            | "paused"
            | "waiting_auth"
            | "retry_wait"
            | "processing"
            | "ready"
            | "failed"
    )
}
