//! Provider-neutral numeric relay facts; no capability or destination secrets.
use serde::{Deserialize, Serialize};
/// Numeric/hash inspection only. This is not a transport or upload receipt.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Inspection {
    pub run_id: String,
    pub manager_joined: bool,
    pub codec_admitted: bool,
    pub input_ended: bool,
    pub interrupted: bool,
    pub fault: Option<String>,
    pub output_chunks: u64,
    pub output_bytes: u64,
    pub output_sha256: String,
    pub video_samples: u64,
    pub audio_samples: u64,
    pub video_end: Option<u64>,
    pub audio_end: Option<u64>,
    pub video_timescale: Option<u32>,
    pub audio_timescale: Option<u32>,
    pub worker_joined: bool,
    pub consumed_chunks: u64,
    pub validated_appends: u64,
    pub duplicate_chunks: u64,
    pub input_peak_bytes: usize,
    pub output_peak_bytes: usize,
    pub input_fault: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec_diagnostic: Option<crate::streaming::relay_ingress::CodecDiagnostic>,
    pub delivery: Option<DeliveryInspection>,
    pub sidecar: Option<SidecarInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_reference: Option<FrameReferenceInspection>,
    #[serde(skip)]
    pub sidecar_directory: Option<std::path::PathBuf>,
    #[serde(skip)]
    pub sidecar_owner: Option<NativeSidecarOwner>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FrameReferenceInspection {
    pub state: String,
    pub worker_started: bool,
    pub worker_joined: bool,
    pub persisted: bool,
    pub dynamic_windows: usize,
    pub reason: Option<String>,
}
impl FrameReferenceInspection {
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self { state: "unavailable".into(), worker_started: false, worker_joined: false,
            persisted: false, dynamic_windows: 0, reason: Some(reason.into()) }
    }
    pub(crate) fn sampling() -> Self {
        Self { state: "sampling".into(), worker_started: true, worker_joined: false,
            persisted: false, dynamic_windows: 0, reason: None }
    }
    pub(crate) fn settled(result: &crate::cloud_recording::live_fingerprint::ReferenceSet) -> Self {
        Self { state: if result.incomplete { "incomplete" } else { "ready" }.into(), worker_started: true,
            worker_joined: true, persisted: true, dynamic_windows: result.windows.len(),
            reason: result.incomplete.then(|| "dynamic_reference_incomplete".into()) }
    }
    pub(crate) fn failed_joined() -> Self {
        Self { state: "unavailable".into(), worker_started: true, worker_joined: true,
            persisted: false, dynamic_windows: 0, reason: Some("reference_publication_unconfirmed".into()) }
    }
    pub(crate) fn valid(&self) -> bool {
        matches!(self.state.as_str(), "unavailable" | "sampling" | "incomplete" | "ready")
            && self.dynamic_windows <= 16 && (!self.worker_joined || self.worker_started)
            && (!self.persisted || self.worker_joined)
            && (self.state != "ready" || (self.persisted && self.dynamic_windows >= 3))
            && self.reason.as_ref().is_none_or(|s| s.len() <= 80 && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
    }
}
/// Created alongside the writer's owner file, before receiving any payload.
/// Native provenance only: never deserialized from a renderer or disk file.
#[derive(Clone, Debug)]
pub(crate) enum NativeSidecarOwner {
    Soop { sidecar_id: String, receiver_id: String, run_id: String, source_id: String,
        document_epoch: u64, channel_id: String, broadcast_id: String },
    Chzzk { run_id: String, session_id: String, source_id: String, document_epoch: u64,
        account_revision: u64, channel_id: String, started_at_ms: u64 },
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeliveryInspection {
    pub segment_acks: u64,
    pub playlist_acks: u64,
    pub requests: u64,
    pub acknowledged_bytes: u64,
    pub end_list_acked: bool,
    pub fault: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SidecarInspection {
    pub sidecar_id: String,
    pub joined: bool,
    pub raw_rows: u64,
    pub event_rows: u64,
    pub bytes: u64,
    pub interrupted: bool,
    pub assets_incomplete: bool,
    pub failure: bool,
    pub text_state: String,
    pub clock_quality: String,
    pub timing_records: u64,
    pub timing_incomplete: bool,
}
