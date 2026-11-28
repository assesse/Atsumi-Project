//! Read-only, exact-resource Live archive metadata. Never playback or clock authority.
use super::{
    model::{channel_id, video_id, CloudError},
    upload::{Request, Response, Transport},
};
use reqwest::{blocking::Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, time::Duration};

pub(super) const WINDOW_MS: u64 = 24 * 60 * 60 * 1000;
pub(super) const MAX_ROUNDS: u32 = 128;
pub(super) const MAX_REQUESTS: u32 = 256;
const MAX_RESPONSE: usize = 512 * 1024;
const RECORD_BYTES: usize = 3072;
const BROADCAST_FIELDS: &str = "items(id,kind,snippet(channelId,actualStartTime,actualEndTime),status(lifeCycleStatus,recordingStatus,privacyStatus),contentDetails/boundStreamId)";
const VIDEO_FIELDS: &str = "items(id,kind,snippet/channelId,status(privacyStatus,uploadStatus,embeddable),contentDetails/duration,processingDetails/processingStatus,liveStreamingDetails(actualStartTime,actualEndTime))";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveState {
    Checking,
    Pending,
    NeedsCheck,
    MetadataVerified,
    Inconclusive,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveReason {
    Nonterminal,
    NotFound,
    Processing,
    MetadataMissing,
    NotEmbeddable,
    OwnerMismatch,
    IdentityMismatch,
    PrivacyMismatch,
    ProcessingFailed,
    InvalidResponse,
    Network,
    Quota,
    Authorization,
    RemoteRejected,
    Deadline,
    RequestLimit,
    Cancelled,
    OwnerChanged,
    SaveFailed,
    WorkerPanic,
    Restarted,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveSnapshot {
    pub state: ArchiveState,
    pub reason: Option<ArchiveReason>,
    pub checked_at: Option<u64>,
    pub deadline_at: u64,
    pub retry_at: Option<u64>,
    pub rounds: u32,
    pub video_id: Option<String>,
    pub duration_seconds: Option<f64>,
    pub embeddable: Option<bool>,
    pub processing_info_unavailable: bool,
    pub playback_verified: bool,
    pub remote_time_origin: String,
    pub synchronized_replay_ready: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Identity {
    pub selection_id: String,
    pub request_id: String,
    pub run_id: String,
    pub source_fingerprint: String,
    pub owner_channel_id: String,
    pub creation_owner_generation: u64,
    pub broadcast_id: String,
    pub stream_id: String,
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|v| !v.is_nil())
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Identity {
    pub(super) fn valid(&self) -> bool {
        uuid(&self.selection_id)
            && uuid(&self.request_id)
            && uuid(&self.run_id)
            && digest(&self.source_fingerprint)
            && channel_id(&self.owner_channel_id)
            && self.creation_owner_generation > 0
            && video_id(&self.broadcast_id)
            && identifier(&self.stream_id)
    }
}
// Native-only authorization, deliberately no Debug/Serialize/Deserialize.
pub(super) struct Authorization {
    token: String,
    owner: String,
    pub generation: u64,
}
impl Authorization {
    pub(super) fn new(
        token: String,
        owner: String,
        generation: u64,
    ) -> Result<Self, ArchiveReason> {
        if token.is_empty()
            || token.len() > 16384
            || !token.bytes().all(|b| b.is_ascii_graphic())
            || !channel_id(&owner)
            || generation == 0
        {
            return Err(ArchiveReason::Authorization);
        }
        Ok(Self {
            token,
            owner,
            generation,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Observation {
    pub broadcast_sha256: Option<String>,
    pub video_sha256: Option<String>,
    pub lifecycle: Option<String>,
    pub recording: Option<String>,
    pub upload: Option<String>,
    pub processing: Option<String>,
    pub broadcast_start: Option<String>,
    pub broadcast_end: Option<String>,
    pub video_start: Option<String>,
    pub video_end: Option<String>,
    pub duration: Option<String>,
    pub duration_seconds: Option<f64>,
    pub embeddable: Option<bool>,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            broadcast_sha256: None,
            video_sha256: None,
            lifecycle: None,
            recording: None,
            upload: None,
            processing: None,
            broadcast_start: None,
            broadcast_end: None,
            video_start: None,
            video_end: None,
            duration: None,
            duration_seconds: None,
            embeddable: None,
        }
    }
}
fn time_valid(s: &str) -> bool {
    s.len() <= 40 && chrono::DateTime::parse_from_rfc3339(s).is_ok()
}
impl Observation {
    fn valid(&self) -> bool {
        [&self.broadcast_sha256, &self.video_sha256]
            .into_iter()
            .all(|x| x.as_deref().is_none_or(digest))
            && [
                &self.lifecycle,
                &self.recording,
                &self.upload,
                &self.processing,
            ]
            .into_iter()
            .all(|x| x.as_deref().is_none_or(|s| s.len() <= 32 && identifier(s)))
            && [
                &self.broadcast_start,
                &self.broadcast_end,
                &self.video_start,
                &self.video_end,
            ]
            .into_iter()
            .all(|x| x.as_deref().is_none_or(time_valid))
            && self.duration.as_deref().is_none_or(|s| {
                super::upload::iso_duration(s).is_some_and(|n| n.is_finite() && n > 0.0)
            })
            && self
                .duration_seconds
                .is_none_or(|s| s.is_finite() && s > 0.0)
            && self
                .duration
                .as_deref()
                .and_then(super::upload::iso_duration)
                == self.duration_seconds
    }
    fn verified(&self) -> bool {
        self.valid()
            && self.broadcast_sha256.is_some()
            && self.video_sha256.is_some()
            && self.lifecycle.as_deref() == Some("complete")
            && self.recording.as_deref() == Some("recorded")
            && self.broadcast_end.is_some()
            && self.upload.as_deref() == Some("processed")
            && matches!(self.processing.as_deref(), Some("succeeded" | "terminated"))
            && self.embeddable == Some(true)
            && self.duration_seconds.is_some()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct VerificationProof {
    pub id: String,
    pub checked_at: u64,
    pub observation: Observation,
    pub sha256: String,
}
fn proof_digest(
    identity: &Identity,
    id: &str,
    at: u64,
    facts: &Observation,
) -> Result<String, ArchiveReason> {
    serde_json::to_vec(&(1u32, identity, id, at, facts))
        .map(|b| sha(&b))
        .map_err(|_| ArchiveReason::InvalidResponse)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    pub version: u32,
    pub identity: Identity,
    pub attempt_id: String,
    pub attempt_number: u64,
    pub check_revision: u64,
    pub authorization_generation: u64,
    pub started_at: u64,
    pub deadline_at: u64,
    pub retry_at: Option<u64>,
    pub checked_at: Option<u64>,
    pub rounds: u32,
    pub requests: u32,
    pub state: ArchiveState,
    pub reason: Option<ArchiveReason>,
    pub last: Option<Observation>,
    pub verified: Option<VerificationProof>,
}
pub(super) fn delay_ms(completed_rounds: u32) -> u64 {
    if completed_rounds <= 4 {
        5_000
    } else if completed_rounds <= 14 {
        60_000
    } else {
        15 * 60_000
    }
}
impl Record {
    pub(super) fn begin(
        identity: Identity,
        generation: u64,
        at: u64,
        prior: Option<&Self>,
    ) -> Result<Self, ArchiveReason> {
        if !identity.valid()
            || generation == 0
            || at == 0
            || prior.is_some_and(|p| p.identity != identity)
        {
            return Err(ArchiveReason::IdentityMismatch);
        }
        Ok(Self {
            version: 1,
            identity,
            attempt_id: uuid::Uuid::new_v4().to_string(),
            attempt_number: prior
                .map_or(Some(1), |p| p.attempt_number.checked_add(1))
                .ok_or(ArchiveReason::RequestLimit)?,
            check_revision: prior
                .map_or(Some(1), |p| p.check_revision.checked_add(1))
                .ok_or(ArchiveReason::RequestLimit)?,
            authorization_generation: generation,
            started_at: at,
            deadline_at: at.checked_add(WINDOW_MS).ok_or(ArchiveReason::Deadline)?,
            retry_at: Some(at),
            checked_at: None,
            rounds: 0,
            requests: 0,
            state: ArchiveState::Pending,
            reason: None,
            last: None,
            verified: prior.and_then(|p| p.verified.clone()),
        })
    }
    pub(super) fn resume(&mut self, generation: u64, at: u64) -> Result<(), ArchiveReason> {
        self.budget(at)?;
        if generation == 0 {
            return Err(ArchiveReason::Authorization);
        }
        self.authorization_generation = generation;
        self.check_revision = self
            .check_revision
            .checked_add(1)
            .ok_or(ArchiveReason::RequestLimit)?;
        self.state = ArchiveState::Pending;
        self.reason = None;
        self.retry_at = Some(at);
        Ok(())
    }
    pub(super) fn budget(&self, at: u64) -> Result<(), ArchiveReason> {
        if at < self.started_at || at >= self.deadline_at {
            Err(ArchiveReason::Deadline)
        } else if self.rounds >= MAX_ROUNDS || self.requests >= MAX_REQUESTS {
            Err(ArchiveReason::RequestLimit)
        } else {
            Ok(())
        }
    }
    pub(super) fn start_round(&mut self, at: u64) -> Result<(), ArchiveReason> {
        self.budget(at)?;
        if self.retry_at.is_some_and(|due| at < due) {
            return Err(ArchiveReason::InvalidResponse);
        }
        self.rounds = self
            .rounds
            .checked_add(1)
            .ok_or(ArchiveReason::RequestLimit)?;
        self.state = ArchiveState::Checking;
        self.retry_at = None;
        self.reason = None;
        Ok(())
    }
    pub(super) fn request(&mut self, at: u64) -> Result<(), ArchiveReason> {
        if at < self.started_at || at >= self.deadline_at {
            return Err(ArchiveReason::Deadline);
        }
        if self.state != ArchiveState::Checking || self.requests >= MAX_REQUESTS {
            return Err(ArchiveReason::RequestLimit);
        }
        self.requests += 1;
        Ok(())
    }
    pub(super) fn stop(&mut self, reason: ArchiveReason) {
        self.state = ArchiveState::Inconclusive;
        self.reason = Some(reason);
        self.retry_at = None;
    }
    pub(super) fn apply(&mut self, result: Round, at: u64) -> Result<(), ArchiveReason> {
        if !result.facts.valid() {
            return Err(ArchiveReason::InvalidResponse);
        }
        self.checked_at = Some(at);
        self.last = Some(result.facts.clone());
        if at >= self.deadline_at {
            self.stop(ArchiveReason::Deadline);
            return Ok(());
        }
        self.state = result.state;
        self.reason = result.reason;
        self.retry_at = None;
        if self.state == ArchiveState::MetadataVerified {
            if !result.facts.verified() {
                return Err(ArchiveReason::InvalidResponse);
            }
            if self.verified.is_none() {
                let id = uuid::Uuid::new_v4().to_string();
                self.verified = Some(VerificationProof {
                    sha256: proof_digest(&self.identity, &id, at, &result.facts)?,
                    id,
                    checked_at: at,
                    observation: result.facts,
                });
            }
        } else if self.state == ArchiveState::Pending {
            if let Err(e) = self.budget(at) {
                self.stop(e);
            } else {
                self.retry_at = Some(
                    at.saturating_add(delay_ms(self.rounds))
                        .min(self.deadline_at),
                );
            }
        }
        Ok(())
    }
    pub(super) fn recover(&mut self) {
        if matches!(self.state, ArchiveState::Checking | ArchiveState::Pending) {
            self.state = ArchiveState::NeedsCheck;
            self.reason = Some(ArchiveReason::Restarted);
            self.retry_at = None;
        }
    }
    pub(super) fn snapshot(&self) -> ArchiveSnapshot {
        let facts = self.last.as_ref();
        ArchiveSnapshot {
            state: self.state,
            reason: self.reason,
            checked_at: self.checked_at,
            deadline_at: self.deadline_at,
            retry_at: self.retry_at,
            rounds: self.rounds,
            video_id: (self.state == ArchiveState::MetadataVerified)
                .then(|| self.identity.broadcast_id.clone()),
            duration_seconds: facts.and_then(|f| f.duration_seconds),
            embeddable: facts.and_then(|f| f.embeddable),
            processing_info_unavailable: facts
                .is_some_and(|f| f.processing.as_deref() == Some("terminated")),
            playback_verified: false,
            remote_time_origin: "unknown".into(),
            synchronized_replay_ready: false,
        }
    }
    pub(super) fn valid(&self) -> bool {
        self.version == 1
            && self.identity.valid()
            && uuid(&self.attempt_id)
            && self.attempt_number > 0
            && self.check_revision > 0
            && self.authorization_generation > 0
            && self.started_at > 0
            && self.started_at.checked_add(WINDOW_MS) == Some(self.deadline_at)
            && self.rounds <= MAX_ROUNDS
            && self.requests <= MAX_REQUESTS
            && self.requests <= self.rounds.saturating_mul(2)
            && self
                .retry_at
                .is_none_or(|t| t >= self.started_at && t <= self.deadline_at)
            && self.checked_at.is_none_or(|t| t >= self.started_at)
            && self.last.as_ref().is_none_or(Observation::valid)
            && (self.state != ArchiveState::MetadataVerified
                || (self.verified.is_some()
                    && self.last.as_ref().is_some_and(Observation::verified)))
            && self.verified.as_ref().is_none_or(|p| {
                uuid(&p.id)
                    && p.checked_at > 0
                    && p.observation.verified()
                    && proof_digest(&self.identity, &p.id, p.checked_at, &p.observation)
                        .is_ok_and(|h| h == p.sha256)
            })
            && serde_json::to_vec(self).is_ok_and(|b| b.len() <= RECORD_BYTES)
    }
}
pub(super) struct Round {
    pub state: ArchiveState,
    pub reason: Option<ArchiveReason>,
    pub facts: Observation,
}
fn result(state: ArchiveState, reason: ArchiveReason, facts: Observation) -> Round {
    Round {
        state,
        reason: Some(reason),
        facts,
    }
}
fn optional_text(value: &Value, key: &str, max: usize) -> Result<Option<String>, ArchiveReason> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= max => Ok(Some(s.clone())),
        _ => Err(ArchiveReason::InvalidResponse),
    }
}
fn status_text(value: &Value, key: &str) -> Result<Option<String>, ArchiveReason> {
    let s = optional_text(value, key, 32)?;
    if s.as_deref().is_some_and(|s| !identifier(s)) {
        Err(ArchiveReason::InvalidResponse)
    } else {
        Ok(s)
    }
}
fn timestamp(value: &Value, key: &str) -> Result<Option<String>, ArchiveReason> {
    let s = optional_text(value, key, 40)?;
    if s.as_deref().is_some_and(|s| !time_valid(s)) {
        Err(ArchiveReason::InvalidResponse)
    } else {
        Ok(s)
    }
}
fn remote_error(response: &Response) -> ArchiveReason {
    if response.status == 401 {
        return ArchiveReason::Authorization;
    }
    let v = serde_json::from_slice::<Value>(&response.body).ok();
    match v
        .as_ref()
        .and_then(|v| v["error"]["errors"][0]["reason"].as_str())
    {
        Some("quotaExceeded" | "dailyLimitExceeded" | "rateLimitExceeded") => ArchiveReason::Quota,
        Some("insufficientPermissions" | "forbidden") => ArchiveReason::Authorization,
        _ => ArchiveReason::RemoteRejected,
    }
}
fn get(
    transport: &dyn Transport,
    auth: &Authorization,
    resource: &str,
    id: &str,
) -> Result<(Option<Value>, String), ArchiveReason> {
    let (parts, fields) = if resource == "liveBroadcasts" {
        ("snippet,status,contentDetails", BROADCAST_FIELDS)
    } else if resource == "videos" {
        (
            "snippet,status,contentDetails,liveStreamingDetails,processingDetails",
            VIDEO_FIELDS,
        )
    } else {
        return Err(ArchiveReason::InvalidResponse);
    };
    let mut url = Url::parse(&format!("https://www.googleapis.com/youtube/v3/{resource}"))
        .map_err(|_| ArchiveReason::InvalidResponse)?;
    url.query_pairs_mut()
        .extend_pairs([("part", parts), ("id", id), ("fields", fields)]);
    let response = transport
        .send(Request {
            method: Method::GET,
            url: url.into(),
            bearer: Some(auth.token.clone()),
            headers: Vec::new(),
            body: Vec::new(),
        })
        .map_err(|e| {
            if e.code == "CLOUD_ARCHIVE_RESPONSE" {
                ArchiveReason::InvalidResponse
            } else {
                ArchiveReason::Network
            }
        })?;
    if response.body.len() > MAX_RESPONSE {
        return Err(ArchiveReason::InvalidResponse);
    }
    let hash = sha(&response.body);
    if response.status == 404 {
        return Ok((None, hash));
    }
    if response.status != 200 {
        return Err(remote_error(&response));
    }
    let value: Value =
        serde_json::from_slice(&response.body).map_err(|_| ArchiveReason::InvalidResponse)?;
    let items = value["items"]
        .as_array()
        .ok_or(ArchiveReason::InvalidResponse)?;
    if items.is_empty() {
        return Ok((None, hash));
    }
    if items.len() != 1 || items[0]["id"].as_str() != Some(id) {
        return Err(ArchiveReason::IdentityMismatch);
    }
    Ok((Some(items[0].clone()), hash))
}
/// Each callback durably reserves one GET and checks identity, owner, cancel and deadline.
pub(super) fn verify_round(
    transport: &dyn Transport,
    auth: &Authorization,
    identity: &Identity,
    mut admit: impl FnMut() -> Result<(), ArchiveReason>,
) -> Round {
    let mut facts = Observation::default();
    let checked = (|| -> Result<Option<ArchiveReason>, ArchiveReason> {
        if !identity.valid() || auth.owner != identity.owner_channel_id {
            return Err(ArchiveReason::OwnerMismatch);
        }
        admit()?;
        let (broadcast, hash) = get(transport, auth, "liveBroadcasts", &identity.broadcast_id)?;
        facts.broadcast_sha256 = Some(hash);
        let Some(b) = broadcast else {
            return Ok(Some(ArchiveReason::NotFound));
        };
        if b["kind"] != "youtube#liveBroadcast" {
            return Err(ArchiveReason::InvalidResponse);
        }
        if b["snippet"]["channelId"] != identity.owner_channel_id {
            return Err(ArchiveReason::OwnerMismatch);
        }
        if b["contentDetails"]["boundStreamId"] != identity.stream_id {
            return Err(ArchiveReason::IdentityMismatch);
        }
        if b["status"]["privacyStatus"] != "unlisted" {
            return Err(ArchiveReason::PrivacyMismatch);
        }
        facts.lifecycle = status_text(&b["status"], "lifeCycleStatus")?;
        facts.recording = status_text(&b["status"], "recordingStatus")?;
        facts.broadcast_start = timestamp(&b["snippet"], "actualStartTime")?;
        facts.broadcast_end = timestamp(&b["snippet"], "actualEndTime")?;
        if facts.lifecycle.as_deref() != Some("complete")
            || facts.recording.as_deref() != Some("recorded")
        {
            return Ok(Some(ArchiveReason::Nonterminal));
        }
        if facts.broadcast_end.is_none() {
            return Ok(Some(ArchiveReason::MetadataMissing));
        }
        admit()?;
        let (video, hash) = get(transport, auth, "videos", &identity.broadcast_id)?;
        facts.video_sha256 = Some(hash);
        let Some(v) = video else {
            return Ok(Some(ArchiveReason::NotFound));
        };
        if v["kind"] != "youtube#video" {
            return Err(ArchiveReason::InvalidResponse);
        }
        if v["snippet"]["channelId"] != identity.owner_channel_id {
            return Err(ArchiveReason::OwnerMismatch);
        }
        if v["status"]["privacyStatus"] != "unlisted" {
            return Err(ArchiveReason::PrivacyMismatch);
        }
        facts.upload = status_text(&v["status"], "uploadStatus")?;
        facts.processing = status_text(&v["processingDetails"], "processingStatus")?;
        facts.video_start = timestamp(&v["liveStreamingDetails"], "actualStartTime")?;
        facts.video_end = timestamp(&v["liveStreamingDetails"], "actualEndTime")?;
        facts.embeddable = match v["status"].get("embeddable") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(b)) => Some(*b),
            _ => return Err(ArchiveReason::InvalidResponse),
        };
        if matches!(
            facts.upload.as_deref(),
            Some("failed" | "rejected" | "deleted")
        ) || facts.processing.as_deref() == Some("failed")
        {
            return Err(ArchiveReason::ProcessingFailed);
        }
        if facts.upload.as_deref() != Some("processed")
            || !matches!(
                facts.processing.as_deref(),
                Some("succeeded" | "terminated")
            )
        {
            return Ok(Some(ArchiveReason::Processing));
        }
        if facts.embeddable == Some(false) {
            return Err(ArchiveReason::NotEmbeddable);
        }
        facts.duration = optional_text(&v["contentDetails"], "duration", 64)?;
        facts.duration_seconds = facts
            .duration
            .as_deref()
            .and_then(super::upload::iso_duration)
            .filter(|n| n.is_finite() && *n > 0.0);
        if facts.duration.is_some() && facts.duration_seconds.is_none() {
            facts.duration = None;
            return Err(ArchiveReason::InvalidResponse);
        }
        if facts.embeddable.is_none() || facts.duration_seconds.is_none() {
            return Ok(Some(ArchiveReason::MetadataMissing));
        }
        Ok(None)
    })();
    match checked {
        Ok(None) => Round {
            state: ArchiveState::MetadataVerified,
            reason: None,
            facts,
        },
        Ok(Some(reason)) => result(ArchiveState::Pending, reason, facts),
        Err(reason) => result(
            if matches!(
                reason,
                ArchiveReason::OwnerMismatch
                    | ArchiveReason::IdentityMismatch
                    | ArchiveReason::PrivacyMismatch
                    | ArchiveReason::ProcessingFailed
                    | ArchiveReason::NotEmbeddable
            ) {
                ArchiveState::Rejected
            } else {
                ArchiveState::Inconclusive
            },
            reason,
            facts,
        ),
    }
}
pub(super) struct GoogleArchiveTransport(Client);
fn response_error() -> CloudError {
    CloudError::new(
        "CLOUD_ARCHIVE_RESPONSE",
        "보관 영상의 확인 응답이 올바르지 않습니다.",
        false,
    )
}
impl GoogleArchiveTransport {
    pub(super) fn new() -> Result<Self, CloudError> {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map(Self)
            .map_err(|_| response_error())
    }
}
fn allowed(request: &Request) -> bool {
    let Ok(url) = Url::parse(&request.url) else {
        return false;
    };
    let resource = match url.path() {
        "/youtube/v3/liveBroadcasts" => "liveBroadcasts",
        "/youtube/v3/videos" => "videos",
        _ => return false,
    };
    let pairs: Vec<_> = url.query_pairs().collect();
    request.method == Method::GET
        && request.body.is_empty()
        && request.headers.is_empty()
        && request.bearer.as_ref().is_some_and(|s| !s.is_empty())
        && url.scheme() == "https"
        && url.host_str() == Some("www.googleapis.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.port().is_none_or(|p| p == 443)
        && pairs.len() == 3
        && pairs.iter().filter(|(k, _)| k == "id").count() == 1
        && pairs.iter().any(|(k, v)| k == "id" && video_id(v))
        && pairs.iter().any(|(k, v)| {
            k == "part"
                && v == if resource == "liveBroadcasts" {
                    "snippet,status,contentDetails"
                } else {
                    "snippet,status,contentDetails,liveStreamingDetails,processingDetails"
                }
        })
        && pairs.iter().any(|(k, v)| {
            k == "fields"
                && v == if resource == "liveBroadcasts" {
                    BROADCAST_FIELDS
                } else {
                    VIDEO_FIELDS
                }
        })
}
impl Transport for GoogleArchiveTransport {
    fn send(&self, request: Request) -> Result<Response, CloudError> {
        if !allowed(&request) {
            return Err(response_error());
        }
        let response = self
            .0
            .get(request.url)
            .bearer_auth(request.bearer.unwrap_or_default())
            .send()
            .map_err(|_| {
                CloudError::new(
                    "CLOUD_ARCHIVE_NETWORK",
                    "보관 영상 확인 연결이 끊겼습니다. 현재 기록은 유지됩니다.",
                    false,
                )
            })?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        response
            .take((MAX_RESPONSE + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| response_error())?;
        if body.len() > MAX_RESPONSE {
            return Err(response_error());
        }
        Ok(Response {
            status,
            headers: Vec::new(),
            body,
        })
    }
}

#[cfg(test)]
#[path = "live_archive_tests.rs"]
mod tests;
