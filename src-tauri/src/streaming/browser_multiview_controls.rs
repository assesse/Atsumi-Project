//! Trusted context selection, per-pane audio, and reload-safe capture lifetime.
use super::*;

impl MultiViewHost {
    fn suspend_capture_views(&self) -> Option<Vec<Arc<Pane>>> {
        let mut state = self.state.lock().ok()?;
        state.epoch = state.epoch.wrapping_add(1).max(1);
        let epoch = state.epoch;
        for pane in &state.panes {
            pane.epoch.store(epoch, Ordering::Release);
            pane.revision.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut viewport) = pane.viewport.lock() {
                viewport.epoch = epoch;
                viewport.request_sequence = None;
                viewport.visible = false;
            }
            if let Some(capture) = &pane.capture {
                if let Ok(mut view) = capture.inner.view.lock() {
                    if let Some(pending) = view.pending_control.take() {
                        capture
                            .inner
                            .contexts
                            .release_pending(capture.label(), &pending.id);
                    }
                }
            }
        }
        Some(state.panes.clone())
    }
}

impl OfficialBrowser {
    fn pane_for_control(&self, id: &str, epoch: u64) -> Result<Arc<Pane>, StreamError> {
        let state = self
            .inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?;
        if state.epoch != epoch || !self.multiview_active() {
            return Err(error("MULTIVIEW_STALE", "이전 마도 요청입니다."));
        }
        state
            .panes
            .iter()
            .find(|pane| {
                pane.id == id && pane.kind == PaneKind::Video && !pane.dead.load(Ordering::Acquire)
            })
            .cloned()
            .ok_or_else(unavailable)
    }
    pub fn request_pane_control(
        &self,
        id: &str,
        action: ControlAction,
        epoch: u64,
    ) -> Result<MultiViewSnapshot, StreamError> {
        self.pane_for_control(id, epoch)?
            .capture
            .as_ref()
            .ok_or_else(unavailable)?
            .request_control(action)?;
        self.multiview_snapshot()
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Keep the existing explicit IPC approval and context fences together"
    )]
    pub fn confirm_pane_control(
        &self,
        app: &AppHandle,
        id: &str,
        request: &str,
        approve: bool,
        rights: bool,
        chat: bool,
        epoch: u64,
    ) -> Result<MultiViewSnapshot, StreamError> {
        self.pane_for_control(id, epoch)?
            .capture
            .as_ref()
            .ok_or_else(unavailable)?
            .confirm_control(app, request, approve, rights, chat)?;
        self.multiview_snapshot()
    }
    pub fn ack_pane_ui_action(
        &self,
        id: &str,
        action_id: &str,
        epoch: u64,
    ) -> Result<MultiViewSnapshot, StreamError> {
        self.pane_for_control(id, epoch)?
            .capture
            .as_ref()
            .ok_or_else(unavailable)?
            .ack_ui_action(action_id)?;
        self.multiview_snapshot()
    }
    pub fn set_pane_audio(
        &self,
        app: &AppHandle,
        id: &str,
        enabled: bool,
        epoch: u64,
    ) -> Result<(), StreamError> {
        let _mutation = self
            .inner
            .multiview
            .mutations
            .try_lock()
            .map_err(|_| busy())?;
        let pane = self.pane_for_control(id, epoch)?;
        let view = app.get_webview(id).ok_or_else(unavailable)?;
        pane.audio.store(enabled, Ordering::Release);
        if let Err(error) = native_mute(&view, pane.clone(), !enabled) {
            pane.audio.store(false, Ordering::Release);
            let _ = native_mute(&view, pane.clone(), true);
            send_audio(&view, &pane, true);
            return Err(error);
        }
        send_audio(&view, &pane, true);
        let mut state = self
            .inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?;
        let owners = state
            .panes
            .iter()
            .filter(|p| p.audio.load(Ordering::Acquire) && !p.dead.load(Ordering::Acquire))
            .map(|p| p.channel.clone())
            .collect::<Vec<_>>();
        state.audio_owner = if owners.len() == 1 {
            owners.into_iter().next()
        } else {
            None
        };
        Ok(())
    }
    pub(crate) fn toggle_pane_audio(&self, app: &AppHandle, id: &str) -> Result<(), StreamError> {
        let (epoch, enabled) = {
            let state = self
                .inner
                .multiview
                .state
                .lock()
                .map_err(|_| unavailable())?;
            let pane = state
                .panes
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(unavailable)?;
            (state.epoch, !pane.audio.load(Ordering::Acquire))
        };
        self.set_pane_audio(app, id, enabled, epoch)
    }
    /// Return None when recorders survive, or the old layout epoch to close.
    /// Capture document epochs remain distinct from trusted-UI viewport epochs.
    pub fn suspend_multiview(&self, app: &AppHandle) -> Option<u64> {
        let _gate = self.inner.contexts.gate.lock().ok()?;
        if self.ui_active_ids().is_empty() {
            return self.detach_multiview(app);
        }
        let panes = self.inner.multiview.suspend_capture_views()?;
        for pane in panes {
            if let Some(view) = app.get_webview(&pane.id) {
                let _ = view.hide();
            }
        }
        None
    }
    pub(super) fn queue_channel_names(&self, app: &AppHandle) {
        let Ok(mut slot) = self.inner.multiview.metadata.lock() else {
            return;
        };
        if slot.is_none() {
            let (sender, receiver) =
                std::sync::mpsc::sync_channel::<(String, Vec<std::sync::Weak<Pane>>)>(4);
            let receiver = Arc::new(Mutex::new(receiver));
            for index in 0..2 {
                let receiver = receiver.clone();
                let app = app.clone();
                let _ = thread::Builder::new()
                    .name(format!("chzzk-channel-name-{index}"))
                    .spawn(move || {
                        let provider = ChzzkProvider::new().ok();
                        loop {
                            let job = receiver.lock().ok().and_then(|rx| rx.recv().ok());
                            let Some((channel, panes)) = job else {
                                break;
                            };
                            if !panes.iter().any(|p| {
                                p.upgrade().is_some_and(|p| !p.dead.load(Ordering::Acquire))
                            }) {
                                continue;
                            }
                            let Some(info) =
                                provider.as_ref().and_then(|p| p.inspect(&channel).ok())
                            else {
                                continue;
                            };
                            let name = info
                                .channel_name
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(120)
                                .collect::<String>();
                            for pane in panes.into_iter().filter_map(|p| p.upgrade()) {
                                if !pane.dead.load(Ordering::Acquire)
                                    && pane.channel == info.channel_id
                                {
                                    if let Ok(mut label) = pane.channel_name.lock() {
                                        *label = name.clone();
                                    }
                                    if pane.kind == PaneKind::Chat {
                                        if let Some(view) = app.get_webview(&pane.id) {
                                            let _ = view.eval(chat_header_script(&pane));
                                        }
                                    }
                                }
                            }
                        }
                    });
            }
            *slot = Some(sender);
        }
        let Ok(state) = self.inner.multiview.state.lock() else {
            return;
        };
        for entry in &state.entries {
            let panes = state
                .panes
                .iter()
                .filter(|p| p.channel == entry.channel_id)
                .map(Arc::downgrade)
                .collect();
            let _ = slot
                .as_ref()
                .unwrap()
                .try_send((entry.channel_id.clone(), panes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn video(root: &OfficialBrowser) -> Arc<Pane> {
        let id = format!("chzzk-mado-{}", uuid::Uuid::new_v4().simple());
        let capture = root.pane_controller(&id, CHANNEL).unwrap();
        Arc::new(Pane {
            id,
            channel: CHANNEL.into(),
            number: 1,
            kind: PaneKind::Video,
            epoch: AtomicU64::new(7),
            viewport: Mutex::new(BrowserViewport {
                epoch: 7,
                request_sequence: Some(5),
                visible: true,
                ..Default::default()
            }),
            revision: Arc::new(AtomicU64::new(3)),
            writes: Mutex::new(()),
            audio: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            initial_blank: AtomicBool::new(false),
            status: Mutex::new("ready".into()),
            channel_name: Mutex::new("채널 이름".into()),
            capture: Some(capture),
        })
    }
    #[test]
    fn main_reload_changes_viewport_epoch_but_preserves_recorders_and_audio() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let panes = (0..4).map(|_| video(&root)).collect::<Vec<_>>();
        for (index, pane) in panes.iter().enumerate() {
            pane.capture
                .as_ref()
                .unwrap()
                .inner
                .view
                .lock()
                .unwrap()
                .recording = Some(format!("id-{index}"));
            pane.audio.store(true, Ordering::Release);
        }
        {
            let mut state = root.inner.multiview.state.lock().unwrap();
            state.epoch = 7;
            state.panes = panes.clone();
        }
        root.inner.multiview.active.store(true, Ordering::Release);
        root.inner.multiview.suspend_capture_views().unwrap();
        assert_eq!(root.active_ids().len(), 4);
        for pane in panes {
            let viewport = pane.viewport.lock().unwrap();
            assert_eq!(viewport.epoch, 8);
            assert_eq!(viewport.request_sequence, None);
            assert!(!viewport.visible);
            assert_eq!(pane.revision.load(Ordering::Acquire), 4);
            assert!(!pane.dead.load(Ordering::Acquire));
            assert!(pane.audio.load(Ordering::Acquire));
            assert_eq!(
                pane.capture
                    .as_ref()
                    .unwrap()
                    .inner
                    .view
                    .lock()
                    .unwrap()
                    .page_generation,
                0
            );
        }
        assert!(root.pane_for_control("old", 7).is_err());
        let snapshot = serde_json::to_value(root.multiview_snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["panes"][0]["channelName"], "채널 이름");
        assert_eq!(snapshot["panes"][0]["audioEnabled"], true);
        {
            let pane = root.inner.multiview.state.lock().unwrap().panes[0].clone();
            let mut state = pane.capture.as_ref().unwrap().inner.view.lock().unwrap();
            state.chat_status = "storage_failed".into();
            state.chat_count = 17;
        }
        let snapshot = serde_json::to_value(root.multiview_snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["panes"][0]["chatStatus"], "storage_failed");
        assert_eq!(snapshot["panes"][0]["chatCount"], 17);
    }
    #[test]
    fn audio_permission_and_approval_do_not_transfer_between_panes() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let first = video(&root);
        let second = video(&root);
        first.audio.store(true, Ordering::Release);
        second.audio.store(true, Ordering::Release);
        first.audio.store(false, Ordering::Release);
        assert!(second.audio.load(Ordering::Acquire));
        assert!(!must_mute(
            second.kind,
            second.audio.load(Ordering::Acquire),
            false,
            false,
            false
        ));
        {
            let mut state = root.inner.multiview.state.lock().unwrap();
            state.epoch = 7;
            state.panes = vec![first.clone(), second.clone()];
        }
        root.inner.multiview.active.store(true, Ordering::Release);
        first
            .capture
            .as_ref()
            .unwrap()
            .inner
            .view
            .lock()
            .unwrap()
            .ready = true;
        let snapshot = root
            .request_pane_control(&first.id, ControlAction::Screenshot, 7)
            .unwrap();
        let pending = snapshot.pending_control.unwrap();
        assert_eq!(pending.pane_id, first.id);
        assert!(second
            .capture
            .as_ref()
            .unwrap()
            .take_control(&pending.control.id, true, true)
            .is_err());
        assert!(root
            .request_pane_control(&first.id, ControlAction::Screenshot, 6)
            .is_err());
    }
}
