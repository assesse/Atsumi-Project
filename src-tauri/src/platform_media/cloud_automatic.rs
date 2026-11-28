//! Cheap catalog candidates only. The upload worker retains final file validation.
use super::*;
use crate::cloud_recording::{AutomaticCandidate, AutomaticReadiness, CloudAutomaticRule, CloudAutomaticTarget, CloudSourceRef};

fn recording_started_at_ms(job: &MediaJob) -> u64 {
    job.started_at_ms.unwrap_or_else(|| job.created_at.saturating_mul(1000))
}
fn selected(rule: &CloudAutomaticRule, job: &MediaJob) -> bool {
    if !rule.enabled || job.reference.provider != "soop" || job.reference.kind != "live" { return false; }
    match &rule.target {
        CloudAutomaticTarget::Channel { provider, channel_id } => provider == "soop"
            && channel_id == &job.reference.external_id && recording_started_at_ms(job) >= rule.activated_at_ms,
        CloudAutomaticTarget::Recording { provider, recording_id } => provider == "soop" && recording_id == &job.job_id,
    }
}
fn readiness(job: &MediaJob, running: bool, persistence_failed: bool) -> AutomaticReadiness {
    if persistence_failed { return AutomaticReadiness::Failed; }
    if job.state == "recording" { return AutomaticReadiness::Recording; }
    if running || matches!(job.state.as_str(), "queued" | "resolving" | "downloading" | "finalizing" | "verifying") {
        return AutomaticReadiness::Processing;
    }
    if job.state == "completed" && job.library.as_ref().is_some_and(|item| item.library_id == job.job_id
        && item.reference == job.reference && item.bytes > 0 && item.duration.is_finite() && item.duration > 0.0
        && item.sha256.len() == 64 && item.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())) {
        AutomaticReadiness::Ready
    } else { AutomaticReadiness::Failed }
}
impl PlatformMediaService {
    pub(crate) fn automatic_upload_candidates(&self, rules: &[CloudAutomaticRule]) -> Result<Vec<AutomaticCandidate>, String> {
        let state = self.inner.state.lock().map_err(|_| "녹화 상태를 확인하지 못했습니다.")?;
        if state.shutting_down || state.persistence_overflow { return Err("녹화 상태 확인을 기다리고 있습니다.".into()); }
        // Do not reuse the display query: it truncates settled jobs after 256.
        let mut result = state.jobs.values().filter(|job| rules.iter().any(|rule| selected(rule, job))).map(|job| AutomaticCandidate {
            source: CloudSourceRef { provider: "soop".into(), recording_id: job.job_id.clone(), part_index: None, plan_id: None, unit_id: None },
            channel_id: job.reference.external_id.clone(), started_at_ms: recording_started_at_ms(job),
            readiness: readiness(job, state.cancel.contains_key(&job.job_id), state.persistence_errors.contains_key(&job.job_id)),
        }).collect::<Vec<_>>();
        for rule in rules.iter().filter(|rule| rule.enabled) {
            if let CloudAutomaticTarget::Recording { provider, recording_id } = &rule.target {
                if provider == "soop" && !result.iter().any(|candidate| &candidate.source.recording_id == recording_id) {
                    result.push(AutomaticCandidate { source: CloudSourceRef { provider: provider.clone(), recording_id: recording_id.clone(), part_index: None, plan_id: None, unit_id: None }, channel_id: String::new(), started_at_ms: 0, readiness: AutomaticReadiness::Missing });
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(target: CloudAutomaticTarget) -> CloudAutomaticRule {
        CloudAutomaticRule { id: uuid::Uuid::new_v4().to_string(), target, enabled: true, visibility: crate::cloud_recording::CloudVisibility::Private, generation: 1,
            activated_at_ms: 2001, owner_channel_id: None, status: "waiting_recording".into(), error: None }
    }
    fn job(index: u128) -> MediaJob {
        let id = uuid::Uuid::from_u128(index).to_string();
        let reference = MediaRef { provider: "soop".into(), kind: "live".into(), external_id: "selected".into() };
        MediaJob { broadcast_id: None, job_id: id.clone(), request_id: id.clone(), reference: reference.clone(), revision: 1,
            state: "completed".into(), title: "fixture".into(), channel_name: "fixture".into(), created_at: index as u64, started_at_ms: None,
            received_bytes: 1, total_bytes: Some(1), error: None, download_root: PathBuf::new(),
            library: Some(LibraryItem { broadcast_id: None, chat_archive: None, capture_outcome: None, library_id: id,
                revision: 1, reference, title: "fixture".into(), channel_name: "fixture".into(), bytes: 1, duration: 1.0,
                sha256: "a".repeat(64), completed_at: index as u64, modified_millis: 1 }) }
    }
    #[test]
    fn channel_cutoff_excludes_old_and_ambiguous_second_but_exact_selection_keeps_old() {
        let channel = rule(CloudAutomaticTarget::Channel { provider: "soop".into(), channel_id: "selected".into() });
        assert!(!selected(&channel, &job(1)));
        assert!(!selected(&channel, &job(2)));
        assert!(selected(&channel, &job(3)));
        let mut old = job(1);
        let exact = rule(CloudAutomaticTarget::Recording { provider: "soop".into(), recording_id: old.job_id.clone() });
        assert!(selected(&exact, &old));
        old.reference.kind = "video".into(); assert!(!selected(&exact, &old));
        old.reference.kind = "live".into(); old.reference.provider = "youtube".into(); assert!(!selected(&exact, &old));
    }
    #[test]
    fn precise_start_in_same_second_respects_channel_activation_boundary() {
        let channel = rule(CloudAutomaticTarget::Channel { provider: "soop".into(), channel_id: "selected".into() });
        let mut recording = job(2);
        recording.started_at_ms = Some(2000);
        assert!(!selected(&channel, &recording));
        recording.started_at_ms = Some(2001);
        assert!(selected(&channel, &recording));
        recording.started_at_ms = Some(2500);
        assert!(selected(&channel, &recording));
        assert_eq!(recording_started_at_ms(&recording), 2500);
    }
    #[test]
    fn legacy_job_json_keeps_omitted_timestamp_and_new_job_retains_precision() {
        let legacy = serde_json::to_vec(&job(2)).unwrap();
        let mut decoded: MediaJob = serde_json::from_slice(&legacy).unwrap();
        assert_eq!(decoded.started_at_ms, None);
        assert!(!serde_json::from_slice::<serde_json::Value>(&legacy).unwrap().as_object().unwrap().contains_key("startedAtMs"));
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), legacy);
        decoded.started_at_ms = Some(2500);
        let new_json = serde_json::to_vec(&decoded).unwrap();
        assert_eq!(serde_json::from_slice::<MediaJob>(&new_json).unwrap().started_at_ms, Some(2500));
    }
    #[test]
    fn completed_metadata_waits_for_finalization_worker_and_durable_proof() {
        let mut value = job(1);
        assert_eq!(readiness(&value, true, false), AutomaticReadiness::Processing);
        assert_eq!(readiness(&value, false, true), AutomaticReadiness::Failed);
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Ready);
        value.library.as_mut().unwrap().sha256.clear();
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Failed);
        value.state = "recording".into(); assert_eq!(readiness(&value, true, false), AutomaticReadiness::Recording);
    }
    #[test]
    fn exact_old_candidate_is_not_lost_beyond_display_limit() {
        let root = tempfile::tempdir().unwrap(); let unused = root.path().join("unused.exe");
        let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
            yt_dlp: unused.clone(), deno: unused.clone(), ffmpeg: unused.clone(), ffprobe: unused }, None).unwrap();
        { let mut state = service.inner.state.lock().unwrap(); for index in 1..=300 { let mut value = job(index); if index == 2 { value.started_at_ms = Some(2500); } state.jobs.insert(value.job_id.clone(), value); } }
        let oldest = job(1).job_id;
        assert!(!service.jobs(Some("soop")).iter().any(|value| value.job_id == oldest));
        let candidates = service.automatic_upload_candidates(&[rule(CloudAutomaticTarget::Recording { provider: "soop".into(), recording_id: oldest.clone() })]).unwrap();
        assert_eq!(candidates.len(), 1); assert_eq!(candidates[0].source.recording_id, oldest);
        assert_eq!(candidates[0].readiness, AutomaticReadiness::Ready);
        let candidates = service.automatic_upload_candidates(&[rule(CloudAutomaticTarget::Channel { provider: "soop".into(), channel_id: "selected".into() })]).unwrap();
        let same_second = candidates.iter().find(|value| value.source.recording_id == job(2).job_id).unwrap();
        assert_eq!(same_second.started_at_ms, 2500);
        service.shutdown();
    }
}
