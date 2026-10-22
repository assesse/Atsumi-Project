use super::{Error, Limits, Result};
use crate::platform_media::live::RecordingOwner;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const PROOF: &str = "proof.json";
pub(super) const JOURNAL: &str = "journal.jsonl";
pub(super) const RAW: [&str; 2] = ["video.raw", "audio.raw"];

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Owner {
    Manual,
    Scheduled {
        schedule_id: String,
        run_id: String,
        broadcast_key: String,
        generation: u64,
    },
}
impl<'de> Deserialize<'de> for Owner {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        // Serde's internally tagged unit variant ignores residual map fields.
        // A zero-field struct variant enforces the same closed schema as Scheduled.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
        enum ClosedOwner {
            Manual {},
            Scheduled {
                schedule_id: String,
                run_id: String,
                broadcast_key: String,
                generation: u64,
            },
        }
        Ok(match ClosedOwner::deserialize(deserializer)? {
            ClosedOwner::Manual {} => Self::Manual,
            ClosedOwner::Scheduled {
                schedule_id,
                run_id,
                broadcast_key,
                generation,
            } => Self::Scheduled {
                schedule_id,
                run_id,
                broadcast_key,
                generation,
            },
        })
    }
}
impl From<RecordingOwner> for Owner {
    fn from(value: RecordingOwner) -> Self {
        match value {
            RecordingOwner::Manual => Self::Manual,
            RecordingOwner::Scheduled {
                schedule_id,
                run_id,
                broadcast_key,
                generation,
            } => Self::Scheduled {
                schedule_id,
                run_id,
                broadcast_key,
                generation,
            },
        }
    }
}
impl Owner {
    fn valid(&self, video: &str, channel: &str) -> bool {
        match self {
            Self::Manual => true,
            Self::Scheduled {
                schedule_id,
                run_id,
                broadcast_key,
                generation,
            } => {
                schedule_id == channel && broadcast_key == video && uuid(run_id) && *generation > 0
            }
        }
    }
}

/// Syntactic ownership identity, NOT consent or current receiver authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SpoolIdentity {
    pub job_id: String,
    pub request_id: String,
    pub receiver_id: String,
    pub source_id: String,
    pub document_id: String,
    pub document_epoch: u64,
    pub source_generation: u64,
    pub video_id: String,
    pub channel_id: String,
    pub owner: Owner,
}
impl SpoolIdentity {
    pub(super) fn validate(&self) -> Result<()> {
        if [
            &self.job_id,
            &self.request_id,
            &self.receiver_id,
            &self.source_id,
            &self.document_id,
        ]
        .into_iter()
        .all(|s| uuid(s))
            && self.document_epoch > 0
            && self.source_generation > 0
            && identifier(&self.video_id, 11)
            && identifier(&self.channel_id, 24)
            && self.channel_id.starts_with("UC")
            && self.owner.valid(&self.video_id, &self.channel_id)
        {
            Ok(())
        } else {
            Err(Error::Identity)
        }
    }
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| !id.is_nil() && id.to_string() == s)
}
fn identifier(s: &str, length: usize) -> bool {
    s.len() == length
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Track {
    Video,
    Audio,
}
impl Track {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Video => 0,
            Self::Audio => 1,
        }
    }
}

/// Claimed immutable source clocks; a later native parser must certify them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClockDescriptor {
    pub video_timestamp_scale_ns: u64,
    pub audio_timescale: u64,
    pub mode: Mode,
    pub append_window: Window,
    pub video_timestamp_offset: i64,
    pub audio_timestamp_offset: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Mode {
    Segments,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Window {
    Default,
}
impl ClockDescriptor {
    pub(super) fn validate(&self) -> Result<()> {
        if (1..=1_000_000_000).contains(&self.video_timestamp_scale_ns)
            && (1..=1_000_000_000).contains(&self.audio_timescale)
            && self.video_timestamp_offset == 0
            && self.audio_timestamp_offset == 0
        {
            Ok(())
        } else {
            Err(Error::Proof)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Proof {
    pub version: u8,
    pub provider: Provider,
    pub branch: Branch,
    pub identity: SpoolIdentity,
    pub clock: ClockDescriptor,
    pub video: VideoFile,
    pub audio: AudioFile,
    pub journal: JournalFile,
    pub limits: Limits,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) enum Provider {
    #[serde(rename = "youtube")]
    Youtube,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) enum Branch {
    #[serde(rename = "youtubeVp9AacRawV1")]
    RawV1,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) enum VideoFile {
    #[serde(rename = "video.raw")]
    Video,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) enum AudioFile {
    #[serde(rename = "audio.raw")]
    Audio,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) enum JournalFile {
    #[serde(rename = "journal.jsonl")]
    Journal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Receipt {
    pub sequence: u64,
    pub track: Track,
    pub append_index: u64,
    pub chunk_index: u32,
    pub offset: u64,
    pub length: u32,
    pub final_chunk: bool,
    pub payload_sha256: String,
    pub elapsed_ms: u64,
    pub previous_record_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Partial {
    pub track: Track,
    pub append_index: u64,
    pub next_chunk: u32,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TrackExtent {
    pub bytes: u64,
    pub sha256: String,
    pub next_append: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SealCause {
    Stopped,
    FormatBoundary,
    Cancelled,
    Limit,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Seal {
    pub cause: SealCause,
    pub receipts: u64,
    pub tracks: [TrackExtent; 2],
    pub partial: Option<Partial>,
    pub previous_record_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Row {
    Header {
        version: u8,
        #[serde(rename = "proofSha256")]
        proof_sha256: String,
    },
    Chunk(Receipt),
    Seal(Seal),
}
pub(super) fn line(row: &Row) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(row).map_err(|_| Error::Proof)?;
    bytes.push(b'\n');
    if bytes.len() > 1024 {
        Err(Error::Bounds)
    } else {
        Ok(bytes)
    }
}
pub(super) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn digest_valid(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
