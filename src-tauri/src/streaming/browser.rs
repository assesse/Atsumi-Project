//! Official-site viewing with a deliberately narrow, native recording bridge.
//! Remote pages get no Tauri command permissions, filesystem paths or account cookies.
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Webview};

#[path = "browser_contexts.rs"]
mod contexts;
#[path = "browser_encoded.rs"]
pub(crate) mod encoded;
#[path = "browser_host.rs"]
mod host_view;
#[path = "browser_multiview.rs"]
pub mod multiview;
#[path = "browser_multiview_commands.rs"]
pub mod multiview_commands;
#[path = "browser_screenshot.rs"]
mod screenshot;
pub use host_view::{BrowserClip, BrowserViewport, InstallerBrowser};

use super::{
    browser_store::{BrowserCaptureStore, BrowserRecording, BrowserRecordingStatus},
    chat_store::ChatStore,
    model::{now_ms, ChatEvent, ChatMessage, ChatReplayClock, StreamError},
    provider::{normalize_channel_input, ChzzkProvider},
};
use crate::interface::{ApiResult, AppState};

pub const WINDOW_LABEL: &str = "chzzk-official";
const MAX_MESSAGE: usize = 300_000;
const BRIDGE_PREFIX: &str = "ATSUMI_BROWSER_CAPTURE:";

fn error(code: &str, message: &str) -> StreamError {
    StreamError::new(code, message, true)
}
fn unavailable() -> StreamError {
    error(
        "BROWSER_UNAVAILABLE",
        "공식 시청 창에 연결하지 못했습니다. 창을 다시 열어 주세요.",
    )
}
fn control_stale() -> StreamError {
    error(
        "BROWSER_CONTROL_STALE",
        "요청이 만료됐거나 방송 화면이 변경됐습니다. 다시 요청해 주세요.",
    )
}

#[derive(Clone)]
pub struct OfficialBrowser {
    inner: Arc<Inner>,
}
struct Inner {
    data_dir: PathBuf,
    label: String,
    detached: AtomicBool,
    contexts: Arc<contexts::ContextGroup>,
    store: Arc<Mutex<BrowserCaptureStore>>,
    merges: super::browser_merge::BrowserMergeWorker,
    replay_assets: super::replay_assets::ReplayAssetCache,
    view: Mutex<ViewState>,
    viewport_writes: Mutex<()>,
    viewport_revision: Arc<AtomicU64>,
    multiview: multiview::MultiViewHost,
    screenshots: Mutex<screenshot::ScreenshotCapture>,
    encoded: Mutex<Option<encoded::EncodedSession>>,
    closing: Arc<AtomicBool>,
    reserved: Arc<AtomicBool>,
    chat: Mutex<Option<ChatWorker>>,
    retired_chat: Mutex<Vec<thread::JoinHandle<()>>>,
    page_chat: Mutex<Option<PageChatLog>>,
    writes: Mutex<()>,
}
struct PageChatLog {
    id: String,
    started_at: u64,
    broadcast_started_at: Option<u64>,
    sequence: u64,
    last_clock: Option<(f64, u64)>,
    log: ChatStore,
}
impl PageChatLog {
    fn observe_clock(
        &mut self,
        value: &Value,
        native_received: u64,
        encoded_source: Option<&str>,
    ) -> Option<ChatReplayClock> {
        let mut clock =
            serde_json::from_value::<ChatReplayClock>(value.get("replayClock")?.clone())
                .ok()?
                .bounded()?;
        if clock.received_at_ms < self.started_at.saturating_sub(60_000)
            || clock.received_at_ms > native_received.saturating_add(60_000)
            || self.last_clock.is_some_and(|(monotonic, generation)| {
                clock.observed_monotonic_ms < monotonic || clock.source_generation < generation
            })
        {
            return None;
        }
        self.last_clock = Some((clock.observed_monotonic_ms, clock.source_generation));
        // The page's claim alone cannot establish which native capture owns a
        // source. Never retain a source-time mapping for a different session.
        if clock.clock != "mse_presentation_v1"
            || clock.source_id.as_deref() != encoded_source
            || encoded_source.is_none()
        {
            clock.observation_only();
        }
        Some(clock)
    }
}
struct ChatWorker {
    cancel: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}
struct Arm {
    id: String,
    root: PathBuf,
    capture_chat: bool,
    created: Instant,
    generation: u64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    RecordStart,
    RecordStop,
    Screenshot,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiAction {
    id: String,
    action: String,
    expires_at: u64,
    #[serde(skip)]
    channel: String,
    #[serde(skip)]
    viewport_epoch: u64,
    #[serde(skip)]
    page_generation: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingControl {
    id: String,
    action: ControlAction,
    channel_id: String,
    expires_at: u64,
    #[serde(skip)]
    generation: u64,
    #[serde(skip)]
    created: Instant,
    #[serde(skip)]
    target_recording: Option<String>,
    #[serde(skip)]
    target_arm: Option<String>,
}
impl PendingControl {
    fn matches(&self, state: &ViewState) -> bool {
        self.created.elapsed() <= Duration::from_secs(20)
            && state.channel.as_deref() == Some(self.channel_id.as_str())
            && state.page_generation == self.generation
            && (self.action != ControlAction::RecordStop
                || (self.target_recording == state.recording
                    && self.target_arm.as_deref() == state.arm.as_ref().map(|a| a.id.as_str())))
    }
}
struct ViewState {
    open: bool,
    channel: Option<String>,
    ready: bool,
    page_recording: bool,
    status: String,
    error: Option<String>,
    recording: Option<String>,
    arm: Option<Arm>,
    accepted_arm: Option<(String, String, u64)>,
    extension: String,
    chat_status: String,
    chat_count: u64,
    capture_chat: bool,
    viewport: BrowserViewport,
    login_status: String,
    loaded_extensions: Vec<String>,
    extension_connecting: bool,
    extension_generation: u64,
    extension_reconnect_enabled: bool,
    video_width: u32,
    video_height: u32,
    video_paused: bool,
    page_generation: u64,
    account_busy: bool,
    viewport_epoch: u64,
    pending_control: Option<PendingControl>,
    confirming_control: Option<PendingControl>,
    last_control_intent: Option<Instant>,
    last_screenshot: Option<screenshot::SavedScreenshot>,
    pending_ui_action: Option<UiAction>,
    last_ui_intent: Option<Instant>,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            open: false,
            channel: None,
            ready: false,
            page_recording: false,
            status: "closed".into(),
            error: None,
            recording: None,
            arm: None,
            accepted_arm: None,
            extension: "not_connected".into(),
            chat_status: "disabled".into(),
            chat_count: 0,
            capture_chat: false,
            viewport: BrowserViewport::default(),
            login_status: "브라우저 세션 유지 · 로그인 여부는 공식 화면에서 확인".into(),
            loaded_extensions: Vec::new(),
            extension_connecting: false,
            extension_generation: 0,
            extension_reconnect_enabled: false,
            video_width: 0,
            video_height: 0,
            video_paused: true,
            page_generation: 0,
            account_busy: false,
            viewport_epoch: 0,
            pending_control: None,
            confirming_control: None,
            last_control_intent: None,
            last_screenshot: None,
            pending_ui_action: None,
            last_ui_intent: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    window_open: bool,
    channel_id: Option<String>,
    ready: bool,
    status: String,
    error: Option<String>,
    recording_id: Option<String>,
    recordings: Vec<BrowserRecording>,
    extension_status: String,
    extension_reconnect_enabled: bool,
    chat_status: String,
    chat_count: u64,
    login_status: String,
    video_width: u32,
    video_height: u32,
    video_paused: bool,
    viewport_epoch: u64,
    pending_control: Option<PendingControl>,
    last_screenshot: Option<screenshot::SavedScreenshot>,
    pending_ui_action: Option<UiAction>,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "atsumiBrowserCapture")]
    marker: u8,
    id: String,
    #[serde(flatten)]
    message: BrowserMessage,
}
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum BrowserMessage {
    ViewIntent {
        channel_id: String,
        action: String,
    },
    EncodedBegin {
        request_id: String,
        channel_id: String,
        title: String,
        source_id: String,
        tracks: Vec<encoded::EncodedTrack>,
    },
    EncodedAppend {
        recording_id: String,
        track_index: u32,
        append_index: u64,
        chunk_index: u64,
        final_chunk: bool,
        data: String,
    },
    EncodedFinish {
        recording_id: String,
        interrupted: bool,
        reason: Option<String>,
    },
    ControlIntent {
        channel_id: String,
        action: ControlAction,
    },
    ScreenshotBegin {
        request_id: String,
        channel_id: String,
        mime_type: String,
        size: usize,
        width: u32,
        height: u32,
    },
    ScreenshotChunk {
        request_id: String,
        chunk_index: u64,
        data: String,
    },
    ScreenshotFinish {
        request_id: String,
    },
    ScreenshotAbort {
        request_id: String,
    },
    Begin {
        request_id: String,
        channel_id: String,
        title: String,
        mime_type: String,
    },
    Chunk {
        recording_id: String,
        segment_index: u64,
        chunk_index: u64,
        data: String,
    },
    Segment {
        recording_id: String,
        segment_index: u64,
        duration_seconds: f64,
    },
    Finish {
        recording_id: String,
        interrupted: bool,
        reason: Option<String>,
    },
    Status {
        channel_id: String,
        ready: bool,
        #[serde(default)]
        recording: bool,
        #[serde(default)]
        detail: String,
        #[serde(default)]
        video_width: u32,
        #[serde(default)]
        video_height: u32,
        #[serde(default)]
        paused: bool,
    },
    ChatBatch {
        recording_id: String,
        events: Vec<Value>,
    },
    ChatStatus {
        recording_id: String,
        detail: String,
        #[serde(default)]
        dropped_messages: u64,
    },
}
impl BrowserMessage {
    fn screenshot(&self) -> bool {
        matches!(
            self,
            Self::ScreenshotBegin { .. }
                | Self::ScreenshotChunk { .. }
                | Self::ScreenshotFinish { .. }
                | Self::ScreenshotAbort { .. }
        )
    }
}

pub fn live_channel(url: &tauri::Url) -> Option<String> {
    if url.scheme() != "https"
        || url.host_str() != Some("chzzk.naver.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let path = url.path().strip_prefix("/live/")?;
    (path.len() == 32 && path.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| path.to_ascii_lowercase())
}
fn allowed_navigation(url: &tauri::Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host_str(), Some("chzzk.naver.com" | "nid.naver.com"))
}
fn parse_message(source: &str, body: &str) -> Option<(String, Envelope)> {
    if body.len() > MAX_MESSAGE {
        return None;
    }
    let channel = live_channel(&tauri::Url::parse(source).ok()?)?;
    let envelope: Envelope = serde_json::from_str(body).ok()?;
    (envelope.marker == 1 && uuid::Uuid::parse_str(&envelope.id).is_ok())
        .then_some((channel, envelope))
}
fn bridge_reason(reason: &str) -> &'static str {
    match reason {
        "seek" => "타임머신 또는 재생 위치 이동으로 녹화를 중단했습니다.",
        "rate_change" => "재생 배속이 변경되어 녹화를 중단했습니다.",
        "video_changed" | "channel_changed" => "방송 또는 영상 소스가 변경되어 녹화를 중단했습니다.",
        "page_hidden" | "window_closed" => "시청 창이 닫히거나 새로고침되어 녹화가 중단됐습니다.",
        "queue_overflow" => "녹화 저장이 수신 속도를 따라가지 못해 중단했습니다. 확정된 파일은 보존됩니다.",
        "no_audio" => "영상의 오디오 트랙을 캡처할 수 없습니다. 공식 플레이어에서 재생을 시작한 뒤 다시 시도하세요.",
        "renderer_failed" => "시청 브라우저가 종료되어 녹화가 중단됐습니다. 확정된 조각과 미완료 파일은 보존됩니다.",
        _ => "브라우저 녹화가 중단됐습니다. 확정된 파일은 보존되며 마지막 미완료 조각은 별도 표시됩니다.",
    }
}
fn chat_gap(status: &str) -> bool {
    matches!(
        status,
        "storage_failed"
            | "queue_overflow"
            | "observer_unavailable"
            | "unsupported_frame"
            | "frame_too_large"
            | "invalid_frame"
            | "message_too_large"
            | "message_truncated"
            | "decode_failed"
            | "observer_overflow"
            | "connection_gap"
            | "partial"
    )
}

impl OfficialBrowser {
    fn control_intent(
        &self,
        source: &str,
        channel: &str,
        action: ControlAction,
    ) -> Result<Value, StreamError> {
        let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
        if state
            .pending_control
            .as_ref()
            .is_some_and(|p| !p.matches(&state))
        {
            if let Some(pending) = state.pending_control.take() {
                self.inner
                    .contexts
                    .release_pending(self.label(), &pending.id);
            }
        }
        if source != channel
            || self.inner.detached.load(Ordering::Acquire)
            || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
            || state.channel.as_deref() != Some(source)
            || self.inner.closing.load(Ordering::Acquire)
            || self.inner.reserved.load(Ordering::Acquire)
            || state.account_busy
            || state.extension_connecting
            || self.multiview_active()
        {
            return Err(unavailable());
        }
        if state.pending_control.is_some()
            || state.confirming_control.is_some()
            || state
                .last_control_intent
                .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
        {
            return Err(error(
                "BROWSER_CONTROL_BUSY",
                "이미 확인 중인 요청이 있습니다. 잠시 후 다시 요청해 주세요.",
            ));
        }
        if action == ControlAction::Screenshot
            && self
                .inner
                .screenshots
                .try_lock()
                .map_or(true, |mut capture| capture.active())
        {
            return Err(error(
                "SCREENSHOT_BUSY",
                "현재 스크린샷 저장이 끝난 뒤 다시 요청해 주세요.",
            ));
        }
        let active = state.recording.is_some() || state.arm.is_some();
        if (action == ControlAction::RecordStart
            && (active || !state.ready || state.page_recording))
            || (action == ControlAction::RecordStop && !active)
            || (action == ControlAction::Screenshot && !state.ready)
        {
            return Err(error(
                "BROWSER_NOT_READY",
                "현재 재생·녹화 상태에서는 이 작업을 요청할 수 없습니다.",
            ));
        }
        state.last_control_intent = Some(Instant::now());
        state.pending_control = Some(PendingControl {
            id: self.inner.contexts.reserve_pending(self.label())?,
            action,
            channel_id: channel.into(),
            expires_at: now_ms().saturating_add(20_000),
            generation: state.page_generation,
            created: Instant::now(),
            target_recording: state.recording.clone(),
            target_arm: state.arm.as_ref().map(|a| a.id.clone()),
        });
        // Remote code receives neither the main confirmation id nor an approval nonce.
        Ok(json!({"pending":true}))
    }
    fn take_control(
        &self,
        id: &str,
        approve: bool,
        rights: bool,
    ) -> Result<Option<PendingControl>, StreamError> {
        let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
        let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
        if !state.pending_control.as_ref().is_some_and(|p| p.id == id) {
            return Err(control_stale());
        }
        let pending = state.pending_control.take().ok_or_else(control_stale)?;
        self.inner
            .contexts
            .release_pending(self.label(), &pending.id);
        if !pending.matches(&state) {
            return Err(control_stale());
        }
        if !approve {
            return Ok(None);
        }
        if pending.action != ControlAction::RecordStop && !rights {
            return Err(error(
                "BROWSER_RIGHTS_REQUIRED",
                "시청·저장 권한 확인란을 선택해 주세요.",
            ));
        }
        if self.inner.closing.load(Ordering::Acquire)
            || self.inner.detached.load(Ordering::Acquire)
            || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
            || self.inner.reserved.load(Ordering::Acquire)
            || state.account_busy
            || state.extension_connecting
            || self.multiview_active()
        {
            return Err(unavailable());
        }
        if pending.action == ControlAction::RecordStart {
            state.confirming_control = Some(pending.clone());
        }
        Ok(Some(pending))
    }
    pub fn confirm_control(
        &self,
        app: &AppHandle,
        id: &str,
        approve: bool,
        rights: bool,
        capture_chat: bool,
    ) -> Result<BrowserSnapshot, StreamError> {
        let Some(pending) = self.take_control(id, approve, rights)? else {
            return self.snapshot();
        };
        let result = match pending.action {
            ControlAction::RecordStart => {
                let result = app.state::<AppState>().start_browser_managed_for(
                    app,
                    Some(self.label()),
                    rights,
                    capture_chat,
                );
                let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                if state
                    .confirming_control
                    .as_ref()
                    .is_some_and(|p| p.id == pending.id)
                {
                    state.confirming_control = None;
                }
                result
            }
            ControlAction::RecordStop => {
                let view = app.get_webview(self.label()).ok_or_else(unavailable)?;
                self.stop_matching(&view, Some(&pending))
            }
            ControlAction::Screenshot => {
                let root = app
                    .state::<AppState>()
                    .settings_snapshot()
                    .map_err(|_| unavailable())?
                    .download_root;
                let view = app.get_webview(self.label()).ok_or_else(unavailable)?;
                let current = view.url().ok().as_ref().and_then(live_channel);
                if current.as_deref() != Some(pending.channel_id.as_str())
                    || self.account_window_open(app)
                {
                    return Err(control_stale());
                }
                let mut capture = self.inner.screenshots.lock().map_err(|_| unavailable())?;
                {
                    let state = self.inner.view.lock().map_err(|_| unavailable())?;
                    if !pending.matches(&state)
                        || !state.ready
                        || state.account_busy
                        || self.inner.closing.load(Ordering::Acquire)
                        || self.inner.reserved.load(Ordering::Acquire)
                        || self.multiview_active()
                    {
                        return Err(control_stale());
                    }
                }
                let nonce =
                    capture.arm(Path::new(&root), &pending.channel_id, pending.generation)?;
                let result = send_command(
                    &view,
                    json!({"kind":"screenshot","requestId":nonce,"channelId":pending.channel_id}),
                );
                if result.is_err() {
                    capture.abort(&nonce);
                }
                result
            }
        };
        result?;
        self.snapshot()
    }
    fn process_screenshot(
        &self,
        channel: &str,
        message: BrowserMessage,
    ) -> Result<Value, StreamError> {
        let generation = {
            let state = self.inner.view.lock().map_err(|_| unavailable())?;
            if state.channel.as_deref() != Some(channel)
                || self.inner.closing.load(Ordering::Acquire)
                || self.inner.reserved.load(Ordering::Acquire)
                || state.account_busy
                || self.multiview_active()
            {
                return Err(control_stale());
            }
            state.page_generation
        };
        let mut capture = self.inner.screenshots.lock().map_err(|_| unavailable())?;
        match message {
            BrowserMessage::ScreenshotBegin {
                request_id,
                channel_id,
                mime_type,
                size,
                width,
                height,
            } => {
                if channel_id != channel {
                    return Err(control_stale());
                }
                capture.begin(
                    &request_id,
                    channel,
                    generation,
                    &mime_type,
                    size,
                    width,
                    height,
                )?;
                Ok(json!({"accepted":true}))
            }
            BrowserMessage::ScreenshotChunk {
                request_id,
                chunk_index,
                data,
            } => {
                if data.len() > ((screenshot::CHUNK + 2) / 3) * 4 {
                    return Err(error("SCREENSHOT_INVALID", "스크린샷 조각이 너무 큽니다."));
                }
                let bytes = STANDARD.decode(data).map_err(|_| {
                    error("SCREENSHOT_INVALID", "스크린샷 데이터가 올바르지 않습니다.")
                })?;
                capture.append(&request_id, channel, generation, chunk_index, &bytes)?;
                Ok(json!({"saved":true,"chunkIndex":chunk_index}))
            }
            BrowserMessage::ScreenshotFinish { request_id } => {
                let _decode = self
                    .inner
                    .contexts
                    .screenshot_decode
                    .try_lock()
                    .map_err(|_| {
                        error(
                            "SCREENSHOT_BUSY",
                            "다른 화면을 저장 중입니다. 잠시 후 다시 시도해 주세요.",
                        )
                    })?;
                let saved = capture.finish_checked(&request_id, channel, generation, || {
                    self.inner.view.lock().is_ok_and(|state| {
                        state.channel.as_deref() == Some(channel)
                            && state.page_generation == generation
                            && !state.account_busy
                    }) && !self.inner.closing.load(Ordering::Acquire)
                        && !self.inner.reserved.load(Ordering::Acquire)
                        && !self.multiview_active()
                })?;
                self.inner
                    .view
                    .lock()
                    .map_err(|_| unavailable())?
                    .last_screenshot = Some(saved.clone());
                Ok(json!({"saved":true,"fileName":saved.file_name}))
            }
            BrowserMessage::ScreenshotAbort { request_id } => {
                capture.abort(&request_id);
                Ok(json!({"aborted":true}))
            }
            _ => Err(unavailable()),
        }
    }
    pub fn new(data_dir: PathBuf) -> Result<Self, StreamError> {
        Self::new_with_media_tools(data_dir, None)
    }
    pub fn new_with_media_tools(
        data_dir: PathBuf,
        tools: Option<super::browser_merge::MediaTools>,
    ) -> Result<Self, StreamError> {
        let store = Arc::new(Mutex::new(BrowserCaptureStore::new(&data_dir)?));
        let replay_assets = super::replay_assets::ReplayAssetCache::new(&data_dir, tools.is_some())
            .unwrap_or_else(|_| super::replay_assets::ReplayAssetCache::disabled());
        let merges = super::browser_merge::BrowserMergeWorker::start(store.clone(), tools)?;
        let mut view = ViewState::default();
        match super::browser_extension::reconnect_choice(&data_dir) {
            Ok(enabled) => view.extension_reconnect_enabled = enabled,
            Err(_) => {
                view.extension =
                    "자동 연결 선택을 읽지 못했습니다 · 네이버 확장 연결을 다시 눌러 주세요".into()
            }
        }
        let host = Self {
            inner: Arc::new(Inner {
                data_dir,
                label: WINDOW_LABEL.into(),
                detached: AtomicBool::new(false),
                contexts: Arc::new(contexts::ContextGroup::default()),
                store,
                merges,
                replay_assets,
                view: Mutex::new(view),
                viewport_writes: Mutex::new(()),
                viewport_revision: Arc::new(AtomicU64::new(0)),
                multiview: multiview::MultiViewHost::default(),
                screenshots: Mutex::new(screenshot::ScreenshotCapture::default()),
                encoded: Mutex::new(None),
                closing: Arc::new(AtomicBool::new(false)),
                reserved: Arc::new(AtomicBool::new(false)),
                chat: Mutex::new(None),
                retired_chat: Mutex::new(Vec::new()),
                page_chat: Mutex::new(None),
                writes: Mutex::new(()),
            }),
        };
        host.inner.contexts.register(&host.inner)?;
        Ok(host)
    }
    pub(crate) fn capture_store(&self) -> Arc<Mutex<BrowserCaptureStore>> {
        self.inner.store.clone()
    }
    pub fn snapshot(&self) -> Result<BrowserSnapshot, StreamError> {
        // An abandoned renderer cannot retain its 16 MiB reservation forever;
        // polling reaps it without waiting on a PNG decode/write in progress.
        if let Ok(mut capture) = self.inner.screenshots.try_lock() {
            capture.expire();
        }
        let recordings = if self.label() == WINDOW_LABEL {
            self.inner
                .store
                .lock()
                .map_err(|_| unavailable())?
                .snapshot()?
        } else {
            Vec::new()
        };
        let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
        if state.pending_ui_action.as_ref().is_some_and(|intent| {
            intent.expires_at <= now_ms()
                || state.channel.as_deref() != Some(intent.channel.as_str())
                || state.viewport_epoch != intent.viewport_epoch
                || state.page_generation != intent.page_generation
        }) {
            state.pending_ui_action = None;
        }
        if state
            .pending_control
            .as_ref()
            .is_some_and(|pending| !pending.matches(&state))
        {
            if let Some(pending) = state.pending_control.take() {
                self.inner
                    .contexts
                    .release_pending(self.label(), &pending.id);
            }
        }
        if state
            .arm
            .as_ref()
            .is_some_and(|arm| arm.created.elapsed() > Duration::from_secs(20))
        {
            state.arm = None;
            state.status = "error".into();
            state.error = Some(
                "녹화 시작 응답이 없습니다. 공식 플레이어의 재생 상태를 확인해 주세요.".into(),
            );
        }
        Ok(BrowserSnapshot {
            window_open: state.open,
            channel_id: state.channel.clone(),
            ready: state.ready,
            status: state.status.clone(),
            error: state.error.clone(),
            recording_id: state.recording.clone(),
            recordings,
            extension_status: state.extension.clone(),
            extension_reconnect_enabled: state.extension_reconnect_enabled,
            chat_status: state.chat_status.clone(),
            chat_count: state.chat_count,
            login_status: state.login_status.clone(),
            video_width: state.video_width,
            video_height: state.video_height,
            video_paused: state.video_paused,
            viewport_epoch: state.viewport_epoch,
            pending_control: state.pending_control.clone(),
            last_screenshot: state.last_screenshot.clone(),
            pending_ui_action: state.pending_ui_action.clone(),
        })
    }
    pub fn active_ids(&self) -> Vec<String> {
        self.inner
            .contexts
            .hosts()
            .iter()
            .flat_map(Self::local_active_ids)
            .collect()
    }
    pub fn reserve_update(&self) -> Result<(), StreamError> {
        let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
        if !self.active_ids().is_empty() {
            return Err(error(
                "RECORDING_ACTIVE",
                "공식 시청 녹화를 중지하고 파일 마무리를 기다린 뒤 업데이트해 주세요.",
            ));
        }
        self.inner.reserved.store(true, Ordering::Release);
        Ok(())
    }
    pub fn release_update(&self) {
        self.inner.reserved.store(false, Ordering::Release);
    }
    pub fn arm(
        &self,
        app: &AppHandle,
        root: PathBuf,
        rights: bool,
        capture_chat: bool,
    ) -> Result<(), StreamError> {
        if !rights {
            return Err(error(
                "BROWSER_RIGHTS_REQUIRED",
                "시청·저장 권한 확인란을 선택해 주세요.",
            ));
        }
        if !root.is_absolute() {
            return Err(error(
                "DOWNLOAD_ROOT_REQUIRED",
                "설정에서 다운로드 폴더를 지정해 주세요.",
            ));
        }
        let window = app.get_webview(self.label()).ok_or_else(unavailable)?;
        // Query the dispatcher before locking state: navigation callbacks also
        // acquire this mutex and must never deadlock with a synchronous URL query.
        let page_channel = window.url().ok().as_ref().and_then(live_channel);
        let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
        if self.inner.detached.load(Ordering::Acquire)
            || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
            || self.primary_profile_busy()
            || (self.active_ids().len() >= 4 && self.local_active_ids().is_empty())
        {
            return Err(unavailable());
        }
        if app
            .webview_windows()
            .keys()
            .any(|label| label.starts_with("chzzk-login"))
        {
            return Err(error(
                "BROWSER_ACCOUNT_BUSY",
                "로그인·계정 창을 닫은 뒤 녹화를 시작해 주세요.",
            ));
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        let channel = {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if self.inner.closing.load(Ordering::Acquire)
                || self.inner.reserved.load(Ordering::Acquire)
                || state.account_busy
                || self.account_window_open(app)
                || self.multiview_active()
            {
                return Err(error(
                    "BROWSER_RESERVED",
                    "앱 종료 또는 업데이트 준비 중에는 녹화를 시작할 수 없습니다.",
                ));
            }
            if state.extension_connecting {
                return Err(error(
                    "BROWSER_EXTENSION_CONNECTING",
                    "네이버 확장 연결이 끝난 뒤 녹화를 시작해 주세요.",
                ));
            }
            if state.recording.is_some() || state.arm.is_some() {
                return Err(error(
                    "BROWSER_RECORDING_ACTIVE",
                    "이미 녹화를 시작했거나 준비 중입니다.",
                ));
            }
            let channel = state
                .channel
                .clone()
                .filter(|_| state.ready && !state.page_recording)
                .ok_or_else(|| {
                    error(
                        "BROWSER_NOT_READY",
                        "공식 시청 창에서 방송을 재생하고 녹화 마무리를 기다린 뒤 다시 시도하세요.",
                    )
                })?;
            if page_channel.as_ref() != Some(&channel) {
                return Err(unavailable());
            }
            // A confirmation is tied to the exact document/channel, not just a
            // still-open player. Keep this check inside the existing arm gate.
            if let Some(approved) = state.confirming_control.as_ref() {
                if approved.action != ControlAction::RecordStart || !approved.matches(&state) {
                    return Err(control_stale());
                }
            }
            state.arm = Some(Arm {
                id: nonce.clone(),
                root,
                capture_chat,
                created: Instant::now(),
                generation: state.page_generation,
            });
            state.status = "starting".into();
            state.error = None;
            channel
        };
        if send_command(
            &window,
            json!({"kind":"start","requestId":nonce,"channelId":channel,"rightsAcknowledged":true}),
        )
        .is_err()
        {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            state.arm = None;
            state.status = "error".into();
            state.error = Some("공식 시청 창에 시작 요청을 보내지 못했습니다.".into());
            return Err(unavailable());
        }
        Ok(())
    }
    pub fn stop(&self, window: &Webview) -> Result<(), StreamError> {
        self.stop_matching(window, None)
    }
    fn stop_matching(
        &self,
        window: &Webview,
        approved: Option<&PendingControl>,
    ) -> Result<(), StreamError> {
        let channel = {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if approved.is_some_and(|p| !p.matches(&state)) {
                return Err(control_stale());
            }
            if state.recording.is_none() && state.arm.is_none() {
                return Ok(());
            }
            state.arm = None;
            state.status = if state.recording.is_some() {
                "stopping"
            } else {
                "ready"
            }
            .into();
            state.channel.clone()
        };
        send_command(window, json!({"kind":"stop","channelId":channel}))
    }
    fn wait_stopped(&self, timeout: Duration) {
        let started = Instant::now();
        while started.elapsed() < timeout && !self.active_ids().is_empty() {
            thread::sleep(Duration::from_millis(100));
        }
    }
    pub fn shutdown_and_wait(&self, app: &AppHandle) {
        self.inner.closing.store(true, Ordering::Release);
        self.inner.merges.shutdown_and_wait();
        self.inner.replay_assets.shutdown_and_wait();
        let hosts = self.inner.contexts.hosts();
        // Request all recorders to flush before waiting; never spend 8 seconds
        // per pane while later panes keep recording unnoticed.
        for host in &hosts {
            host.inner
                .screenshots
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .cancel();
            if let Some(pending) = host
                .inner
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pending_control
                .take()
            {
                host.inner
                    .contexts
                    .release_pending(host.label(), &pending.id);
            }
            if let Some(view) = app.get_webview(host.label()) {
                let _ = host.stop(&view);
            }
        }
        self.wait_stopped(Duration::from_secs(8));
        for host in &hosts {
            host.interrupt("window_closed");
        }
        let _ = self
            .inner
            .store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutdown();
        for host in hosts {
            host.stop_chat();
            let handles = std::mem::take(
                &mut *host
                    .inner
                    .retired_chat
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()),
            );
            for handle in handles {
                let _ = handle.join();
            }
        }
    }
    fn interrupt(&self, reason: &str) {
        self.interrupt_matching(reason, None);
    }
    fn interrupt_matching(&self, reason: &str, expected: Option<&str>) {
        let _write = self.inner.writes.lock().unwrap_or_else(|p| p.into_inner());
        let recording = {
            let mut state = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            if expected.is_some() && state.recording.as_deref() != expected {
                return;
            }
            state.arm = None;
            state.recording.clone()
        };
        self.inner
            .encoded
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(id) = recording {
            let message = bridge_reason(reason);
            self.stop_chat();
            let (capture_chat, chat_status, chat_count) = {
                let mut state = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
                if state.capture_chat && !chat_gap(&state.chat_status) {
                    state.chat_status = "partial".into();
                }
                (
                    state.capture_chat,
                    if state.capture_chat {
                        state.chat_status.clone()
                    } else {
                        "disabled".into()
                    },
                    if state.capture_chat {
                        state.chat_count
                    } else {
                        0
                    },
                )
            };
            let metadata = self
                .inner
                .store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .finish_with_chat(
                    &id,
                    true,
                    Some(message),
                    capture_chat,
                    &chat_status,
                    chat_count,
                );
            self.inner.merges.wake();
            let mut state = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            state.recording = None;
            state.ready = false;
            state.error = Some(message.into());
            if metadata.is_err() {
                state.chat_status = "storage_failed".into();
                state.error=Some("녹화가 중단됐고 채팅 저장 상태를 기록하지 못했습니다. 기존 영상 파일은 보존됩니다.".into());
            }
            state.status = "error".into();
            drop(state);
            self.stop_chat();
        }
    }
    fn process(&self, channel: &str, message: BrowserMessage) -> Result<Value, StreamError> {
        if self.inner.detached.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        // Pane channels are native-created and immutable. A same-origin frame
        // cannot retarget a controller by sending a different live URL/status.
        if self.label() != WINDOW_LABEL
            && self
                .inner
                .view
                .lock()
                .map_err(|_| unavailable())?
                .channel
                .as_deref()
                != Some(channel)
        {
            return Err(unavailable());
        }
        if let BrowserMessage::ViewIntent { channel_id, action } = message {
            return self.view_intent(channel, &channel_id, &action);
        }
        if matches!(
            &message,
            BrowserMessage::EncodedBegin { .. }
                | BrowserMessage::EncodedAppend { .. }
                | BrowserMessage::EncodedFinish { .. }
        ) {
            return self.process_encoded(channel, message);
        }
        if let BrowserMessage::ControlIntent { channel_id, action } = message {
            return self.control_intent(channel, &channel_id, action);
        }
        if message.screenshot() {
            return self.process_screenshot(channel, message);
        }
        if let BrowserMessage::Status {
            channel_id,
            ready,
            recording,
            detail,
            video_width,
            video_height,
            paused,
        } = message
        {
            if channel_id != channel {
                return Err(unavailable());
            }
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            // A lost begin/finish ACK can leave native state active after the
            // renderer has already abandoned its recorder. Don't leave an orphan
            // session holding exit/update reservations indefinitely.
            if !recording
                && matches!(
                    detail.as_str(),
                    "native_rejected"
                        | "recorder_error"
                        | "empty_segment"
                        | "queue_overflow"
                        | "no_audio"
                )
            {
                if let Some(id) = state.recording.clone() {
                    drop(state);
                    self.interrupt_matching(&detail, Some(&id));
                    state = self.inner.view.lock().map_err(|_| unavailable())?;
                }
            }
            if state.channel.as_deref() != Some(channel)
                && (state.recording.is_some() || state.arm.is_some())
            {
                drop(state);
                self.interrupt("channel_changed");
                state = self.inner.view.lock().map_err(|_| unavailable())?;
            }
            state.channel = Some(channel.into());
            state.open = true;
            state.ready = ready && !state.account_busy;
            state.video_width = video_width.min(16384);
            state.video_height = video_height.min(16384);
            state.video_paused = paused;
            state.page_recording = recording;
            if !recording
                && matches!(
                    detail.as_str(),
                    "no_audio"
                        | "unavailable"
                        | "recorder_error"
                        | "native_rejected"
                        | "rights_required"
                        | "rate_change"
                        | "seek"
                )
            {
                state.arm = None;
            }
            if state.recording.is_none() && state.arm.is_none() {
                state.status = if ready { "ready" } else { "waiting" }.into();
            }
            if !detail.is_empty()
                && detail != "ready"
                && detail != "recording"
                && detail != "idle"
                && detail != "waiting_video"
            {
                if matches!(
                    detail.as_str(),
                    "no_audio" | "recorder_error" | "native_rejected" | "queue_overflow"
                ) {
                    state.error = Some(bridge_reason(&detail).into());
                }
            }
            return Ok(Value::Null);
        }
        let _write = self.inner.writes.lock().map_err(|_| unavailable())?;
        if matches!(
            &message,
            BrowserMessage::Chunk { .. }
                | BrowserMessage::Segment { .. }
                | BrowserMessage::Finish { .. }
        ) && self
            .inner
            .encoded
            .lock()
            .map_err(|_| unavailable())?
            .is_some()
        {
            return Err(error(
                "ENCODED_SESSION_INVALID",
                "원본 녹화 전송 형식이 일치하지 않습니다.",
            ));
        }
        if let BrowserMessage::Begin {
            request_id,
            channel_id,
            title,
            mime_type,
        } = message
        {
            if channel_id != channel || title.len() > 2048 || mime_type.len() > 120 {
                return Err(unavailable());
            }
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if let Some((nonce, id, generation)) = &state.accepted_arm {
                if nonce == &request_id
                    && state.recording.as_ref() == Some(id)
                    && state.channel.as_deref() == Some(channel)
                    && *generation == state.page_generation
                    && !state.account_busy
                    && !self.inner.closing.load(Ordering::Acquire)
                    && !self.inner.reserved.load(Ordering::Acquire)
                {
                    return Ok(json!({"id":id,"captureChat":state.capture_chat}));
                }
            }
            if state.channel.as_deref() != Some(channel)
                || self.inner.closing.load(Ordering::Acquire)
                || self.inner.reserved.load(Ordering::Acquire)
                || state.account_busy
            {
                return Err(unavailable());
            }
            if !state.arm.as_ref().is_some_and(|arm| {
                arm.id == request_id
                    && arm.created.elapsed() <= Duration::from_secs(20)
                    && arm.generation == state.page_generation
            }) {
                return Err(error(
                    "BROWSER_NOT_ARMED",
                    "앱에서 녹화 시작을 눌러 주세요.",
                ));
            }
            let arm = state.arm.take().ok_or_else(unavailable)?;
            // Keep the reservation lock until begin is committed; shutdown cannot overtake it.
            let session = self
                .inner
                .store
                .lock()
                .map_err(|_| unavailable())?
                .begin(&arm.root, channel, &title, &mime_type)?;
            state.recording = Some(session.id.clone());
            state.accepted_arm = Some((request_id, session.id.clone(), state.page_generation));
            state.status = "recording".into();
            state.chat_count = 0;
            state.capture_chat = arm.capture_chat;
            state.chat_status = if arm.capture_chat {
                "connecting"
            } else {
                "disabled"
            }
            .into();
            drop(state);
            if arm.capture_chat {
                self.start_chat(&session);
            }
            return Ok(json!({"id":session.id,"captureChat":arm.capture_chat}));
        }
        let recording_id = match &message {
            BrowserMessage::Chunk { recording_id, .. }
            | BrowserMessage::Segment { recording_id, .. }
            | BrowserMessage::Finish { recording_id, .. }
            | BrowserMessage::ChatBatch { recording_id, .. }
            | BrowserMessage::ChatStatus { recording_id, .. } => recording_id,
            _ => return Err(unavailable()),
        };
        {
            let state = self.inner.view.lock().map_err(|_| unavailable())?;
            if state.recording.as_ref() != Some(recording_id)
                || state.channel.as_deref() != Some(channel)
            {
                return Err(error(
                    "BROWSER_SESSION_INVALID",
                    "이미 종료됐거나 다른 채널의 녹화입니다.",
                ));
            }
        }
        match message {
            BrowserMessage::ChatBatch {
                recording_id,
                events,
            } => {
                let enabled = self
                    .inner
                    .view
                    .lock()
                    .map_err(|_| unavailable())?
                    .capture_chat;
                if !enabled
                    || events.len() > 32
                    || serde_json::to_vec(&events).map_or(true, |bytes| bytes.len() > 128 * 1024)
                {
                    return Err(error(
                        "BROWSER_CHAT_INVALID",
                        "채팅 저장 요청이 올바르지 않습니다.",
                    ));
                }
                let result = (|| {
                    let encoded_source = self
                        .inner
                        .encoded
                        .lock()
                        .map_err(|_| unavailable())?
                        .as_ref()
                        .filter(|session| session.recording_id == recording_id)
                        .map(|session| session.source_id.clone());
                    let mut chat = self.inner.page_chat.lock().map_err(|_| unavailable())?;
                    let chat = chat
                        .as_mut()
                        .filter(|chat| chat.id == recording_id)
                        .ok_or_else(unavailable)?;
                    for value in events {
                        if let Some(ChatEvent::Message {
                            sender,
                            text,
                            server_time,
                            rich,
                        }) = super::chat::parse_message(&value)
                        {
                            let native_received = now_ms();
                            let replay_clock = chat.observe_clock(
                                &value,
                                native_received,
                                encoded_source.as_deref(),
                            );
                            let received = replay_clock
                                .as_ref()
                                .map_or(native_received, |clock| clock.received_at_ms);
                            let message = ChatMessage {
                                sequence: chat.sequence + 1,
                                sender,
                                text,
                                server_time,
                                received_at: received,
                                offset_seconds: received.saturating_sub(chat.started_at) as f64
                                    / 1000.0,
                                broadcast_offset_seconds: chat.broadcast_started_at.map(|start| {
                                    server_time.unwrap_or(received).saturating_sub(start) as f64
                                        / 1000.0
                                }),
                                replay_clock,
                                sender_key: value
                                    .get("senderKey")
                                    .and_then(Value::as_str)
                                    .and_then(super::model::bounded_sender_key),
                                rich,
                            };
                            chat.log.append(&message)?;
                            self.inner.replay_assets.submit(message.rich.as_ref());
                            chat.sequence += 1;
                        }
                    }
                    // ACK means the batch is durable, not merely queued in JavaScript.
                    chat.log.sync()?;
                    Ok::<u64, StreamError>(chat.sequence)
                })();
                let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                match result {
                    Ok(count) => {
                        state.chat_count = count;
                        if !chat_gap(&state.chat_status) {
                            state.chat_status = "page_connected".into();
                        }
                        Ok(json!({"saved":true,"count":count}))
                    }
                    Err(err) => {
                        state.chat_status = "storage_failed".into();
                        Err(err)
                    }
                }
            }
            BrowserMessage::ChatStatus {
                detail,
                dropped_messages,
                ..
            } => {
                let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                if !state.capture_chat {
                    return Err(unavailable());
                }
                if chat_gap(&detail)
                    || matches!(
                        detail.as_str(),
                        "observing"
                            | "receiving"
                            | "waiting_socket"
                            | "connected"
                            | "disconnected"
                            | "stopped"
                    )
                {
                    if !chat_gap(&state.chat_status) || detail == "storage_failed" {
                        state.chat_status = if dropped_messages > 0 && !chat_gap(&detail) {
                            "partial".into()
                        } else {
                            detail
                        };
                    }
                }
                Ok(Value::Null)
            }
            BrowserMessage::Chunk {
                recording_id,
                segment_index,
                chunk_index,
                data,
            } => {
                if data.len() > 180_000 {
                    return Err(error(
                        "BROWSER_CHUNK_TOO_LARGE",
                        "녹화 조각이 허용 크기를 초과했습니다.",
                    ));
                }
                let bytes = STANDARD.decode(data).map_err(|_| {
                    error("BROWSER_CHUNK_INVALID", "녹화 데이터가 올바르지 않습니다.")
                })?;
                let ack = self.inner.store.lock().map_err(|_| unavailable())?.append(
                    &recording_id,
                    segment_index,
                    chunk_index,
                    &bytes,
                )?;
                Ok(serde_json::to_value(ack).unwrap_or(Value::Null))
            }
            BrowserMessage::Segment {
                recording_id,
                segment_index,
                duration_seconds,
            } => {
                self.inner
                    .store
                    .lock()
                    .map_err(|_| unavailable())?
                    .finish_segment(&recording_id, segment_index, duration_seconds)?;
                Ok(json!({"saved":true,"segmentIndex":segment_index}))
            }
            BrowserMessage::Finish {
                recording_id,
                interrupted,
                reason,
            } => {
                let note = interrupted
                    .then(|| bridge_reason(reason.as_deref().unwrap_or("recorder_error")));
                // Final chat fsync may fail independently of the video. Preserve
                // that sticky warning in recording.json before acknowledging.
                self.stop_chat();
                let (capture_chat, chat_status, chat_count) = {
                    let state = self.inner.view.lock().map_err(|_| unavailable())?;
                    (
                        state.capture_chat,
                        if state.capture_chat {
                            state.chat_status.clone()
                        } else {
                            "disabled".into()
                        },
                        if state.capture_chat {
                            state.chat_count
                        } else {
                            0
                        },
                    )
                };
                let finished = self
                    .inner
                    .store
                    .lock()
                    .map_err(|_| unavailable())?
                    .finish_with_chat(
                        &recording_id,
                        interrupted,
                        note,
                        capture_chat,
                        &chat_status,
                        chat_count,
                    )?;
                self.inner.merges.wake();
                let interrupted = finished.status != BrowserRecordingStatus::Stopped;
                {
                    let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                    state.recording = None;
                    state.ready = false;
                    state.status = if interrupted { "error" } else { "ready" }.into();
                    state.error = finished.last_error.clone();
                }
                self.stop_chat();
                Ok(json!({"stopped":true,"interrupted":interrupted,"status":finished.status}))
            }
            _ => Err(unavailable()),
        }
    }
    fn start_chat(&self, recording: &BrowserRecording) {
        self.stop_chat();
        match ChatStore::create(Path::new(&recording.output_dir)) {
            Ok(log) => {
                *self
                    .inner
                    .page_chat
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(PageChatLog {
                    id: recording.id.clone(),
                    started_at: recording.started_at,
                    broadcast_started_at: None,
                    sequence: 0,
                    last_clock: None,
                    log,
                });
            }
            Err(_) => {
                self.inner
                    .view
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .chat_status = "storage_failed".into();
                return;
            }
        }
        // The official page's existing socket supplies chat. This worker only
        // reads optional public broadcast timing metadata, never cookies/tokens
        // or a second anonymous chat socket.
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let host = self.clone();
        let recording = recording.clone();
        let handle = thread::Builder::new()
            .name("chzzk-browser-timing".into())
            .spawn(move || {
                let info = ChzzkProvider::new()
                    .and_then(|provider| provider.inspect(&recording.channel_id));
                if stop.load(Ordering::Acquire) {
                    return;
                }
                if let Ok(info) = info {
                    if let Some(chat) = host
                        .inner
                        .page_chat
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .as_mut()
                        .filter(|chat| chat.id == recording.id)
                    {
                        chat.broadcast_started_at = info.broadcast_started_at;
                    }
                }
            });
        if let Ok(handle) = handle {
            *self.inner.chat.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(ChatWorker { cancel, handle });
        }
    }
    fn stop_chat(&self) {
        // Page stop drains its ACK queue before video finish; a crash preserves
        // every batch already accepted by the native host.
        let chat = self
            .inner
            .page_chat
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(mut chat) = chat {
            if chat.log.sync().is_err() {
                self.inner
                    .view
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .chat_status = "storage_failed".into();
            }
        }
        let worker = self
            .inner
            .chat
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(worker) = worker {
            worker.cancel.store(true, Ordering::Release);
            // Metadata/chat handshakes can still be waiting on a network timeout.
            // Never hold up the video writer or its final ACK for that network work.
            let mut retired = self
                .inner
                .retired_chat
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            retired.push(worker.handle);
            let mut index = 0;
            while index < retired.len() {
                if retired[index].is_finished() {
                    let handle = retired.swap_remove(index);
                    let _ = handle.join();
                } else {
                    index += 1;
                }
            }
        }
    }
    pub fn file(&self, id: &str, index: Option<u64>) -> Result<PathBuf, StreamError> {
        let records = self
            .inner
            .store
            .lock()
            .map_err(|_| unavailable())?
            .snapshot()?;
        let record = records
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(unavailable)?;
        let root = PathBuf::from(&record.output_dir)
            .canonicalize()
            .map_err(|_| unavailable())?;
        let path = if let Some(index) = index {
            root.join(
                &record
                    .segments
                    .iter()
                    .find(|s| s.index == index)
                    .ok_or_else(unavailable)?
                    .file,
            )
            .canonicalize()
            .map_err(|_| unavailable())?
        } else {
            root.clone()
        };
        if !path.starts_with(&root) {
            return Err(unavailable());
        }
        Ok(path)
    }
}

fn send_command(window: &Webview, value: Value) -> Result<(), StreamError> {
    window
        .eval(&format!(
            "window.dispatchEvent(new CustomEvent('atsumi-browser-command',{{detail:{value}}}));"
        ))
        .map_err(|_| unavailable())
}
fn reply(window: &Webview, id: &str, result: Result<Value, StreamError>) {
    let response = match result {
        Ok(data) => json!({"id":id,"ok":true,"data":data}),
        Err(error) => {
            json!({"id":id,"ok":false,"error":{"code":error.code,"message":error.message}})
        }
    };
    let _=window.eval(&format!("if(location.origin==='https://chzzk.naver.com')window.dispatchEvent(new CustomEvent('atsumi-browser-reply',{{detail:{response}}}));"));
}

#[cfg(windows)]
fn bridge_worker(
    window: &Webview,
    host: &OfficialBrowser,
    capacity: usize,
    name: &str,
) -> Result<std::sync::mpsc::SyncSender<(String, u64, Envelope)>, StreamError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<(String, u64, Envelope)>(capacity);
    let worker_window = window.clone();
    let worker_host = host.clone();
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Ok((channel, generation, envelope)) = receiver.recv() {
                let notification = matches!(
                    &envelope.message,
                    BrowserMessage::Status { .. } | BrowserMessage::ChatStatus { .. }
                );
                let recording_id = match &envelope.message {
                    BrowserMessage::Chunk { recording_id, .. }
                    | BrowserMessage::Segment { recording_id, .. }
                    | BrowserMessage::Finish { recording_id, .. }
                    | BrowserMessage::EncodedAppend { recording_id, .. }
                    | BrowserMessage::EncodedFinish { recording_id, .. } => Some(recording_id.clone()),
                    _ => None,
                };
                let current = worker_host
                    .inner
                    .view
                    .lock()
                    .is_ok_and(|state| state.page_generation == generation);
                let toggle_audio=matches!(&envelope.message,BrowserMessage::ViewIntent {action,..} if action=="audio_toggle");
                let mut result = if current {
                    worker_host.process(&channel, envelope.message)
                } else {
                    Err(control_stale())
                };
                if result.is_ok() && toggle_audio {
                    result=worker_host.capture_context(None).and_then(|root|root.toggle_pane_audio(worker_window.app_handle(),worker_host.label())).map(|_|json!({"accepted":true}));
                }
                if !notification {
                    if result.is_err() {
                        if let Some(id) = recording_id {
                            worker_host.interrupt_matching("native_rejected", Some(&id));
                        }
                    }
                    reply(&worker_window, &envelope.id, result);
                }
            }
        })
        .map_err(|_| unavailable())?;
    Ok(sender)
}

#[cfg(windows)]
fn attach_native(window: &Webview, host: OfficialBrowser) -> Result<(), StreamError> {
    use webview2_com::{CoTaskMemPWSTR, ProcessFailedEventHandler, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    let sender = bridge_worker(window, &host, 4, "chzzk-browser-writer")?;
    // PNG validation/fsync never delays recorder chunks; both queues stay bounded.
    let screenshot_sender = bridge_worker(window, &host, 2, "chzzk-screenshot-writer")?;
    let event_window = window.clone();
    let attach_host = host.clone();
    let source_host = host.clone();
    window
        .with_webview(move |platform| unsafe {
            let outcome = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                let mut token = 0;
                core.add_WebMessageReceived(
                    &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                        if let Some(args) = args {
                            let mut source = PWSTR::null();
                            args.Source(&mut source)?;
                            let source = CoTaskMemPWSTR::from(source).to_string();
                            if tauri::Url::parse(&source)
                                .ok()
                                .as_ref()
                                .and_then(live_channel)
                                .is_none()
                            {
                                return Ok(());
                            }
                            let mut body = PWSTR::null();
                            if args.TryGetWebMessageAsString(&mut body).is_err() {
                                return Ok(());
                            }
                            let body = CoTaskMemPWSTR::from(body).to_string();
                            let Some(body) = body.strip_prefix(BRIDGE_PREFIX) else {
                                return Ok(());
                            };
                            if let Some((channel, envelope)) = parse_message(&source, body) {
                                let generation = source_host
                                    .inner
                                    .view
                                    .lock()
                                    .map(|state| state.page_generation)
                                    .unwrap_or(u64::MAX);
                                let queue = if envelope.message.screenshot() {
                                    &screenshot_sender
                                } else {
                                    &sender
                                };
                                match queue.try_send((channel, generation, envelope)) {
                                    Ok(()) => {}
                                    Err(std::sync::mpsc::TrySendError::Full((_, _, envelope))) => {
                                        if !matches!(
                                            envelope.message,
                                            BrowserMessage::Status { .. }
                                        ) {
                                            reply(
                                                &event_window,
                                                &envelope.id,
                                                Err(error(
                                                    "BRIDGE_BUSY",
                                                    "녹화 저장 요청이 너무 빠릅니다.",
                                                )),
                                            );
                                        }
                                    }
                                    Err(_) => {}
                                }
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
                core.add_ProcessFailed(
                    &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                        let host = host.clone();
                        thread::spawn(move || {
                            host.interrupt("renderer_failed");
                        });
                        Ok(())
                    })),
                    &mut token,
                )?;
                Ok(())
            })();
            if outcome.is_err() {
                let mut state = attach_host
                    .inner
                    .view
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                state.error = Some("공식 시청 녹화 연결을 초기화하지 못했습니다.".into());
                state.ready = false;
            }
        })
        .map_err(|_| unavailable())
}
#[cfg(not(windows))]
fn attach_native(_: &Webview, _: OfficialBrowser) -> Result<(), StreamError> {
    Err(error(
        "PLATFORM_UNSUPPORTED",
        "공식 시청 통합 녹화는 Windows에서 지원합니다.",
    ))
}

fn require_main(window: &Webview) -> Result<(), StreamError> {
    if window.label() != "main" || window.window().label() != "main" {
        return Err(unavailable());
    }
    Ok(())
}
fn host(app: &AppHandle) -> Result<OfficialBrowser, StreamError> {
    app.state::<AppState>().official_browser()
}

#[tauri::command]
pub async fn chzzk_browser_open(
    app: AppHandle,
    window: Webview,
    input: String,
) -> ApiResult<BrowserSnapshot> {
    let result = (|| {
        require_main(&window)?;
        let host = host(&app)?;
        host.open(&app, &input)?;
        host.snapshot()
    })();
    result.into()
}
#[tauri::command]
pub async fn chzzk_browser_snapshot(app: AppHandle, window: Webview) -> ApiResult<BrowserSnapshot> {
    let result = (|| {
        require_main(&window)?;
        host(&app)?.snapshot()
    })();
    result.into()
}
#[tauri::command]
pub async fn chzzk_browser_set_viewport(
    app: AppHandle,
    window: Webview,
    viewport: BrowserViewport,
) -> ApiResult<()> {
    (|| {
        require_main(&window)?;
        if !viewport
            .request_sequence
            .is_some_and(|n| (1..=9_007_199_254_740_991).contains(&n))
        {
            return Err(error(
                "VIEWPORT_SEQUENCE_REQUIRED",
                "시청 화면을 새로고침한 뒤 다시 시도해 주세요.",
            ));
        }
        host(&app)?.set_viewport(&app, viewport)
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_login(app: AppHandle, window: Webview) -> ApiResult<BrowserSnapshot> {
    (|| {
        require_main(&window)?;
        let host = host(&app)?;
        host.login(&app)?;
        host.snapshot()
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_logout(app: AppHandle, window: Webview) -> ApiResult<BrowserSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        let host = host(&app)?;
        host.logout(&app)?;
        host.snapshot()
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_open_installer(
    app: AppHandle,
    window: Webview,
    browser: Option<InstallerBrowser>,
) -> ApiResult<()> {
    (|| {
        require_main(&window)?;
        host(&app)?.open_installer(browser.unwrap_or_default())
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_start(
    app: AppHandle,
    window: Webview,
    rights_acknowledged: bool,
    capture_chat: bool,
) -> ApiResult<BrowserSnapshot> {
    let result = (|| {
        require_main(&window)?;
        app.state::<AppState>()
            .start_browser_managed(&app, rights_acknowledged, capture_chat)?;
        host(&app)?.snapshot()
    })();
    result.into()
}
#[tauri::command]
pub async fn chzzk_browser_confirm_control(
    app: AppHandle,
    window: Webview,
    request_id: String,
    approve: bool,
    rights_acknowledged: bool,
    capture_chat: bool,
) -> ApiResult<BrowserSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err(error).into();
    }
    tauri::async_runtime::spawn_blocking(move || {
        host(&app)?.confirm_control(
            &app,
            &request_id,
            approve,
            rights_acknowledged,
            capture_chat,
        )
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_request_control(
    app: AppHandle,
    window: Webview,
    action: ControlAction,
) -> ApiResult<BrowserSnapshot> {
    (|| {
        require_main(&window)?;
        host(&app)?.request_control(action)
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_ack_ui_action(
    app: AppHandle,
    window: Webview,
    id: String,
) -> ApiResult<BrowserSnapshot> {
    (|| {
        require_main(&window)?;
        host(&app)?.ack_ui_action(&id)
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_stop(app: AppHandle, window: Webview) -> ApiResult<BrowserSnapshot> {
    let result = (|| {
        require_main(&window)?;
        let host = host(&app)?;
        if let Some(view) = app.get_webview(WINDOW_LABEL) {
            host.stop(&view)?;
        } else {
            host.interrupt("window_closed");
        }
        host.snapshot()
    })();
    result.into()
}
#[tauri::command]
pub async fn chzzk_browser_connect_extension(
    app: AppHandle,
    window: Webview,
) -> ApiResult<BrowserSnapshot> {
    let result = (|| {
        require_main(&window)?;
        let host = host(&app)?;
        host.connect_extension(&app)?;
        host.snapshot()
    })();
    result.into()
}
#[tauri::command]
pub async fn chzzk_browser_open_folder(
    app: AppHandle,
    window: Webview,
    recording_id: String,
) -> ApiResult<()> {
    open_record_file(app, window, recording_id, None).into()
}
#[tauri::command]
pub async fn chzzk_browser_open_segment(
    app: AppHandle,
    window: Webview,
    recording_id: String,
    index: u64,
) -> ApiResult<()> {
    open_record_file(app, window, recording_id, Some(index)).into()
}
#[tauri::command]
pub async fn chzzk_browser_open_merged(
    app: AppHandle,
    window: Webview,
    recording_id: String,
) -> ApiResult<()> {
    (|| {
        require_main(&window)?;
        let browser = host(&app)?;
        let path = browser
            .inner
            .store
            .lock()
            .map_err(|_| unavailable())?
            .merged_file(&recording_id)?;
        open_local_record_file(path)
    })()
    .into()
}
#[tauri::command]
pub async fn chzzk_browser_retry_merge(
    app: AppHandle,
    window: Webview,
    recording_id: String,
) -> ApiResult<BrowserSnapshot> {
    if let Err(error) = require_main(&window) {
        return Err::<BrowserSnapshot, _>(error).into();
    }
    let browser = match host(&app) {
        Ok(browser) => browser,
        Err(error) => return Err::<BrowserSnapshot, _>(error).into(),
    };
    match tauri::async_runtime::spawn_blocking(move || {
        browser.inner.merges.retry(Some(&recording_id))?;
        browser.snapshot()
    })
    .await
    {
        Ok(result) => result.into(),
        Err(_) => Err::<BrowserSnapshot, _>(unavailable()).into(),
    }
}
fn open_record_file(
    app: AppHandle,
    window: Webview,
    id: String,
    index: Option<u64>,
) -> Result<(), StreamError> {
    require_main(&window)?;
    let path = host(&app)?.file(&id, index)?;
    open_local_record_file(path)
}
fn open_local_record_file(path: PathBuf) -> Result<(), StreamError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR, Win32::UI::Shell::ShellExecuteW,
            Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        };
        let path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("open"),
                PCWSTR(path.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err(error(
                "BROWSER_FILE_OPEN_FAILED",
                "파일을 여는 프로그램을 찾지 못했습니다. 저장 폴더에서 확인해 주세요.",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "b3e262a2795f17734c149afc738ad250";
    const WEBM: &[u8] = b"\x1a\x45\xdf\xa3\x9fwebm-fixture";
    fn armed_host() -> (tempfile::TempDir, OfficialBrowser, String) {
        let root = tempfile::tempdir().unwrap();
        let host = OfficialBrowser::new(root.path().to_owned()).unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        {
            let mut state = host.inner.view.lock().unwrap();
            state.channel = Some(CHANNEL.into());
            state.ready = true;
            state.arm = Some(Arm {
                id: request.clone(),
                root: root.path().to_owned(),
                capture_chat: false,
                created: Instant::now(),
                generation: 0,
            });
        }
        (root, host, request)
    }
    fn begin(host: &OfficialBrowser, request: &str) -> Result<Value, StreamError> {
        host.process(
            CHANNEL,
            BrowserMessage::Begin {
                request_id: request.into(),
                channel_id: CHANNEL.into(),
                title: "synthetic".into(),
                mime_type: "video/webm;codecs=vp8,opus".into(),
            },
        )
    }
    fn ready_host() -> (tempfile::TempDir, OfficialBrowser) {
        let root = tempfile::tempdir().unwrap();
        let host = OfficialBrowser::new(root.path().to_owned()).unwrap();
        {
            let mut state = host.inner.view.lock().unwrap();
            state.open = true;
            state.channel = Some(CHANNEL.into());
            state.ready = true;
        }
        (root, host)
    }
    #[test]
    fn player_intent_never_arms_or_reveals_confirmation_nonce() {
        let (_root, host) = ready_host();
        assert!(host
            .control_intent("wrong", CHANNEL, ControlAction::RecordStart)
            .is_err());
        assert_eq!(
            host.control_intent(CHANNEL, CHANNEL, ControlAction::RecordStart)
                .unwrap(),
            json!({"pending":true})
        );
        assert!(host.active_ids().is_empty());
        let snapshot = host.snapshot().unwrap();
        assert!(snapshot.recordings.is_empty());
        let pending = snapshot.pending_control.unwrap();
        assert!(pending.expires_at >= now_ms() && pending.expires_at <= now_ms() + 20_000);
        let value = serde_json::to_value(&pending).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(value["action"], "record_start");
        assert!(host
            .control_intent(CHANNEL, CHANNEL, ControlAction::Screenshot)
            .is_err());
        assert!(host.take_control("wrong", true, true).is_err());
        assert!(host
            .take_control(&pending.id, false, false)
            .unwrap()
            .is_none());
        assert!(host.take_control(&pending.id, true, true).is_err());
        // Rejecting a request does not remove the two-second anti-spam cooldown.
        assert!(host
            .control_intent(CHANNEL, CHANNEL, ControlAction::Screenshot)
            .is_err());
        assert!(host.active_ids().is_empty());
    }
    #[test]
    fn control_approval_is_one_use_requires_rights_and_expires_with_document() {
        let (_root, host) = ready_host();
        host.control_intent(CHANNEL, CHANNEL, ControlAction::Screenshot)
            .unwrap();
        let pending = host.snapshot().unwrap().pending_control.unwrap();
        assert!(host.take_control(&pending.id, true, false).is_err());
        assert!(host.take_control(&pending.id, true, true).is_err());
        host.inner.view.lock().unwrap().last_control_intent = None;
        host.control_intent(CHANNEL, CHANNEL, ControlAction::RecordStart)
            .unwrap();
        let pending = host.snapshot().unwrap().pending_control.unwrap();
        host.inner.view.lock().unwrap().page_generation += 1;
        assert!(host.take_control(&pending.id, true, true).is_err());
        host.inner.view.lock().unwrap().last_control_intent = None;
        host.control_intent(CHANNEL, CHANNEL, ControlAction::Screenshot)
            .unwrap();
        let id = {
            let mut state = host.inner.view.lock().unwrap();
            let pending = state.pending_control.as_mut().unwrap();
            pending.created = Instant::now() - Duration::from_secs(21);
            pending.id.clone()
        };
        assert!(host.take_control(&id, true, true).is_err());
        assert!(host.active_ids().is_empty());
    }
    #[test]
    fn stop_intent_cannot_stop_a_replacement_recording_and_arm_rejects_old_generation() {
        let (_root, host, request) = armed_host();
        host.control_intent(CHANNEL, CHANNEL, ControlAction::RecordStop)
            .unwrap();
        let pending = host.snapshot().unwrap().pending_control.unwrap();
        host.inner.view.lock().unwrap().arm.as_mut().unwrap().id = "replacement".into();
        assert!(host.take_control(&pending.id, true, false).is_err());
        {
            let mut state = host.inner.view.lock().unwrap();
            state.arm.as_mut().unwrap().id = request.clone();
            state.page_generation += 1;
        }
        assert!(begin(&host, &request).is_err());
        assert!(host.snapshot().unwrap().recordings.is_empty());
    }
    #[test]
    fn screenshot_failure_never_interrupts_recording_and_requires_bound_nonce() {
        let (root, host, request) = armed_host();
        let recording = begin(&host, &request).unwrap();
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ScreenshotBegin {
                    request_id: "forged".into(),
                    channel_id: CHANNEL.into(),
                    mime_type: "image/png".into(),
                    size: 1,
                    width: 1,
                    height: 1
                }
            )
            .is_err());
        let nonce = host
            .inner
            .screenshots
            .lock()
            .unwrap()
            .arm(root.path(), CHANNEL, 0)
            .unwrap();
        host.process(
            CHANNEL,
            BrowserMessage::ScreenshotBegin {
                request_id: nonce.clone(),
                channel_id: CHANNEL.into(),
                mime_type: "image/png".into(),
                size: 1,
                width: 1,
                height: 1,
            },
        )
        .unwrap();
        host.process(
            CHANNEL,
            BrowserMessage::ScreenshotChunk {
                request_id: nonce.clone(),
                chunk_index: 0,
                data: STANDARD.encode(b"x"),
            },
        )
        .unwrap();
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ScreenshotFinish { request_id: nonce }
            )
            .is_err());
        assert_eq!(
            host.active_ids(),
            vec![recording["id"].as_str().unwrap().to_owned()]
        );
        assert!(!root.path().join("CHZZK/Screenshots").exists());
        assert!(host.snapshot().unwrap().last_screenshot.is_none());
        host.interrupt("window_closed");
    }
    #[test]
    fn begin_requires_host_nonce_and_retry_keeps_one_recording() {
        let (_root, host, request) = armed_host();
        assert!(begin(&host, "wrong").is_err());
        assert_eq!(host.snapshot().unwrap().recordings.len(), 0);
        let first = begin(&host, &request).unwrap();
        assert_eq!(begin(&host, &request).unwrap(), first);
        assert_eq!(host.snapshot().unwrap().recordings.len(), 1);
        assert!(host.reserve_update().is_err());
        host.interrupt("window_closed");
        assert!(host.active_ids().is_empty());
        host.reserve_update().unwrap();
    }
    #[test]
    fn accepted_begin_retry_remains_bound_to_channel_and_document() {
        let (_root, host, request) = armed_host();
        let first = begin(&host, &request).unwrap();
        let other = "0123456789abcdef0123456789abcdef";
        assert!(host
            .process(
                other,
                BrowserMessage::Begin {
                    request_id: request.clone(),
                    channel_id: other.into(),
                    title: "fixture".into(),
                    mime_type: "video/webm;codecs=vp8,opus".into()
                }
            )
            .is_err());
        assert_eq!(begin(&host, &request).unwrap(), first);
        host.inner.view.lock().unwrap().page_generation += 1;
        assert!(begin(&host, &request).is_err());
        assert_eq!(host.snapshot().unwrap().recordings.len(), 1);
        host.interrupt("window_closed");
    }
    #[test]
    fn socket_clock_requires_bounded_ordered_observations_and_a_matching_encoded_source() {
        let directory = tempfile::tempdir().unwrap();
        let mut chat = PageChatLog {
            id: "fixture".into(),
            started_at: 1_000_000,
            broadcast_started_at: None,
            sequence: 0,
            last_clock: None,
            log: ChatStore::create(directory.path()).unwrap(),
        };
        let source = "40000000-0000-4000-8000-000000000001";
        let clock = json!({"replayClock": {
            "version":1,"receivedAtMs":1_000_250,"observedMonotonicMs":250.0,
            "sourceGeneration":1,"clock":"mse_presentation_v1","mediaTimeSeconds":4002.5,
            "sourceTimeSeconds":4002.5,"sourceId":source,"playbackRate":2.0
        }});
        let observed = chat.observe_clock(&clock, 1_001_000, Some(source)).unwrap();
        assert_eq!(observed.clock, "mse_presentation_v1");
        assert_eq!(observed.source_time_seconds, Some(4002.5));
        assert_eq!(observed.received_at_ms, 1_000_250);
        for encoded in [None, Some("50000000-0000-4000-8000-000000000001")] {
            let unknown = chat.observe_clock(&clock, 1_001_000, encoded).unwrap();
            assert_eq!(unknown.clock, "player_observation");
            assert!(unknown.source_time_seconds.is_none());
            assert!(unknown.source_id.is_none());
        }
        for (field, value) in [
            ("version", json!(2)),
            ("observedMonotonicMs", json!(249)),
            ("sourceGeneration", json!(0)),
            ("sourceGeneration", json!(9_007_199_254_740_992_u64)),
            ("mediaTimeSeconds", json!(-1)),
            ("sourceTimeSeconds", json!(4002.6)),
            ("receivedAtMs", json!(2_000_000)),
            ("playbackRate", json!(20)),
            ("sourceId", json!("https://private.invalid/token")),
        ] {
            let mut invalid = clock.clone();
            invalid["replayClock"][field] = value;
            assert!(
                chat.observe_clock(&invalid, 1_001_000, Some(source))
                    .is_none(),
                "{field}"
            );
        }
        let mut later = clock.clone();
        later["replayClock"]["sourceGeneration"] = json!(2);
        later["replayClock"]["observedMonotonicMs"] = json!(300);
        assert!(chat
            .observe_clock(&later, 1_001_000, Some(source))
            .is_some());
        assert!(chat
            .observe_clock(&clock, 1_001_000, Some(source))
            .is_none());
    }

    #[test]
    fn page_chat_is_durable_beyond_200_and_does_not_store_private_profile_fields() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let record = host.snapshot().unwrap().recordings.remove(0);
        host.inner.view.lock().unwrap().capture_chat = true;
        *host.inner.page_chat.lock().unwrap() = Some(PageChatLog {
            id: id.clone(),
            started_at: record.started_at,
            broadcast_started_at: Some(record.started_at - 10_000),
            sequence: 0,
            last_clock: None,
            log: ChatStore::create(Path::new(&record.output_dir)).unwrap(),
        });
        let event = json!({"msgTypeCode":1,"msg":"test message","msgTime":record.started_at,"profile":{"nickname":"viewer","userIdHash":"PRIVATE_MARKER","token":"PRIVATE_MARKER"}});
        for _ in 0..8 {
            host.process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: vec![event.clone(); 32],
                },
            )
            .unwrap();
        }
        let snapshot = host.snapshot().unwrap();
        assert_eq!(snapshot.chat_count, 256);
        let raw =
            std::fs::read_to_string(Path::new(&record.output_dir).join("chat.jsonl")).unwrap();
        assert_eq!(raw.lines().count(), 256);
        assert!(!raw.contains("PRIVATE_MARKER"));
        let first: ChatMessage = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        assert_eq!(first.broadcast_offset_seconds, Some(10.0));
        let mut observed = event.clone();
        observed["replayClock"] = json!({"version":1,"receivedAtMs":record.started_at,
            "observedMonotonicMs":100.0,"sourceGeneration":1,"clock":"mse_presentation_v1",
            "mediaTimeSeconds":4001.0,"sourceTimeSeconds":4001.0,
            "sourceId":"40000000-0000-4000-8000-000000000001","playbackRate":1.0});
        observed["senderKey"] = json!(format!("sha256:{}", "a".repeat(64)));
        host.process(
            CHANNEL,
            BrowserMessage::ChatBatch {
                recording_id: id.clone(),
                events: vec![observed],
            },
        )
        .unwrap();
        let path = Path::new(&record.output_dir).join("chat.jsonl");
        let saved = std::fs::read_to_string(&path).unwrap();
        let last: ChatMessage = serde_json::from_str(saved.lines().last().unwrap()).unwrap();
        assert_eq!(last.received_at, record.started_at);
        assert_eq!(last.offset_seconds, 0.0);
        assert_eq!(last.sender_key, Some(format!("sha256:{}", "a".repeat(64))));
        let clock = last.replay_clock.unwrap();
        assert_eq!(clock.observed_monotonic_ms, 100.0);
        // A MediaRecorder session cannot promote a forged source claim.
        assert_eq!(clock.clock, "player_observation");
        assert!(clock.source_time_seconds.is_none());
        assert!(!saved.contains("PRIVATE_MARKER"));
        host.process(
            CHANNEL,
            BrowserMessage::ChatStatus {
                recording_id: id.clone(),
                detail: "connection_gap".into(),
                dropped_messages: 0,
            },
        )
        .unwrap();
        host.process(
            CHANNEL,
            BrowserMessage::ChatBatch {
                recording_id: id.clone(),
                events: vec![event.clone()],
            },
        )
        .unwrap();
        assert_eq!(host.snapshot().unwrap().chat_status, "connection_gap");
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: vec![event.clone(); 33]
                }
            )
            .is_err());
        assert_eq!(host.active_ids(), vec![id.clone()]);
        host.interrupt("window_closed");
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id,
                    events: vec![event]
                }
            )
            .is_err());
    }
    #[test]
    fn page_chat_requires_opt_in_and_matching_active_recording() {
        let (_root, host, request) = armed_host();
        let ack = begin(&host, &request).unwrap();
        assert_eq!(ack["captureChat"], false);
        let id = ack["id"].as_str().unwrap().to_owned();
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: vec![]
                }
            )
            .is_err());
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatStatus {
                    recording_id: "old".into(),
                    detail: "storage_failed".into(),
                    dropped_messages: 1
                }
            )
            .is_err());
        assert_eq!(host.active_ids(), vec![id]);
        host.interrupt("window_closed");
    }
    #[test]
    fn update_reservation_blocks_an_armed_begin() {
        let (_root, host, request) = armed_host();
        host.inner.reserved.store(true, Ordering::Release);
        assert!(begin(&host, &request).is_err());
        assert!(host.snapshot().unwrap().recordings.is_empty());
    }
    #[test]
    fn unfinished_capture_cannot_report_normal_stop() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        host.process(
            CHANNEL,
            BrowserMessage::Chunk {
                recording_id: id.clone(),
                segment_index: 0,
                chunk_index: 0,
                data: STANDARD.encode(WEBM),
            },
        )
        .unwrap();
        let reply = host
            .process(
                CHANNEL,
                BrowserMessage::Finish {
                    recording_id: id,
                    interrupted: false,
                    reason: None,
                },
            )
            .unwrap();
        assert_eq!(reply["interrupted"], true);
        let snapshot = host.snapshot().unwrap();
        assert_eq!(
            snapshot.recordings[0].status,
            BrowserRecordingStatus::Interrupted
        );
        assert!(!snapshot.ready);
        assert!(snapshot.error.is_some());
    }
    #[test]
    fn finalized_capture_waits_for_page_ack_before_restart() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        host.process(
            CHANNEL,
            BrowserMessage::Chunk {
                recording_id: id.clone(),
                segment_index: 0,
                chunk_index: 0,
                data: STANDARD.encode(WEBM),
            },
        )
        .unwrap();
        host.process(
            CHANNEL,
            BrowserMessage::Segment {
                recording_id: id.clone(),
                segment_index: 0,
                duration_seconds: 15.0,
            },
        )
        .unwrap();
        assert_eq!(
            host.process(
                CHANNEL,
                BrowserMessage::Finish {
                    recording_id: id,
                    interrupted: false,
                    reason: None
                }
            )
            .unwrap()["interrupted"],
            false
        );
        assert!(!host.snapshot().unwrap().ready);
        host.process(
            CHANNEL,
            BrowserMessage::Status {
                channel_id: CHANNEL.into(),
                ready: true,
                recording: false,
                detail: "saved".into(),
                video_width: 1920,
                video_height: 1080,
                paused: false,
            },
        )
        .unwrap();
        assert!(host.snapshot().unwrap().ready);
    }
    #[test]
    fn stale_write_error_cannot_interrupt_a_different_recording() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        host.interrupt_matching("native_rejected", Some("old-recording"));
        assert_eq!(host.active_ids(), vec![id]);
    }
    #[test]
    fn renderer_abandoning_a_lost_begin_ack_releases_native_session() {
        let (_root, host, request) = armed_host();
        begin(&host, &request).unwrap();
        host.process(
            CHANNEL,
            BrowserMessage::Status {
                channel_id: CHANNEL.into(),
                ready: true,
                recording: false,
                detail: "native_rejected".into(),
                video_width: 0,
                video_height: 0,
                paused: true,
            },
        )
        .unwrap();
        assert!(host.active_ids().is_empty());
        assert_eq!(
            host.snapshot().unwrap().recordings[0].status,
            BrowserRecordingStatus::Interrupted
        );
        host.reserve_update().unwrap();
    }
    #[test]
    fn wrong_channel_cannot_write_and_page_start_failure_clears_arm() {
        let (_root, host, request) = armed_host();
        host.process(
            CHANNEL,
            BrowserMessage::Status {
                channel_id: CHANNEL.into(),
                ready: true,
                recording: false,
                detail: "no_audio".into(),
                video_width: 0,
                video_height: 0,
                paused: true,
            },
        )
        .unwrap();
        assert!(host.active_ids().is_empty());
        assert!(begin(&host, &request).is_err());
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(host
            .process(
                "00000000000000000000000000000000",
                BrowserMessage::Chunk {
                    recording_id: id,
                    segment_index: 0,
                    chunk_index: 0,
                    data: STANDARD.encode(WEBM)
                }
            )
            .is_err());
    }
    #[test]
    fn bridge_is_limited_to_exact_live_origin() {
        let id = "b3e262a2795f17734c149afc738ad250";
        assert_eq!(
            live_channel(
                &format!("https://chzzk.naver.com/live/{id}")
                    .parse()
                    .unwrap()
            )
            .as_deref(),
            Some(id)
        );
        for url in [
            format!("http://chzzk.naver.com/live/{id}"),
            format!("https://chzzk.naver.com.evil.test/live/{id}"),
            format!("https://user@chzzk.naver.com/live/{id}"),
            format!("https://chzzk.naver.com/video/{id}"),
        ] {
            assert!(live_channel(&url.parse().unwrap()).is_none());
        }
    }
    #[test]
    fn bridge_rejects_other_messages_and_oversized_payload() {
        let source = "https://chzzk.naver.com/live/b3e262a2795f17734c149afc738ad250";
        assert!(parse_message(source, "{\"cmd\":\"settings_factory_reset\"}").is_none());
        assert!(parse_message(source, &" ".repeat(MAX_MESSAGE + 1)).is_none());
        let message=json!({"atsumiBrowserCapture":1,"id":uuid::Uuid::new_v4().to_string(),"kind":"status","channelId":"b3e262a2795f17734c149afc738ad250","ready":true}).to_string();
        assert!(parse_message(source, &message).is_some());
        assert!(parse_message("https://nid.naver.com", &message).is_none());
    }
    #[test]
    fn site_navigation_does_not_allow_arbitrary_web_or_file_urls() {
        for url in [
            "file:///C:/Windows",
            "https://evil.test",
            "https://chzzk.naver.com.evil.test",
            "http://nid.naver.com",
        ] {
            assert!(!allowed_navigation(&url.parse().unwrap()));
        }
    }
}
