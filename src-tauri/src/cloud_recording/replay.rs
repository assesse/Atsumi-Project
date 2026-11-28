//! Native receipt authorization for chat-only remote replay. No media opens or grants.
use super::{catalog::Job, model, source, CloudError, CloudService, CloudSourceRef};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug)]
pub(crate) struct CloudReplayInput {
    pub index: u32,
    pub part_index: Option<u32>,
    pub sha256: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    pub media_offset_seconds: f64,
    pub modified_nanos: Option<u128>,
}

/// Constructed only from the native encrypted receipt and validated source plan.
/// This type is deliberately not deserializable or exposed through IPC.
#[derive(Clone, Debug)]
pub(crate) struct CloudReplayAuthorization {
    pub job_id: String,
    pub source: CloudSourceRef,
    pub owner_channel_id: String,
    pub cohort_id: String,
    pub source_sha256: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    pub media_offset_seconds: f64,
    pub original_inputs: Vec<CloudReplayInput>,
    pub plan_fingerprint: Option<String>,
    pub input_parts: bool,
    pub description: source::SourceDescription,
}

impl CloudReplayAuthorization {
    /// CHZZK supplies its unchanged catalog generation; platform media uses "".
    /// Matching only the plan's self-described inputs would not bind its digest.
    pub(crate) fn matches_recording_generation(&self, generation: &str) -> bool {
        let Some(expected) = &self.plan_fingerprint else { return true; };
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Identity<'a> {
            bytes: u64,
            modified_nanos: u128,
            digest: Option<&'a str>,
            duration: f64,
            offset: f64,
        }
        let Some(inputs) = self.original_inputs.iter().map(|input| Some(Identity {
            bytes: input.bytes, modified_nanos: input.modified_nanos?,
            digest: Some(input.sha256.as_str()), duration: input.duration_seconds,
            offset: input.media_offset_seconds,
        })).collect::<Option<Vec<_>>>() else { return false; };
        let bare = CloudSourceRef { provider: self.source.provider.clone(), recording_id: self.source.recording_id.clone(), part_index: None, plan_id: None, unit_id: None };
        serde_json::to_vec(&(bare, generation, inputs)).ok()
            .is_some_and(|value| format!("{:x}", Sha256::digest(value)) == *expected)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudReplayOpen {
    pub provider: String,
    pub token: String,
    pub cohort_id: String,
    pub description: source::SourceDescription,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<crate::streaming::replay::ReplaySession>,
}

fn unavailable() -> CloudError {
    CloudError::new("CLOUD_REPLAY_SOURCE", "이 보관 영상과 저장된 기록의 연결을 확인하지 못했습니다.", false)
}

pub(super) fn cohort_id(job: &Job) -> Option<String> {
    if !model::channel_id(&job.channel_id) || source::validate_reference(&job.view.source).is_err() {
        return None;
    }
    let identity = if let Some(plan) = &job.view.source.plan_id {
        format!("plan:{plan}")
    } else {
        let hash = job.source_sha256.as_deref()?;
        if hash.len() != 64 || !hash.bytes().all(|v| v.is_ascii_hexdigit()) { return None; }
        format!("source:{}:{:?}", hash.to_ascii_lowercase(), job.view.source.part_index)
    };
    let bytes = serde_json::to_vec(&(1u32, &job.channel_id, &job.view.source.provider, &job.view.source.recording_id, identity)).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

impl CloudService {
    pub(crate) fn replay_authorization(&self, data_dir: &Path, job_id: &str) -> Result<CloudReplayAuthorization, CloudError> {
        if !uuid::Uuid::parse_str(job_id).is_ok_and(|id| id.to_string() == job_id) {
            return Err(unavailable());
        }
        let job = {
            let inner = self.lock()?;
            Self::admit(&inner)?;
            // This read has no relationship to the currently connected upload grant.
            inner.catalog.jobs.iter().find(|job| job.view.id == job_id)
                .filter(|job| job.view.state == "ready" && !job.view.recording_plan && job.has_upload_receipt())
                .cloned().ok_or_else(unavailable)?
        };
        source::validate_reference(&job.view.source).map_err(|_| unavailable())?;
        if !job.view.media_offset_seconds.is_finite() || !(0.0..=1_000_000_000.0).contains(&job.view.media_offset_seconds) {
            return Err(unavailable());
        }
        let cohort_id = cohort_id(&job).ok_or_else(unavailable)?;
        let hash = job.source_sha256.as_ref().ok_or_else(unavailable)?.to_ascii_lowercase();
        let (description, original_inputs, plan_fingerprint, input_parts) = if job.view.source.plan_id.is_some() {
            source::replay_plan_binding(data_dir, &job.view.source, &hash, job.view.total_bytes,
                job.view.duration_seconds, job.view.media_offset_seconds).map_err(|_| unavailable())?
        } else {
            let reference = &job.view.source;
            let input = CloudReplayInput { index: reference.part_index.unwrap_or(0), part_index: reference.part_index,
                sha256: hash.clone(), bytes: job.view.total_bytes, duration_seconds: job.view.duration_seconds,
                media_offset_seconds: job.view.media_offset_seconds, modified_nanos: None };
            let description = source::SourceDescription {
                source: CloudSourceRef { provider: reference.provider.clone(), recording_id: reference.recording_id.clone(), part_index: None, plan_id: None, unit_id: None },
                title: job.view.title.clone(), plan_id: None,
                coverage: source::SourceCoverage { state: "incomplete".into(), expected_segments: None, covered_segments: None, missing_from_segment: None,
                    reason: Some("선택한 보관 구간만 확인되었습니다. 전체 녹화 완료 여부는 별도로 확인해 주세요.".into()), capture_warning: None },
                preparation_required: false, duration_seconds: job.view.media_offset_seconds + job.view.duration_seconds,
                bytes: job.view.total_bytes,
                parts: vec![source::SourcePart { source: reference.clone(), unit_index: 0, part_index: reference.part_index,
                    duration_seconds: job.view.duration_seconds, media_offset_seconds: job.view.media_offset_seconds,
                    bytes: job.view.total_bytes, kind: "direct".into() }],
            };
            (description, vec![input], None, reference.part_index.is_some())
        };
        let authorization = CloudReplayAuthorization { job_id: job.view.id, source: job.view.source,
            owner_channel_id: job.channel_id, cohort_id, source_sha256: hash, bytes: job.view.total_bytes,
            duration_seconds: job.view.duration_seconds, media_offset_seconds: job.view.media_offset_seconds,
            original_inputs, plan_fingerprint, input_parts, description };
        if authorization.source.provider != "chzzk" && !authorization.matches_recording_generation("") {
            return Err(unavailable());
        }
        Ok(authorization)
    }
}
