use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use super::{DownloadReviewKind, GalleryId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRef {
    pub job_id: String,
    pub reused: bool,
    #[serde(skip)]
    pub worker_attempt: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    ResolvingMetadata,
    Downloading,
    Hashing,
    Verifying,
    RetryWait,
    ReviewRequired,
    Interrupted,
    Failed,
    Completed,
    Quarantined,
    Cancelled,
}

impl JobState {
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::ResolvingMetadata
                | Self::Downloading
                | Self::Hashing
                | Self::Verifying
                | Self::RetryWait
        )
    }

    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Interrupted | Self::Failed | Self::Cancelled)
    }

    pub const fn allows_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Queued,
                Self::ResolvingMetadata | Self::Interrupted | Self::Failed | Self::Cancelled
            ) | (
                Self::ResolvingMetadata,
                Self::Downloading
                    | Self::RetryWait
                    | Self::ReviewRequired
                    | Self::Interrupted
                    | Self::Failed
                    | Self::Cancelled
            ) | (
                Self::Downloading,
                Self::Hashing
                    | Self::RetryWait
                    | Self::ReviewRequired
                    | Self::Interrupted
                    | Self::Failed
                    | Self::Cancelled
            ) | (
                Self::Hashing,
                Self::Verifying
                    | Self::RetryWait
                    | Self::ReviewRequired
                    | Self::Interrupted
                    | Self::Failed
                    | Self::Cancelled
            ) | (
                Self::Verifying,
                Self::Completed
                    | Self::RetryWait
                    | Self::ReviewRequired
                    | Self::Interrupted
                    | Self::Failed
                    | Self::Quarantined
                    | Self::Cancelled
            ) | (
                Self::RetryWait,
                Self::Queued | Self::Interrupted | Self::Failed | Self::Cancelled
            ) | (
                Self::ReviewRequired,
                Self::Queued | Self::Failed | Self::Quarantined | Self::Cancelled
            ) | (
                Self::Interrupted | Self::Failed | Self::Cancelled,
                Self::Queued
            ) | (Self::Interrupted, Self::Failed)
                | (Self::Interrupted | Self::Failed, Self::Cancelled)
                | (
                    Self::Completed,
                    Self::Quarantined | Self::ReviewRequired | Self::Failed
                )
                | (Self::Quarantined, Self::Completed)
        )
    }
}

impl fmt::Display for JobState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Queued => "queued",
            Self::ResolvingMetadata => "resolving_metadata",
            Self::Downloading => "downloading",
            Self::Hashing => "hashing",
            Self::Verifying => "verifying",
            Self::RetryWait => "retry_wait",
            Self::ReviewRequired => "review_required",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Quarantined => "quarantined",
            Self::Cancelled => "cancelled",
        };
        formatter.write_str(value)
    }
}

impl FromStr for JobState {
    type Err = super::ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "resolving_metadata" => Ok(Self::ResolvingMetadata),
            "downloading" => Ok(Self::Downloading),
            "hashing" => Ok(Self::Hashing),
            "verifying" => Ok(Self::Verifying),
            "retry_wait" => Ok(Self::RetryWait),
            "review_required" => Ok(Self::ReviewRequired),
            "interrupted" => Ok(Self::Interrupted),
            "failed" => Ok(Self::Failed),
            "completed" => Ok(Self::Completed),
            "quarantined" => Ok(Self::Quarantined),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(super::ValidationError::new(
                "downloadState",
                "contains an unsupported value",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gallery_id: Option<i64>,
    pub revision: u64,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadChangedEvent {
    pub entry_id: String,
    pub gallery_id: i64,
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
    pub review_kind: Option<DownloadReviewKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadJobDescriptor {
    pub job_id: String,
    pub entry_id: String,
    pub gallery_id: GalleryId,
    pub worker_attempt: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureDownloadJobStep {
    ResolvingMetadata,
    FoundationUnavailable,
}

impl FixtureDownloadJobStep {
    pub const fn state(self) -> JobState {
        match self {
            Self::ResolvingMetadata => JobState::ResolvingMetadata,
            Self::FoundationUnavailable => JobState::Interrupted,
        }
    }

    pub const fn follows(self, current: JobState) -> bool {
        current.allows_transition_to(self.state())
            && matches!(
                (current, self),
                (JobState::Queued, Self::ResolvingMetadata)
                    | (JobState::ResolvingMetadata, Self::FoundationUnavailable)
            )
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::ResolvingMetadata => "Fixture download is validating the queued request",
            Self::FoundationUnavailable => {
                "Download interrupted: the remote artifact pipeline is not implemented yet"
            }
        }
    }

    pub const fn completed_units(self) -> u64 {
        0
    }

    pub const fn total_units(self) -> u64 {
        1
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadJobProjection {
    pub job: JobEvent,
    pub download: DownloadChangedEvent,
}
