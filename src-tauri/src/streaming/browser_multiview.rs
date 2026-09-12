//! Official panes with isolated capture controllers and a shared catalog.
//! Chat panes never receive the capture bridge.
use super::*;
use std::collections::HashSet;
use tauri::{LogicalPosition, LogicalSize, WebviewBuilder, WebviewUrl};
#[path = "browser_multiview_controls.rs"]
mod controls;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MultiViewEntry {
    pub channel_id: String,
    pub video: bool,
    pub chat: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PaneKind {
    Video,
    Chat,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiViewPaneSnapshot {
    pane_id: String,
    channel_id: String,
    kind: PaneKind,
    status: String,
    channel_name: String,
    audio_enabled: bool,
    ready: bool,
    recording_id: Option<String>,
    recording_status: String,
    chat_status: String,
    chat_count: u64,
    error: Option<String>,
    last_screenshot: Option<screenshot::SavedScreenshot>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneControl {
    pane_id: String,
    #[serde(flatten)]
    control: PendingControl,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneUiAction {
    pane_id: String,
    #[serde(flatten)]
    action: UiAction,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiViewSnapshot {
    active: bool,
    epoch: u64,
    audio_owner: Option<String>,
    panes: Vec<MultiViewPaneSnapshot>,
    pending_control: Option<PaneControl>,
    pending_ui_action: Option<PaneUiAction>,
}

#[derive(Default)]
pub(super) struct MultiViewHost {
    active: AtomicBool,
    lifecycle: AtomicU64,
    mutations: Mutex<()>,
    state: Mutex<MultiState>,
    metadata: Mutex<Option<std::sync::mpsc::SyncSender<(String, Vec<std::sync::Weak<Pane>>)>>>,
}

#[derive(Default)]
struct MultiState {
    epoch: u64,
    entries: Vec<MultiViewEntry>,
    panes: Vec<Arc<Pane>>,
    audio_owner: Option<String>,
}

struct Pane {
    id: String,
    channel: String,
    kind: PaneKind,
    epoch: AtomicU64,
    viewport: Mutex<BrowserViewport>,
    revision: Arc<AtomicU64>,
    writes: Mutex<()>,
    audio: AtomicBool,
    dead: AtomicBool,
    initial_blank: AtomicBool,
    status: Mutex<String>,
    channel_name: Mutex<String>,
    capture: Option<OfficialBrowser>,
}

impl MultiViewHost {
    fn invalidate(&self, expected_epoch: Option<u64>) -> Option<(u64, Vec<Arc<Pane>>)> {
        let state = self.state.lock().ok()?;
        if expected_epoch.is_some_and(|epoch| state.epoch != epoch) {
            return None;
        }
        self.lifecycle.fetch_add(1, Ordering::AcqRel);
        for pane in &state.panes {
            if let Some(capture) = &pane.capture {
                capture.inner.detached.store(true, Ordering::Release);
            }
            pane.dead.store(true, Ordering::Release);
            pane.audio.store(false, Ordering::Release);
            pane.revision.fetch_add(1, Ordering::AcqRel);
        }
        Some((state.epoch, state.panes.clone()))
    }
}

fn must_mute(
    kind: PaneKind,
    allowed: bool,
    dead: bool,
    cancelled: bool,
    requested_mute: bool,
) -> bool {
    requested_mute || cancelled || dead || !allowed || kind == PaneKind::Chat
}

fn busy() -> StreamError {
    error(
        "MULTIVIEW_BUSY",
        "마도 구성을 변경 중입니다. 잠시 후 다시 시도해 주세요.",
    )
}
fn invalid() -> StreamError {
    error(
        "MULTIVIEW_INVALID",
        "마도는 서로 다른 1~4개 채널과 채널별 영상 또는 채팅이 필요합니다.",
    )
}
fn validate_entries(entries: Vec<MultiViewEntry>) -> Result<Vec<MultiViewEntry>, StreamError> {
    if entries.is_empty() || entries.len() > 4 {
        return Err(invalid());
    }
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .map(|mut entry| {
            // Command contracts accept IDs only, never arbitrary remote URLs.
            if entry.channel_id.len() != 32
                || !entry.channel_id.bytes().all(|b| b.is_ascii_hexdigit())
                || (!entry.video && !entry.chat)
            {
                return Err(invalid());
            }
            entry.channel_id.make_ascii_lowercase();
            if !seen.insert(entry.channel_id.clone()) {
                return Err(invalid());
            }
            Ok(entry)
        })
        .collect()
}

fn pane_url(channel: &str, kind: PaneKind) -> String {
    // Official public bundle inspected 2026-09-11 (not just a third-party URL):
    // https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/index-C4sif-4p.js
    // Its player popup constructs `/live/${j}/chat` (studio alone adds a query).
    format!(
        "https://chzzk.naver.com/live/{channel}{}",
        if kind == PaneKind::Chat { "/chat" } else { "" }
    )
}
fn exact_navigation(url: &tauri::Url, channel: &str, kind: PaneKind) -> bool {
    url.as_str() == pane_url(channel, kind)
}
fn fits(kind: PaneKind, viewport: &BrowserViewport) -> bool {
    let (w, h) = match kind {
        PaneKind::Video => (160.0, 90.0),
        PaneKind::Chat => (220.0, 200.0),
    };
    viewport.width >= w
        && viewport.height >= h
        && (viewport.occluded
            || viewport
                .clip
                .as_ref()
                .is_none_or(|c| c.width >= 1.0 && c.height >= 1.0))
}
fn set_status(pane: &Pane, status: &str) {
    if let Ok(mut current) = pane.status.lock() {
        *current = status.into();
    }
}
fn send_audio(view: &Webview, pane: &Pane) {
    let enabled = pane.kind == PaneKind::Video
        && pane.audio.load(Ordering::Acquire)
        && !pane.dead.load(Ordering::Acquire);
    let _ = view.eval(&format!("window.dispatchEvent(new CustomEvent('atsumi-multiview-audio',{{detail:{{enabled:{enabled}}}}}));window.__atsumiPlayerUI?.configure({{multiview:true,audioEnabled:{enabled}}});"));
}

impl OfficialBrowser {
    pub fn multiview_active(&self) -> bool {
        self.inner.multiview.active.load(Ordering::Acquire)
    }

    pub fn multiview_snapshot(&self) -> Result<MultiViewSnapshot, StreamError> {
        let state = self
            .inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?;
        let mut pending_control = None;
        let mut pending_ui_action = None;
        let mut panes = Vec::with_capacity(state.panes.len());
        for pane in &state.panes {
            let capture = pane
                .capture
                .as_ref()
                .map(OfficialBrowser::snapshot)
                .transpose()?;
            if let Some(snapshot) = &capture {
                if let Some(control) = snapshot.pending_control.clone() {
                    pending_control = Some(PaneControl {
                        pane_id: pane.id.clone(),
                        control,
                    });
                }
                if let Some(action) = snapshot.pending_ui_action.clone() {
                    pending_ui_action = Some(PaneUiAction {
                        pane_id: pane.id.clone(),
                        action,
                    });
                }
            }
            panes.push(MultiViewPaneSnapshot {
                pane_id: pane.id.clone(),
                channel_id: pane.channel.clone(),
                kind: pane.kind,
                status: pane
                    .status
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_else(|_| "unavailable".into()),
                channel_name: pane
                    .channel_name
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default(),
                audio_enabled: pane.audio.load(Ordering::Acquire)
                    && !pane.dead.load(Ordering::Acquire),
                ready: capture.as_ref().is_some_and(|s| s.ready),
                recording_id: capture.as_ref().and_then(|s| s.recording_id.clone()),
                recording_status: capture
                    .as_ref()
                    .map(|s| s.status.clone())
                    .unwrap_or_else(|| "disabled".into()),
                chat_status: capture
                    .as_ref()
                    .map(|s| s.chat_status.clone())
                    .unwrap_or_else(|| "disabled".into()),
                chat_count: capture.as_ref().map_or(0, |s| s.chat_count),
                error: capture.as_ref().and_then(|s| s.error.clone()),
                last_screenshot: capture.and_then(|s| s.last_screenshot),
            });
        }
        Ok(MultiViewSnapshot {
            active: self.multiview_active(),
            epoch: state.epoch,
            audio_owner: state.audio_owner.clone(),
            panes,
            pending_control,
            pending_ui_action,
        })
    }

    pub fn configure_multiview(
        &self,
        app: &AppHandle,
        entries: Vec<MultiViewEntry>,
    ) -> Result<MultiViewSnapshot, StreamError> {
        let entries = validate_entries(entries)?;
        let _mutation = self
            .inner
            .multiview
            .mutations
            .try_lock()
            .map_err(|_| busy())?;
        let lifecycle = self.inner.multiview.lifecycle.load(Ordering::Acquire);
        let _configuration = {
            let _gate = self.inner.contexts.gate.lock().map_err(|_| busy())?;
            if !self.active_ids().is_empty() {
                return Err(error(
                    "RECORDING_ACTIVE",
                    "모든 방송의 녹화를 종료한 뒤 배치를 변경해 주세요.",
                ));
            }
            if self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0 {
                return Err(busy());
            }
            self.inner
                .contexts
                .reconfiguring
                .fetch_add(1, Ordering::AcqRel);
            contexts::Reconfigure(&self.inner.contexts.reconfiguring)
        };
        {
            let state = self
                .inner
                .multiview
                .state
                .lock()
                .map_err(|_| unavailable())?;
            if self.multiview_active()
                && state.entries == entries
                && !state.panes.is_empty()
                && state
                    .panes
                    .iter()
                    .all(|pane| !pane.dead.load(Ordering::Acquire))
            {
                drop(state);
                return self.multiview_snapshot();
            }
        }
        let loaded_ids = {
            let state = self.inner.view.lock().map_err(|_| unavailable())?;
            if self.inner.closing.load(Ordering::Acquire)
                || self.inner.reserved.load(Ordering::Acquire)
            {
                return Err(unavailable());
            }
            if state.recording.is_some() || state.arm.is_some() || state.page_recording {
                return Err(error(
                    "RECORDING_ACTIVE",
                    "공식 시청 녹화를 먼저 종료한 뒤 마도로 이동해 주세요.",
                ));
            }
            if state.account_busy || state.extension_connecting || self.account_window_open(app) {
                return Err(busy());
            }
            // Reserve while holding the single-view state lock. All ordinary
            // open/account/recording entry points consult this same guard.
            self.inner.multiview.active.store(true, Ordering::Release);
            state.loaded_extensions.clone()
        };
        if let Err(cause) = self.close_multiview_panes(app) {
            return Err(cause);
        }
        // No hidden fifth stream: close the existing single player BEFORE any
        // new official URL is navigated. Never stop/delete a recording here.
        self.detach_viewport(app);
        if let Some(view) = app.get_webview(WINDOW_LABEL) {
            if view.close().is_err() {
                self.inner.multiview.active.store(false, Ordering::Release);
                return Err(unavailable());
            }
        }
        {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            state.open = false;
            state.ready = false;
            state.status = "closed".into();
            state.extension_generation = state.extension_generation.wrapping_add(1);
        }
        let epoch = {
            let mut state = self
                .inner
                .multiview
                .state
                .lock()
                .map_err(|_| unavailable())?;
            state.epoch = state.epoch.wrapping_add(1).max(1);
            state.epoch
        };
        for entry in &entries {
            for kind in [PaneKind::Video, PaneKind::Chat] {
                if !(if kind == PaneKind::Video {
                    entry.video
                } else {
                    entry.chat
                }) {
                    continue;
                }
                let id = format!("chzzk-mado-{}", uuid::Uuid::new_v4().simple());
                let capture = if kind == PaneKind::Video {
                    Some(self.pane_controller(&id, &entry.channel_id)?)
                } else {
                    None
                };
                let pane = Arc::new(Pane {
                    id,
                    channel: entry.channel_id.clone(),
                    kind,
                    epoch: AtomicU64::new(epoch),
                    viewport: Mutex::new(BrowserViewport {
                        epoch,
                        ..Default::default()
                    }),
                    revision: Arc::new(AtomicU64::new(0)),
                    writes: Mutex::new(()),
                    audio: AtomicBool::new(false),
                    dead: AtomicBool::new(false),
                    initial_blank: AtomicBool::new(true),
                    status: Mutex::new("loading".into()),
                    channel_name: Mutex::new(String::new()),
                    capture,
                });
                self.inner
                    .multiview
                    .state
                    .lock()
                    .map_err(|_| unavailable())?
                    .panes
                    .push(pane.clone());
                if let Err(cause) =
                    self.create_multiview_pane(app, pane, loaded_ids.clone(), lifecycle)
                {
                    let closed = self.close_multiview_panes(app).is_ok();
                    self.inner
                        .multiview
                        .active
                        .store(!closed, Ordering::Release);
                    return Err(cause);
                }
            }
        }
        self.inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?
            .entries = entries;
        self.queue_channel_names();
        self.multiview_snapshot()
    }

    fn create_multiview_pane(
        &self,
        app: &AppHandle,
        pane: Arc<Pane>,
        loaded_ids: Vec<String>,
        lifecycle: u64,
    ) -> Result<(), StreamError> {
        if self.inner.multiview.lifecycle.load(Ordering::Acquire) != lifecycle {
            return Err(busy());
        }
        let parent = app.get_window("main").ok_or_else(unavailable)?;
        let profile = self.inner.data_dir.join("chzzk-browser-profile");
        std::fs::create_dir_all(&profile).map_err(|_| unavailable())?;
        let nav = pane.clone();
        let page = pane.clone();
        let builder = WebviewBuilder::new(
            &pane.id,
            WebviewUrl::External("about:blank".parse().map_err(|_| unavailable())?),
        )
        .data_directory(profile)
        .browser_extensions_enabled(true)
        .initialization_script(include_str!("browser_multiview.js"))
        .initialization_script(if pane.kind == PaneKind::Video {
            include_str!("browser_page_chat.js")
        } else {
            ""
        })
        .initialization_script(if pane.kind == PaneKind::Video {
            include_str!("browser_encoded_capture.js")
        } else {
            ""
        })
        .initialization_script(if pane.kind == PaneKind::Video {
            include_str!("browser_capture.js")
        } else {
            ""
        })
        .initialization_script(if pane.kind == PaneKind::Video {
            include_str!("browser_player_ui.js")
        } else {
            ""
        })
        .on_navigation(move |url| {
            let allowed = !nav.dead.load(Ordering::Acquire)
                && ((url.as_str() == "about:blank" && nav.initial_blank.load(Ordering::Acquire))
                    || exact_navigation(url, &nav.channel, nav.kind));
            if !allowed {
                set_status(&nav, "navigation_blocked_exit_mado_for_login");
            }
            allowed
        })
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_page_load(move |view, payload| {
            if !exact_navigation(payload.url(), &page.channel, page.kind)
                || page.dead.load(Ordering::Acquire)
            {
                return;
            }
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = view.eval(include_str!("browser_multiview.js"));
                if page.capture.is_some() {
                    let _ = view.eval(include_str!("browser_page_chat.js"));
                    let _ = view.eval(include_str!("browser_encoded_capture.js"));
                    let _ = view.eval(include_str!("browser_capture.js"));
                    let _ = view.eval(include_str!("browser_player_ui.js"));
                }
                send_audio(&view, &page);
                set_status(&page, "page_loaded");
            } else {
                if let Some(capture) = &page.capture {
                    let recording = {
                        let mut state =
                            capture.inner.view.lock().unwrap_or_else(|p| p.into_inner());
                        state.page_generation = state.page_generation.wrapping_add(1);
                        state.ready = false;
                        state.arm = None;
                        state.confirming_control = None;
                        state.recording.clone()
                    };
                    if let Some(id) = recording {
                        let capture = capture.clone();
                        thread::spawn(move || capture.interrupt_matching("page_hidden", Some(&id)));
                    }
                }
                set_status(&page, "loading");
            }
        });
        let view = parent
            .add_child(
                builder,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1.0, 1.0),
            )
            .map_err(|_| unavailable())?;
        view.hide().map_err(|_| unavailable())?;
        initialize_native_pane(&view, pane.clone(), loaded_ids)?;
        if let Some(capture) = &pane.capture {
            attach_native(&view, capture.clone())?;
        }
        if pane.dead.load(Ordering::Acquire)
            || self.inner.multiview.lifecycle.load(Ordering::Acquire) != lifecycle
        {
            return Err(busy());
        }
        pane.initial_blank.store(false, Ordering::Release);
        view.navigate(
            pane_url(&pane.channel, pane.kind)
                .parse()
                .map_err(|_| unavailable())?,
        )
        .map_err(|_| unavailable())
    }

    fn close_multiview_panes(&self, app: &AppHandle) -> Result<(), StreamError> {
        let panes = self
            .inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?
            .panes
            .clone();
        let mut failed = false;
        for pane in &panes {
            if let Some(capture) = &pane.capture {
                capture.detach_controller();
            }
            pane.dead.store(true, Ordering::Release);
            pane.audio.store(false, Ordering::Release);
            pane.revision.fetch_add(1, Ordering::AcqRel);
            if let Some(view) = app.get_webview(&pane.id) {
                let _ = native_mute(&view, pane.clone(), true);
                let _ = view.hide();
                if view.close().is_err() {
                    failed = true;
                    set_status(pane, "close_failed");
                }
            }
        }
        let mut state = self
            .inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?;
        state.audio_owner = None;
        state.entries.clear();
        if failed {
            return Err(unavailable());
        }
        state.panes.clear();
        Ok(())
    }

    /// UI-thread safe cancellation/hide only. Complete destruction on a worker
    /// using close_multiview(Some(returned_epoch)); never a delayed wildcard.
    pub fn detach_multiview(&self, app: &AppHandle) -> Option<u64> {
        self.detach_multiview_epoch(app, None)
    }

    fn detach_multiview_epoch(&self, app: &AppHandle, expected_epoch: Option<u64>) -> Option<u64> {
        let panes = self.inner.multiview.invalidate(expected_epoch)?;
        // Even a reloading main document must preempt an in-flight configure.
        // Its owner will observe lifecycle/dead and close partially made panes.
        for pane in panes.1 {
            if let Some(view) = app.get_webview(&pane.id) {
                queue_mute(&view);
                let _ = view.hide();
            }
        }
        Some(panes.0)
    }

    pub fn close_multiview(
        &self,
        app: &AppHandle,
        expected_epoch: Option<u64>,
    ) -> Result<(), StreamError> {
        let _configuration = {
            let _gate = self.inner.contexts.gate.lock().map_err(|_| busy())?;
            if !self.active_ids().is_empty() {
                return Err(error(
                    "RECORDING_ACTIVE",
                    "모든 방송의 녹화를 먼저 종료해 주세요.",
                ));
            }
            self.inner
                .contexts
                .reconfiguring
                .fetch_add(1, Ordering::AcqRel);
            contexts::Reconfigure(&self.inner.contexts.reconfiguring)
        };
        let Some(epoch) = self.detach_multiview_epoch(app, expected_epoch) else {
            return Ok(());
        };
        // Call only on a worker, including main-document reload. A close must
        // not abandon partially created panes because configure was moving.
        let _mutation = self.inner.multiview.mutations.lock().map_err(|_| busy())?;
        if self
            .inner
            .multiview
            .state
            .lock()
            .map(|s| s.epoch != epoch)
            .unwrap_or(true)
        {
            return Ok(());
        }
        self.close_multiview_panes(app)?;
        self.inner.multiview.active.store(false, Ordering::Release);
        Ok(())
    }

    pub fn set_multiview_viewport(
        &self,
        app: &AppHandle,
        pane_id: &str,
        mut viewport: BrowserViewport,
    ) -> Result<(), StreamError> {
        let validation = viewport.validate();
        if viewport.request_sequence.is_none() {
            return Err(error(
                "VIEWPORT_INVALID",
                "마도 영역 요청 순서가 필요합니다.",
            ));
        }
        let pane = self
            .inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .cloned()
            .ok_or_else(unavailable)?;
        if pane.dead.load(Ordering::Acquire) || viewport.epoch != pane.epoch.load(Ordering::Acquire)
        {
            return Err(error("VIEWPORT_STALE", "이전 마도 영역 요청입니다."));
        }
        let view = app.get_webview(&pane.id).ok_or_else(unavailable)?;
        let revision = {
            let mut previous = pane.viewport.lock().map_err(|_| unavailable())?;
            if pane.dead.load(Ordering::Acquire) {
                return Err(error("VIEWPORT_STALE", "종료된 마도 영역 요청입니다."));
            }
            host_view::viewport_sequence(&viewport, &previous, pane.epoch.load(Ordering::Acquire))?;
            if validation.is_err() {
                previous.visible = false;
                previous.occluded = false;
                previous.request_sequence = viewport.request_sequence;
            } else {
                if !fits(pane.kind, &viewport) {
                    viewport.visible = false;
                    viewport.occluded = false;
                }
                *previous = viewport.clone();
            }
            pane.revision.fetch_add(1, Ordering::AcqRel) + 1
        };
        // Detach can land between the first dead check and revision increment.
        // Never let this late request replace detach's fence with a showable one.
        if pane.dead.load(Ordering::Acquire) {
            pane.revision.fetch_add(1, Ordering::AcqRel);
            let _ = view.hide();
            return Err(error("VIEWPORT_STALE", "종료된 마도 영역 요청입니다."));
        }
        // Privacy hides bypass the visible transaction gate and invalidate an
        // already queued/moving native callback before it can show the child.
        if validation.is_err() || !viewport.visible {
            view.hide().map_err(|_| unavailable())?;
            return validation;
        }
        let result = if viewport.occluded {
            host_view::apply_pane_viewport(&view, &viewport, pane.revision.clone(), revision)
        } else {
            match pane.writes.try_lock() {
                Ok(_write) => host_view::apply_pane_viewport(
                    &view,
                    &viewport,
                    pane.revision.clone(),
                    revision,
                ),
                Err(_) => Err(busy()),
            }
        };
        if result.is_err() {
            if let Ok(mut current) = pane.viewport.lock() {
                if pane
                    .revision
                    .compare_exchange(revision, revision + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    current.visible = false;
                    current.occluded = false;
                    let _ = view.hide();
                }
            }
        }
        result
    }

    pub fn set_multiview_audio(
        &self,
        app: &AppHandle,
        channel_id: Option<String>,
        epoch: u64,
    ) -> Result<(), StreamError> {
        let _mutation = self
            .inner
            .multiview
            .mutations
            .try_lock()
            .map_err(|_| busy())?;
        let panes = {
            let state = self
                .inner
                .multiview
                .state
                .lock()
                .map_err(|_| unavailable())?;
            if state.epoch != epoch {
                return Err(error("MULTIVIEW_STALE", "이전 마도 음성 요청입니다."));
            }
            state.panes.clone()
        };
        if !self.multiview_active() {
            return Err(unavailable());
        }
        if channel_id.as_ref().is_some_and(|id| {
            !panes.iter().any(|pane| {
                pane.kind == PaneKind::Video
                    && &pane.channel == id
                    && !pane.dead.load(Ordering::Acquire)
            })
        }) {
            return Err(invalid());
        }
        // Mute/readback ALL panes before granting any next owner. Failure is
        // mute-all, never a partially acknowledged second audible player.
        self.inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?
            .audio_owner = None;
        let mut failed = false;
        for pane in &panes {
            pane.audio.store(false, Ordering::Release);
            if let Some(view) = app.get_webview(&pane.id) {
                if native_mute(&view, pane.clone(), true).is_err() {
                    failed = true;
                }
                send_audio(&view, pane);
            } else {
                failed = true;
            }
        }
        if failed {
            return Err(unavailable());
        }
        if let Some(id) = &channel_id {
            let pane = panes
                .iter()
                .find(|p| p.kind == PaneKind::Video && &p.channel == id)
                .ok_or_else(invalid)?;
            let view = app.get_webview(&pane.id).ok_or_else(unavailable)?;
            pane.audio.store(true, Ordering::Release);
            if let Err(cause) = native_mute(&view, pane.clone(), false) {
                pane.audio.store(false, Ordering::Release);
                let _ = native_mute(&view, pane.clone(), true);
                return Err(cause);
            }
            send_audio(&view, pane);
        }
        self.inner
            .multiview
            .state
            .lock()
            .map_err(|_| unavailable())?
            .audio_owner = channel_id;
        Ok(())
    }
}

#[cfg(windows)]
fn native_mute(view: &Webview, pane: Arc<Pane>, muted: bool) -> Result<(), StreamError> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows::core::{Interface, BOOL};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let current = cancelled.clone();
    view.with_webview(move |platform| unsafe {
        let result = (|| {
            let core = platform
                .controller()
                .CoreWebView2()
                .and_then(|c| c.cast::<ICoreWebView2_8>())
                .map_err(|_| unavailable())?;
            let selected = must_mute(
                pane.kind,
                pane.audio.load(Ordering::Acquire),
                pane.dead.load(Ordering::Acquire),
                current.load(Ordering::Acquire),
                muted,
            );
            core.SetIsMuted(selected).map_err(|_| unavailable())?;
            let mut actual = BOOL::default();
            core.IsMuted(&mut actual).map_err(|_| unavailable())?;
            if actual.as_bool() != selected {
                return Err(unavailable());
            }
            // A background close may invalidate permission while SetIsMuted
            // is executing. Recheck after readback and fail closed on that race.
            if !selected
                && must_mute(
                    pane.kind,
                    pane.audio.load(Ordering::Acquire),
                    pane.dead.load(Ordering::Acquire),
                    current.load(Ordering::Acquire),
                    muted,
                )
            {
                let _ = core.SetIsMuted(true);
                return Err(unavailable());
            }
            if !muted && selected {
                return Err(unavailable());
            }
            Ok(())
        })();
        let _ = tx.try_send(result);
    })
    .map_err(|_| unavailable())?;
    let result = rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| Err(unavailable()));
    if result.is_err() {
        cancelled.store(true, Ordering::Release);
    }
    result
}

#[cfg(windows)]
fn queue_mute(view: &Webview) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows::core::Interface;
    let _ = view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform
            .controller()
            .CoreWebView2()
            .and_then(|core| core.cast::<ICoreWebView2_8>())
        {
            let _ = core.SetIsMuted(true);
        }
    });
}

#[cfg(windows)]
fn initialize_native_pane(
    view: &Webview,
    pane: Arc<Pane>,
    loaded_ids: Vec<String>,
) -> Result<(), StreamError> {
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Settings2, ICoreWebView2_8, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
        },
        ProcessFailedEventHandler, WebResourceRequestedEventHandler,
    };
    use windows::{
        core::{Interface, BOOL, HSTRING, PWSTR},
        Win32::System::Com::IStream,
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let result = (|| {
            if pane.dead.load(Ordering::Acquire) {
                return Err(unavailable());
            }
            let core = platform
                .controller()
                .CoreWebView2()
                .map_err(|_| unavailable())?;
            let audio = core.cast::<ICoreWebView2_8>().map_err(|_| unavailable())?;
            audio.SetIsMuted(true).map_err(|_| unavailable())?;
            let mut actual = BOOL::default();
            audio.IsMuted(&mut actual).map_err(|_| unavailable())?;
            if !actual.as_bool() {
                return Err(unavailable());
            }
            if !loaded_ids.is_empty() {
                let settings = core
                    .Settings()
                    .and_then(|s| s.cast::<ICoreWebView2Settings2>())
                    .map_err(|_| unavailable())?;
                let mut raw = PWSTR::null();
                settings.UserAgent(&mut raw).map_err(|_| unavailable())?;
                let selected = super::super::browser_extension::aligned_user_agent(
                    &CoTaskMemPWSTR::from(raw).to_string(),
                    &loaded_ids,
                )?;
                settings
                    .SetUserAgent(&HSTRING::from(&selected))
                    .map_err(|_| unavailable())?;
                let mut raw = PWSTR::null();
                settings.UserAgent(&mut raw).map_err(|_| unavailable())?;
                if CoTaskMemPWSTR::from(raw).to_string() != selected {
                    return Err(unavailable());
                }
            }
            if pane.kind == PaneKind::Chat {
                core.AddWebResourceRequestedFilter(
                    &HSTRING::from("*"),
                    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
                )
                .map_err(|_| unavailable())?;
                let environment = platform.environment();
                let mut token = 0;
                core.add_WebResourceRequested(
                    &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                        if let Some(args) = args {
                            let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA;
                            args.ResourceContext(&mut context)?;
                            if context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA {
                                let response = environment.CreateWebResourceResponse(
                                    None::<&IStream>,
                                    403,
                                    &HSTRING::from("Watch-only chat"),
                                    &HSTRING::from("Content-Length: 0\r\nCache-Control: no-store"),
                                )?;
                                args.SetResponse(&response)?;
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(|_| unavailable())?;
            }
            let failed_pane = pane.clone();
            let failed_controller = platform.controller();
            let mut token = 0;
            core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                    failed_pane.dead.store(true, Ordering::Release);
                    failed_pane.audio.store(false, Ordering::Release);
                    failed_pane.revision.fetch_add(1, Ordering::AcqRel);
                    set_status(&failed_pane, "renderer_failed_reapply_configuration");
                    let _ = audio.SetIsMuted(true);
                    let _ = failed_controller.SetIsVisible(false);
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|_| unavailable())?;
            Ok(())
        })();
        // COM only in this callback; never re-enter Tauri or a host mutex.
        let _ = tx.try_send(result);
    })
    .map_err(|_| unavailable())?;
    rx.recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| Err(unavailable()))
}

#[cfg(not(windows))]
fn queue_mute(_: &Webview) {}
#[cfg(not(windows))]
fn native_mute(_: &Webview, _: Arc<Pane>, _: bool) -> Result<(), StreamError> {
    Err(unavailable())
}
#[cfg(not(windows))]
fn initialize_native_pane(_: &Webview, _: Arc<Pane>, _: Vec<String>) -> Result<(), StreamError> {
    Err(unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn entry(id: &str, video: bool, chat: bool) -> MultiViewEntry {
        MultiViewEntry {
            channel_id: id.into(),
            video,
            chat,
        }
    }
    fn fake_pane() -> Arc<Pane> {
        Arc::new(Pane {
            id: "synthetic-pane".into(),
            channel: A.into(),
            kind: PaneKind::Video,
            epoch: AtomicU64::new(9),
            viewport: Mutex::new(BrowserViewport::default()),
            revision: Arc::new(AtomicU64::new(10)),
            writes: Mutex::new(()),
            audio: AtomicBool::new(true),
            dead: AtomicBool::new(false),
            initial_blank: AtomicBool::new(false),
            status: Mutex::new("synthetic".into()),
            channel_name: Mutex::new(String::new()),
            capture: None,
        })
    }
    #[test]
    fn stale_close_cannot_invalidate_new_panes() {
        let host = MultiViewHost::default();
        let pane = fake_pane();
        {
            let mut state = host.state.lock().unwrap();
            state.epoch = 9;
            state.panes.push(pane.clone());
        }
        assert!(host.invalidate(Some(8)).is_none());
        assert_eq!(host.lifecycle.load(Ordering::Acquire), 0);
        assert_eq!(pane.revision.load(Ordering::Acquire), 10);
        assert!(!pane.dead.load(Ordering::Acquire));
        assert!(pane.audio.load(Ordering::Acquire));
    }
    #[test]
    fn detach_preempts_visible_revision_and_audio_without_waiting_for_mutation() {
        let host = MultiViewHost::default();
        let pane = fake_pane();
        {
            let mut state = host.state.lock().unwrap();
            state.epoch = 9;
            state.panes.push(pane.clone());
        }
        let _busy = host.mutations.lock().unwrap();
        assert_eq!(host.invalidate(Some(9)).unwrap().0, 9);
        assert_eq!(host.lifecycle.load(Ordering::Acquire), 1);
        assert_eq!(pane.revision.load(Ordering::Acquire), 11);
        assert!(pane.dead.load(Ordering::Acquire));
        assert!(!pane.audio.load(Ordering::Acquire));
    }
    #[test]
    fn initial_epoch_zero_also_cancels_a_pending_configuration() {
        let host = MultiViewHost::default();
        let before = host.lifecycle.load(Ordering::Acquire);
        assert_eq!(host.invalidate(None).unwrap().0, 0);
        assert_ne!(host.lifecycle.load(Ordering::Acquire), before);
    }
    #[test]
    fn audio_requires_a_live_owned_video_and_never_a_chat_pane() {
        assert!(!must_mute(PaneKind::Video, true, false, false, false));
        for kind in [PaneKind::Video, PaneKind::Chat] {
            assert!(must_mute(kind, false, false, false, false));
            assert!(must_mute(kind, true, true, false, false));
            assert!(must_mute(kind, true, false, true, false));
            assert!(must_mute(kind, true, false, false, true));
        }
        assert!(must_mute(PaneKind::Chat, true, false, false, false));
    }
    #[test]
    fn entries_are_bounded_unique_ids() {
        assert!(validate_entries(vec![]).is_err());
        assert!(validate_entries(vec![entry(A, true, true); 5]).is_err());
        assert!(validate_entries(vec![entry(A, false, false)]).is_err());
        assert!(validate_entries(vec![
            entry(A, true, false),
            entry(&A.to_uppercase(), false, true)
        ])
        .is_err());
        assert!(validate_entries(vec![entry(
            &format!("https://chzzk.naver.com/live/{A}"),
            true,
            true
        )])
        .is_err());
    }
    #[test]
    fn chat_only_entry_does_not_allocate_video() {
        let entries = validate_entries(vec![
            entry(A, false, true),
            entry("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", true, false),
        ])
        .unwrap();
        assert_eq!(entries.iter().filter(|e| e.video).count(), 1);
        assert_eq!(entries.iter().filter(|e| e.chat).count(), 1);
    }
    #[test]
    fn chat_navigation_has_no_full_live_or_account_fallback() {
        let allowed: tauri::Url = pane_url(A, PaneKind::Chat).parse().unwrap();
        assert!(exact_navigation(&allowed, A, PaneKind::Chat));
        for url in [
            pane_url(A, PaneKind::Video),
            format!("{allowed}?studio=true"),
            format!("{allowed}#x"),
            format!("{allowed}/"),
            "https://nid.naver.com/".into(),
            "https://evil.example/".into(),
        ] {
            assert!(!exact_navigation(&url.parse().unwrap(), A, PaneKind::Chat));
        }
    }
    #[test]
    fn undersized_chat_is_hidden_not_scaled_over_input() {
        let mut v = BrowserViewport {
            width: 219.0,
            height: 600.0,
            visible: true,
            ..Default::default()
        };
        assert!(!fits(PaneKind::Chat, &v));
        assert!(fits(PaneKind::Video, &v));
        v.width = 300.0;
        v.height = 199.0;
        assert!(!fits(PaneKind::Chat, &v));
        v.height = 200.0;
        assert!(fits(PaneKind::Chat, &v));
        v.clip = Some(BrowserClip {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 100.0,
        });
        assert!(!fits(PaneKind::Chat, &v));
        v.occluded = true;
        assert!(fits(PaneKind::Chat, &v));
        assert!(v.validate().is_ok());
        v.height = 199.0;
        assert!(!fits(PaneKind::Chat, &v));
    }
}
