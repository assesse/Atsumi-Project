//! Metadata admission for explicitly selected automatic archival targets.
use super::*;
use crate::cloud_recording::{AutomaticCandidate, AutomaticReadiness, CloudAutomaticRule, CloudAutomaticTarget, CloudSourceRef};

fn selected(rule: &CloudAutomaticRule, recording: &BrowserRecording) -> bool {
    if !rule.enabled { return false; }
    match &rule.target {
        CloudAutomaticTarget::Channel { provider, channel_id } => provider == "chzzk" && channel_id == &recording.channel_id
            && recording.started_at >= rule.activated_at_ms,
        CloudAutomaticTarget::Recording { provider, recording_id } => provider == "chzzk" && recording_id == &recording.id,
    }
}
fn readiness(recording: &BrowserRecording, busy: bool, unverified: bool) -> AutomaticReadiness {
    if recording.deletion_pending || recording.media_removed_at.is_some() { return AutomaticReadiness::Missing; }
    if recording.status == BrowserRecordingStatus::Recording { return AutomaticReadiness::Recording; }
    if unverified || recording.storage_check_pending || recording.summary_pending { return AutomaticReadiness::Processing; }
    if recording.status == BrowserRecordingStatus::Failed || recording.partial.is_some()
        || recording.merge.as_ref().is_some_and(|merge| matches!(merge.status, BrowserMergeStatus::Blocked | BrowserMergeStatus::Failed))
        || recording.archive.as_ref().is_some_and(|archive| archive.status == ArchiveStatus::Blocked) {
        return AutomaticReadiness::Failed;
    }
    if busy || recording.archive.as_ref().is_some_and(|archive| archive.status != ArchiveStatus::Complete) {
        return AutomaticReadiness::Processing;
    }
    if recording.merge.as_ref().is_some_and(|merge| merge.status == BrowserMergeStatus::Complete
        && merge.bytes.is_some_and(|bytes| bytes > 0) && merge.duration_seconds.is_some_and(|seconds| seconds.is_finite() && seconds > 0.0)) {
        AutomaticReadiness::Ready
    } else { AutomaticReadiness::Processing }
}
impl BrowserCaptureStore {
    pub(crate) fn automatic_upload_candidates(&self, rules: &[CloudAutomaticRule]) -> Result<Vec<AutomaticCandidate>, StreamError> {
        let selected_ids = {
            let state = self.lock()?;
            if state.closing { return Err(inactive()); }
            state.recordings.iter().filter(|recording| rules.iter().any(|rule| selected(rule, recording)))
                .map(|recording| recording.id.clone()).collect::<Vec<_>>()
        };
        // Cached startup summaries are display-only. Recover selected metadata,
        // outside the catalog mutex, without running merger/cleanup workers.
        let failed = selected_ids.iter().filter(|id| self.ensure_recovered(id).is_err()).cloned().collect::<HashSet<_>>();
        let state = self.lock()?;
        if state.closing { return Err(inactive()); }
        let mut result = state.recordings.iter().filter(|recording| rules.iter().any(|rule| selected(rule, recording))).map(|recording| {
            let busy = state.active.contains_key(&recording.id) || state.deleting.contains_key(&recording.id)
                || state.merge_job.as_ref().is_some_and(|(id, _)| id == &recording.id) || state.range_evictions.contains(&recording.id);
            AutomaticCandidate { source: CloudSourceRef { provider: "chzzk".into(), recording_id: recording.id.clone(), part_index: None, plan_id: None, unit_id: None },
                channel_id: recording.channel_id.clone(), started_at_ms: recording.started_at,
                readiness: if failed.contains(&recording.id) { AutomaticReadiness::Failed } else { readiness(recording, busy, state.unverified.contains(&recording.id)) } }
        }).collect::<Vec<_>>();
        for rule in rules.iter().filter(|rule| rule.enabled) {
            if let CloudAutomaticTarget::Recording { provider, recording_id } = &rule.target {
                if provider == "chzzk" && !result.iter().any(|candidate| &candidate.source.recording_id == recording_id) {
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
    const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
    fn rule(target: CloudAutomaticTarget, cutoff: u64) -> CloudAutomaticRule {
        CloudAutomaticRule { id: uuid::Uuid::new_v4().to_string(), target, enabled: true, visibility: crate::cloud_recording::CloudVisibility::Private, generation: 1,
            activated_at_ms: cutoff, owner_channel_id: None, status: "waiting_recording".into(), error: None }
    }
    fn recording() -> (tempfile::TempDir, BrowserRecording) {
        let dir = tempfile::tempdir().unwrap(); let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let mut value = store.begin(dir.path(), CHANNEL, "fixture", "video/webm").unwrap();
        value.status = BrowserRecordingStatus::Stopped;
        value.merge = Some(BrowserMerge { status: BrowserMergeStatus::Complete, segment_count: 1, updated_at: 1,
            file: Some("fixture.webm".into()), timeline_file: Some("fixture.timeline.jsonl".into()), bytes: Some(10),
            duration_seconds: Some(1.0), last_error: None, source_cleanup: None });
        (dir, value)
    }
    #[test]
    fn old_channel_recording_requires_exact_selection() {
        let (_dir, value) = recording();
        let mut channel = rule(CloudAutomaticTarget::Channel { provider: "chzzk".into(), channel_id: CHANNEL.into() }, value.started_at + 1);
        assert!(!selected(&channel, &value)); channel.activated_at_ms = value.started_at; assert!(selected(&channel, &value));
        let exact = rule(CloudAutomaticTarget::Recording { provider: "chzzk".into(), recording_id: value.id.clone() }, value.started_at + 1);
        assert!(selected(&exact, &value));
    }
    #[test]
    fn cached_complete_summary_and_partial_or_pending_archive_are_not_ready() {
        let (dir, mut value) = recording();
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Ready);
        assert_eq!(readiness(&value, true, false), AutomaticReadiness::Processing);
        assert_eq!(readiness(&value, false, true), AutomaticReadiness::Processing);
        value.summary_pending = true; assert_eq!(readiness(&value, false, false), AutomaticReadiness::Processing); value.summary_pending = false;
        value.storage_check_pending = true; assert_eq!(readiness(&value, false, false), AutomaticReadiness::Processing); value.storage_check_pending = false;
        value.archive = Some(archive::ArchiveTransfer::new(dir.path(), dir.path()));
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Processing);
        value.archive.as_mut().unwrap().status = ArchiveStatus::Blocked;
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Failed);
        value.archive.as_mut().unwrap().status = ArchiveStatus::Complete;
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Ready);
        value.partial = Some(BrowserPartial { index: 1, file: "fragment.part".into(), bytes: 2 });
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Failed);
        value.partial = None; value.media_removed_at = Some(1);
        assert_eq!(readiness(&value, false, false), AutomaticReadiness::Missing);
    }
}
