//! Re-open only an empty, confirmed-live receiver. Never interrupt a capture.
use super::*;

fn empty_receiver(state: &ViewState, channel: &str) -> bool {
    let diagnostics = state.capture_diagnostics.as_ref();
    state.open
        && state.channel.as_deref() == Some(channel)
        && !state.ready
        && !state.page_recording
        && state.video_width == 0
        && state.video_height == 0
        && state.recording.is_none()
        && state.arm.is_none()
        && state.continuation.is_none()
        && state.receiver_reload.is_none()
        && !state.account_busy
        && !state.auth_checking
        && !state.extension_connecting
        && state.pending_control.is_none()
        && state.confirming_control.is_none()
        && state.pending_ui_action.is_none()
        && diagnostics.is_some_and(|d| {
            d["reason"].as_str() == Some("waiting_video")
                && d["appendBytes"].as_u64() == Some(0)
                && d["sources"].as_array().is_some_and(Vec::is_empty)
        })
}

pub(super) fn finished(state: &mut ViewState) -> bool {
    // An old document's late Finished event cannot release the reservation.
    if state
        .receiver_reload
        .is_some_and(|generation| generation != state.page_generation)
    {
        state.receiver_reload = None;
        return true;
    }
    false
}

impl OfficialBrowser {
    pub(super) fn reload_empty_live_receiver(
        &self,
        app: &AppHandle,
        channel: &str,
        live_key: &str,
    ) -> Result<bool, StreamError> {
        if !self
            .inner
            .auto_record
            .live_recovery_allowed(channel, live_key, now_ms())
        {
            return Ok(false);
        }
        let view = app.get_webview(self.label()).ok_or_else(unavailable)?;
        // Dispatcher calls precede state locks, as in start(), to avoid UI-thread deadlocks.
        if view.url().ok().as_ref().and_then(live_channel).as_deref() != Some(channel)
            || self.account_window_open(app)
        {
            return Ok(false);
        }
        let generation = {
            let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
            if self.inner.closing.load(Ordering::Acquire)
                || self.inner.detached.load(Ordering::Acquire)
                || self.inner.reserved.load(Ordering::Acquire)
                || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
                || self.primary_profile_busy()
                || self.multiview_active()
            {
                return Ok(false);
            }
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if !empty_receiver(&state, channel) {
                return Ok(false);
            }
            let generation = state.page_generation;
            // start() uses this same gate and refuses until the new document finishes.
            state.receiver_reload = Some(generation);
            state.status = "reconnecting".into();
            generation
        };
        if view.reload().is_err() {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if state.receiver_reload == Some(generation) {
                state.receiver_reload = None;
            }
            return Err(unavailable());
        }
        tracing::info!(
            diag_stage = "auto_receiver_reconnect",
            channel_id = channel,
            live_key,
            webview = self.label(),
            generation,
            "empty live receiver reconnect requested"
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn empty() -> ViewState {
        ViewState {
            open: true,
            channel: Some(CHANNEL.into()),
            page_generation: 7,
            capture_diagnostics: Some(
                json!({"reason":"waiting_video","appendBytes":0,"sources":[]}),
            ),
            ..Default::default()
        }
    }
    #[test]
    fn only_explicitly_empty_player_is_recoverable() {
        assert!(empty_receiver(&empty(), CHANNEL));
        assert!(!empty_receiver(
            &empty(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ));
        for diag in [
            None,
            Some(json!({"reason":"waiting_video"})),
            Some(json!({"reason":"waiting_tracks","appendBytes":0,"sources":[]})),
            Some(json!({"reason":"waiting_video","appendBytes":1,"sources":[]})),
            Some(json!({"reason":"waiting_video","appendBytes":0,"sources":[{}]})),
        ] {
            let mut s = empty();
            s.capture_diagnostics = diag;
            assert!(!empty_receiver(&s, CHANNEL));
        }
    }
    #[test]
    fn playable_paused_and_active_recorders_are_never_reloaded() {
        for change in [
            (|s: &mut ViewState| s.ready = true) as fn(&mut ViewState),
            |s| s.video_width = 1920,
            |s| s.video_height = 1080,
            |s| s.page_recording = true,
            |s| s.recording = Some("active".into()),
            |s| s.account_busy = true,
            |s| s.auth_checking = true,
            |s| s.extension_connecting = true,
            |s| s.receiver_reload = Some(7),
            |s| s.open = false,
        ] {
            let mut s = empty();
            s.video_paused = true;
            change(&mut s);
            assert!(!empty_receiver(&s, CHANNEL));
        }
    }
    #[test]
    fn pending_start_continuation_or_confirmation_prevents_reload() {
        let arm = || Arm {
            id: "pending".into(),
            root: PathBuf::new(),
            archive_root: None,
            capture_chat: false,
            created: Instant::now(),
            generation: 7,
        };
        let mut s = empty();
        s.arm = Some(arm());
        assert!(!empty_receiver(&s, CHANNEL));
        s.arm = None;
        s.continuation = Some(arm());
        assert!(!empty_receiver(&s, CHANNEL));
        s.continuation = None;
        s.pending_control = Some(PendingControl {
            id: "approval".into(),
            action: ControlAction::RecordStart,
            channel_id: CHANNEL.into(),
            expires_at: now_ms() + 20_000,
            generation: 7,
            created: Instant::now(),
            target_recording: None,
            target_arm: None,
        });
        assert!(!empty_receiver(&s, CHANNEL));
        s.confirming_control = s.pending_control.take();
        assert!(!empty_receiver(&s, CHANNEL));
    }
    #[test]
    fn old_document_cannot_release_new_document_reservation() {
        let mut s = empty();
        s.receiver_reload = Some(7);
        finished(&mut s);
        assert_eq!(s.receiver_reload, Some(7));
        s.page_generation = 8;
        finished(&mut s);
        assert!(s.receiver_reload.is_none());
    }
}
