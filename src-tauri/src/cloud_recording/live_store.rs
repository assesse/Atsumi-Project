//! Bounded durable facts, never restart authority or ingestion credentials.
use super::{
    live_api::{PreparationProgress, PreparationStage},
    model::*,
};
use serde::{Deserialize, Serialize};

pub(super) const RECEIPTS: usize = 128;
const RECORD_BYTES: usize = 8192;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceFacts {
    pub provider: String,
    pub channel_id: String,
    pub broadcast_key: Option<String>,
    pub session_id: String,
    pub epoch: u64,
    pub account_revision: u64,
}
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|v| !v.is_nil())
}
fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
impl SourceFacts {
    fn valid(&self) -> bool {
        matches!(self.provider.as_str(), "soop" | "chzzk")
            && identifier(&self.channel_id, 128)
            && self
                .broadcast_key
                .as_ref()
                .is_none_or(|v| identifier(v, 128))
            && (self.provider != "soop" || self.broadcast_key.is_some())
            && uuid(&self.session_id)
            && self.epoch > 0
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LiveReceipt {
    pub version: u32,
    pub view: CloudLiveOperation,
    pub source: SourceFacts,
    pub request_id: String,
    pub plan: Option<CloudLivePlan>,
    pub owner_generation: Option<u64>,
    pub stream_owner_verified: bool,
    pub broadcast_owner_verified: bool,
    pub sidecar_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<super::live_archive::Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<super::live_sidecar::Record>,
}
pub(super) fn terminal(state: &str) -> bool {
    matches!(state, "joined" | "failed" | "recovery_required")
}
pub(super) fn stage(stage: PreparationStage) -> &'static str {
    match stage {
        PreparationStage::Selected => "selected",
        PreparationStage::CreatingStream => "creating_stream",
        PreparationStage::StreamCreated => "stream_created",
        PreparationStage::CreatingBroadcast => "creating_broadcast",
        PreparationStage::BroadcastCreated => "broadcast_created",
        PreparationStage::Binding => "binding",
        PreparationStage::Prepared => "prepared",
    }
}
impl LiveReceipt {
    pub(super) fn selected(id: String, source: SourceFacts) -> Self {
        Self {
            version: 1,
            view: CloudLiveOperation {
                receipt_persisted: true,
                selection_id: id,
                revision: 1,
                provider: source.provider.clone(),
                channel_id: source.channel_id.clone(),
                broadcast_key: source.broadcast_key.clone(),
                state: "selected".into(),
                preparation_stage: None,
                destination_channel_id: None,
                stream_id: None,
                broadcast_id: None,
                outcome_uncertain: false,
                run_id: None,
                stop_requested: false,
                interrupted: false,
                segment_acks: 0,
                playlist_acks: 0,
                end_list_acked: false,
                manager_joined: false,
                worker_joined: false,
                sidecar_joined: false,
                sidecar_text_state: None,
                assets_incomplete: false,
                timing_incomplete: false,
                clock_quality: None,
                archive_ready: false,
                archive_verification: None,
                error: None,
            },
            source,
            request_id: uuid::Uuid::new_v4().to_string(),
            plan: None,
            owner_generation: None,
            stream_owner_verified: false,
            broadcast_owner_verified: false,
            sidecar_id: None,
            created_at: now(),
            updated_at: now(),
            archive: None,
            sidecar: None,
        }
    }
    pub(super) fn changed(&mut self) -> Result<(), CloudError> {
        self.view.revision = self
            .view
            .revision
            .checked_add(1)
            .ok_or_else(CloudError::storage)?;
        self.updated_at = now();
        Ok(())
    }
    pub(super) fn progress(&mut self, progress: &PreparationProgress) -> Result<(), CloudError> {
        if progress.request_id != self.request_id {
            return Err(CloudError::protocol());
        }
        self.view.preparation_stage = Some(stage(progress.stage).into());
        self.view.outcome_uncertain = progress.outcome_uncertain;
        if let Some(resource) = &progress.stream {
            if self
                .view
                .stream_id
                .as_ref()
                .is_some_and(|old| old != &resource.id)
            {
                return Err(CloudError::protocol());
            }
            self.view.stream_id = Some(resource.id.clone());
            self.stream_owner_verified = resource.owner_verified;
        }
        if let Some(resource) = &progress.broadcast {
            if self
                .view
                .broadcast_id
                .as_ref()
                .is_some_and(|old| old != &resource.id)
            {
                return Err(CloudError::protocol());
            }
            self.view.broadcast_id = Some(resource.id.clone());
            self.broadcast_owner_verified = resource.owner_verified;
        }
        self.changed()
    }
    pub(super) fn valid(&self) -> bool {
        let v = &self.view;
        self.version == 1
            && v.receipt_persisted
            && uuid(&v.selection_id)
            && uuid(&self.request_id)
            && v.revision > 0
            && self.source.valid()
            && v.provider == self.source.provider
            && v.channel_id == self.source.channel_id
            && v.broadcast_key == self.source.broadcast_key
            && matches!(
                v.state.as_str(),
                "selected"
                    | "preparing"
                    | "starting"
                    | "running"
                    | "stopping"
                    | "joined"
                    | "failed"
                    | "recovery_required"
            )
            && v.preparation_stage.as_deref().is_none_or(|s| {
                matches!(
                    s,
                    "selected"
                        | "creating_stream"
                        | "stream_created"
                        | "creating_broadcast"
                        | "broadcast_created"
                        | "binding"
                        | "prepared"
                )
            })
            && v.destination_channel_id.as_deref().is_none_or(channel_id)
            && v.stream_id.as_ref().is_none_or(|s| identifier(s, 128))
            && v.broadcast_id.as_deref().is_none_or(video_id)
            && v.run_id.as_deref().is_none_or(uuid)
            && self.sidecar_id.as_deref().is_none_or(uuid)
            && self.plan.as_ref().is_none_or(CloudLivePlan::valid)
            && self.owner_generation.is_none_or(|v| v > 0)
            && !v.archive_ready
            && v.archive_verification == self.archive.as_ref().map(super::live_archive::Record::snapshot)
            && self.archive.as_ref().is_none_or(|a| a.valid() && self.archive_identity().as_ref() == Some(&a.identity))
            && self.sidecar.as_ref().is_none_or(|s| s.valid_for(self))
            && (!self.stream_owner_verified || v.stream_id.is_some())
            && (!self.broadcast_owner_verified || v.broadcast_id.is_some())
            && (!v.end_list_acked || v.run_id.is_some())
            && (!v.manager_joined || v.run_id.is_some())
            && (v.state != "joined" || (v.manager_joined && v.worker_joined && v.end_list_acked))
            && v.sidecar_text_state.as_deref().is_none_or(|s| {
                matches!(
                    s,
                    "complete" | "partial" | "unknown" | "incomplete" | "ended"
                )
            })
            && v.clock_quality.as_ref().is_none_or(|s| identifier(s, 80))
            && v.error
                .as_ref()
                .is_none_or(|e| identifier(&e.code, 64) && e.message.len() <= 2048)
            && serde_json::to_vec(self).is_ok_and(|b| b.len() <= RECORD_BYTES)
    }
    pub(super) fn recover(&mut self) {
        let before = self.archive.as_ref().map(|a| a.state);
        if let Some(archive) = &mut self.archive {
            archive.recover();
            self.view.archive_verification = Some(archive.snapshot());
        }
        if before != self.archive.as_ref().map(|a| a.state) { let _ = self.changed(); }
        if !terminal(&self.view.state) {
            self.view.state = "recovery_required".into();
            self.view.interrupted = true;
            self.view.stop_requested = true;
            self.view.error.get_or_insert_with(|| CloudError::new("CLOUD_LIVE_RECOVERY", "이전 실시간 전송의 생성·종료 상태 확인이 필요합니다. 자동으로 다시 시작하지 않습니다.", false));
            let _ = self.changed();
        }
    }
    pub(super) fn archive_identity(&self) -> Option<super::live_archive::Identity> {
        if !self.view.receipt_persisted || !self.stream_owner_verified || !self.broadcast_owner_verified
            || self.view.outcome_uncertain || self.view.preparation_stage.as_deref() != Some("prepared")
            || !self.view.manager_joined || !self.view.worker_joined || !self.view.sidecar_joined
            || !terminal(&self.view.state) { return None; }
        let identity = super::live_archive::Identity {
            selection_id: self.view.selection_id.clone(), request_id: self.request_id.clone(),
            run_id: self.view.run_id.clone()?, source_fingerprint: super::live_archive::sha(&serde_json::to_vec(&self.source).ok()?),
            owner_channel_id: self.view.destination_channel_id.clone()?, creation_owner_generation: self.owner_generation?,
            broadcast_id: self.view.broadcast_id.clone()?, stream_id: self.view.stream_id.clone()?,
        };
        identity.valid().then_some(identity)
    }
}
pub(super) fn valid(receipts: &[LiveReceipt]) -> bool {
    let mut selections = std::collections::HashSet::new();
    let mut requests = std::collections::HashSet::new();
    receipts.len() <= RECEIPTS
        && receipts.iter().all(|r| {
            r.valid() && selections.insert(&r.view.selection_id) && requests.insert(&r.request_id)
        })
}

#[cfg(test)]
#[path = "live_store_tests.rs"]
mod tests;
