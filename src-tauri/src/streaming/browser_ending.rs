//! Broadcast identity and bounded end confirmation, independent of presentation.
use super::super::model::{LiveInfo, LiveStatus};
use super::*;

pub(super) fn live_key(info: &LiveInfo) -> Option<String> {
    info.live_id
        .as_ref()
        .filter(|id| !id.is_empty())
        .map(|id| format!("id:{id}"))
        .or_else(|| info.broadcast_started_at.map(|at| format!("start:{at}")))
}

fn classify(trigger: &str, saved: &BrowserRecording) -> &'static str {
    if saved.status == BrowserRecordingStatus::Failed || saved.partial.is_some() {
        return "storage_error";
    }
    if saved.segment_count == 0 {
        return "start_failed";
    }
    match trigger {
        "user_stop" => "user_stopped",
        "app_shutdown" => "app_shutdown",
        "broadcast_ended" => "broadcast_ended",
        "broadcast_changed" => "broadcast_changed",
        "video_ended" | "source_changed" | "video_changed" => "checking",
        "timeline_changed" => "timeline_discontinuity",
        "page_hidden" | "window_closed" | "renderer_failed" => "app_interrupted",
        "native_rejected" | "queue_overflow" | "bridge_ack_timeout" | "bridge_busy"
        | "bridge_post_failed" => "storage_error",
        _ => "source_error",
    }
}

/// A live answer is not enough to call a source failure a new broadcast unless
/// both identities use the same scheme. Missing IDs are not a broadcast change.
fn confirmation(info: &LiveInfo, original: Option<&str>, offline: &mut u8) -> Option<&'static str> {
    match info.status {
        LiveStatus::Offline => {
            *offline += 1;
            (*offline >= 2).then_some("broadcast_ended")
        }
        LiveStatus::Live => {
            *offline = 0;
            let current = live_key(info)?;
            original
                .filter(|old| old.split(':').next() == current.split(':').next() && *old != current)
                .map(|_| "broadcast_changed")
        }
        _ => {
            *offline = 0;
            None
        }
    }
}

impl OfficialBrowser {
    pub(super) fn finish_reason(
        &self,
        recording: &BrowserRecording,
        trigger: &str,
    ) -> &'static str {
        let reason = classify(trigger, recording);
        let id = recording.id.clone();
        if self
            .inner
            .store
            .lock()
            .is_ok_and(|store| store.record_end(&id, reason, trigger).is_err())
        {
            tracing::warn!(recording_id = %id, "could not persist recording end reason");
        }
        tracing::info!(recording_id = %id, channel_id = %recording.channel_id, reason, trigger, "recording capture ended");
        if reason != "checking" {
            return reason;
        }
        let root = self.clone();
        let recording = recording.clone();
        let _ = thread::Builder::new().name("chzzk-end-confirm".into()).spawn(move || {
            let mut offline = 0;
            let mut confirmed = None;
            if let Ok(provider) = ChzzkProvider::new() {
                for attempt in 0..4 {
                    if root.inner.closing.load(Ordering::Acquire) { break; }
                    match provider.inspect_for_browser(&recording.channel_id) {
                        Ok(info) => {
                            confirmed = confirmation(&info, recording.broadcast_key.as_deref(), &mut offline);
                            tracing::info!(recording_id = %id, attempt, status = ?info.status, offline_confirmations = offline, "recording end metadata checked");
                        }
                        Err(cause) => { offline = 0; tracing::warn!(recording_id = %id, code = %cause.code, "recording end metadata unavailable"); }
                    }
                    if confirmed.is_some() || attempt == 3 { break; }
                    for _ in 0..60 {
                        if root.inner.closing.load(Ordering::Acquire) { break; }
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            let final_reason = confirmed.unwrap_or("end_unconfirmed");
            if let Ok(store) = root.inner.store.lock() { let _ = store.confirm_end(&id, final_reason); }
        });
        reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn info(status: LiveStatus, id: Option<&str>) -> LiveInfo {
        LiveInfo {
            channel_id: "a".repeat(32),
            channel_name: String::new(),
            title: String::new(),
            live_id: id.map(String::from),
            broadcast_started_at: Some(1000),
            status,
            chat_available: false,
            chat_channel_id: None,
            notice: None,
        }
    }
    #[test]
    fn capture_end_classification_preserves_manual_shutdown_and_failed_storage() {
        let mut recording: BrowserRecording = serde_json::from_value(json!({
            "id":"test", "channelId":"a".repeat(32), "title":"test", "startedAt":1,
            "updatedAt":2, "status":"stopped", "mimeType":"video/mp4", "outputDir":"",
            "segmentCount":1, "bytesWritten":10, "durationSeconds":1.0, "segments":[]
        }))
        .unwrap();
        for (trigger, reason) in [
            ("user_stop", "user_stopped"),
            ("app_shutdown", "app_shutdown"),
            ("broadcast_ended", "broadcast_ended"),
            ("broadcast_changed", "broadcast_changed"),
            ("video_ended", "checking"),
            ("source_changed", "checking"),
            ("window_closed", "app_interrupted"),
            ("timeline_changed", "timeline_discontinuity"),
        ] {
            assert_eq!(classify(trigger, &recording), reason);
        }
        recording.segment_count = 0;
        assert_eq!(classify("video_ended", &recording), "start_failed");
        recording.status = BrowserRecordingStatus::Failed;
        assert_eq!(classify("broadcast_ended", &recording), "storage_error");
    }

    #[test]
    fn ending_requires_two_offline_answers_and_never_confuses_missing_id_with_new_live() {
        let mut offline = 0;
        assert_eq!(
            confirmation(&info(LiveStatus::Offline, None), Some("id:1"), &mut offline),
            None
        );
        assert_eq!(
            confirmation(&info(LiveStatus::Live, None), Some("id:1"), &mut offline),
            None
        );
        assert_eq!(offline, 0);
        assert_eq!(
            confirmation(
                &info(LiveStatus::Live, Some("2")),
                Some("id:1"),
                &mut offline
            ),
            Some("broadcast_changed")
        );
        assert_eq!(
            confirmation(&info(LiveStatus::Offline, None), None, &mut offline),
            None
        );
        assert_eq!(
            confirmation(&info(LiveStatus::Offline, None), None, &mut offline),
            Some("broadcast_ended")
        );
    }
}
