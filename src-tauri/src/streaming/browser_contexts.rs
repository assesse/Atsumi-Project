//! One catalog and lifecycle authority, with isolated per-video controllers.
//! Registry entries are native-created labels; remote payloads never select them.
use super::*;
use std::sync::Weak;

#[derive(Default)]
pub(super) struct ContextGroup {
    pub gate: Mutex<()>,
    members: Mutex<Vec<Weak<Inner>>>,
    pending: Mutex<Option<(String, String, Instant)>>,
    pub reconfiguring: AtomicU64,
    pub screenshot_decode: Mutex<()>,
    notices: Mutex<std::collections::VecDeque<String>>,
}

impl ContextGroup {
    pub fn notice(&self, message: &str) {
        let mut notices = self.notices.lock().unwrap_or_else(|p| p.into_inner());
        if notices.len() == 32 {
            notices.pop_front();
        }
        notices.push_back(message.chars().take(240).collect());
    }
    pub fn take_notices(&self) -> Vec<String> {
        self.notices
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
            .collect()
    }
    pub(super) fn register(&self, inner: &Arc<Inner>) -> Result<(), StreamError> {
        let mut members = self.members.lock().map_err(|_| unavailable())?;
        members.retain(|entry| {
            entry
                .upgrade()
                .is_some_and(|inner| !inner.detached.load(Ordering::Acquire))
        });
        if members.len() >= 9
            || members
                .iter()
                .filter_map(Weak::upgrade)
                .any(|entry| entry.label == inner.label)
        {
            return Err(unavailable());
        }
        members.push(Arc::downgrade(inner));
        Ok(())
    }
    pub fn hosts(&self) -> Vec<OfficialBrowser> {
        self.members
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
            .map(|inner| OfficialBrowser { inner })
            .collect()
    }
    pub fn reserve_pending(&self, label: &str) -> Result<String, StreamError> {
        let mut pending = self.pending.lock().map_err(|_| unavailable())?;
        if pending
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed() <= Duration::from_secs(20))
        {
            return Err(error(
                "BROWSER_CONTROL_BUSY",
                "이미 확인 중인 요청이 있습니다.",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        *pending = Some((label.into(), id.clone(), Instant::now()));
        Ok(id)
    }
    pub fn release_pending(&self, label: &str, id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            if pending
                .as_ref()
                .is_some_and(|(owner, nonce, _)| owner == label && nonce == id)
            {
                *pending = None;
            }
        }
    }
}

impl OfficialBrowser {
    pub(crate) fn label(&self) -> &str {
        &self.inner.label
    }
    pub(crate) fn capture_context(&self, label: Option<&str>) -> Result<Self, StreamError> {
        let label = label.unwrap_or(WINDOW_LABEL);
        self.inner
            .contexts
            .hosts()
            .into_iter()
            .find(|host| host.label() == label && !host.inner.detached.load(Ordering::Acquire))
            .ok_or_else(unavailable)
    }
    pub(super) fn pane_controller(&self, label: &str, channel: &str) -> Result<Self, StreamError> {
        if label
            .strip_prefix("chzzk-mado-")
            .or_else(|| label.strip_prefix("chzzk-auto-"))
            .is_none_or(|suffix| suffix.len() != 32 || uuid::Uuid::parse_str(suffix).is_err())
        {
            return Err(unavailable());
        }
        let state = ViewState {
            channel: Some(channel.into()),
            open: true,
            ..ViewState::default()
        };
        let child = Self {
            inner: Arc::new(Inner {
                data_dir: self.inner.data_dir.clone(),
                label: label.into(),
                detached: AtomicBool::new(false),
                contexts: self.inner.contexts.clone(),
                auto_record: self.inner.auto_record.clone(),
                store: self.inner.store.clone(),
                merges: self.inner.merges.clone(),
                replay_assets: self.inner.replay_assets.clone(),
                view: Mutex::new(state),
                viewport_writes: Mutex::new(()),
                viewport_revision: Arc::new(AtomicU64::new(0)),
                multiview: multiview::MultiViewHost::default(),
                screenshots: Mutex::new(screenshot::ScreenshotCapture::default()),
                encoded: Mutex::new(None),
                closing: self.inner.closing.clone(),
                reserved: self.inner.reserved.clone(),
                chat: Mutex::new(None),
                retired_chat: Mutex::new(Vec::new()),
                page_chat: Mutex::new(None),
                writes: Mutex::new(()),
            }),
        };
        self.inner.contexts.register(&child.inner)?;
        Ok(child)
    }
    pub(super) fn local_active_ids(&self) -> Vec<String> {
        let state = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(id) = &state.recording {
            vec![id.clone()]
        } else if let Some(arm) = &state.arm {
            vec![format!("browser-pending-{}", arm.id)]
        } else if let Some(control) = &state.confirming_control {
            vec![format!("browser-confirming-{}", control.id)]
        } else {
            Vec::new()
        }
    }
    /// Layout changes may close visible players, never independent automatic
    /// receivers. Exit/update guards continue to use the global active_ids().
    pub(super) fn ui_active_ids(&self) -> Vec<String> {
        self.inner
            .contexts
            .hosts()
            .iter()
            .filter(|host| !host.label().starts_with("chzzk-auto-"))
            .flat_map(Self::local_active_ids)
            .collect()
    }
    pub(super) fn primary_profile_busy(&self) -> bool {
        if self.label() == WINDOW_LABEL {
            return false;
        }
        self.capture_context(None).ok().is_none_or(|root| {
            root.inner
                .view
                .lock()
                .map_or(true, |s| s.account_busy || s.extension_connecting)
        })
    }
    pub(super) fn detach_controller(&self) {
        self.inner.detached.store(true, Ordering::Release);
        self.interrupt("window_closed");
        self.inner
            .screenshots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancel();
    }
    pub fn request_control(&self, action: ControlAction) -> Result<BrowserSnapshot, StreamError> {
        let channel = self
            .inner
            .view
            .lock()
            .map_err(|_| unavailable())?
            .channel
            .clone()
            .ok_or_else(unavailable)?;
        self.control_intent(&channel, &channel, action)?;
        self.snapshot()
    }
    pub fn ack_ui_action(&self, id: &str) -> Result<BrowserSnapshot, StreamError> {
        {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if state
                .pending_ui_action
                .as_ref()
                .is_some_and(|action| action.id == id)
            {
                state.pending_ui_action = None;
            }
        }
        self.snapshot()
    }
    pub(super) fn view_intent(
        &self,
        source: &str,
        channel: &str,
        action: &str,
    ) -> Result<Value, StreamError> {
        if self.inner.detached.load(Ordering::Acquire)
            || !matches!(action, "exit_focus" | "open_settings" | "audio_toggle")
            || (matches!(action, "open_settings") && self.label() != WINDOW_LABEL)
        {
            return Err(unavailable());
        }
        let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
        if source != channel
            || state.channel.as_deref() != Some(source)
            || state
                .last_ui_intent
                .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return Err(unavailable());
        }
        state.last_ui_intent = Some(Instant::now());
        // These requests only open main-app UI. They cannot authorize recording,
        // account access, extension installation or any other privileged action.
        if matches!(action, "exit_focus" | "open_settings") {
            state.pending_ui_action = Some(UiAction {
                id: uuid::Uuid::new_v4().to_string(),
                action: action.into(),
                expires_at: now_ms().saturating_add(8_000),
                channel: channel.into(),
                viewport_epoch: state.viewport_epoch,
                page_generation: state.page_generation,
            });
        } else if self.label() == WINDOW_LABEL {
            return Err(unavailable());
        }
        Ok(json!({"accepted":true}))
    }
}

// A cancelling close can overlap an in-flight configure. Each reservation owns
// its count; completing one must not reopen capture while the other still runs.
pub(super) struct Reconfigure<'a>(pub &'a AtomicU64);
impl Drop for Reconfigure<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn pane(root: &OfficialBrowser) -> OfficialBrowser {
        root.pane_controller(
            &format!("chzzk-mado-{}", uuid::Uuid::new_v4().simple()),
            CHANNEL,
        )
        .unwrap()
    }
    #[test]
    fn background_recordings_participate_in_exit_but_not_unrelated_view_layout_guards() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let background = root
            .pane_controller(
                &format!("chzzk-auto-{}", uuid::Uuid::new_v4().simple()),
                CHANNEL,
            )
            .unwrap();
        background.inner.view.lock().unwrap().recording = Some("auto-record".into());
        assert_eq!(root.active_ids(), vec!["auto-record"]);
        assert!(root.ui_active_ids().is_empty());
        assert!(root.reserve_update().is_err());
        let visible = pane(&root);
        visible.inner.view.lock().unwrap().recording = Some("visible".into());
        assert_eq!(root.ui_active_ids(), vec!["visible"]);
    }
    #[test]
    fn stop_is_not_delayed_by_the_previous_start_click_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let capture = pane(&root);
        {
            let mut state = capture.inner.view.lock().unwrap();
            state.recording = Some("running".into());
            state.last_control_intent = Some(Instant::now());
        }
        assert!(capture.request_control(ControlAction::RecordStop).is_ok());
        let id = capture.snapshot().unwrap().pending_control.unwrap().id;
        assert_eq!(
            capture
                .take_control(&id, true, true)
                .unwrap()
                .unwrap()
                .action,
            ControlAction::RecordStop
        );
    }
    #[test]
    fn four_panes_share_store_and_exit_authority_but_not_approval_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let panes = (0..4).map(|_| pane(&root)).collect::<Vec<_>>();
        assert!(root
            .pane_controller(&format!("chzzk-mado-{}", uuid::Uuid::new_v4()), CHANNEL)
            .is_err());
        for (index, pane) in panes.iter().enumerate() {
            assert!(Arc::ptr_eq(&root.inner.store, &pane.inner.store));
            assert!(Arc::ptr_eq(&root.inner.closing, &pane.inner.closing));
            assert!(Arc::ptr_eq(&root.inner.reserved, &pane.inner.reserved));
            pane.inner.view.lock().unwrap().recording = Some(format!("recording-{index}"));
        }
        assert_eq!(root.active_ids().len(), 4);
        assert!(root.reserve_update().is_err());
        panes[0].inner.view.lock().unwrap().recording = None;
        assert_eq!(root.active_ids().len(), 3);
        assert!(panes[0].snapshot().unwrap().recordings.is_empty());
    }
    #[test]
    fn pending_approval_is_global_but_nonce_and_ui_actions_are_pane_bound() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let first = pane(&root);
        let second = pane(&root);
        for pane in [&first, &second] {
            pane.inner.view.lock().unwrap().ready = true;
        }
        first.request_control(ControlAction::RecordStart).unwrap();
        let id = first.snapshot().unwrap().pending_control.unwrap().id;
        assert!(second.request_control(ControlAction::RecordStart).is_err());
        assert!(second.take_control(&id, true, true).is_err());
        first.take_control(&id, true, true).unwrap();
        assert_eq!(root.active_ids().len(), 1); // confirmation itself protects close
        second.inner.view.lock().unwrap().last_control_intent = None;
        second.request_control(ControlAction::Screenshot).unwrap();
        first.view_intent(CHANNEL, CHANNEL, "exit_focus").unwrap();
        let id = first.snapshot().unwrap().pending_ui_action.unwrap().id;
        second.ack_ui_action(&id).unwrap();
        assert!(first.snapshot().unwrap().pending_ui_action.is_some());
        first.ack_ui_action(&id).unwrap();
        assert!(first.snapshot().unwrap().pending_ui_action.is_none());
    }
    #[test]
    fn player_settings_and_focus_intents_are_ui_only_and_main_window_bound() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        root.inner.view.lock().unwrap().channel = Some(CHANNEL.into());
        let child = pane(&root);
        {
            let action = "open_settings";
            assert!(child.view_intent(CHANNEL, CHANNEL, action).is_err());
            assert!(root
                .view_intent("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", CHANNEL, action)
                .is_err());
            root.inner.view.lock().unwrap().last_ui_intent = None;
            root.view_intent(CHANNEL, CHANNEL, action).unwrap();
            let snapshot = root.snapshot().unwrap();
            let intent = snapshot.pending_ui_action.unwrap();
            assert_eq!(intent.action, action);
            assert!(snapshot.pending_control.is_none());
            assert!(snapshot.recording_id.is_none());
            assert!(root.active_ids().is_empty());
            root.ack_ui_action("unrelated").unwrap();
            assert!(root.snapshot().unwrap().pending_ui_action.is_some());
            root.ack_ui_action(&intent.id).unwrap();
            assert!(root.snapshot().unwrap().pending_ui_action.is_none());
        }
        for action in ["login", "start", "extension_connect", "open_file"] {
            assert!(root.view_intent(CHANNEL, CHANNEL, action).is_err());
        }
    }
    #[test]
    fn player_ui_state_stays_synchronized_and_old_intents_expire() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        root.inner.view.lock().unwrap().channel = Some(CHANNEL.into());
        for action in ["presentation_full", "presentation_clean", "toggle_focus"] {
            root.inner.view.lock().unwrap().last_ui_intent = None;
            assert!(root.view_intent(CHANNEL, CHANNEL, action).is_err());
        }
        for invalidate in ["expired", "new_ui", "new_page", "other_channel"] {
            {
                let mut state = root.inner.view.lock().unwrap();
                state.channel = Some(CHANNEL.into());
                state.last_ui_intent = None;
            }
            root.view_intent(CHANNEL, CHANNEL, "open_settings").unwrap();
            {
                let mut state = root.inner.view.lock().unwrap();
                match invalidate {
                    "expired" => state.pending_ui_action.as_mut().unwrap().expires_at = 0,
                    "new_ui" => state.viewport_epoch += 1,
                    "new_page" => state.page_generation += 1,
                    _ => state.channel = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                }
            }
            assert!(root.snapshot().unwrap().pending_ui_action.is_none());
        }
    }
    #[test]
    fn stale_or_other_channel_messages_cannot_retarget_a_pane() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let child = pane(&root);
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert!(child
            .process(
                other,
                BrowserMessage::Status {
                    channel_id: other.into(),
                    ready: true,
                    recording: false,
                    detail: String::new(),
                    video_width: 1920,
                    video_height: 1080,
                    paused: false,
                    capture_diagnostics: None,
                }
            )
            .is_err());
        assert_eq!(
            child.snapshot().unwrap().channel_id.as_deref(),
            Some(CHANNEL)
        );
        child.detach_controller();
        assert!(child.request_control(ControlAction::Screenshot).is_err());
        assert!(child.view_intent(CHANNEL, CHANNEL, "exit_focus").is_err());
        assert!(root.capture_context(Some(child.label())).is_err());
    }
    #[test]
    fn cancelling_close_keeps_configuration_reserved_until_both_finish() {
        let group = ContextGroup::default();
        group.reconfiguring.fetch_add(2, Ordering::AcqRel);
        let configuring = Reconfigure(&group.reconfiguring);
        let closing = Reconfigure(&group.reconfiguring);
        drop(configuring);
        assert_eq!(group.reconfiguring.load(Ordering::Acquire), 1);
        drop(closing);
        assert_eq!(group.reconfiguring.load(Ordering::Acquire), 0);
    }
    #[test]
    fn screenshot_decode_budget_is_shared_across_all_panes() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let first = pane(&root);
        let second = pane(&root);
        let guard = first.inner.contexts.screenshot_decode.try_lock().unwrap();
        assert!(second.inner.contexts.screenshot_decode.try_lock().is_err());
        drop(guard);
        assert!(second.inner.contexts.screenshot_decode.try_lock().is_ok());
    }

    #[test]
    fn pre_begin_seek_or_rate_rejection_releases_arm_but_running_capture_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let pane = pane(&root);
        for detail in ["seek", "rate_change"] {
            pane.inner.view.lock().unwrap().arm = Some(Arm {
                id: uuid::Uuid::new_v4().to_string(),
                root: dir.path().into(),
                capture_chat: false,
                created: Instant::now(),
                generation: 0,
            });
            let status = |recording| BrowserMessage::Status {
                channel_id: CHANNEL.into(),
                ready: false,
                recording,
                detail: detail.into(),
                video_width: 1920,
                video_height: 1080,
                paused: false,
                capture_diagnostics: None,
            };
            pane.process(CHANNEL, status(true)).unwrap();
            assert!(pane.inner.view.lock().unwrap().arm.is_some());
            pane.process(CHANNEL, status(false)).unwrap();
            assert!(pane.inner.view.lock().unwrap().arm.is_none());
        }
    }

    #[test]
    fn four_native_controllers_write_independent_files_and_one_failure_preserves_others() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let panes = (0..4).map(|_| pane(&root)).collect::<Vec<_>>();
        let mut ids = Vec::new();
        let webm = [
            0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 1, 0, 0, 0, 0, 0, 0, 0,
        ];
        for (index, pane) in panes.iter().enumerate() {
            let nonce = uuid::Uuid::new_v4().to_string();
            {
                let mut state = pane.inner.view.lock().unwrap();
                state.ready = true;
                state.arm = Some(Arm {
                    id: nonce.clone(),
                    root: dir.path().into(),
                    capture_chat: false,
                    created: Instant::now(),
                    generation: 0,
                });
            }
            let ack = pane
                .process(
                    CHANNEL,
                    BrowserMessage::Begin {
                        request_id: nonce,
                        channel_id: CHANNEL.into(),
                        title: format!("synthetic-{index}"),
                        mime_type: "video/webm".into(),
                    },
                )
                .unwrap();
            let id = ack["id"].as_str().unwrap().to_owned();
            pane.process(
                CHANNEL,
                BrowserMessage::Chunk {
                    recording_id: id.clone(),
                    segment_index: 0,
                    chunk_index: 0,
                    data: STANDARD.encode(webm),
                },
            )
            .unwrap();
            ids.push(id);
        }
        assert_eq!(root.active_ids().len(), 4);
        assert!(panes[1]
            .process(
                CHANNEL,
                BrowserMessage::Chunk {
                    recording_id: ids[0].clone(),
                    segment_index: 0,
                    chunk_index: 1,
                    data: STANDARD.encode(b"wrong pane")
                }
            )
            .is_err());
        panes[0].interrupt_matching("native_rejected", Some(&ids[0]));
        assert_eq!(root.active_ids().len(), 3);
        for (index, pane) in panes.iter().enumerate().skip(1) {
            pane.process(
                CHANNEL,
                BrowserMessage::Chunk {
                    recording_id: ids[index].clone(),
                    segment_index: 0,
                    chunk_index: 1,
                    data: STANDARD.encode([index as u8]),
                },
            )
            .unwrap();
            pane.process(
                CHANNEL,
                BrowserMessage::Segment {
                    recording_id: ids[index].clone(),
                    segment_index: 0,
                    duration_seconds: 15.0,
                },
            )
            .unwrap();
            pane.process(
                CHANNEL,
                BrowserMessage::Finish {
                    recording_id: ids[index].clone(),
                    interrupted: false,
                    reason: None,
                },
            )
            .unwrap();
        }
        assert!(root.active_ids().is_empty());
        drop(panes);
        drop(root);
        let recovered = OfficialBrowser::new(dir.path().into())
            .unwrap()
            .snapshot()
            .unwrap()
            .recordings;
        assert_eq!(recovered.len(), 4);
        let first = recovered.iter().find(|r| r.id == ids[0]).unwrap();
        assert!(first.partial.is_some());
        for (index, id) in ids.iter().enumerate().skip(1) {
            let record = recovered.iter().find(|r| &r.id == id).unwrap();
            assert_eq!(record.segment_count, 1);
            let bytes = std::fs::read(Path::new(&record.output_dir).join(&record.segments[0].file))
                .unwrap();
            assert_eq!(bytes, [webm.as_slice(), &[index as u8]].concat());
        }
    }
}
