use super::{
    model::*,
    oauth::{ClientConfig, Grant},
};
use serde::{Deserialize, Serialize};

/// The entire document is encrypted by Vault, including resumable upload URLs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub version: u32,
    pub client: Option<ClientConfig>,
    pub grant: Option<Grant>,
    #[serde(default)]
    pub live_grant: Option<super::oauth::LiveGrant>,
    #[serde(default)]
    pub live_generation: u64,
    #[serde(default)]
    pub live_receipts: Vec<super::live_store::LiveReceipt>,
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub automatic: super::automatic::Config,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            client: None,
            grant: None,
            live_grant: None,
            live_generation: 0,
            live_receipts: Vec::new(),
            jobs: Vec::new(),
            automatic: super::automatic::Config::default(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Job {
    pub view: CloudJobSnapshot,
    pub channel_id: String,
    pub source_sha256: Option<String>,
    pub session_url: Option<String>,
    pub attempts: u32,
    /// Kept across restart so each scheduled metadata request cannot renew the
    /// processing deadline. The remote response is checked before this deadline.
    #[serde(default)]
    pub processing_started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic: Option<super::automatic::Origin>,
}
impl Job {
    /// A late response can add a session, acknowledged bytes, or a video ID
    /// after pause/disconnect. Those facts are durable even while work is held.
    pub(super) fn set_active_phase(&mut self, state: &str) {
        if !matches!(self.view.state.as_str(), "paused" | "waiting_auth") {
            self.view.state = state.into();
            self.view.error = None;
            self.view.retry_at = None;
        }
    }

    pub(crate) fn has_upload_receipt(&self) -> bool {
        self.view.video_id.as_ref().is_some_and(|id| video_id(id))
            && self
                .source_sha256
                .as_ref()
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            && self.view.total_bytes > 0
            && self.view.uploaded_bytes == self.view.total_bytes
            && self.view.duration_seconds.is_finite()
            && self.view.duration_seconds > 0.0
            && self.view.duration_seconds <= 43_200.0
    }
}
impl Catalog {
    pub(crate) fn valid(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        self.version == 1
            && super::live_store::valid(&self.live_receipts)
            && self.live_grant.as_ref().is_none_or(|g| self.client.is_some() && self.live_generation > 0 && g.valid())
            && self.automatic.valid()
            && self.jobs.len() <= 2048
            && (self.grant.is_none() || self.client.is_some())
            && self.client.as_ref().is_none_or(ClientConfig::valid)
            && self.grant.as_ref().is_none_or(Grant::valid)
            && self.jobs.iter().all(|job| {
                let v = &job.view;
                uuid::Uuid::parse_str(&v.id).is_ok()
                    && job.automatic.as_ref().is_none_or(super::automatic::Origin::valid)
                    && ids.insert(&v.id)
                    && v.source.valid()
                    && channel_id(&job.channel_id)
                    && job_state(&v.state)
                    && v.title.chars().count() <= 100
                    && v.uploaded_bytes <= v.total_bytes
                    && v.total_bytes <= super::upload::MAX_BYTES
                    && v.duration_seconds.is_finite()
                    && (0.0..=43_200.0).contains(&v.duration_seconds)
                    && v.media_offset_seconds.is_finite()
                    && (0.0..=1_000_000_000.0).contains(&v.media_offset_seconds)
                    && v.video_id.as_ref().is_none_or(|id| video_id(id))
                    && job
                        .source_sha256
                        .as_ref()
                        .is_none_or(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                    && job
                        .session_url
                        .as_ref()
                        .is_none_or(|u| super::upload::valid_session_url(u))
                    && (v.video_id.is_none() || job.has_upload_receipt())
                    && (!matches!(v.state.as_str(), "ready" | "processing")
                        || job.has_upload_receipt())
                    && job
                        .processing_started_at
                        .is_none_or(|at| at > 0 && v.video_id.is_some())
                    && v.preparation
                        .as_ref()
                        .is_none_or(SourcePreparationProgress::valid)
                    && (!v.recording_plan
                        || (v.source.plan_id.is_none()
                            && v.source.unit_id.is_none()
                            && v.source.part_index.is_none()
                            && v.video_id.is_none()
                            && job.session_url.is_none()
                            && job.source_sha256.is_none()
                            && v.total_bytes == 0
                            && v.uploaded_bytes == 0))
            })
    }
    pub(crate) fn recover(&mut self) {
        for receipt in &mut self.live_receipts { receipt.recover(); }
        for job in &mut self.jobs {
            if matches!(
                job.view.state.as_str(),
                "queued" | "validating" | "uploading" | "retry_wait" | "processing"
            ) {
                job.view.state = "paused".into();
                job.view.retry_at = None;
                job.view.error = Some(CloudError::new(
                    "CLOUD_RESTART_PAUSED",
                    "앱 종료 전 업로드입니다. 이어서 전송할 수 있습니다.",
                    true,
                ));
            }
        }
    }
}
