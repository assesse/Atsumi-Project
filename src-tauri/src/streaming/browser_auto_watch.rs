//! Attach the common live UI to an existing receiver. Commands are scoped to its
//! native-created lease and page generation; detach never stops its recording.
use super::*;

#[derive(Default)]
pub(super) struct WatchState {
    lifecycle: AtomicU64,
    mutations: Mutex<()>,
    lease: Mutex<Option<Arc<WatchLease>>>,
}
struct WatchLease {
    id: String,
    recording_id: String,
    pane: Arc<Pane>,
    epoch: u64,
    active: AtomicBool,
    requested_audio: AtomicBool,
    generation: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchTarget {
    kind: &'static str,
    watch_id: Option<String>,
    channel_id: String,
    recording_id: String,
    channel_name: String,
    epoch: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchSnapshot {
    recording: bool,
    status: String,
    error: Option<String>,
    audio_enabled: bool,
    chat_count: u64,
}
fn ended() -> StreamError {
    error(
        "AUTO_WATCH_ENDED",
        "해당 녹화가 종료됐거나 변경됐습니다. 자동 녹화 목록을 다시 확인해 주세요.",
    )
}
fn stale() -> StreamError {
    error(
        "VIEWPORT_STALE",
        "이전 실시간 보기 요청입니다. 자동 녹화 목록에서 다시 열어 주세요.",
    )
}
fn recording_matches(host: &OfficialBrowser, channel: &str, recording: &str) -> bool {
    !host.inner.detached.load(Ordering::Acquire)
        && host.inner.view.lock().is_ok_and(|state| {
            state.channel.as_deref() == Some(channel)
                && state.recording.as_deref() == Some(recording)
                && state.status == "recording"
        })
}
impl WatchLease {
    fn native_audio_muted(&self, viewport: &BrowserViewport) -> bool {
        // Visibility never participates: the same media can be playing in PiP
        // or simply in the background. Privacy is a reversible native gate.
        !self.active.load(Ordering::Acquire)
            || self.pane.dead.load(Ordering::Acquire)
            || !self.requested_audio.load(Ordering::Acquire)
            || viewport.suspend_audio
    }
    fn connected(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && !self.pane.dead.load(Ordering::Acquire)
            && self.pane.capture.as_ref().is_some_and(|capture| {
                !capture.inner.detached.load(Ordering::Acquire)
                    && capture.inner.view.lock().is_ok_and(|state| {
                        state.open
                            && state.channel.as_deref() == Some(self.pane.channel.as_str())
                            && state.page_generation == self.generation
                    })
            })
    }
    fn recording(&self) -> bool {
        !self.pane.dead.load(Ordering::Acquire)
            && self
                .pane
                .capture
                .as_ref()
                .is_some_and(|host| recording_matches(host, &self.pane.channel, &self.recording_id))
    }
    fn invalidate(&self) -> u64 {
        self.active.store(false, Ordering::Release);
        self.pane.audio.store(false, Ordering::Release);
        let mut viewport = self.pane.viewport.lock().unwrap_or_else(|p| p.into_inner());
        viewport.visible = false;
        viewport.occluded = false;
        self.pane.revision.fetch_add(1, Ordering::AcqRel) + 1
    }
    fn hide_failed_revision(&self, expected: u64) -> Option<u64> {
        let mut viewport = self.pane.viewport.lock().ok()?;
        if !self.active.load(Ordering::Acquire)
            || self.pane.revision.load(Ordering::Acquire) != expected
        {
            return None;
        }
        viewport.visible = false;
        viewport.occluded = false;
        viewport.preserve_background = false;
        viewport.occlusions.clear();
        self.pane.audio.store(false, Ordering::Release);
        Some(self.pane.revision.fetch_add(1, Ordering::AcqRel) + 1)
    }
}

/// The scheduler may finish its job while the user is still watching or has
/// started a new recording. Do not close that same receiver underneath them.
impl OfficialBrowser {
    pub(crate) fn retire_auto_receiver(&self, label: &str) {
        self.inner
            .multiview
            .retired_auto_panes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(label.into());
    }
    pub(crate) fn claim_auto_receiver(&self, label: &str) {
        self.inner
            .multiview
            .retired_auto_panes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(label);
    }
    fn auto_receiver_in_use(&self, pane: &Pane) -> bool {
        self.inner
            .multiview
            .auto_watch
            .lease
            .lock()
            .is_ok_and(|slot| {
                slot.as_ref()
                    .is_some_and(|lease| lease.pane.id == pane.id && lease.connected())
            })
            || pane.capture.as_ref().is_some_and(|capture| {
                let recording_or_request = capture.inner.view.lock().is_ok_and(|s| {
                    s.recording.is_some()
                        || s.arm.is_some()
                        || s.confirming_control.is_some()
                        || s.pending_control.as_ref().is_some_and(|p| p.matches(&s))
                });
                recording_or_request
                    || capture
                        .inner
                        .screenshots
                        .try_lock()
                        .map_or(true, |mut shots| shots.active())
            })
    }
    pub(crate) fn reap_retired_auto_receivers(&self, app: &AppHandle) {
        let Ok(_mutation) = self.inner.multiview.auto_watch.mutations.try_lock() else {
            return;
        };
        let ids = self
            .inner
            .multiview
            .retired_auto_panes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        for id in ids {
            let pane = self
                .inner
                .multiview
                .auto_panes
                .lock()
                .ok()
                .and_then(|panes| panes.get(&id).cloned());
            if let Some(pane) = pane {
                if !self.auto_receiver_in_use(&pane) {
                    if let Some(capture) = &pane.capture {
                        capture.close_auto_view(app);
                    }
                }
            } else {
                self.claim_auto_receiver(&id);
            }
        }
    }
    fn auto_watch_capture(
        &self,
        id: &str,
    ) -> Result<(Arc<WatchLease>, OfficialBrowser), StreamError> {
        let lease = self.watch_lease(id)?;
        if !lease.connected() {
            return Err(ended());
        }
        let capture = lease.pane.capture.clone().ok_or_else(ended)?;
        Ok((lease, capture))
    }
    pub fn auto_watch_browser_snapshot(&self, id: &str) -> Result<BrowserSnapshot, StreamError> {
        let (lease, capture) = self.auto_watch_capture(id)?;
        let mut snapshot = capture.snapshot()?;
        snapshot.viewport_epoch = lease.epoch;
        // Account/profile state is shared, but playback/control state belongs
        // only to this receiver. Never copy a primary player's pending commands.
        {
            let profile = self.inner.view.lock().map_err(|_| unavailable())?;
            snapshot.login_status = profile.login_status.clone();
            snapshot.auth_status = profile.auth_status;
            snapshot.auth_checking = profile.auth_checking;
            snapshot.auth_error = profile.auth_error.clone();
            snapshot.account_busy = profile.account_busy;
            snapshot.extension_status = profile.extension.clone();
        }
        snapshot.recordings = self
            .inner
            .store
            .lock()
            .map_err(|_| unavailable())?
            .snapshot()?
            .into_iter()
            .filter(|item| {
                item.id == lease.recording_id || Some(&item.id) == snapshot.recording_id.as_ref()
            })
            .collect();
        Ok(snapshot)
    }
    fn auto_watch_browser_action(
        &self,
        app: &AppHandle,
        id: &str,
        operation: WatchOperation,
    ) -> Result<BrowserSnapshot, StreamError> {
        let (_lease, capture) = self.auto_watch_capture(id)?;
        match operation {
            WatchOperation::Snapshot {} => {}
            WatchOperation::Start {
                rights_acknowledged,
                capture_chat,
            } => {
                app.state::<AppState>().start_browser_managed_for(
                    app,
                    Some(capture.label()),
                    rights_acknowledged,
                    capture_chat,
                )?;
            }
            WatchOperation::Stop {} => {
                // The common one-use control pins the recording ID, suppresses
                // auto-restart for this broadcast and uses the normal stop path.
                let snapshot = capture.request_control(ControlAction::RecordStop)?;
                let pending = snapshot.pending_control.ok_or_else(control_stale)?;
                capture.confirm_control(app, &pending.id, true, true, true)?;
            }
            WatchOperation::RequestControl { action } => {
                capture.request_control(action)?;
            }
            WatchOperation::ConfirmControl {
                request_id,
                approve,
                rights_acknowledged,
                capture_chat,
            } => {
                capture.confirm_control(
                    app,
                    &request_id,
                    approve,
                    rights_acknowledged,
                    capture_chat,
                )?;
            }
            WatchOperation::AckUiAction { id } => {
                capture.ack_ui_action(&id)?;
            }
        }
        self.auto_watch_browser_snapshot(id)
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WatchOperation {
    Snapshot {},
    Start {
        rights_acknowledged: bool,
        capture_chat: bool,
    },
    Stop {},
    RequestControl {
        action: ControlAction,
    },
    ConfirmControl {
        request_id: String,
        approve: bool,
        rights_acknowledged: bool,
        capture_chat: bool,
    },
    AckUiAction {
        id: String,
    },
}

#[tauri::command]
pub async fn chzzk_auto_watch_browser(
    app: AppHandle,
    window: Webview,
    watch_id: String,
    operation: WatchOperation,
) -> ApiResult<BrowserSnapshot> {
    if let Err(cause) = require_main(&window) {
        return Err::<BrowserSnapshot, _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.auto_watch_browser_action(&app, &watch_id, operation)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
fn parked_viewport(epoch: u64) -> BrowserViewport {
    BrowserViewport {
        epoch,
        x: auto_surface::LEFT,
        y: 0.0,
        width: 1280.0,
        height: 720.0,
        visible: true,
        ..BrowserViewport::default()
    }
}
fn presentation_script(revision: u64, viewing: bool) -> String {
    format!("window.__atsumiAutoReceiver?.configure({{revision:{revision},viewing:{viewing}}});")
}
#[cfg(windows)]
fn queue_hide_audio(view: &Webview, revision: Arc<AtomicU64>, expected: u64, silence: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows::core::Interface;
    let _ = view.with_webview(move |platform| unsafe {
        if revision.load(Ordering::Acquire) != expected {
            return;
        }
        if let Ok(core) = platform
            .controller()
            .CoreWebView2()
            .and_then(|core| core.cast::<ICoreWebView2_8>())
        {
            if silence {
                let _ = core.SetIsMuted(true);
            }
        }
        let _ = platform.controller().SetIsVisible(false);
    });
}
#[cfg(not(windows))]
fn queue_hide_audio(_: &Webview, _: Arc<AtomicU64>, _: u64, _: bool) {}
fn queue_hide(view: &Webview, revision: Arc<AtomicU64>, expected: u64) {
    queue_hide_audio(view, revision, expected, true);
}

/// Also used by the offline WebView2 probe. Hidden presentation remains an
/// off-canvas renderer, without a new navigation, player, or media receiver.
pub fn apply_auto_receiver_viewport(
    view: &Webview,
    viewport: &BrowserViewport,
    revision: Arc<AtomicU64>,
    expected: u64,
) -> Result<(), StreamError> {
    viewport.validate()?;
    let projected = if viewport.visible {
        viewport.clone()
    } else {
        parked_viewport(viewport.epoch)
    };
    host_view::apply_pane_viewport(view, &projected, revision, expected)
}
fn park(app: &AppHandle, lease: &WatchLease, revision: u64) -> Result<(), StreamError> {
    if lease.pane.dead.load(Ordering::Acquire)
        || lease.pane.revision.load(Ordering::Acquire) != revision
    {
        return Ok(());
    }
    let Some(view) = app.get_webview(&lease.pane.id) else {
        return Ok(());
    };
    let _ = view.eval(presentation_script(revision, false));
    // A native hide by itself suspends the recording renderer. Restore its
    // existing off-canvas surface, using the same late-callback fence as Mado.
    apply_auto_receiver_viewport(
        &view,
        &BrowserViewport {
            epoch: lease.epoch,
            ..BrowserViewport::default()
        },
        lease.pane.revision.clone(),
        revision,
    )
}
fn recover_failed_presentation(app: &AppHandle, lease: &WatchLease, expected: u64) {
    if let Some(revision) = lease.hide_failed_revision(expected) {
        if let Some(view) = app.get_webview(&lease.pane.id) {
            queue_hide(&view, lease.pane.revision.clone(), revision);
        }
        if let Err(cause) = park(app, lease, revision) {
            tracing::warn!(code = %cause.code, "could not restore automatic receiver after presentation failure");
        }
    }
}

impl OfficialBrowser {
    fn watch_lease(&self, id: &str) -> Result<Arc<WatchLease>, StreamError> {
        self.inner
            .multiview
            .auto_watch
            .lease
            .lock()
            .map_err(|_| unavailable())?
            .as_ref()
            .filter(|lease| lease.id == id && lease.active.load(Ordering::Acquire))
            .cloned()
            .ok_or_else(stale)
    }
    fn take_watch(&self, expected: Option<&str>) -> Option<Arc<WatchLease>> {
        let mut slot = self.inner.multiview.auto_watch.lease.lock().ok()?;
        if expected.is_some_and(|id| slot.as_ref().is_none_or(|lease| lease.id != id)) {
            return None;
        }
        slot.take()
    }
    pub fn open_auto_watch(
        &self,
        app: &AppHandle,
        channel: &str,
        recording: &str,
    ) -> Result<WatchTarget, StreamError> {
        if channel.len() != 32
            || !channel.bytes().all(|byte| byte.is_ascii_hexdigit())
            || recording.is_empty()
            || recording.len() > 160
            || self.inner.closing.load(Ordering::Acquire)
        {
            return Err(ended());
        }
        let channel = channel.to_ascii_lowercase();
        let name = self
            .inner
            .auto_record
            .watch_channel_name(&channel, recording)
            .ok_or_else(ended)?;
        let capture = self
            .inner
            .contexts
            .hosts()
            .into_iter()
            .find(|host| recording_matches(host, &channel, recording))
            .ok_or_else(ended)?;
        let mut target = WatchTarget {
            kind: "automatic",
            watch_id: None,
            channel_id: channel,
            recording_id: recording.into(),
            channel_name: name,
            epoch: 0,
        };
        // These players already have a UI owner. Resume it, never navigate,
        // configure a replacement layout, or borrow an unrelated recording.
        if capture.label() == WINDOW_LABEL {
            target.kind = "live";
            return Ok(target);
        }
        if capture.label().starts_with("chzzk-mado-") {
            if !self.multiview_active() {
                return Err(ended());
            }
            target.kind = "mado";
            return Ok(target);
        }
        let _mutation = self
            .inner
            .multiview
            .auto_watch
            .mutations
            .try_lock()
            .map_err(|_| busy())?;
        let epoch = self
            .inner
            .multiview
            .auto_watch
            .lifecycle
            .fetch_add(1, Ordering::AcqRel)
            + 1;
        let pane = self
            .inner
            .multiview
            .auto_panes
            .lock()
            .map_err(|_| unavailable())?
            .get(capture.label())
            .cloned()
            .ok_or_else(ended)?;
        if pane.dead.load(Ordering::Acquire) {
            return Err(ended());
        }
        if let Some(previous) = self.take_watch(None) {
            let revision = previous.invalidate();
            if let Some(view) = app.get_webview(&previous.pane.id) {
                queue_hide(&view, previous.pane.revision.clone(), revision);
            }
            park(app, &previous, revision)?;
        }
        if !recording_matches(&capture, &target.channel_id, recording) {
            return Err(ended());
        }
        let lease = Arc::new(WatchLease {
            id: uuid::Uuid::new_v4().to_string(),
            recording_id: recording.into(),
            pane,
            epoch,
            active: AtomicBool::new(true),
            requested_audio: AtomicBool::new(true),
            generation: capture
                .inner
                .view
                .lock()
                .map_err(|_| unavailable())?
                .page_generation,
        });
        {
            let mut slot = self
                .inner
                .multiview
                .auto_watch
                .lease
                .lock()
                .map_err(|_| unavailable())?;
            if self
                .inner
                .multiview
                .auto_watch
                .lifecycle
                .load(Ordering::Acquire)
                != epoch
            {
                return Err(stale());
            }
            let mut viewport = lease.pane.viewport.lock().map_err(|_| unavailable())?;
            *viewport = BrowserViewport {
                epoch,
                ..BrowserViewport::default()
            };
            lease.pane.epoch.store(epoch, Ordering::Release);
            lease.pane.revision.fetch_add(1, Ordering::AcqRel);
            *slot = Some(lease.clone());
        }
        target.watch_id = Some(lease.id.clone());
        target.epoch = epoch;
        Ok(target)
    }
    pub fn auto_watch_snapshot(&self, id: &str) -> Result<WatchSnapshot, StreamError> {
        let lease = self.watch_lease(id)?;
        let recording = lease.recording();
        let capture = lease.pane.capture.as_ref().ok_or_else(ended)?;
        let state = capture.inner.view.lock().map_err(|_| unavailable())?;
        Ok(WatchSnapshot {
            recording,
            status: if recording {
                state.status.clone()
            } else {
                "ended".into()
            },
            error: if recording {
                state.error.clone()
            } else {
                Some(ended().message)
            },
            audio_enabled: lease.requested_audio.load(Ordering::Acquire),
            chat_count: state.chat_count,
        })
    }
    pub fn set_auto_watch_viewport(
        &self,
        app: &AppHandle,
        id: &str,
        mut viewport: BrowserViewport,
    ) -> Result<(), StreamError> {
        let lease = self.watch_lease(id)?;
        let validation = viewport.validate();
        let live = lease.connected();
        let revision = {
            let mut previous = lease.pane.viewport.lock().map_err(|_| unavailable())?;
            if !lease.active.load(Ordering::Acquire) || lease.pane.dead.load(Ordering::Acquire) {
                return Err(stale());
            }
            viewport.request_sequence =
                host_view::viewport_sequence(&viewport, &previous, lease.epoch)?;
            viewport.epoch = lease.epoch;
            viewport.visible =
                validation.is_ok() && live && viewport.visible && fits(PaneKind::Video, &viewport);
            if !viewport.visible {
                viewport.occluded = false;
                viewport.preserve_background = false;
                viewport.occlusions.clear();
            }
            *previous = viewport.clone();
            lease.pane.audio.store(
                validation.is_ok() && live && lease.requested_audio.load(Ordering::Acquire),
                Ordering::Release,
            );
            lease.pane.revision.fetch_add(1, Ordering::AcqRel) + 1
        };
        let Some(view) = app.get_webview(&lease.pane.id) else {
            return Err(ended());
        };
        if validation.is_ok() {
            chat_popup::sync_privacy(
                app,
                &lease.pane.channel,
                viewport.suspend_audio,
                lease.pane.revision.clone(),
                revision,
            );
        }
        if !viewport.visible {
            queue_hide_audio(
                &view,
                lease.pane.revision.clone(),
                revision,
                validation.is_err() || !live || viewport.suspend_audio,
            );
            if validation.is_err() || !live {
                park(app, &lease, revision)?;
                validation?;
                return Err(ended());
            }
        }
        // A hidden tab is still a viewer. Keep the same off-canvas receiver,
        // PiP, user volume and chat; only explicit close returns to record-only.
        let result =
            apply_auto_receiver_viewport(&view, &viewport, lease.pane.revision.clone(), revision);
        if !lease.active.load(Ordering::Acquire)
            || lease.pane.revision.load(Ordering::Acquire) != revision
        {
            return Err(stale());
        }
        if let Err(cause) = result {
            recover_failed_presentation(app, &lease, revision);
            return Err(cause);
        }
        // Update the native audio gate first. The shared player retains the
        // viewer's own mute choice; geometry updates must never unmute it.
        send_audio(&view, &lease.pane, false);
        let result = view
            .eval(presentation_script(revision, true))
            .map_err(|_| unavailable())
            .and_then(|_| {
                native_mute_guarded(
                    &view,
                    lease.pane.clone(),
                    lease.native_audio_muted(&viewport),
                    Some(revision),
                )
            });
        if let Err(cause) = result {
            recover_failed_presentation(app, &lease, revision);
            return Err(cause);
        }
        Ok(())
    }
    pub fn set_auto_watch_audio(
        &self,
        app: &AppHandle,
        id: &str,
        enabled: bool,
    ) -> Result<WatchSnapshot, StreamError> {
        let lease = self.watch_lease(id)?;
        if !lease.connected() {
            return Err(ended());
        }
        let (suspended, revision) = {
            let viewport = lease.pane.viewport.lock().map_err(|_| unavailable())?;
            if !lease.active.load(Ordering::Acquire) {
                return Err(stale());
            }
            lease.requested_audio.store(enabled, Ordering::Release);
            lease.pane.audio.store(enabled, Ordering::Release);
            (
                viewport.suspend_audio,
                lease.pane.revision.load(Ordering::Acquire),
            )
        };
        let view = app.get_webview(&lease.pane.id).ok_or_else(ended)?;
        if let Err(cause) = native_mute_guarded(
            &view,
            lease.pane.clone(),
            !enabled || suspended,
            Some(revision),
        ) {
            recover_failed_presentation(app, &lease, revision);
            return Err(cause);
        }
        send_audio(&view, &lease.pane, true);
        self.auto_watch_snapshot(id)
    }
    pub fn close_auto_watch(&self, app: &AppHandle, id: &str) -> Result<(), StreamError> {
        let Some(lease) = self.take_watch(Some(id)) else {
            return Ok(());
        };
        self.inner
            .multiview
            .auto_watch
            .lifecycle
            .fetch_add(1, Ordering::AcqRel);
        let revision = lease.invalidate();
        if let Some(view) = app.get_webview(&lease.pane.id) {
            queue_hide(&view, lease.pane.revision.clone(), revision);
        }
        park(app, &lease, revision)
    }
    /// Main-document reload can run on the UI thread. Invalidate/hide immediately;
    /// dispatch the off-canvas placement on a worker without waiting on this thread.
    pub fn detach_auto_watch(&self, app: &AppHandle) {
        self.inner
            .multiview
            .auto_watch
            .lifecycle
            .fetch_add(1, Ordering::AcqRel);
        let Some(lease) = self.take_watch(None) else {
            return;
        };
        let revision = lease.invalidate();
        if let Some(view) = app.get_webview(&lease.pane.id) {
            queue_hide(&view, lease.pane.revision.clone(), revision);
        }
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(cause) = park(&app, &lease, revision) {
                tracing::warn!(code = %cause.code, "could not park automatic recording presentation");
            }
        });
    }
    pub(super) fn forget_auto_watch_pane(&self, label: &str) {
        self.inner
            .multiview
            .retired_auto_panes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(label);
        let pane = self
            .inner
            .multiview
            .auto_panes
            .lock()
            .ok()
            .and_then(|mut panes| panes.remove(label));
        if let Some(pane) = pane {
            pane.dead.store(true, Ordering::Release);
            pane.audio.store(false, Ordering::Release);
            pane.revision.fetch_add(1, Ordering::AcqRel);
        }
        let mut slot = self
            .inner
            .multiview
            .auto_watch
            .lease
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|lease| lease.pane.id == label) {
            if let Some(lease) = slot.take() {
                lease.invalidate();
            }
        }
    }
}

#[tauri::command]
pub async fn chzzk_auto_watch_open(
    app: AppHandle,
    window: Webview,
    channel_id: String,
    recording_id: String,
) -> ApiResult<WatchTarget> {
    if let Err(cause) = require_main(&window) {
        return Err::<WatchTarget, _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.open_auto_watch(&app, &channel_id, &recording_id)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_auto_watch_snapshot(
    app: AppHandle,
    window: Webview,
    watch_id: String,
) -> ApiResult<WatchSnapshot> {
    (|| {
        require_main(&window)?;
        host(&app)?.auto_watch_snapshot(&watch_id)
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_auto_watch_viewport(
    app: AppHandle,
    window: Webview,
    watch_id: String,
    viewport: BrowserViewport,
) -> ApiResult<()> {
    if let Err(cause) = require_main(&window) {
        return Err::<(), _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.set_auto_watch_viewport(&app, &watch_id, viewport)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_auto_watch_audio(
    app: AppHandle,
    window: Webview,
    watch_id: String,
    enabled: bool,
) -> ApiResult<WatchSnapshot> {
    if let Err(cause) = require_main(&window) {
        return Err::<WatchSnapshot, _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.set_auto_watch_audio(&app, &watch_id, enabled)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_auto_watch_close(
    app: AppHandle,
    window: Webview,
    watch_id: String,
) -> ApiResult<()> {
    if let Err(cause) = require_main(&window) {
        return Err::<(), _>(cause).into();
    }
    tauri::async_runtime::spawn_blocking(move || host(&app)?.close_auto_watch(&app, &watch_id))
        .await
        .unwrap_or_else(|_| Err(unavailable()))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn lease(root: &OfficialBrowser, id: &str) -> Arc<WatchLease> {
        let capture = root
            .pane_controller(
                &format!("chzzk-auto-{}", uuid::Uuid::new_v4().simple()),
                CHANNEL,
            )
            .unwrap();
        {
            let mut state = capture.inner.view.lock().unwrap();
            state.recording = Some("original-recording".into());
            state.status = "recording".into();
            state.page_generation = 12;
            state.chat_count = 42;
        }
        let pane = Arc::new(Pane {
            id: capture.label().into(),
            channel: CHANNEL.into(),
            number: 0,
            kind: PaneKind::Video,
            epoch: AtomicU64::new(7),
            viewport: Mutex::new(BrowserViewport {
                epoch: 7,
                visible: true,
                width: 640.0,
                height: 360.0,
                ..BrowserViewport::default()
            }),
            revision: Arc::new(AtomicU64::new(9)),
            writes: Mutex::new(()),
            audio: AtomicBool::new(true),
            dead: AtomicBool::new(false),
            initial_blank: AtomicBool::new(false),
            status: Mutex::new("page_loaded".into()),
            channel_name: Mutex::new("방송".into()),
            capture: Some(capture),
        });
        root.inner
            .multiview
            .auto_panes
            .lock()
            .unwrap()
            .insert(pane.id.clone(), pane.clone());
        Arc::new(WatchLease {
            id: id.into(),
            recording_id: "original-recording".into(),
            pane,
            epoch: 7,
            active: AtomicBool::new(true),
            requested_audio: AtomicBool::new(true),
            generation: 12,
        })
    }
    #[test]
    fn background_tabs_and_pip_keep_audio_but_privacy_and_explicit_close_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let lease = lease(&root, "watch");
        let mut viewport = BrowserViewport::default();
        for visible in [true, false, true, false] {
            viewport.visible = visible;
            assert!(!lease.native_audio_muted(&viewport));
            assert!(lease.recording());
        }
        viewport.suspend_audio = true;
        assert!(lease.native_audio_muted(&viewport));
        assert!(lease.requested_audio.load(Ordering::Acquire));
        viewport.suspend_audio = false;
        assert!(!lease.native_audio_muted(&viewport));
        lease.requested_audio.store(false, Ordering::Release);
        assert!(lease.native_audio_muted(&viewport));
        viewport.visible = true;
        assert!(lease.native_audio_muted(&viewport));
        lease.requested_audio.store(true, Ordering::Release);
        lease.invalidate();
        assert!(lease.native_audio_muted(&viewport));
        assert!(lease.recording());
    }
    #[test]
    fn releasing_presentation_keeps_capture_identity_generation_and_exit_protection() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let lease = lease(&root, "watch");
        assert!(lease.recording());
        assert!(root.reserve_update().is_err());
        let revision = lease.invalidate();
        assert_eq!(revision, 10);
        assert!(!lease.active.load(Ordering::Acquire));
        assert!(!lease.pane.audio.load(Ordering::Acquire));
        assert!(!lease.pane.viewport.lock().unwrap().visible);
        let capture = lease.pane.capture.as_ref().unwrap();
        let state = capture.inner.view.lock().unwrap();
        assert_eq!(state.recording.as_deref(), Some("original-recording"));
        assert_eq!(state.page_generation, 12);
        assert_eq!(state.chat_count, 42);
        assert!(!capture.inner.detached.load(Ordering::Acquire));
        drop(state);
        assert!(lease.recording());
        assert_eq!(root.active_ids(), vec!["original-recording"]);
        assert!(root.ui_active_ids().is_empty());
    }
    #[test]
    fn stale_close_cannot_take_a_newer_watch_or_touch_its_recording() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let current = lease(&root, "current");
        *root.inner.multiview.auto_watch.lease.lock().unwrap() = Some(current.clone());
        assert!(root.take_watch(Some("old")).is_none());
        assert!(Arc::ptr_eq(&root.watch_lease("current").unwrap(), &current));
        assert!(root.watch_lease("old").is_err());
        assert!(root.auto_watch_snapshot("current").unwrap().recording);
        assert_eq!(root.auto_watch_snapshot("current").unwrap().chat_count, 42);
    }
    #[test]
    fn a_replaced_or_stopping_recording_is_not_adopted_by_the_old_viewer() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let current = lease(&root, "watch");
        *root.inner.multiview.auto_watch.lease.lock().unwrap() = Some(current.clone());
        let capture = current.pane.capture.as_ref().unwrap();
        capture.inner.view.lock().unwrap().status = "stopping".into();
        assert!(!root.auto_watch_snapshot("watch").unwrap().recording);
        {
            let mut state = capture.inner.view.lock().unwrap();
            state.status = "recording".into();
            state.recording = Some("new-manual-recording".into());
        }
        assert!(!root.auto_watch_snapshot("watch").unwrap().recording);
        current.invalidate();
        assert_eq!(
            capture.inner.view.lock().unwrap().recording.as_deref(),
            Some("new-manual-recording")
        );
    }
    #[test]
    fn parking_stays_outside_the_parent_and_old_geometry_cannot_reveal_a_new_lease() {
        let parked = parked_viewport(7);
        parked.validate().unwrap();
        assert!(parked.visible);
        assert!(parked.x + parked.width <= 0.0);
        assert_eq!(parked.width, 1280.0);
        assert_eq!(parked.height, 720.0);
        let old = BrowserViewport {
            epoch: 6,
            visible: true,
            width: 640.0,
            height: 360.0,
            request_sequence: Some(100),
            ..BrowserViewport::default()
        };
        let current = BrowserViewport {
            epoch: 7,
            request_sequence: Some(1),
            ..BrowserViewport::default()
        };
        assert!(host_view::viewport_sequence(&old, &current, 7).is_err());
    }
    #[test]
    fn scheduler_cleanup_invalidates_only_the_matching_watch() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let current = lease(&root, "watch");
        *root.inner.multiview.auto_watch.lease.lock().unwrap() = Some(current.clone());
        root.forget_auto_watch_pane("different-pane");
        assert!(root.watch_lease("watch").is_ok());
        root.forget_auto_watch_pane(&current.pane.id);
        assert!(root.watch_lease("watch").is_err());
        assert!(current.pane.dead.load(Ordering::Acquire));
        assert!(!current.active.load(Ordering::Acquire));
    }
    #[test]
    fn failed_geometry_can_park_itself_but_not_a_newer_display_or_its_capture() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let current = lease(&root, "watch");
        assert!(current.hide_failed_revision(8).is_none());
        assert!(current.pane.viewport.lock().unwrap().visible);
        assert_eq!(current.hide_failed_revision(9), Some(10));
        assert!(!current.pane.viewport.lock().unwrap().visible);
        assert!(!current.pane.audio.load(Ordering::Acquire));
        assert!(current.recording());
        assert!(current.active.load(Ordering::Acquire));
        assert!(current.hide_failed_revision(9).is_none());
    }
    #[test]
    fn common_live_snapshot_retains_view_after_stop_and_uses_only_its_own_commands() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let current = lease(&root, "watch");
        *root.inner.multiview.auto_watch.lease.lock().unwrap() = Some(current.clone());
        let capture = current.pane.capture.as_ref().unwrap();
        {
            let mut state = capture.inner.view.lock().unwrap();
            state.ready = true;
            state.recording = None;
            state.status = "ready".into();
        }
        assert!(current.connected());
        assert!(!current.recording());
        let snapshot = root.auto_watch_browser_snapshot("watch").unwrap();
        assert!(snapshot.window_open);
        assert_eq!(snapshot.viewport_epoch, 7);
        assert_eq!(snapshot.channel_id.as_deref(), Some(CHANNEL));
        assert!(snapshot.recording_id.is_none());
        capture.request_control(ControlAction::Screenshot).unwrap();
        let pending = root
            .auto_watch_browser_snapshot("watch")
            .unwrap()
            .pending_control
            .unwrap();
        assert_eq!(pending.action, ControlAction::Screenshot);
        assert!(root.snapshot().unwrap().pending_control.is_none());
        assert!(root.take_control(&pending.id, true, true).is_err());
        capture.take_control(&pending.id, false, false).unwrap();
        capture
            .view_intent(CHANNEL, CHANNEL, "open_settings")
            .unwrap();
        assert_eq!(
            root.auto_watch_browser_snapshot("watch")
                .unwrap()
                .pending_ui_action
                .unwrap()
                .action,
            "open_settings"
        );
        assert!(root.snapshot().unwrap().pending_ui_action.is_none());
        capture.inner.view.lock().unwrap().page_generation += 1;
        assert!(root.auto_watch_capture("watch").is_err());
        assert!(root.auto_watch_browser_snapshot("old-watch").is_err());
    }
    #[test]
    fn scheduler_retirement_keeps_watching_or_manual_capture_but_reaps_idle_detached_receivers() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let current = lease(&root, "watch");
        *root.inner.multiview.auto_watch.lease.lock().unwrap() = Some(current.clone());
        root.retire_auto_receiver(&current.pane.id);
        let capture = current.pane.capture.as_ref().unwrap();
        capture.inner.view.lock().unwrap().recording = None;
        assert!(root.auto_receiver_in_use(&current.pane)); // viewing after stop
        capture.inner.view.lock().unwrap().recording = Some("manually-started".into());
        current.invalidate();
        assert!(root.auto_receiver_in_use(&current.pane)); // record-only keeps new capture
        capture.inner.view.lock().unwrap().recording = None;
        assert!(!root.auto_receiver_in_use(&current.pane));
        let writing = capture.inner.screenshots.lock().unwrap();
        assert!(root.auto_receiver_in_use(&current.pane));
        drop(writing);
        root.claim_auto_receiver(&current.pane.id);
        assert!(root
            .inner
            .multiview
            .retired_auto_panes
            .lock()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn watch_operations_do_not_accept_remote_context_selection_or_unbounded_actions() {
        assert!(serde_json::from_value::<WatchOperation>(json!({"kind":"confirmControl","requestId":"id","approve":true,"rightsAcknowledged":true,"captureChat":true})).is_ok());
        for value in [
            json!({"kind":"snapshot","label":WINDOW_LABEL}),
            json!({"kind":"open","url":"https://example.com"}),
            json!({"kind":"requestControl","action":"delete"}),
        ] {
            assert!(serde_json::from_value::<WatchOperation>(value).is_err());
        }
    }
}
