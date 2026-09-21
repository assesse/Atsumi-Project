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

#[path = "browser_auto.rs"]
pub mod auto_record;
#[path = "browser_live_channels.rs"]
pub mod live_channels;
#[path = "browser_live_profiles.rs"]
pub mod live_profiles;

#[path = "browser_auth.rs"]
mod auth;
#[path = "browser_chat_popup.rs"]
mod chat_popup;
#[path = "browser_clip_popup.rs"]
mod clip_popup;
#[path = "browser_contexts.rs"]
mod contexts;
#[path = "browser_diagnostics.rs"]
mod diagnostics;
#[path = "browser_encoded.rs"]
pub(crate) mod encoded;
#[path = "browser_ending.rs"]
mod ending;
#[path = "browser_host.rs"]
mod host_view;
#[path = "browser_multiview.rs"]
pub mod multiview;
#[path = "browser_multiview_commands.rs"]
pub mod multiview_commands;
#[path = "browser_recording_profile.rs"]
pub mod recording_profile;
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
    auto_record: Arc<auto_record::AutoRecorder>,
    favorites: Arc<live_channels::Favorites>,
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
    chat_channel_id: Option<String>,
    sequence: u64,
    last_clock: Option<(f64, u64)>,
    last_batch: Option<(u64, Vec<u8>, u64)>,
    failed_batch: bool,
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
    chat_heartbeat: Option<Instant>,
    viewport: BrowserViewport,
    login_status: String,
    auth_status: auth::AuthStatus,
    auth_generation: u64,
    auth_last_probe: Option<Instant>,
    auth_checking: bool,
    auth_error: Option<String>,
    loaded_extensions: Vec<String>,
    extension_connecting: bool,
    extension_generation: u64,
    extension_reconnect_enabled: bool,
    video_width: u32,
    video_height: u32,
    video_paused: bool,
    capture_diagnostics: Option<Value>,
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
            chat_heartbeat: None,
            viewport: BrowserViewport::default(),
            login_status: "브라우저 세션 유지 · 로그인 여부는 공식 화면에서 확인".into(),
            auth_status: auth::AuthStatus::Unknown,
            auth_generation: 0,
            auth_last_probe: None,
            auth_checking: false,
            auth_error: None,
            loaded_extensions: Vec::new(),
            extension_connecting: false,
            extension_generation: 0,
            extension_reconnect_enabled: false,
            video_width: 0,
            video_height: 0,
            video_paused: true,
            capture_diagnostics: None,
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
    capture_chat_enabled: bool,
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
    auth_status: auth::AuthStatus,
    video_width: u32,
    auth_checking: bool,
    auth_error: Option<String>,
    account_busy: bool,
    video_height: u32,
    video_paused: bool,
    capture_diagnostics: Option<Value>,
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
        #[serde(default)]
        request_id: Option<String>,
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
        #[serde(default)]
        capture_diagnostics: Option<Value>,
    },
    ChatBatch {
        recording_id: String,
        events: Vec<Value>,
        #[serde(default)]
        batch_id: Option<u64>,
    },
    ChatStatus {
        recording_id: String,
        detail: String,
        #[serde(default)]
        dropped_messages: u64,
    },
}
impl BrowserMessage {
    fn chat(&self) -> bool {
        matches!(self, Self::ChatBatch { .. } | Self::ChatStatus { .. })
    }
    fn screenshot(&self) -> bool {
        matches!(
            self,
            Self::ScreenshotBegin { .. }
                | Self::ScreenshotChunk { .. }
                | Self::ScreenshotFinish { .. }
                | Self::ScreenshotAbort { .. }
        )
    }
    fn screenshot_lane(&self) -> bool {
        self.screenshot()
            || matches!(
                self,
                Self::ControlIntent {
                    action: ControlAction::Screenshot,
                    ..
                }
            )
    }
}

// Remote diagnostics are untrusted. Persist/display only bounded capability
// facts; discard every unknown field, URL, identifier and free-form message.
fn sanitized_capture_diagnostics(value: Option<Value>) -> Option<Value> {
    let value = value?;
    let reason = value["reason"].as_str()?;
    let reason = match reason {
        "ready"
        | "buffering"
        | "waiting_video"
        | "waiting_tracks"
        | "waiting_init"
        | "mse_unavailable"
        | "observer_changed"
        | "encrypted"
        | "source_object_unsupported"
        | "source_not_observed"
        | "timeline_unsupported"
        | "timeline_changed"
        | "tracks_unsupported"
        | "codec_unsupported"
        | "source_blocked"
        | "buffer_blocked"
        | "native_init_unsupported"
        | "init_unsupported"
        | "init_changed"
        | "container_unsupported"
        | "append_unsupported"
        | "observer_failed"
        | "source_buffer_error"
        | "codec_changed"
        | "track_changed" => reason,
        _ => "unsupported",
    };
    let sources: Vec<Value> = value["sources"].as_array().into_iter().flatten().take(8).map(|source| {
        let tracks: Vec<Value> = source["tracks"].as_array().into_iter().flatten().take(2).map(|track| {
            let mime = track["mimeType"].as_str().filter(|s|s.len()<=120).unwrap_or("").replace([' ', '"'], "").to_ascii_lowercase();
            let valid_mime = mime.strip_prefix("video/mp4;codecs=").or_else(||mime.strip_prefix("audio/mp4;codecs=")).is_some_and(|codecs| {
                let parts: Vec<_> = codecs.split(',').collect();
                !parts.is_empty() && parts.len()<=2 && parts.iter().all(|codec| *codec=="mp4a.40.2" ||
                    codec.strip_prefix("avc1.").or_else(||codec.strip_prefix("avc3.")).is_some_and(|p|p.len()==6 && p.bytes().all(|b|b.is_ascii_hexdigit())))
            });
            json!({"mimeType": if valid_mime {mime.as_str()} else {"unsupported"},
                "initBytes":track["initBytes"].as_u64().unwrap_or(0).min(65536),
                "timestampOffset":track["timestampOffset"].as_f64().filter(|v|v.is_finite() && v.abs()<=1e9),
                "blocked":track["blocked"].as_bool().unwrap_or(false)})
        }).collect();
        json!({"selected":source["selected"].as_bool().unwrap_or(false), "tracks":tracks})
    }).collect();
    let last_stop = value.get("lastStop").filter(|v| v.is_object()).map(|v| json!({
        "reason": match v["reason"].as_str() { Some("video_ended") => "video_ended", Some("source_changed") => "source_changed", _ => "other" },
        "ended": v["ended"].as_bool().unwrap_or(false), "sourceAttached":v["sourceAttached"].as_bool().unwrap_or(false),
        "sourceClosed":v["sourceClosed"].as_bool().unwrap_or(false), "readyState":v["readyState"].as_u64().unwrap_or(0).min(4)
    }));
    let fault = value.get("lastTransportFault").filter(|v| v.is_object()).map(|v| json!({
        "code": match v["code"].as_str() { Some("BROWSER_ACK_TIMEOUT") => "BROWSER_ACK_TIMEOUT", Some("BRIDGE_BUSY") => "BRIDGE_BUSY", Some("BRIDGE_POST_FAILED") => "BRIDGE_POST_FAILED", Some("BROWSER_CONTROL_STALE") => "BROWSER_CONTROL_STALE", Some("BROWSER_ENCODED_INVALID") => "BROWSER_ENCODED_INVALID", _ => "other" },
        "at":v["at"].as_u64().unwrap_or(0), "attempt":v["attempt"].as_u64().unwrap_or(0).min(100),
        "appendIndex":v["appendIndex"].as_u64().unwrap_or(0).min(1_000_000),
        "chunkIndex":v["chunkIndex"].as_u64().unwrap_or(0).min(1024),
        "queuedBytes":v["queuedBytes"].as_u64().unwrap_or(0).min(64*1024*1024)
    }));
    Some(
        json!({"reason":reason, "installed":value["installed"].as_bool().unwrap_or(false),
        "lastTransportFault":fault,"queuedBytes":value["queuedBytes"].as_u64().unwrap_or(0).min(64*1024*1024),
        "maxAckMs":value["maxAckMs"].as_u64().unwrap_or(0).min(3_600_000),
        "appendCount":value["appendCount"].as_u64().unwrap_or(0).min(1_000_000_000),
        "appendBytes":value["appendBytes"].as_u64().unwrap_or(0).min(1_000_000_000_000_000), "sources":sources, "lastStop":last_stop}),
    )
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
        "bridge_ack_timeout" => "영상 저장 응답이 제한 시간 안에 도착하지 않아 녹화를 중단했습니다. 저장된 영상은 보존됩니다.",
        "bridge_busy" => "영상 저장 대기열의 혼잡이 해소되지 않아 녹화를 중단했습니다. 저장된 영상은 보존됩니다.",
        "bridge_post_failed" => "플레이어와 영상 저장부의 연결이 끊겼습니다. 저장된 영상은 보존됩니다.",
        "seek" => "타임머신 또는 재생 위치 이동으로 녹화를 중단했습니다.",
        "rate_change" => "재생 배속이 변경되어 녹화를 중단했습니다.",
        "video_changed" | "channel_changed" | "source_changed" => "방송 또는 영상 소스가 변경되어 녹화를 중단했습니다.",
        "codec_changed" | "track_changed" | "init_changed" => "영상·음성 형식이 변경되어 원본 녹화를 중단했습니다. 저장된 조각은 보존됩니다.",
        "observer_changed" | "observer_failed" => "플레이어의 수신 경로가 변경되어 원본 녹화를 중단했습니다. 방송을 다시 연결해 주세요.",
        "encrypted" => "암호화된 영상으로 변경되어 원본 녹화를 중단했습니다.",
        "timeline_changed" | "container_unsupported" | "init_unsupported" | "append_unsupported" | "source_buffer_error" => "수신 영상의 형식 또는 시간축을 안전하게 저장할 수 없어 녹화를 중단했습니다. 저장된 조각은 보존됩니다.",
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
            || (action != ControlAction::RecordStop
                && state
                    .last_control_intent
                    .is_some_and(|at| at.elapsed() < Duration::from_millis(200)))
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
        let profile_busy = self.primary_profile_busy();
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
            || profile_busy
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
    // Immediate screenshots have no PendingControl: React polling must never
    // turn a short native authorization into a confirmation dialog/occlusion.
    fn arm_immediate_screenshot(
        &self,
        app: &AppHandle,
        channel: &str,
        generation: Option<u64>,
    ) -> Result<String, StreamError> {
        let root = app
            .state::<AppState>()
            .settings_snapshot()
            .map_err(|_| unavailable())?
            .download_root;
        let view = app.get_webview(self.label()).ok_or_else(unavailable)?;
        if view.url().ok().as_ref().and_then(live_channel).as_deref() != Some(channel)
            || self.account_window_open(app)
        {
            return Err(control_stale());
        }
        let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
        let profile_busy = self.primary_profile_busy();
        let mut capture = self.inner.screenshots.try_lock().map_err(|_| {
            error(
                "SCREENSHOT_BUSY",
                "현재 스크린샷 저장이 끝난 뒤 다시 요청해 주세요.",
            )
        })?;
        let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
        if state.channel.as_deref() != Some(channel)
            || generation.is_some_and(|g| g != state.page_generation)
            || !state.ready
            || state.account_busy
            || state.extension_connecting
            || profile_busy
            || self.inner.detached.load(Ordering::Acquire)
            || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
            || self.inner.closing.load(Ordering::Acquire)
            || self.inner.reserved.load(Ordering::Acquire)
            || self.multiview_active()
        {
            return Err(control_stale());
        }
        if state.pending_control.is_some()
            || state.confirming_control.is_some()
            || state
                .last_control_intent
                .is_some_and(|at| at.elapsed() < Duration::from_millis(200))
        {
            return Err(error("BROWSER_CONTROL_BUSY", "잠시 후 다시 요청해 주세요."));
        }
        let nonce = capture.arm(Path::new(&root), channel, state.page_generation)?;
        state.last_control_intent = Some(Instant::now());
        Ok(nonce)
    }
    pub fn request_control_from_ui(
        &self,
        app: &AppHandle,
        action: ControlAction,
    ) -> Result<BrowserSnapshot, StreamError> {
        if action != ControlAction::Screenshot {
            return self.request_control(action);
        }
        let (channel, generation) = {
            let state = self.inner.view.lock().map_err(|_| unavailable())?;
            (
                state.channel.clone().ok_or_else(unavailable)?,
                state.page_generation,
            )
        };
        let nonce = self.arm_immediate_screenshot(app, &channel, Some(generation))?;
        let result = app
            .get_webview(self.label())
            .ok_or_else(unavailable)
            .and_then(|view| {
                send_command(
                    &view,
                    json!({"kind":"screenshot","requestId":nonce,"channelId":channel}),
                )
            });
        if result.is_err() {
            if let Ok(mut capture) = self.inner.screenshots.lock() {
                capture.abort(&nonce);
            }
        }
        result?;
        self.snapshot()
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
                self.remember_manual_stop(&pending.channel_id);
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
                if data.len() > screenshot::CHUNK.div_ceil(3) * 4 {
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
        let store = Arc::new(Mutex::new(BrowserCaptureStore::new_deferred(&data_dir)?));
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
                auto_record: Arc::new(auto_record::AutoRecorder::load(&data_dir)),
                favorites: Arc::new(live_channels::Favorites::load(&data_dir)),
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
        if state.capture_chat
            && state.recording.is_some()
            && state
                .chat_heartbeat
                .is_some_and(|at| at.elapsed() > Duration::from_secs(45))
            && !chat_gap(&state.chat_status)
        {
            state.chat_status = "partial".into();
        }
        Ok(BrowserSnapshot {
            capture_chat_enabled: self.inner.auto_record.capture_chat(),
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
            auth_status: state.auth_status,
            auth_checking: state.auth_checking,
            auth_error: state.auth_error.clone(),
            account_busy: state.account_busy,
            video_width: state.video_width,
            video_height: state.video_height,
            video_paused: state.video_paused,
            capture_diagnostics: state.capture_diagnostics.clone(),
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
        self.arm_checked(app, root, rights, capture_chat, None)
    }
    pub(crate) fn arm_checked(
        &self,
        app: &AppHandle,
        root: PathBuf,
        rights: bool,
        capture_chat: bool,
        expected_channel: Option<&str>,
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
            || self
                .active_ids()
                .len()
                .saturating_sub(usize::from(!self.local_active_ids().is_empty()))
                >= 4
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
        if expected_channel.is_some() && page_channel.as_deref() != expected_channel {
            return Err(control_stale());
        }
        for other in self.inner.contexts.hosts() {
            if other.label() == self.label() {
                continue;
            }
            let state = other.inner.view.lock().map_err(|_| unavailable())?;
            if page_channel.is_some()
                && state.channel == page_channel
                && (state.recording.is_some() || state.arm.is_some())
            {
                return Err(error(
                    "BROWSER_CHANNEL_RECORDING",
                    "이 방송은 이미 녹화 중입니다.",
                ));
            }
        }
        let channel = {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if expected_channel.is_some() && state.channel.as_deref() != expected_channel {
                return Err(control_stale());
            }
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
        let reason = if self.inner.closing.load(Ordering::Acquire) {
            "app_shutdown"
        } else {
            "user_stop"
        };
        send_command(
            window,
            json!({"kind":"stop","channelId":channel,"reason":reason}),
        )
    }
    fn wait_stopped(&self, timeout: Duration) {
        let started = Instant::now();
        while started.elapsed() < timeout && !self.active_ids().is_empty() {
            thread::sleep(Duration::from_millis(100));
        }
    }
    pub(super) fn remember_manual_stop(&self, channel: &str) {
        if self.inner.auto_record.suppress(channel).is_err() {
            self.inner.contexts.notice("녹화는 중지합니다. 자동 녹화 중지 이력을 저장하지 못했으므로 앱 재시작 전에 설정을 확인해 주세요.");
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
            if let Ok(saved) = &metadata {
                self.finish_reason(saved, reason);
            }
            self.inner
                .contexts
                .notice("녹화가 중단되었습니다. 녹화 목록에서 확인해 주세요.");
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
            request_id,
            ready,
            recording,
            detail,
            video_width,
            video_height,
            paused,
            capture_diagnostics,
        } = message
        {
            if channel_id != channel {
                return Err(unavailable());
            }
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            // Status has a separate lane: a delayed terminal notification from
            // recording A must never cancel a newer recording/arm B. The nonce
            // is the native-issued start capability, not a page-selected ID.
            let expected = state
                .arm
                .as_ref()
                .map(|a| a.id.as_str())
                .or_else(|| state.accepted_arm.as_ref().map(|a| a.0.as_str()));
            if (state.recording.is_some() || state.arm.is_some())
                && request_id.as_deref() != expected
            {
                return Ok(Value::Null);
            }
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
            let diagnostics = sanitized_capture_diagnostics(capture_diagnostics);
            if diagnostics.as_ref().map(|d| &d["lastTransportFault"])
                != state
                    .capture_diagnostics
                    .as_ref()
                    .map(|d| &d["lastTransportFault"])
            {
                diagnostics::record(
                    self,
                    "capture_transport",
                    json!({"recordingId":state.recording,"diagnostics":diagnostics}),
                );
            }
            if diagnostics.as_ref().map(|d| &d["reason"])
                != state.capture_diagnostics.as_ref().map(|d| &d["reason"])
            {
                tracing::info!(
                    channel_id = channel,
                    webview = self.label(),
                    diagnostics = ?diagnostics,
                    "original recording source capability changed"
                );
            }
            state.capture_diagnostics = diagnostics;
            if state.recording.is_some()
                && state.status != "stopping"
                && matches!(detail.as_str(), "recording" | "waiting_source")
            {
                state.status = detail.clone();
            }
            if !recording
                && matches!(
                    detail.as_str(),
                    "no_audio"
                        | "unavailable"
                        | "original_unavailable"
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
                state.status = if detail == "original_unavailable" {
                    "error"
                } else if ready {
                    "ready"
                } else {
                    "waiting"
                }
                .into();
                if detail == "original_unavailable" && state.error.is_none() {
                    state.error = Some("현재 영상의 원본 저장 경로를 확인할 수 없습니다. 방송을 다시 연결해 주세요. 화면 재녹화로 전환하지 않았습니다.".into());
                }
            }
            if !detail.is_empty()
                && detail != "ready"
                && detail != "recording"
                && detail != "idle"
                && detail != "waiting_video"
                && matches!(
                    detail.as_str(),
                    "no_audio" | "recorder_error" | "native_rejected" | "queue_overflow"
                )
                && state.error.is_none()
            {
                state.error = Some(bridge_reason(&detail).into());
            }
            return Ok(Value::Null);
        }
        // Chat has its own ordered worker and PageChatLog mutex. Holding the
        // video write reservation across a chat fsync stalls all media packets.
        // Finish takes PageChatLog before closing it, so ACK still means durable.
        let _write = if message.chat() {
            None
        } else {
            Some(self.inner.writes.lock().map_err(|_| unavailable())?)
        };
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
                .begin_progressive(&arm.root, channel, &title, &mime_type)?;
            state.recording = Some(session.id.clone());
            self.inner.contexts.notice("녹화를 시작했습니다.");
            state.accepted_arm = Some((request_id, session.id.clone(), state.page_generation));
            state.status = "recording".into();
            state.chat_count = 0;
            state.capture_chat = arm.capture_chat;
            state.chat_heartbeat = arm.capture_chat.then(Instant::now);
            state.chat_status = if arm.capture_chat {
                "connecting"
            } else {
                "disabled"
            }
            .into();
            drop(state);
            if let Some(key) = self.inner.auto_record.observed_live_key(channel) {
                let _ = self
                    .inner
                    .store
                    .lock()
                    .map(|store| store.set_broadcast_key(&session.id, &key));
            }
            self.inner
                .replay_assets
                .submit_channel(Path::new(&session.output_dir), channel);
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
                batch_id,
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
                    let batch_bytes = serde_json::to_vec(&events).map_err(|_| unavailable())?;
                    if let Some(batch_id) = batch_id {
                        if let Some((previous, bytes, count)) = &chat.last_batch {
                            if batch_id == *previous && &batch_bytes == bytes {
                                return Ok(*count);
                            }
                            if batch_id != previous + 1 {
                                return Err(unavailable());
                            }
                        } else if batch_id != 1 {
                            return Err(unavailable());
                        }
                    }
                    if chat.failed_batch {
                        return Err(unavailable());
                    }
                    chat.failed_batch = true; // Partial writes must never be blindly retried.
                    for value in events {
                        if value.get("atsumiViewerSample").and_then(Value::as_u64) == Some(1) {
                            if let Some(clock) =
                                chat.observe_clock(&value, now_ms(), encoded_source.as_deref())
                            {
                                if let Some(sample) = super::viewer_metrics::ViewerSample::from_page(
                                    &value,
                                    clock,
                                    chat.started_at,
                                    chat.broadcast_started_at,
                                    channel,
                                ) {
                                    chat.log.append_viewer(&sample);
                                }
                            }
                            continue;
                        }
                        if let Some(ChatEvent::Message {
                            sender,
                            text,
                            server_time,
                            rich,
                        }) = super::chat::parse_message_with_chat_channel(
                            &value,
                            chat.chat_channel_id.as_deref(),
                        ) {
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
                            chat.log.append_batched(&message)?;
                            self.inner
                                .replay_assets
                                .submit_recording(chat.log.recording_root(), message.rich.as_ref());
                            chat.sequence += 1;
                        }
                    }
                    // ACK means the batch is durable, not merely queued in JavaScript.
                    chat.log.sync()?;
                    chat.failed_batch = false;
                    if let Some(batch_id) = batch_id {
                        chat.last_batch = Some((batch_id, batch_bytes, chat.sequence));
                    }
                    Ok::<u64, StreamError>(chat.sequence)
                })();
                let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                if state.recording.as_deref() != Some(&recording_id) {
                    return Err(control_stale());
                }
                match result {
                    Ok(count) => {
                        state.chat_heartbeat = Some(Instant::now());
                        let received_messages = count > state.chat_count;
                        state.chat_count = count;
                        if received_messages && !chat_gap(&state.chat_status) {
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
                recording_id,
                detail,
                dropped_messages,
                ..
            } => {
                let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                if state.recording.as_deref() != Some(&recording_id) {
                    return Err(control_stale());
                }
                if !state.capture_chat {
                    return Err(unavailable());
                }
                state.chat_heartbeat = Some(Instant::now());
                if chat_gap(&detail) && state.chat_status != detail {
                    tracing::warn!(channel_id = channel, reason = %detail, dropped_messages, "recording chat continuity warning");
                }
                if (chat_gap(&detail)
                    || matches!(
                        detail.as_str(),
                        "observing"
                            | "receiving"
                            | "waiting_socket"
                            | "connected"
                            | "disconnected"
                            | "stopped"
                    ))
                    && (!chat_gap(&state.chat_status) || detail == "storage_failed")
                {
                    state.chat_status = if dropped_messages > 0 && !chat_gap(&detail) {
                        "partial".into()
                    } else {
                        detail
                    };
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
                // One later retry for a transient profile fetch failure. This
                // is not part of the video/chat write path and does no UI I/O.
                self.inner
                    .replay_assets
                    .submit_channel(Path::new(&finished.output_dir), &finished.channel_id);
                let interrupted = finished.status != BrowserRecordingStatus::Stopped;
                let ending = self.finish_reason(&finished, reason.as_deref().unwrap_or("unknown"));
                {
                    let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
                    state.recording = None;
                    state.ready = false;
                    state.status = if interrupted { "error" } else { "ready" }.into();
                    state.error = finished.last_error.clone();
                }
                self.stop_chat();
                self.inner.contexts.notice(if ending == "checking" {
                    "영상 수신이 끝나 방송 종료 여부를 확인합니다. 녹화 파일은 보존됩니다."
                } else if ending == "broadcast_ended" {
                    "방송이 종료되어 녹화를 저장했습니다."
                } else if interrupted {
                    "녹화가 중단되었습니다. 녹화 목록에서 확인해 주세요."
                } else {
                    "녹화를 종료했습니다."
                });
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
                    chat_channel_id: None,
                    sequence: 0,
                    last_clock: None,
                    last_batch: None,
                    failed_batch: false,
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
        // reads optional public broadcast timing/chat-color metadata, never cookies/tokens
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
                    if let Some(key) = ending::live_key(&info) {
                        let _ = host
                            .inner
                            .store
                            .lock()
                            .map(|store| store.set_broadcast_key(&recording.id, &key));
                    }
                    if let Some(chat) = host
                        .inner
                        .page_chat
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .as_mut()
                        .filter(|chat| chat.id == recording.id)
                    {
                        chat.broadcast_started_at = info.broadcast_started_at;
                        chat.chat_channel_id = info.chat_channel_id;
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
        {
            let mut state = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            if state.capture_chat
                && state
                    .chat_heartbeat
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(45))
                && !chat_gap(&state.chat_status)
            {
                state.chat_status = "partial".into();
            }
        }
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
            if record.media_removed_at.is_some() {
                return Err(error(
                    "RECORDING_MEDIA_REMOVED",
                    "영상만 정리된 기록입니다. 오류·채팅 기록은 폴더에 보존되어 있습니다.",
                ));
            }
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
        .eval(format!(
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
    let _=window.eval(format!("if(location.origin==='https://chzzk.naver.com')window.dispatchEvent(new CustomEvent('atsumi-browser-reply',{{detail:{response}}}));"));
}

#[cfg(windows)]
fn bridge_worker(
    window: &Webview,
    host: &OfficialBrowser,
    capacity: usize,
    name: &str,
) -> Result<std::sync::mpsc::SyncSender<(String, u64, Envelope, Instant)>, StreamError> {
    let (sender, receiver) =
        std::sync::mpsc::sync_channel::<(String, u64, Envelope, Instant)>(capacity);
    let worker_window = window.clone();
    let worker_host = host.clone();
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut last_sample = Instant::now();
            while let Ok((channel, generation, envelope, queued)) = receiver.recv() {
                let began = Instant::now();
                let queued_ms = queued.elapsed().as_millis();
                let packet = diagnostics::packet(&envelope.message);
                let notification = matches!(
                    &envelope.message,
                    BrowserMessage::Status { .. }
                );
                let recording_id = match &envelope.message {
                    BrowserMessage::Chunk { recording_id, .. }
                    | BrowserMessage::Segment { recording_id, .. }
                    | BrowserMessage::Finish { recording_id, .. }
                    | BrowserMessage::EncodedAppend { recording_id, .. }
                    | BrowserMessage::EncodedFinish { recording_id, .. } => Some(recording_id.clone()),
                    _ => None,
                };
                // Late packets from a finished recording are rejected, but must
                // not overwrite that recording's persisted completion reason.
                let recording_id = recording_id.filter(|id| worker_host.inner.view.lock()
                    .is_ok_and(|state| state.recording.as_ref() == Some(id)));
                let current = worker_host
                    .inner
                    .view
                    .lock()
                    .is_ok_and(|state| state.page_generation == generation);
                let toggle_audio=matches!(&envelope.message,BrowserMessage::ViewIntent {action,..} if action=="audio_toggle");
                let immediate_record = matches!(&envelope.message, BrowserMessage::ControlIntent { action: ControlAction::RecordStart | ControlAction::RecordStop, .. });
                let original = matches!(&envelope.message, BrowserMessage::EncodedBegin { .. } | BrowserMessage::EncodedAppend { .. } | BrowserMessage::EncodedFinish { .. });
                let mut result = if current {
                    if let BrowserMessage::ControlIntent { channel_id, action: ControlAction::Screenshot } = &envelope.message {
                        if channel_id != &channel { Err(control_stale()) }
                        else { worker_host.arm_immediate_screenshot(worker_window.app_handle(), &channel, Some(generation))
                            .map(|nonce| json!({"requestId":nonce,"channelId":channel})) }
                    } else { worker_host.process(&channel, envelope.message) }
                } else {
                    Err(control_stale())
                };
                if result.is_ok() && toggle_audio {
                    result=worker_host.capture_context(None).and_then(|root|root.toggle_pane_audio(worker_window.app_handle(),worker_host.label())).map(|_|json!({"accepted":true}));
                }
                if result.is_ok() && immediate_record {
                    // The native bridge has already checked the originating
                    // channel/document. Consume its one-use intent immediately;
                    // recording must not depend on a mounted React dialog.
                    let pending = worker_host.inner.view.lock().ok()
                        .and_then(|state| state.pending_control.clone());
                    result = pending.ok_or_else(control_stale).and_then(|pending| {
                        worker_host.confirm_control(worker_window.app_handle(), &pending.id, true, true,
                            worker_host.inner.auto_record.capture_chat())
                    }).map(|_| json!({"accepted":true}));
                    if let Err(cause) = &result {
                        worker_host.inner.contexts.notice(&cause.message);
                    }
                }
                let process_ms = began.elapsed().as_millis();
                if result.is_err() || queued_ms > 500 || process_ms > 500 || last_sample.elapsed() > Duration::from_secs(15) {
                    diagnostics::record(&worker_host, "bridge_processed", json!({"packet":packet,"queuedMs":queued_ms,"processMs":process_ms,"errorCode":result.as_ref().err().map(|e| &e.code)}));
                    last_sample = Instant::now();
                }
                if !notification {
                    if let Err(error) = &result {
                        if let Some(id) = recording_id {
                            worker_host.interrupt_matching("native_rejected", Some(&id));
                            let _ = worker_host.inner.store.lock().map(|store| store.note_capture_error(&id, &error.code, &error.message));
                        }
                        if original {
                            tracing::warn!(code = %error.code, reason = %error.message, "original recording request failed");
                            if let Ok(mut state) = worker_host.inner.view.lock() {
                                if !state.error.as_deref().is_some_and(|e| e.starts_with("원본 저장 오류:")) {
                                    state.error = Some(format!("원본 저장 오류: {}", error.message));
                                }
                            }
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
    let chat_sender = bridge_worker(window, &host, 4, "chzzk-chat-writer")?;
    let status_sender = bridge_worker(window, &host, 1, "chzzk-capture-status")?;
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
                                // WebMessageReceived runs on the shared UI/COM
                                // thread. Never wait here for a writer that is
                                // holding ViewState while opening/flushing a file.
                                let generation = match source_host.inner.view.try_lock() {
                                    Ok(state) => state.page_generation,
                                    Err(_) => {
                                        if !matches!(&envelope.message, BrowserMessage::Status { .. }) {
                                            diagnostics::record(&source_host, "bridge_state_busy", diagnostics::packet(&envelope.message));
                                            reply(&event_window, &envelope.id, Err(error("BRIDGE_BUSY", "녹화 상태 처리 중입니다. 잠시 후 다시 시도합니다.")));
                                        }
                                        return Ok(());
                                    }
                                };
                                let queue = if envelope.message.screenshot_lane() {
                                    &screenshot_sender
                                } else if envelope.message.chat() {
                                    &chat_sender
                                } else if matches!(&envelope.message, BrowserMessage::Status { .. }) {
                                    &status_sender
                                } else {
                                    &sender
                                };
                                match queue.try_send((channel, generation, envelope, Instant::now())) {
                                    Ok(()) => {}
                                    Err(std::sync::mpsc::TrySendError::Full((_, _, envelope, _))) => {
                                        if !matches!(
                                            envelope.message,
                                            BrowserMessage::Status { .. }
                                        ) {
                                            diagnostics::record(&source_host, "bridge_busy", diagnostics::packet(&envelope.message));
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
pub async fn chzzk_browser_snapshot(
    app: AppHandle,
    window: Webview,
    refresh_auth: Option<bool>,
) -> ApiResult<BrowserSnapshot> {
    let result = async {
        require_main(&window)?;
        let host = host(&app)?;
        host.refresh_login_status(&app, refresh_auth.unwrap_or(false))
            .await
    }
    .await;
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
    tauri::async_runtime::spawn_blocking(move || {
        require_main(&window)?;
        host(&app)?.request_control_from_ui(&app, action)
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()))
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
            if let Some(channel) = host
                .inner
                .view
                .lock()
                .map_err(|_| unavailable())?
                .channel
                .clone()
            {
                host.remember_manual_stop(&channel);
            }
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
pub async fn chzzk_browser_delete_recordings(
    app: AppHandle,
    window: Webview,
    recording_ids: Vec<String>,
) -> ApiResult<super::browser_store::deletion::DeleteReport> {
    use super::browser_store::deletion::{self, DeleteFailure, DeleteReport};
    let prepared = (|| {
        require_main(&window)?;
        deletion::validate_ids(&recording_ids)?;
        let browser = host(&app)?;
        let replay = app
            .try_state::<super::replay::ReplayService>()
            .ok_or_else(unavailable)?
            .inner()
            .clone();
        Ok::<_, StreamError>((browser, replay))
    })();
    let (browser, replay) = match prepared {
        Ok(value) => value,
        Err(cause) => return Err::<DeleteReport, _>(cause).into(),
    };
    match tauri::async_runtime::spawn_blocking(move || {
        let mut report = DeleteReport::default();
        for id in recording_ids {
            let result = (|| {
                let job = replay.prepare_recording_delete(&id)?;
                // Long filesystem work must not stall another channel's capture.
                let result =
                    deletion::remove_files(&job).and_then(|()| replay.delete_recording_cache(&id));
                browser
                    .inner
                    .store
                    .lock()
                    .map_err(|_| unavailable())?
                    .finish_delete(&job, result)
            })();
            match result {
                Ok(()) => report.deleted_ids.push(id),
                Err(error) => report.failures.push(DeleteFailure { id, error }),
            }
        }
        report
    })
    .await
    {
        Ok(report) => Ok::<_, StreamError>(report).into(),
        Err(_) => Err::<DeleteReport, _>(unavailable()).into(),
    }
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
    #[test]
    fn source_diagnostics_retain_only_bounded_nonsecret_facts() {
        let value = json!({"reason":"ready", "installed":true, "cookie":"secret", "url":"https://example.invalid/secret",
            "sources":[{"selected":true,"sourceId":"secret","tracks":[
                {"mimeType":"video/mp4;codecs=mp4a.40.2,avc1.4D001F", "timestampOffset":-13745.920976833331,"initBytes":1225},
                {"mimeType":"https://example.invalid/secret","timestampOffset":1e30,"init":"secret"}]}]});
        let clean = sanitized_capture_diagnostics(Some(value)).unwrap();
        assert!(!clean.to_string().contains("secret"));
        assert_eq!(
            clean["sources"][0]["tracks"][0]["timestampOffset"],
            -13745.920976833331
        );
        assert_eq!(clean["sources"][0]["tracks"][1]["mimeType"], "unsupported");
        assert!(clean["sources"][0]["tracks"][1]["timestampOffset"].is_null());
        assert_eq!(
            sanitized_capture_diagnostics(Some(json!({"reason":"secret"}))).unwrap()["reason"],
            "unsupported"
        );
    }
    #[test]
    fn unsupported_original_start_releases_arm_and_preserves_parser_reason() {
        let (_root, host, request) = armed_host();
        host.inner.view.lock().unwrap().error =
            Some("원본 저장 불가: 지원하지 않는 MP4 형식".into());
        let message = serde_json::from_value(json!({"kind":"status","channelId":CHANNEL,"requestId":request,"ready":false,"recording":false,"detail":"original_unavailable"})).unwrap();
        host.process(CHANNEL, message).unwrap();
        let snapshot = host.snapshot().unwrap();
        assert_eq!(snapshot.status, "error");
        assert!(snapshot.error.unwrap().contains("MP4 형식"));
        assert!(begin(&host, &request).is_err());
        assert!(snapshot.recordings.is_empty());
    }
    #[test]
    fn input_waiting_is_an_active_session_not_an_interrupt() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        for detail in ["waiting_source", "recording"] {
            let message = serde_json::from_value(json!({"kind":"status","channelId":CHANNEL,"requestId":request,"ready":true,"recording":true,"detail":detail,"paused":true})).unwrap();
            host.process(CHANNEL, message).unwrap();
            assert_eq!(host.snapshot().unwrap().status, detail);
            assert_eq!(host.active_ids(), vec![id.clone()]);
        }
    }
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
    fn screenshot_intents_use_the_photo_lane_not_the_recording_writer() {
        let photo = BrowserMessage::ControlIntent {
            channel_id: CHANNEL.into(),
            action: ControlAction::Screenshot,
        };
        assert!(photo.screenshot_lane());
        assert!(!photo.screenshot());
        for action in [ControlAction::RecordStart, ControlAction::RecordStop] {
            assert!(!BrowserMessage::ControlIntent {
                channel_id: CHANNEL.into(),
                action
            }
            .screenshot_lane());
        }
        assert!(BrowserMessage::ScreenshotFinish {
            request_id: uuid::Uuid::new_v4().to_string()
        }
        .screenshot_lane());
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
            chat_channel_id: None,
            sequence: 0,
            last_clock: None,
            log: ChatStore::create(directory.path()).unwrap(),
            last_batch: None,
            failed_batch: false,
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
    fn viewer_only_batches_do_not_claim_chat_connection_or_inflate_chat_count() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let record = host.snapshot().unwrap().recordings.remove(0);
        {
            let mut view = host.inner.view.lock().unwrap();
            view.capture_chat = true;
            view.chat_status = "waiting_socket".into();
        }
        *host.inner.page_chat.lock().unwrap() = Some(PageChatLog {
            id: id.clone(),
            started_at: record.started_at,
            broadcast_started_at: None,
            chat_channel_id: None,
            sequence: 0,
            last_clock: None,
            log: ChatStore::create(Path::new(&record.output_dir)).unwrap(),
            last_batch: None,
            failed_batch: false,
        });
        let sample = json!({"atsumiViewerSample":1,"viewerCount":17,"replayClock":{"version":1,"receivedAtMs":record.started_at,"observedMonotonicMs":100.0,"sourceGeneration":1,"clock":"mse_presentation_v1","mediaTimeSeconds":4001.0,"sourceTimeSeconds":4001.0,"sourceId":"40000000-0000-4000-8000-000000000001","playbackRate":1.0}});
        host.process(
            CHANNEL,
            BrowserMessage::ChatBatch {
                recording_id: id,
                events: vec![sample],
                batch_id: None,
            },
        )
        .unwrap();
        let snapshot = host.snapshot().unwrap();
        assert_eq!(snapshot.chat_count, 0);
        assert_eq!(snapshot.chat_status, "waiting_socket");
        let raw =
            std::fs::read_to_string(Path::new(&record.output_dir).join("viewer-metrics.jsonl"))
                .unwrap();
        let row: Value = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(row["viewerCount"], 17);
        assert_eq!(row["replayClock"]["clock"], "player_observation");
        assert!(row["replayClock"].get("sourceTimeSeconds").is_none());
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
            chat_channel_id: None,
            sequence: 0,
            last_clock: None,
            log: ChatStore::create(Path::new(&record.output_dir)).unwrap(),
            last_batch: None,
            failed_batch: false,
        });
        let event = json!({"msgTypeCode":1,"msg":"test message","msgTime":record.started_at,"profile":{"nickname":"viewer","userIdHash":"PRIVATE_MARKER","token":"PRIVATE_MARKER"}});
        for _ in 0..8 {
            host.process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: vec![event.clone(); 32],
                    batch_id: None,
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
                batch_id: None,
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
                batch_id: None,
            },
        )
        .unwrap();
        assert_eq!(host.snapshot().unwrap().chat_status, "connection_gap");
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: vec![event.clone(); 33],
                    batch_id: None,
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
                    events: vec![event],
                    batch_id: None,
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
                    events: vec![],
                    batch_id: None,
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
    fn lost_chat_ack_retries_exact_batch_without_duplicating_messages_or_viewers() {
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
            broadcast_started_at: None,
            chat_channel_id: None,
            sequence: 0,
            last_clock: None,
            last_batch: None,
            failed_batch: false,
            log: ChatStore::create(Path::new(&record.output_dir)).unwrap(),
        });
        let events = vec![json!({"msgTypeCode":1,"msg":"once","profile":{"nickname":"test"}})];
        for _ in 0..3 {
            let response = host
                .process(
                    CHANNEL,
                    BrowserMessage::ChatBatch {
                        recording_id: id.clone(),
                        events: events.clone(),
                        batch_id: Some(1),
                    },
                )
                .unwrap();
            assert_eq!(response["count"], 1);
        }
        assert_eq!(
            std::fs::read_to_string(Path::new(&record.output_dir).join("chat.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: vec![],
                    batch_id: Some(1)
                }
            )
            .is_err());
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id.clone(),
                    events: events.clone(),
                    batch_id: Some(3)
                }
            )
            .is_err());
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::ChatBatch {
                    recording_id: id,
                    events,
                    batch_id: Some(2)
                }
            )
            .is_ok());
        assert_eq!(host.snapshot().unwrap().chat_count, 2);
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
                request_id: None,
                ready: true,
                recording: false,
                detail: "saved".into(),
                video_width: 1920,
                video_height: 1080,
                paused: false,
                capture_diagnostics: None,
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
    fn delayed_status_from_another_start_cannot_interrupt_the_current_recording() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        for nonce in [None, Some(uuid::Uuid::new_v4().to_string())] {
            host.process(
                CHANNEL,
                serde_json::from_value(json!({"kind":"status", "channelId":CHANNEL,
                "requestId":nonce,"ready":true,"recording":false,"detail":"native_rejected"}))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(host.active_ids(), vec![id.clone()]);
        }
    }
    #[test]
    fn chat_does_not_wait_for_the_video_write_reservation() {
        let (_root, host, request) = armed_host();
        let id = begin(&host, &request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let held = host.inner.writes.lock().unwrap();
        let other = host.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let result = other.process(
                CHANNEL,
                BrowserMessage::ChatStatus {
                    recording_id: id,
                    detail: "connected".into(),
                    dropped_messages: 0,
                },
            );
            let _ = send.send(result);
        });
        let result = receive.recv_timeout(Duration::from_secs(2));
        drop(held);
        worker.join().unwrap();
        assert!(result.is_ok(), "chat waited on the video writer");
    }
    #[test]
    fn renderer_abandoning_a_lost_begin_ack_releases_native_session() {
        let (_root, host, request) = armed_host();
        begin(&host, &request).unwrap();
        host.process(
            CHANNEL,
            BrowserMessage::Status {
                channel_id: CHANNEL.into(),
                request_id: Some(request.clone()),
                ready: true,
                recording: false,
                detail: "native_rejected".into(),
                video_width: 0,
                video_height: 0,
                paused: true,
                capture_diagnostics: None,
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
                request_id: Some(request.clone()),
                ready: true,
                recording: false,
                detail: "no_audio".into(),
                video_width: 0,
                video_height: 0,
                paused: true,
                capture_diagnostics: None,
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
