//! YouTube-owned original document receiver. Viewer and recorder share this child.
#[path = "original_account_observer.rs"]
mod account_observer;
#[path = "original_continuation.rs"]
mod continuation;
#[path = "original_escape_intent.rs"]
mod escape_intent;
#[path = "original_input_diagnostics.rs"]
mod input_diagnostics;
#[path = "original_policy.rs"]
mod policy;
#[path = "original_vod.rs"]
mod vod;
use crate::platform_media::{
    encoded::{EncodedAppend, EncodedArchive, EncodedTrack, OfficialReceiverLease, RawInitDecision, YouTubeRawArchive},
    live::RecordingOwner,
    MediaRef, PlatformMediaService,
};
use crate::platform_native_geometry::{self, NativeViewport};
use crate::platform_native_player_intent as player_intent;
use crate::platform_window_fullscreen::{self as window_fullscreen, Grant};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, Manager, Webview, WebviewBuilder, WebviewUrl, WebviewWindowBuilder,
};

const UNAVAILABLE: &str = "YouTube 원본 플레이어를 준비하지 못했습니다.";
const STALE: &str = "YouTube 방송 상태가 변경되었습니다. 다시 확인해 주세요.";
// Root enables only after the integrated raw writer, framing and timing gates.
const RAW_CAPTURE_ENABLED: bool = true;
const RAW_FORMAT: &str = "youtubeVp9AacRawV1";
enum Archive {
    Encoded(EncodedArchive),
    Raw(YouTubeRawArchive),
}
impl Archive {
    fn snapshot(&self) -> Result<crate::platform_media::MediaJob, String> {
        match self { Self::Encoded(archive) => archive.snapshot(), Self::Raw(archive) => archive.snapshot() }
    }
    fn is_cancelled(&self) -> bool {
        match self { Self::Encoded(archive) => archive.is_cancelled(), Self::Raw(archive) => archive.is_cancelled() }
    }
    fn is_raw(&self) -> bool { matches!(self, Self::Raw(_)) }
    fn startup_ready(&self) -> bool {
        match self { Self::Encoded(_) => true, Self::Raw(archive) => archive.startup_ready() }
    }
    fn append(&mut self, packet: EncodedAppend, boundary: Option<u64>) -> Result<(), String> {
        match self {
            Self::Encoded(archive) if boundary.is_none() => archive.append(packet).map(|_| ()),
            Self::Encoded(_) => Err(STALE.into()),
            Self::Raw(archive) => archive.append(packet, boundary).map(|_| ()),
        }
    }
    fn finish(self, error: Option<String>) -> Result<crate::platform_media::MediaJob, String> {
        match self { Self::Encoded(archive) => archive.finish(error), Self::Raw(archive) => archive.finish(error.is_some()) }
    }
    fn format_change_cancel_guard(&self) -> Option<Arc<AtomicBool>> {
        // Raw same-config init is handled within the job. A changed config is
        // explicitly interrupted until raw continuation itself is verified.
        match self { Self::Encoded(archive) => archive.format_change_cancel_guard(), Self::Raw(_) => None }
    }
    fn finish_format_change(self) -> Result<(crate::platform_media::MediaJob, bool), String> {
        match self {
            Self::Encoded(archive) => archive.finish_format_change(),
            Self::Raw(archive) => archive.finish(true).map(|job| (job, false)),
        }
    }
}
fn raw_begin_format(fields: &Value) -> Result<bool, String> {
    match fields.get("format") {
        None => Ok(false),
        Some(Value::String(format)) if format == RAW_FORMAT
            && fields["rawMode"].as_str() == Some("segments")
            && fields["rawAppendWindow"].as_str() == Some("default")
            && fields["rawVideoTimestampOffset"].as_f64() == Some(0.0)
            && fields["rawAudioTimestampOffset"].as_f64() == Some(0.0)
            && closed_fields(fields, &["requestId", "channelId", "title", "sourceId", "tracks", "format", "rawMode", "rawAppendWindow", "rawVideoTimestampOffset", "rawAudioTimestampOffset"])
            && fields["tracks"].as_array().is_some_and(|tracks| tracks.len() == 2 && tracks.iter().all(|track| closed_fields(track, &["trackIndex", "mimeType", "init"]))) => Ok(true),
        _ => Err(STALE.into()),
    }
}
fn closed_fields(fields: &Value, allowed: &[&str]) -> bool {
    fields.as_object().is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
}
fn raw_boundary(fields: &Value) -> Result<Option<u64>, String> {
    match fields.get("rawFirstUnitEnd") {
        None => Ok(None),
        Some(value) => value.as_u64().filter(|value| *value > 0 && *value <= 33_554_432).map(Some).ok_or_else(|| STALE.into()),
    }
}
#[derive(Debug)]
pub(super) enum ScheduledStartError {
    Busy,
    Other(String),
}
impl ScheduledStartError {
    fn message(self) -> String {
        match self {
            Self::Busy => "이미 녹화 중이거나 준비 중입니다.".into(),
            Self::Other(error) => error,
        }
    }
}
impl From<String> for ScheduledStartError {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}
impl From<&str> for ScheduledStartError {
    fn from(error: &str) -> Self {
        Self::Other(error.into())
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalSnapshot {
    pub token: String,
    pub channel_id: String,
    pub broadcast_id: String,
    /// Presentation-only: successful current-message live descriptor binding.
    pub canonical_channel_id: Option<String>,
    pub state: String,
    pub authenticated: Option<bool>,
    pub chat_visible: bool,
    pub capture_ready: bool,
    pub capture_unavailable_reason: Option<String>,
    pub recording_job: Option<String>,
    pub recording_stopping: bool,
    pub fullscreen: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub authenticated: Option<bool>,
    pub open: bool,
    pub error: Option<String>,
}
#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Session>>,
    tokens: HashMap<String, String>,
    requests: HashMap<String, String>,
}
struct Session {
    id: String,
    label: String,
    chat_label: String,
    channel: String,
    broadcast: String,
    canonical_channel: Mutex<Option<String>>,
    automatic_ready: AtomicBool,
    title: String,
    name: String,
    epoch: AtomicU64,
    document_id: Mutex<String>,
    raw_source_generation: AtomicU64,
    account_revision: AtomicU64,
    account_refreshing: AtomicBool,
    paint: Arc<AtomicU64>,
    closed: AtomicBool,
    viewers: Mutex<HashSet<String>>,
    ui_intent: Arc<player_intent::Intent>,
    close_enabled: AtomicBool,
    status: Mutex<OriginalSnapshot>,
    fullscreen_grant: Mutex<Option<Grant>>,
    arm: Mutex<Option<(policy::Arm, RecordingOwner)>>,
    archive: Mutex<Option<Archive>>,
    source: Mutex<Option<String>>,
    owner: Mutex<Option<RecordingOwner>>,
    continuation: Mutex<continuation::Run>,
    continuation_cancel: Mutex<Option<Arc<AtomicBool>>>,
    stable_arm: AtomicBool,
    first_append_deadline: Mutex<Option<Instant>>,
    stopping_since: Mutex<Option<Instant>>,
    lease: OfficialReceiverLease,
    viewport: Mutex<Option<NativeViewport>>,
    viewport_sequences: Mutex<HashMap<String, u64>>,
}
#[derive(Clone, Default)]
pub struct YouTubeOriginalHost {
    registry: Arc<Mutex<Registry>>,
    watch_only: vod::WatchHost,
    admission: Arc<Mutex<()>>,
    closing: Arc<AtomicBool>,
    tray_muted: Arc<AtomicBool>,
    authenticated: Arc<Mutex<policy::AccountObservation>>,
    account_flow: Arc<Mutex<policy::AccountFlow>>,
    latest_scheduled: Arc<
        Mutex<
            VecDeque<(
                crate::platform_media::live::ScheduledCapture,
                Option<String>,
            )>,
        >,
    >,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    #[serde(skip)]
    intent: Option<player_intent::Binding>,
    id: String,
    epoch: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    fields: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitChangeTrack {
    kind: u8,
    codec_family: u8,
    width: u16,
    height: u16,
    timescale: u32,
    audio_channels: u8,
    audio_sample_rate: u32,
    codec_profile: u8,
    codec_compatibility: u8,
    codec_level: u8,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitChangeShape {
    init_bytes: u32,
    tracks: Vec<InitChangeTrack>,
}
impl InitChangeShape {
    fn valid(&self) -> bool {
        (8..=65_536).contains(&self.init_bytes)
            && (1..=2).contains(&self.tracks.len())
            && !(self.tracks.len() == 2 && self.tracks[0].kind == self.tracks[1].kind)
            && self.tracks.iter().all(|track| {
                (1..=1_000_000_000).contains(&track.timescale)
                    && match track.kind {
                        1 => {
                            matches!(track.codec_family, 1 | 2)
                                && track.width > 0
                                && track.height > 0
                                && track.codec_profile > 0
                                && track.audio_channels == 0
                                && track.audio_sample_rate == 0
                        }
                        2 => {
                            track.codec_family == 3
                                && track.width == 0
                                && track.height == 0
                                && (1..=32).contains(&track.audio_channels)
                                && (1..=768_000).contains(&track.audio_sample_rate)
                                && track.codec_profile == 0
                                && track.codec_compatibility == 0
                                && track.codec_level == 0
                        }
                        _ => false,
                    }
            })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum InitChangeDropped {
    SizeLimit,
    StructureLimit,
    Unsupported,
    Invalid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitChangeDiagnostic {
    version: u8,
    track_index: u8,
    change_mask: Option<u8>,
    previous: Option<InitChangeShape>,
    next: Option<InitChangeShape>,
    dropped: Option<InitChangeDropped>,
}
fn parse_init_change(value: &Value) -> Option<InitChangeDiagnostic> {
    if !value.is_object() || value.to_string().len() > 4_096 {
        return None;
    }
    let diagnostic: InitChangeDiagnostic = serde_json::from_value(value.clone()).ok()?;
    if diagnostic.version != 1 || diagnostic.track_index >= 2 {
        return None;
    }
    let valid = if diagnostic.dropped.is_some() {
        diagnostic.change_mask.is_none()
            && diagnostic.previous.is_none()
            && diagnostic.next.is_none()
    } else {
        diagnostic.change_mask.is_some_and(|mask| mask != 0)
            && diagnostic
                .previous
                .as_ref()
                .is_some_and(InitChangeShape::valid)
            && diagnostic.next.as_ref().is_some_and(InitChangeShape::valid)
    };
    valid.then_some(diagnostic)
}
fn trace_init_change(value: &Value) {
    let Some(diagnostic) = parse_init_change(value) else {
        return;
    };
    let track_index = u64::from(diagnostic.track_index);
    if let Some(dropped) = diagnostic.dropped {
        let error_code = match dropped {
            InitChangeDropped::SizeLimit => "INIT_SIZE_LIMIT",
            InitChangeDropped::StructureLimit => "INIT_STRUCTURE_LIMIT",
            InitChangeDropped::Unsupported => "INIT_SHAPE_UNSUPPORTED",
            InitChangeDropped::Invalid => "INIT_SHAPE_INVALID",
        };
        tracing::info!(
            diag_stage = "youtube_init_change_dropped",
            track_index,
            error_code
        );
        return;
    }
    for (previous, shape) in [(true, diagnostic.previous), (false, diagnostic.next)] {
        let Some(shape) = shape else {
            continue;
        };
        for track in shape.tracks {
            let stage = match (previous, track.kind) {
                (true, 1) => "youtube_init_previous_video",
                (true, _) => "youtube_init_previous_audio",
                (false, 1) => "youtube_init_next_video",
                (false, _) => "youtube_init_next_audio",
            };
            tracing::info!(
                diag_stage = stage,
                track_index,
                init_bytes = u64::from(shape.init_bytes),
                width = u64::from(track.width),
                height = u64::from(track.height),
                timescale = u64::from(track.timescale),
                audio_channels = u64::from(track.audio_channels),
                audio_sample_rate = u64::from(track.audio_sample_rate),
                codec_family = u64::from(track.codec_family),
                codec_profile = u64::from(track.codec_profile),
                codec_compatibility = u64::from(track.codec_compatibility),
                codec_level = u64::from(track.codec_level)
            );
        }
    }
    tracing::info!(
        diag_stage = "youtube_init_change",
        track_index,
        init_change_mask = u64::from(diagnostic.change_mask.unwrap_or(0))
    );
}
#[cfg(test)]
mod init_change_diagnostic_tests {
    use super::*;
    fn sample() -> Value {
        let shape = json!({"initBytes":420,"tracks":[{"kind":1,"codecFamily":1,"width":1280,"height":720,"timescale":90000,
            "audioChannels":0,"audioSampleRate":0,"codecProfile":100,"codecCompatibility":0,"codecLevel":31}]});
        json!({"version":1,"trackIndex":0,"changeMask":2,"previous":shape,"next":shape})
    }
    #[test]
    fn allows_only_bounded_numeric_shape_and_static_drop() {
        assert!(parse_init_change(&sample()).is_some());
        assert!(parse_init_change(
            &json!({"version":1,"trackIndex":1,"dropped":"structure_limit"})
        )
        .is_some());
        let mut value = sample();
        value["next"]["tracks"][0] = json!({"kind":2,"codecFamily":3,"width":0,"height":0,"timescale":48000,
            "audioChannels":2,"audioSampleRate":48000,"codecProfile":0,"codecCompatibility":0,"codecLevel":0});
        assert!(parse_init_change(&value).is_some());
    }
    #[test]
    fn rejects_unknown_fields_payloads_and_non_integer_values() {
        for (pointer, value) in [
            ("/version", json!(2)),
            ("/trackIndex", json!(2)),
            ("/changeMask", json!(256)),
            ("/previous/initBytes", json!(65_537)),
            ("/previous/tracks/0/timescale", json!(1_000_000_001_u64)),
            ("/previous/tracks/0/timescale", json!(90000.0)),
            ("/previous/tracks/0/codecFamily", json!(4)),
            ("/previous/tracks/0/codecLevel", json!(-1)),
            ("/previous/tracks/0/width", json!(0)),
        ] {
            let mut candidate = sample();
            *candidate.pointer_mut(pointer).unwrap() = value;
            assert!(parse_init_change(&candidate).is_none(), "{pointer}");
        }
        for pointer in ["", "/previous", "/previous/tracks/0"] {
            let mut candidate = sample();
            candidate
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("rawBytes".into(), json!("private-payload"));
            assert!(parse_init_change(&candidate).is_none());
        }
        let mut candidate = sample();
        candidate["previous"]["tracks"] = json!([]);
        assert!(parse_init_change(&candidate).is_none());
        let mut candidate = sample();
        let track = candidate["previous"]["tracks"][0].clone();
        candidate["previous"]["tracks"] = json!([track, track, track]);
        assert!(parse_init_change(&candidate).is_none());
        let mut candidate = sample();
        candidate["dropped"] = json!("invalid");
        assert!(parse_init_change(&candidate).is_none());
        assert!(
            parse_init_change(&json!({"version":1,"trackIndex":0,"dropped":"private-url"}))
                .is_none()
        );
    }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
fn recording_approval<'a, T>(
    approval: &'a Mutex<Option<T>>,
    closed: &AtomicBool,
    refreshing: &AtomicBool,
    ready: impl FnOnce() -> bool,
) -> Result<std::sync::MutexGuard<'a, Option<T>>, String> {
    let guard = lock(approval);
    if closed.load(Ordering::Acquire) || refreshing.load(Ordering::Acquire) || !ready() {
        return Err("원본 영상이 준비된 뒤 녹화를 시작해 주세요.".into());
    }
    Ok(guard)
}
#[cfg(test)]
mod account_recording_tests {
    use super::*;
    #[test]
    fn account_refresh_blocks_delayed_recording_even_if_old_status_reports_ready() {
        let approval = Arc::new(Mutex::new(None::<()>));
        let ready = Arc::new(AtomicBool::new(true));
        let refreshing = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let refresh_gate = lock(&approval);
        let (tx, rx) = std::sync::mpsc::channel();
        let a = approval.clone();
        let r = ready.clone();
        let p = refreshing.clone();
        let c = closed.clone();
        let worker = std::thread::spawn(move || {
            assert!(r.load(Ordering::Acquire));
            tx.send(()).unwrap();
            recording_approval(&a, &c, &p, || r.load(Ordering::Acquire)).is_err()
        });
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        refreshing.store(true, Ordering::Release);
        ready.store(false, Ordering::Release);
        drop(refresh_gate);
        assert!(worker.join().unwrap());
        ready.store(true, Ordering::Release); // Old page readiness alone cannot reopen admission.
        assert!(recording_approval(&approval, &closed, &refreshing, || ready
            .load(Ordering::Acquire))
        .is_err());
        refreshing.store(false, Ordering::Release);
        ready.store(false, Ordering::Release);
        assert!(recording_approval(&approval, &closed, &refreshing, || ready
            .load(Ordering::Acquire))
        .is_err());
        ready.store(true, Ordering::Release);
        assert!(recording_approval(&approval, &closed, &refreshing, || ready
            .load(Ordering::Acquire))
        .is_ok());
        closed.store(true, Ordering::Release);
        assert!(recording_approval(&approval, &closed, &refreshing, || true).is_err());
    }
}
fn profile(app: &AppHandle) -> Result<PathBuf, String> {
    crate::browser_identity::profile(app).map_err(|_| UNAVAILABLE.into())
}
impl YouTubeOriginalHost {
    pub fn open_vod(
        &self,
        app: &AppHandle,
        video: String,
        request_id: String,
    ) -> Result<OriginalSnapshot, String> {
        if self.closing.load(Ordering::Acquire)
            || !policy::valid_channel(&video)
            || uuid::Uuid::parse_str(&request_id).is_err()
        {
            return Err(STALE.into());
        }
        let _admission = lock(&self.admission);
        if self.closing.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        if let Some(value) = self.watch_only.existing(&request_id, &video)? {
            return Ok(value);
        }
        let registry = lock(&self.registry);
        if registry.requests.contains_key(&request_id)
            || registry.sessions.values().any(|s| s.channel == video)
        {
            return Err("이 영상은 이미 열려 있습니다.".into());
        }
        // A reusable VOD may become blocked by SourceChanged before open.
        // Reserve permission for a new document independently of alias reuse.
        let can_create_document = registry.sessions.len() + self.watch_only.count() < 4;
        drop(registry);
        self.watch_only.open(app, self.clone(), video, request_id, can_create_document)
    }
    pub fn open(
        &self,
        app: &AppHandle,
        media: PlatformMediaService,
        channel: String,
        request_id: String,
        download_root: PathBuf,
    ) -> Result<OriginalSnapshot, String> {
        if self.closing.load(Ordering::Acquire)
            || !policy::valid_channel(&channel)
            || uuid::Uuid::parse_str(&request_id).is_err()
        {
            return Err(STALE.into());
        }
        // YouTube video IDs are case-sensitive; never normalize their case.
        {
            let registry = lock(&self.registry);
            if let Some(token) = registry.requests.get(&request_id) {
                return self.snapshot_locked(&registry, token);
            }
        }
        let info = super::metadata(super::video_target(&channel))?;
        if info.is_live == Some(false) {
            return Err("현재 라이브가 아닙니다. 영상 상세에서 재생해 주세요.".into());
        }
        let broadcast = channel.clone();
        // Official player, not the anonymous extractor, enforces account/age/paid restrictions.
        let _admission = lock(&self.admission);
        if self.watch_only.contains_video(&channel) {
            return Err("이 영상은 이미 열려 있습니다.".into());
        }
        let mut registry = lock(&self.registry);
        if self.closing.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        // Metadata preparation happens outside the registry lock. Concurrent
        // retries must still consume a request UUID only once.
        if let Some(token) = registry.requests.get(&request_id) {
            return self.snapshot_locked(&registry, token);
        }
        let existing = registry
            .sessions
            .values()
            .find(|s| {
                s.channel == channel
                    && s.broadcast == broadcast
                    && !s.closed.load(Ordering::Acquire)
            })
            .cloned();
        let token = uuid::Uuid::new_v4().to_string();
        let session = if let Some(session) = existing {
            session
        } else {
            if registry.sessions.len() + self.watch_only.count() >= 4 {
                return Err("YouTube 방송은 최대 4개까지 열 수 있습니다.".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            let lease = media.reserve_official_receiver(
                MediaRef {
                    provider: "youtube".into(),
                    kind: "live".into(),
                    external_id: channel.clone(),
                },
                broadcast.clone(),
                id.clone(),
                download_root,
            )?;
            let session = Arc::new(Session {
                id: id.clone(),
                label: format!("youtube-original-{}", id.replace('-', "")),
                chat_label: format!("youtube-original-chat-{}", id.replace('-', "")),
                channel: channel.clone(),
                broadcast: broadcast.clone(),
                canonical_channel: Mutex::new(None),
                automatic_ready: AtomicBool::new(false),
                title: info.title,
                name: info.author,
                epoch: AtomicU64::new(0),
                document_id: Mutex::new(uuid::Uuid::new_v4().to_string()),
                raw_source_generation: AtomicU64::new(0),
                account_revision: AtomicU64::new(lock(&self.authenticated).revision),
                account_refreshing: AtomicBool::new(false),
                paint: Arc::new(AtomicU64::new(0)),
                closed: AtomicBool::new(false),
                viewers: Mutex::new(HashSet::from([token.clone()])),
                ui_intent: Arc::new(player_intent::Intent::default()),
                close_enabled: AtomicBool::new(false),
                status: Mutex::new(OriginalSnapshot {
                    token: String::new(),
                    channel_id: channel.clone(),
                    broadcast_id: broadcast,
                    canonical_channel_id: None,
                    state: "loading".into(),
                    authenticated: None,
                    chat_visible: false,
                    capture_ready: false,
                    capture_unavailable_reason: None,
                    recording_job: None,
                    recording_stopping: false,
                    fullscreen: false,
                    error: None,
                }),
                fullscreen_grant: Mutex::new(None),
                arm: Mutex::new(None),
                archive: Mutex::new(None),
                source: Mutex::new(None),
                owner: Mutex::new(None),
                continuation: Mutex::new(continuation::Run::default()),
                continuation_cancel: Mutex::new(None),
                stable_arm: AtomicBool::new(false),
                first_append_deadline: Mutex::new(None),
                stopping_since: Mutex::new(None),
                lease,
                viewport: Mutex::new(None),
                viewport_sequences: Mutex::new(HashMap::new()),
            });
            registry.sessions.insert(id, session.clone());
            if let Err(error) = self.create_view(app, &session) {
                registry.sessions.remove(&session.id);
                if let Some(view) = app.get_webview(&session.label) {
                    let _ = view.close();
                }
                if let Some(chat) = app.get_webview(&session.chat_label) {
                    let _ = chat.close();
                }
                session.closed.store(true, Ordering::Release);
                session.lease.close();
                return Err(error);
            }
            session
        };
        lock(&session.viewers).insert(token.clone());
        registry.tokens.insert(token.clone(), session.id.clone());
        registry.requests.insert(request_id, token.clone());
        let mut result = lock(&session.status).clone();
        result.token = token;
        Ok(result)
    }
    fn snapshot_locked(
        &self,
        registry: &Registry,
        token: &str,
    ) -> Result<OriginalSnapshot, String> {
        let session = registry
            .tokens
            .get(token)
            .and_then(|id| registry.sessions.get(id))
            .ok_or(STALE)?;
        let mut result = lock(&session.status).clone();
        result.token = token.into();
        Ok(result)
    }
    fn session(&self, token: &str) -> Result<Arc<Session>, String> {
        let registry = lock(&self.registry);
        registry
            .tokens
            .get(token)
            .and_then(|id| registry.sessions.get(id))
            .cloned()
            .ok_or_else(|| STALE.into())
    }
    pub fn snapshot(&self, token: &str) -> Result<OriginalSnapshot, String> {
        if vod::is_token(token) {
            return self.watch_only.snapshot(token);
        }
        self.snapshot_locked(&lock(&self.registry), token)
    }
    pub fn fullscreen(&self, app: &AppHandle, token: &str, enabled: bool) -> Result<(), String> {
        if vod::is_token(token) {
            return self.watch_only.fullscreen(app, token, enabled);
        }
        let session = self.session(token)?;
        set_fullscreen(app, &session, enabled)
    }
    pub fn screenshot(
        &self,
        app: &AppHandle,
        token: &str,
        request_id: &str,
    ) -> Result<crate::platform_media::screenshots::Item, String> {
        let session = self.session(token)?;
        let permitted = || {
            !session.closed.load(Ordering::Acquire)
                && !self.tray_muted.load(Ordering::Acquire)
                && lock(&session.viewport)
                    .as_ref()
                    .is_some_and(|v| v.visible && !v.muted && v.player.is_some())
        };
        if !permitted() || uuid::Uuid::parse_str(request_id).is_err() {
            return Err("현재 표시 중인 영상만 캡처할 수 있습니다.".into());
        }
        let epoch = session.epoch.load(Ordering::Acquire);
        let view = app.get_webview(&session.label).ok_or(UNAVAILABLE)?;
        let url = view.url().map_err(|_| UNAVAILABLE)?;
        if !policy::matches_document(&url, &session.channel, false) {
            return Err(STALE.into());
        }
        let png = crate::platform_native_capture::video_png(
            &view,
            url.as_str(),
            crate::platform_native_capture::VideoSurface::YouTube,
        )?;
        if !permitted() || epoch != session.epoch.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        session
            .lease
            .save_screenshot(request_id, &png, &session.title, &session.name)
    }
    pub fn undo_screenshot(&self, token: &str, id: &str) -> Result<(), String> {
        self.session(token)?.lease.undo_screenshot(id)
    }
    /// Preparation waits outside the scheduler's admission/core lock; a watcher token
    /// reuses the existing exact-video receiver and never takes ownership of manual capture.
    pub fn prepare_scheduled(
        &self,
        app: &AppHandle,
        media: PlatformMediaService,
        channel: String,
        video: &str,
        request: String,
        root: PathBuf,
    ) -> Result<String, String> {
        if !valid_canonical_channel(&channel) || !policy::valid_channel(video) {
            return Err(STALE.into());
        }
        let opened = self.open(app, media, video.into(), request, root)?;
        let session = self.session(&opened.token)?;
        let until = Instant::now() + Duration::from_secs(45);
        while Instant::now() < until && !self.closing.load(Ordering::Acquire) {
            let status = lock(&session.status).clone();
            let observed = lock(&session.canonical_channel).clone();
            if observed.as_ref().is_some_and(|value| value != &channel) {
                let _ = self.close(app, &opened.token);
                return Err("예약한 채널과 실제 방송 채널이 다릅니다.".into());
            }
            if status.capture_ready
                && session.automatic_ready.load(Ordering::Acquire)
                && observed.as_deref() == Some(&channel)
            {
                return Ok(opened.token);
            }
            if session.closed.load(Ordering::Acquire)
                || status.state == "ended"
                || status.error.is_some()
            {
                let _ = self.close(app, &opened.token);
                return Err(status
                    .error
                    .unwrap_or_else(|| "방송이 종료되었거나 변경되었습니다.".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.close(app, &opened.token);
        Err("YouTube 원본 영상과 채팅 준비 시간이 초과되었습니다.".into())
    }
    /// Caller must not hold scheduler/core admission locks while waiting for native ACK.
    pub(super) fn start_scheduled(
        &self,
        app: &AppHandle,
        token: &str,
        owner: RecordingOwner,
    ) -> Result<crate::platform_media::live::ScheduledCapture, ScheduledStartError> {
        let session = self.session(token)?;
        if !scheduled_owner_matches(
            &owner,
            lock(&session.canonical_channel).as_deref(),
            &session.broadcast,
        ) {
            return Err(STALE.into());
        }
        self.arm_typed(app, &session, owner.clone())?;
        let until = Instant::now() + Duration::from_secs(3);
        let mut failure = None;
        while Instant::now() < until && !self.closing.load(Ordering::Acquire) {
            let status = lock(&session.status).clone();
            if let Some(job_id) = status.recording_job {
                if lock(&session.owner).as_ref() == Some(&owner) {
                    return Ok(crate::platform_media::live::ScheduledCapture {
                        session_id: session.id.clone(),
                        job_id,
                        owner,
                    });
                }
                failure = Some("다른 녹화 작업이 진행 중입니다.".into());
                break;
            }
            if status.error.is_some() {
                failure = status.error;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.cancel_scheduled_start(app, &session, &owner);
        Err(ScheduledStartError::Other(failure.unwrap_or_else(|| {
            "예약 녹화 시작 응답이 지연되었습니다.".into()
        })))
    }
    fn cancel_scheduled_start(
        &self,
        app: &AppHandle,
        session: &Arc<Session>,
        owner: &RecordingOwner,
    ) {
        let mut arm = lock(&session.arm);
        lock(&session.continuation).cancel(Some(owner));
        if arm.as_ref().is_some_and(|(_, current)| current == owner) {
            arm.take();
        }
        if lock(&session.owner).as_ref() == Some(owner) {
            let job = lock(&session.status).recording_job.clone();
            if let Some(job) = job {
                let _ = stop_exact_recording(app, session, &job);
            }
        }
    }
    pub fn stop_scheduled(
        &self,
        app: &AppHandle,
        capture: &crate::platform_media::live::ScheduledCapture,
    ) -> Result<(), String> {
        let session = {
            lock(&self.registry)
                .sessions
                .get(&capture.session_id)
                .cloned()
        };
        if let Some(session) = session {
            let mut arm = lock(&session.arm);
            if lock(&session.owner).as_ref() == Some(&capture.owner)
                || arm
                    .as_ref()
                    .is_some_and(|(_, owner)| owner == &capture.owner)
            {
                lock(&session.continuation).cancel(Some(&capture.owner));
                if let Some((pending, owner)) = arm.as_ref() {
                    if owner == &capture.owner {
                        cancel_pending(app, &session, &pending.id);
                        arm.take();
                    }
                }
                let job = lock(&session.status).recording_job.clone();
                if let Some(job) = job {
                    stop_exact_recording(app, &session, &job)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn scheduled_poll(
        &self,
        capture: &crate::platform_media::live::ScheduledCapture,
        media: &PlatformMediaService,
    ) -> Result<
        (
            crate::platform_media::live::ScheduledCapture,
            crate::platform_media::live::ScheduledState,
        ),
        String,
    > {
        let session = lock(&self.registry)
            .sessions
            .get(&capture.session_id)
            .cloned();
        // Hold the same transition gate through the media snapshot. Otherwise
        // old finalization can race this query and hide an active successor.
        let arm = session.as_ref().map(|session| lock(&session.arm));
        let (pending, error) = if let Some(session) = session.as_ref() {
            if lock(&session.owner).as_ref() == Some(&capture.owner) {
                (
                    arm.as_ref().is_some_and(|arm| {
                        arm.as_ref()
                            .is_some_and(|(_, owner)| owner == &capture.owner)
                    }),
                    lock(&session.status).error.clone(),
                )
            } else {
                (false, None)
            }
        } else {
            (false, None)
        };
        let (current, terminal_error) = lock(&self.latest_scheduled)
            .iter()
            .find(|(current, _)| {
                current.session_id == capture.session_id && current.owner == capture.owner
            })
            .cloned()
            .ok_or("예약 녹화의 현재 파일 소유권을 확인하지 못했습니다.")?;
        let mut state = media.encoded_scheduled_state(&current)?;
        if pending {
            state.active = true;
            state.state = "starting".into();
            state.error = None;
        } else if let Some(error) = error.or(terminal_error) {
            state.error = Some(error);
        }
        Ok((current, state))
    }
    fn remember_scheduled(&self, session: &Session, job_id: &str, owner: &RecordingOwner) {
        if !matches!(owner, RecordingOwner::Scheduled { .. }) {
            return;
        }
        let mut latest = lock(&self.latest_scheduled);
        latest.retain(|(capture, _)| capture.session_id != session.id || capture.owner != *owner);
        if latest.len() >= 256 {
            latest.pop_front();
        }
        latest.push_back((
            crate::platform_media::live::ScheduledCapture {
                session_id: session.id.clone(),
                job_id: job_id.into(),
                owner: owner.clone(),
            },
            None,
        ));
    }
    fn remember_terminal(&self, session: &Session) {
        let owner = lock(&session.owner).clone();
        let error = lock(&session.status).error.clone();
        if let (Some(owner), Some(error)) = (owner, error) {
            if let Some((_, stored)) = lock(&self.latest_scheduled)
                .iter_mut()
                .find(|(capture, _)| capture.session_id == session.id && capture.owner == owner)
            {
                *stored = Some(error);
            }
        }
    }
    pub fn viewport(
        &self,
        app: &AppHandle,
        token: &str,
        mut viewport: NativeViewport,
    ) -> Result<(), String> {
        if vod::is_token(token) {
            return self.watch_only.viewport(
                app,
                token,
                viewport,
                self.tray_muted.load(Ordering::Acquire),
            );
        }
        viewport.validate()?;
        let session = self.session(token)?;
        if session.closed.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        let mut sequences = lock(&session.viewport_sequences);
        if !lock(&session.viewers).contains(token) {
            return Err(STALE.into());
        }
        let last = sequences.entry(token.into()).or_insert(0);
        if !policy::sequence_newer(*last, viewport.sequence) {
            return Ok(());
        }
        *last = viewport.sequence;
        session.ui_intent.present(token, session.epoch.load(Ordering::Acquire), &viewport);
        // Store the user's mute intent, not the tray-forced result.
        *lock(&session.viewport) = Some(viewport.clone());
        viewport.muted |= self.tray_muted.load(Ordering::Acquire);
        let result = paint_session(app, &session, viewport);
        drop(sequences);
        result
    }
    pub fn record(
        &self,
        app: &AppHandle,
        token: &str,
        enabled: bool,
    ) -> Result<OriginalSnapshot, String> {
        if vod::is_token(token) {
            return Err("일반 영상은 상세보기의 다운로드를 사용해 주세요.".into());
        }
        let session = self.session(token)?;
        if enabled {
            self.arm(app, &session, RecordingOwner::Manual)?;
        } else {
            self.stop_recording(app, &session)?;
        }
        self.snapshot(token)
    }
    pub fn controls(&self, app:&AppHandle, token:&str, can_close:bool, capture_busy:bool)->Result<(),String>{
        let session=self.session(token)?;
        let Some(binding)=ui_binding(&session) else{return Ok(());};
        if binding.token!=token{return Ok(());}
        session.close_enabled.store(can_close,Ordering::Release);
        let status=lock(&session.status).clone();
        let pending=lock(&session.arm).is_some();
        app.get_webview(&session.label).ok_or(UNAVAILABLE)?.eval(&format!("window.__atsumiYouTubeOriginal?.controls({})",json!({"epoch":binding.epoch,"canClose":can_close,"captureBusy":capture_busy,"canRecord":status.capture_ready,"captureReason":status.capture_unavailable_reason,"recording":status.recording_job.is_some(),"recordBusy":pending||status.recording_stopping}))).map_err(|_|UNAVAILABLE.into())
    }
    pub fn resume(&self,app:&AppHandle,token:&str)->Result<(),String>{
        let session=self.session(token)?;let binding=ui_binding(&session).filter(|b|b.token==token).ok_or(STALE)?;
        let weak=Arc::downgrade(&session);let tray=self.tray_muted.clone();let app2=app.clone();
        app.run_on_main_thread(move||{if let Some(session)=weak.upgrade(){if !tray.load(Ordering::Acquire)&&ui_binding(&session).as_ref()==Some(&binding){if let Some(view)=app2.get_webview(&session.label){let _=view.eval(&format!("window.__atsumiYouTubeOriginal?.resume({})",binding.epoch));}}}}).map_err(|_|UNAVAILABLE.into())
    }
    fn arm(
        &self,
        app: &AppHandle,
        session: &Arc<Session>,
        owner: RecordingOwner,
    ) -> Result<(), String> {
        self.arm_typed(app, session, owner)
            .map_err(ScheduledStartError::message)
    }
    fn arm_typed(
        &self,
        app: &AppHandle,
        session: &Arc<Session>,
        owner: RecordingOwner,
    ) -> Result<(), ScheduledStartError> {
        if session.lease.youtube_chat_preparing() {
            return Err("녹화 채팅 연결을 준비하고 있습니다. 잠시 후 다시 시도해 주세요.".into());
        }
        let automatic = matches!(owner, RecordingOwner::Scheduled { .. });
        let mut approval = recording_approval(
            &session.arm,
            &session.closed,
            &session.account_refreshing,
            || {
                lock(&session.status).capture_ready
                    && (!automatic || session.automatic_ready.load(Ordering::Acquire))
            },
        )?;
        if lock(&session.archive).is_some() || approval.is_some() {
            return Err(ScheduledStartError::Busy);
        }
        // A later manual start must not erase the previous reservation's
        // terminal continuation error before its scheduler observes it.
        self.remember_terminal(session);
        let arm = policy::Arm {
            id: uuid::Uuid::new_v4().to_string(),
            epoch: session.epoch.load(Ordering::Acquire),
            created: Instant::now(),
        };
        let id = arm.id.clone();
        lock(&session.continuation).start(owner.clone(), arm.epoch);
        *lock(&session.continuation_cancel) = None;
        session.stable_arm.store(automatic, Ordering::Release);
        *lock(&session.owner) = Some(owner.clone());
        *approval = Some((arm, owner));
        drop(approval);
        lock(&session.status).error = None;
        app.get_webview(&session.label)
            .ok_or(UNAVAILABLE)?
            .eval(&format!(
                "window.__atsumiYouTubeOriginal?.start({}, {})",
                json!(id),
                automatic
            ))
            .map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
    fn stop_recording(&self, app: &AppHandle, session: &Arc<Session>) -> Result<(), String> {
        let mut arm = lock(&session.arm);
        lock(&session.continuation).cancel(None);
        if let Some((pending, _)) = arm.take() {
            cancel_pending(app, session, &pending.id);
        }
        if lock(&session.archive).is_none() {
            return Ok(());
        }
        let job = lock(&session.status).recording_job.clone();
        if let Some(job) = job {
            stop_exact_recording(app, session, &job)?;
        }
        Ok(())
    }
    pub fn close(&self, app: &AppHandle, token: &str) -> Result<(), String> {
        if vod::is_token(token) {
            let _admission = lock(&self.admission);
            return self.watch_only.close(app, token);
        }
        let session = {
            let mut registry = lock(&self.registry);
            let Some(id) = registry.tokens.remove(token) else {
                return Ok(());
            };
            registry.requests.retain(|_, value| value != token);
            registry.sessions.get(&id).cloned()
        };
        if let Some(session) = session {
            session.ui_intent.hide(token);
            lock(&session.viewers).remove(token);
            lock(&session.viewport_sequences).remove(token);
            if lock(&session.viewers).is_empty() {
                if lock(&session.status).fullscreen {
                    let _ = set_fullscreen(app, &session, false);
                }
                *lock(&session.viewport) = Some(hidden_viewport());
                let _ = paint_session(app, &session, hidden_viewport());
                if lock(&session.archive).is_none() && lock(&session.arm).is_none() {
                    self.dispose(app, &session);
                }
            }
        }
        Ok(())
    }
    fn dispose(&self, app: &AppHandle, session: &Arc<Session>) {
        if session.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.remember_terminal(session);
        session.paint.fetch_add(1, Ordering::AcqRel);
        if lock(&session.status).fullscreen {
            let _ = set_fullscreen(app, session, false);
        }
        if let Some(view) = app.get_webview(&session.label) {
            let _ = view.close();
        }
        if let Some(chat) = app.get_webview(&session.chat_label) {
            let _ = chat.close();
        }
        session.lease.close();
        let mut registry = lock(&self.registry);
        registry.sessions.remove(&session.id);
        registry.tokens.retain(|_, id| id != &session.id);
        let remaining = registry.tokens.keys().cloned().collect::<HashSet<_>>();
        registry
            .requests
            .retain(|_, token| remaining.contains(token));
    }
    pub fn account(&self, app: &AppHandle) -> AccountSnapshot {
        let registry = lock(&self.registry);
        let states = registry
            .sessions
            .values()
            .map(|s| lock(&s.status).authenticated)
            .collect::<Vec<_>>();
        let known = lock(&self.authenticated).value;
        AccountSnapshot {
            authenticated: if known.is_some() {
                known
            } else if states.contains(&Some(true)) {
                Some(true)
            } else if states.contains(&Some(false)) {
                Some(false)
            } else {
                None
            },
            open: app.get_webview_window("youtube-original-account").is_some(),
            error: None,
        }
    }
    pub fn login(&self, app: &AppHandle) -> Result<AccountSnapshot, String> {
        self.account_window(app, false)
    }
    pub fn logout(&self, app: &AppHandle) -> Result<AccountSnapshot, String> {
        if lock(&self.registry)
            .sessions
            .values()
            .any(|s| lock(&s.archive).is_some() || lock(&s.arm).is_some())
        {
            return Err("녹화를 마친 뒤 로그아웃해 주세요.".into());
        }
        self.account_window(app, true)
    }
    fn account_window(&self, app: &AppHandle, sign_out: bool) -> Result<AccountSnapshot, String> {
        if self.closing.load(Ordering::Acquire) {
            return Err(UNAVAILABLE.into());
        }
        let url = policy::account_entry(sign_out);
        if let Some(window) = app.get_webview_window("youtube-original-account") {
            let nonce = lock(&self.account_flow).nonce().ok_or(UNAVAILABLE)?;
            let changed = lock(&self.account_flow)
                .intent(&nonce, sign_out)
                .ok_or(UNAVAILABLE)?;
            // Reopening the same intent must not discard a challenge in progress.
            if changed {
                window.navigate(url).map_err(|_| UNAVAILABLE)?;
            } else if window.url().ok().as_ref().is_some_and(policy::account_home) {
                self.probe_account_window(app, window.clone(), nonce);
            }
            window.show().map_err(|_| UNAVAILABLE)?;
            window.set_focus().map_err(|_| UNAVAILABLE)?;
            return Ok(self.account(app));
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        lock(&self.account_flow).begin(nonce.clone(), sign_out);
        let account_host = self.clone();
        let page_nonce = nonce.clone();
        let navigation_host = self.clone();
        let navigation_nonce = nonce.clone();
        let popup_host = self.clone();
        let popup_nonce = nonce.clone();
        let window =
            WebviewWindowBuilder::new(app, "youtube-original-account", WebviewUrl::External(url))
                .title("YouTube 로그인 · Google 보안 확인은 직접 진행해 주세요")
                .on_page_load(move |view, payload| {
                    if lock(&account_host.account_flow).diagnostic(&page_nonce) {
                        tracing::info!(
                            diag_stage = "youtube_account_page_load",
                            account_step = policy::account_stage(payload.url()),
                            finished =
                                matches!(payload.event(), tauri::webview::PageLoadEvent::Finished),
                            completion_candidate = policy::account_home(payload.url()),
                            "YouTube account page state"
                        );
                    }
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                        lock(&account_host.account_flow).navigate(&page_nonce);
                        return;
                    }
                    if policy::account_home(payload.url())
                        && view.url().ok().as_ref() == Some(payload.url())
                    {
                        if let Some(window) = view
                            .app_handle()
                            .get_webview_window("youtube-original-account")
                        {
                            account_host.probe_account_window(
                                view.app_handle(),
                                window,
                                page_nonce.clone(),
                            );
                        }
                    }
                })
                .data_directory(profile(app)?)
                .browser_extensions_enabled(false)
                .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
                .inner_size(720.0, 780.0)
                .min_inner_size(400.0, 450.0)
                .on_navigation(move |url| {
                    let mut flow = lock(&navigation_host.account_flow);
                    let allowed = flow.navigate_account_window(&navigation_nonce, url);
                    if flow.diagnostic(&navigation_nonce) {
                        tracing::info!(
                            diag_stage = "youtube_account_navigation",
                            account_step = policy::account_stage(url),
                            allowed,
                            "YouTube account navigation"
                        );
                    }
                    allowed
                })
                .on_new_window(move |url, _| {
                    let mut flow = lock(&popup_host.account_flow);
                    if let Some(stage) = flow.popup_diagnostic(&popup_nonce, &url) {
                        tracing::info!(
                            diag_stage = "youtube_account_popup_denied",
                            account_step = stage,
                            allowed = false,
                            "YouTube account popup denied"
                        );
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .on_download(|_, _| false)
                .build()
                .map_err(|_| UNAVAILABLE)?;
        let destroyed = self.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                destroyed.account_probe_diagnostic(
                    &nonce,
                    account_observer::Diagnostic::WindowDestroyed,
                );
                lock(&destroyed.account_flow).invalidate(&nonce);
            }
        });
        Ok(self.account(app))
    }
    fn account_probe_diagnostic(&self, nonce: &str, event: account_observer::Diagnostic) {
        if lock(&self.account_flow).diagnostic(nonce) {
            tracing::info!(diag_stage = event.stage(), "YouTube account observer state");
        }
    }
    fn account_probe_active(
        &self,
        window: &tauri::WebviewWindow,
        nonce: &str,
        epoch: u64,
        started: Instant,
        attempt: u16,
    ) -> bool {
        if self.closing.load(Ordering::Acquire) {
            return false;
        }
        let active =
            lock(&self.account_flow).probe_active(nonce, epoch, started, attempt, Instant::now());
        active && window.url().ok().as_ref().is_some_and(policy::account_home)
    }
    fn account_probe_wait_diagnostic(
        &self,
        nonce: &str,
        failure: account_observer::WaitFailure,
        callback: bool,
    ) {
        use account_observer::{Diagnostic, WaitFailure};
        self.account_probe_diagnostic(
            nonce,
            match failure {
                WaitFailure::Deadline => Diagnostic::Deadline,
                WaitFailure::Invalidated => Diagnostic::Invalidated,
                WaitFailure::Disconnected if callback => Diagnostic::CallbackLost,
                WaitFailure::Disconnected => Diagnostic::ActionLost,
            },
        );
    }
    fn probe_account_window(&self, app: &AppHandle, window: tauri::WebviewWindow, nonce: String) {
        let Some(epoch) = lock(&self.account_flow).start_probe(&nonce) else {
            return;
        };
        let host = self.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let deadline = started + account_observer::DEADLINE;
            for attempt in 0..policy::ACCOUNT_PROBE_LIMIT {
                if !host.account_probe_active(&window, &nonce, epoch, started, attempt) {
                    host.account_probe_diagnostic(
                        &nonce,
                        if Instant::now() >= deadline {
                            account_observer::Diagnostic::Deadline
                        } else {
                            account_observer::Diagnostic::Invalidated
                        },
                    );
                    break;
                }
                // Only this issued request is awaited, even when its callback is slow.
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                let issued_at = Instant::now();
                if window
                    .eval_with_callback(include_str!("original_account.js"), move |value| {
                        let _ = tx.send(value);
                    })
                    .is_err()
                {
                    host.account_probe_diagnostic(&nonce, account_observer::Diagnostic::EvalFailed);
                    break;
                }
                let raw = match account_observer::receive(&rx, deadline, || {
                    host.account_probe_active(&window, &nonce, epoch, started, attempt)
                }) {
                    Ok(raw) => raw,
                    Err(failure) => {
                        host.account_probe_wait_diagnostic(&nonce, failure, true);
                        break;
                    }
                };
                let observation = if raw.len() <= 4096 {
                    serde_json::from_str::<Value>(&raw).ok()
                } else {
                    None
                };
                let Some(observation) = observation else {
                    host.account_probe_diagnostic(
                        &nonce,
                        account_observer::Diagnostic::InvalidResponse,
                    );
                    account_observer::pause(deadline, || {
                        host.account_probe_active(&window, &nonce, epoch, started, attempt)
                    });
                    continue;
                };
                if !account_observer::fresh(issued_at, Instant::now()) {
                    host.account_probe_diagnostic(
                        &nonce,
                        account_observer::Diagnostic::StaleObservation,
                    );
                    account_observer::pause(deadline, || {
                        host.account_probe_active(&window, &nonce, epoch, started, attempt)
                    });
                    continue;
                }
                let action_host = host.clone();
                let app_for_action = app.clone();
                let action_window = window.clone();
                let action_nonce = nonce.clone();
                let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
                if app.run_on_main_thread(move||{
                    let host=action_host;let window=action_window;let nonce=action_nonce;
                    let stop=(||{
                        if !host.account_probe_active(&window,&nonce,epoch,started,attempt){return true;}
                        if !account_observer::fresh(issued_at,Instant::now()){
                            host.account_probe_diagnostic(&nonce,account_observer::Diagnostic::StaleObservation);return false;
                        }
                        let Some(value)=observation["authenticated"].as_bool()else{return false;};
                        let (changed,sign_out)={
                            let mut flow=lock(&host.account_flow);
                            let Some(changed)=flow.observe(&nonce,epoch,value)else{return true;};
                            let Some(sign_out)=flow.is_sign_out(&nonce,epoch)else{return true;};
                            if changed{lock(&host.authenticated).confirm(value);}
                            (changed,sign_out)
                        };
                        if changed{
                            let account_revision=lock(&host.authenticated).revision;
                            host.watch_only.refresh_account(&app_for_action,account_revision);
                            let sessions=lock(&host.registry).sessions.values().cloned().collect::<Vec<_>>();
                            for session in sessions{
                                if !lock(&host.account_flow).current(&nonce,epoch){return true;}
                                let mut arm=lock(&session.arm);let archive=lock(&session.archive);
                                if session.closed.load(Ordering::Acquire){continue;}
                                // Account changes revoke future files even while
                                // the current archive is still being finalized.
                                lock(&session.continuation).cancel(None);
                                if archive.is_some(){
                                    lock(&session.status).error=Some("계정이 변경되었습니다. 녹화 종료 후 방송을 다시 열어 주세요.".into());continue;
                                }
                                if let Some((pending,_))=arm.take(){cancel_pending(&app_for_action,&session,&pending.id);}
                                // The same gate as record admission: a queued arm cannot
                                // reuse earlier readiness after refresh wins this lock.
                                session.account_refreshing.store(true,Ordering::Release);
                                {let mut status=lock(&session.status);status.authenticated=None;status.capture_ready=false;}
                                drop(archive);drop(arm);
                                let player_reloaded=app_for_action.get_webview(&session.label).is_some_and(|view|view.reload().is_ok());
                                if player_reloaded{
                                    if let Some(chat)=app_for_action.get_webview(&session.chat_label){let _=chat.reload();}
                                }else{
                                    let _arm=lock(&session.arm);
                                    session.account_refreshing.store(false,Ordering::Release);
                                    lock(&session.status).error=Some("계정 변경 후 방송을 다시 열어 주세요.".into());
                                }
                            }
                        }
                        if !host.account_probe_active(&window,&nonce,epoch,started,attempt){return true;}
                        if !account_observer::fresh(issued_at,Instant::now()){
                            host.account_probe_diagnostic(&nonce,account_observer::Diagnostic::StaleObservation);return false;
                        }
                        let Some(url)=window.url().ok().filter(policy::account_home)else{return true;};
                        // Close the captured native window, never a later label replacement.
                        match account_observer::request_close(issued_at,Instant::now(),policy::account_completed(&url,&observation,sign_out),||window.close()){
                            account_observer::CloseOutcome::Requested=>{
                                host.account_probe_diagnostic(&nonce,account_observer::Diagnostic::CloseRequested);return true;
                            },
                            account_observer::CloseOutcome::Failed=>host.account_probe_diagnostic(&nonce,account_observer::Diagnostic::CloseFailed),
                            account_observer::CloseOutcome::NotReady=>{},
                        }
                        // A fresh signed-out home may still contain an interactive
                        // prompt. Keep the bounded observer until it is safe to close.
                        false
                    })();
                    let _=done_tx.send(stop);
                }).is_err(){host.account_probe_diagnostic(&nonce,account_observer::Diagnostic::DispatchFailed);break;}
                match account_observer::receive(&done_rx, deadline, || {
                    host.account_probe_active(&window, &nonce, epoch, started, attempt)
                }) {
                    Ok(false) => {}
                    Ok(true) => break,
                    Err(failure) => {
                        host.account_probe_wait_diagnostic(&nonce, failure, false);
                        break;
                    }
                }
                account_observer::pause(deadline, || {
                    host.account_probe_active(&window, &nonce, epoch, started, attempt)
                });
            }
        });
    }
    pub fn shutdown(&self, app: &AppHandle) {
        self.closing.store(true, Ordering::Release);
        let admission = lock(&self.admission);
        self.watch_only.detach(app);
        drop(admission);
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in &sessions {
            let mut arm = lock(&session.arm);
            lock(&session.continuation).cancel(None);
            arm.take();
            if let Some(view) = app.get_webview(&session.label) {
                let _ = view.eval("window.__atsumiYouTubeOriginal?.stop()");
            }
        }
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until
            && sessions
                .iter()
                .any(|s| lock(&s.status).recording_job.is_some())
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        for session in sessions {
            lock(&session.arm).take();
            if let Some(archive) = lock(&session.archive).take() {
                let _ = archive.finish(Some("앱 종료로 원본 저장을 마무리했습니다.".into()));
            }
            self.dispose(app, &session);
        }
        if let Some(account) = app.get_webview_window("youtube-original-account") {
            let _ = account.close();
        }
    }
    pub fn detach_views(&self, app: &AppHandle) {
        {
            let _admission = lock(&self.admission);
            self.watch_only.detach(app);
        }
        let sessions = {
            let mut registry = lock(&self.registry);
            registry.tokens.clear();
            registry.requests.clear();
            registry.sessions.values().cloned().collect::<Vec<_>>()
        };
        for session in sessions {
            lock(&session.viewers).clear();
            lock(&session.viewport_sequences).clear();
            if lock(&session.status).fullscreen {
                let _ = set_fullscreen(app, &session, false);
            }
            *lock(&session.viewport) = Some(hidden_viewport());
            let _ = paint_session(app, &session, hidden_viewport());
            if lock(&session.archive).is_none() && lock(&session.arm).is_none() {
                self.dispose(app, &session);
            }
        }
    }
    pub fn set_tray_muted(&self, app: &AppHandle, muted: bool) {
        self.tray_muted.store(muted, Ordering::Release);
        self.watch_only.mute(app, muted);
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            let mut viewport = lock(&session.viewport)
                .clone()
                .unwrap_or_else(hidden_viewport);
            viewport.muted |= muted || lock(&session.viewers).is_empty();
            let _ = paint_session(app, &session, viewport);
        }
    }
    fn create_view(&self, app: &AppHandle, session: &Arc<Session>) -> Result<(), String> {
        let parent = app.get_window("main").ok_or(UNAVAILABLE)?;
        for is_chat in [false, true] {
            let url = if is_chat {
                policy::chat_url(&session.channel)?
            } else {
                policy::player_url(&session.channel, &session.broadcast)?
            };
            let label = if is_chat {
                &session.chat_label
            } else {
                &session.label
            };
            let navigation = session.clone();
            let page = session.clone();
            let page_app = app.clone();
            let page_muted = self.tray_muted.clone();
            let page_account = self.authenticated.clone();
            let nonce = uuid::Uuid::new_v4().to_string();
            let mut builder=WebviewBuilder::new(label,WebviewUrl::External("about:blank".parse().unwrap()))
                .data_directory(profile(app)?).focused(false).browser_extensions_enabled(false)
                .initialization_script(platform_native_geometry::pointer_script(url.as_str(),&nonce))
                .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
                .on_navigation(move|next| {
                    if next.as_str()=="about:blank" { return navigation.epoch.load(Ordering::Acquire)==0 || is_chat; }
                    if policy::matches_document(next,&navigation.channel,is_chat) {return true;}
                    if policy::account_url(next){lock(&navigation.status).error=Some("방송 조작 창에서 YouTube 로그인을 진행해 주세요.".into());}
                    false
                })
                .on_new_window({let app=app.clone();move|url,features|open_popup(&app,url,features,0)})
                .on_download(|_,_|false)
                .on_page_load(move|view,payload|{
                    if !policy::matches_document(payload.url(),&page.channel,is_chat){return;}
                    if matches!(payload.event(),tauri::webview::PageLoadEvent::Started){
                        if !is_chat{
                            let _=set_fullscreen(&page_app,&page,false);
                            let mut approval=lock(&page.arm);
                            lock(&page.continuation).cancel(None);
                            *lock(&page.canonical_channel)=None;
                            page.automatic_ready.store(false,Ordering::Release);
                            let epoch=page.epoch.fetch_add(1,Ordering::AcqRel)+1;page.ui_intent.document(epoch);
                            *lock(&page.document_id)=uuid::Uuid::new_v4().to_string();
                            page.account_revision.store(lock(&page_account).revision,Ordering::Release);
                            approval.take();
                            let archive=lock(&page.archive).take();
                            {let mut s=lock(&page.status);s.state="loading".into();s.canonical_channel_id=None;s.recording_job=None;s.recording_stopping=false;s.capture_ready=false;}
                            page.account_refreshing.store(false,Ordering::Release);
                            drop(approval);
                            // Detach the old archive synchronously. A delayed
                            // finalizer must never clear a new generation.
                            if let Some(archive)=archive{
                                tracing::info!(diag_stage="youtube_capture_page_changed");
                                if archive.is_raw() { let _=archive.finish(Some("YouTube 원본 페이지가 변경되어 저장을 마무리했습니다.".into())); }
                                else { std::thread::spawn(move||{let _=archive.finish(Some("YouTube 원본 페이지가 변경되어 저장을 마무리했습니다.".into()));}); }
                            }
                        }
                    }else{
                        let config=json!({"epoch":page.epoch.load(Ordering::Acquire),"channel":page.channel,"fullscreen":lock(&page.status).fullscreen,"rawCaptureEnabled":RAW_CAPTURE_ENABLED});
                        let _=view.eval(&format!("window.__atsumiYouTubeOriginal?.configure({config})"));
                        if let Some(mut viewport)=lock(&page.viewport).clone(){
                            viewport.muted|=page_muted.load(Ordering::Acquire)||lock(&page.viewers).is_empty();
                            let _=paint_session(&page_app,&page,viewport);
                        }else{let _=paint_session(&page_app,&page,hidden_viewport());}
                    }
                });
            if !is_chat {
                builder = builder.initialization_script(include_str!("original_raw_framing.js"));
                builder = builder.initialization_script(include_str!("original_capture.js"));
            }
            builder = builder.initialization_script(include_str!("original_page.js"));
            let view = parent
                .add_child(
                    builder,
                    tauri::LogicalPosition::new(0.0, 0.0),
                    tauri::LogicalSize::new(1.0, 1.0),
                )
                .map_err(|_| UNAVAILABLE)?;
            view.hide().map_err(|_| UNAVAILABLE)?;
            let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
            platform_native_geometry::paint(
                &view,
                &hidden_viewport(),
                session.paint.clone(),
                revision,
            )?;
            platform_native_geometry::install_pointer_focus(&view, url.to_string(), nonce)
                .map_err(|_| UNAVAILABLE)?;
            attach_bridge(&view, session.clone(), self.clone(), is_chat)?;
            view.navigate(url).map_err(|_| UNAVAILABLE)?;
        }
        Ok(())
    }
}
fn hidden_viewport() -> NativeViewport {
    NativeViewport {
        sequence: 1,
        player: None,
        chat: None,
        visible: false,
        muted: true,
        occlusions: Vec::new(),
    }
}
fn valid_canonical_channel(channel: &str) -> bool {
    channel.len() == 24
        && channel.starts_with("UC")
        && channel
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn scheduled_owner_matches(owner: &RecordingOwner, channel: Option<&str>, video: &str) -> bool {
    matches!(owner,RecordingOwner::Scheduled{schedule_id,broadcast_key,..} if valid_canonical_channel(schedule_id)&&channel==Some(schedule_id.as_str())&&broadcast_key==video)
}
/// Arm lock is held by the caller. The browser also checks the exact job so a
/// delayed scheduled stop can never stop a later manually started recording.
fn stop_exact_recording(app: &AppHandle, session: &Arc<Session>, job: &str) -> Result<(), String> {
    lock(&session.status).recording_stopping = true;
    *lock(&session.stopping_since) = Some(Instant::now());
    app.get_webview(&session.label).ok_or(UNAVAILABLE)?.eval(&format!("(()=>{{if(window.__atsumiYouTubeEncodedCapture?.getStatus?.().recordingId==={})return window.__atsumiYouTubeOriginal?.stop();}})()",json!(job))).map_err(|_|UNAVAILABLE)?;
    Ok(())
}
fn cancel_pending(app: &AppHandle, session: &Arc<Session>, request: &str) {
    if let Some(view) = app.get_webview(&session.label) {
        let _ = view.eval(&format!(
            "window.__atsumiYouTubeEncodedCapture?.cancelPending({})",
            json!(request)
        ));
    }
}
#[cfg(test)]
mod scheduled_identity_tests {
    use super::*;
    #[test]
    fn scheduled_owner_binds_channel_and_video_separately() {
        let channel = "UC1234567890123456789012";
        let video = "AbCdEf123_-";
        let owner = RecordingOwner::Scheduled {
            schedule_id: channel.into(),
            broadcast_key: video.into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
        };
        assert!(scheduled_owner_matches(&owner, Some(channel), video));
        assert!(!scheduled_owner_matches(&owner, None, video));
        assert!(!scheduled_owner_matches(
            &owner,
            Some("UCabcdefghijklmnopqrstuv"),
            video
        ));
        assert!(!scheduled_owner_matches(
            &owner,
            Some(channel),
            "abcdef123_-"
        ));
        assert!(!scheduled_owner_matches(
            &RecordingOwner::Manual,
            Some(channel),
            video
        ));
        assert!(!valid_canonical_channel("@NASA"));
        assert!(!valid_canonical_channel(video));
    }
}
fn public_broadcast_ended(video: &str) -> bool {
    // The HTML video ending can also mean an interrupted stream. Only an
    // explicit, bounded official endTimestamp for this video makes it normal.
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Atsumi/2.2 YouTube public status")
        .build()
    else {
        return false;
    };
    let Ok(url) = policy::player_url(video, video) else {
        return false;
    };
    let Ok(bytes) = super::bounded_get(&client, url, 4 * 1024 * 1024) else {
        return false;
    };
    let Ok(html) = std::str::from_utf8(&bytes) else {
        return false;
    };
    let Some(raw) = super::search::object_after(html, "var ytInitialPlayerResponse =")
        .or_else(|| super::search::object_after(html, "ytInitialPlayerResponse ="))
    else {
        return false;
    };
    serde_json::from_str::<Value>(raw)
        .ok()
        .is_some_and(|value| policy::confirmed_ended(&value, video))
}
fn paint_session(
    app: &AppHandle,
    session: &Arc<Session>,
    viewport: NativeViewport,
) -> Result<(), String> {
    let revision = session.paint.fetch_add(1, Ordering::AcqRel) + 1;
    for (is_chat, label) in [(false, &session.label), (true, &session.chat_label)] {
        if let Some(view) = app.get_webview(label) {
            let config = json!({"epoch":session.epoch.load(Ordering::Acquire),"channel":session.channel,"fullscreen":lock(&session.status).fullscreen,"rawCaptureEnabled":RAW_CAPTURE_ENABLED});
            let _ = view.eval(&format!(
                "window.__atsumiYouTubeOriginal?.configure({config})"
            ));
            let mut pane = viewport.clone();
            if is_chat {
                pane.player = None;
                pane.muted = true;
            } else {
                pane.chat = None;
            }
            pane.visible &= if is_chat {
                pane.chat.is_some()
            } else {
                pane.player.is_some()
            };
            view.eval(&format!(
                "window.__atsumiYouTubeOriginal?.layout({})",
                json!(pane)
            ))
            .map_err(|_| UNAVAILABLE)?;
            platform_native_geometry::paint(&view, &pane, session.paint.clone(), revision)?;
        }
    }
    Ok(())
}
fn set_fullscreen(app: &AppHandle, session: &Arc<Session>, enabled: bool) -> Result<(), String> {
    if !enabled {
        let grant = lock(&session.fullscreen_grant).clone();
        if let Some(grant) = grant {
            window_fullscreen::release(app, grant)?;
        }
        return Ok(());
    }
    let epoch = session.epoch.load(Ordering::Acquire);
    let valid = Arc::downgrade(session);
    let acquired = Arc::downgrade(session);
    let lost = Arc::downgrade(session);
    let acquired_app = app.clone();
    let lost_app = app.clone();
    window_fullscreen::acquire(
        app,
        session.id.clone(),
        epoch,
        Arc::new(move || {
            valid.upgrade().is_some_and(|session| {
                !session.closed.load(Ordering::Acquire)
                    && session.epoch.load(Ordering::Acquire) == epoch
                    && !lock(&session.viewers).is_empty()
                    && lock(&session.viewport)
                        .as_ref()
                        .is_some_and(|v| v.visible && v.player.is_some())
            })
        }),
        Arc::new(move |grant| {
            if let Some(session) = acquired.upgrade() {
                publish_fullscreen(&acquired_app, &session, grant, true);
            }
        }),
        Arc::new(move |grant| {
            if let Some(session) = lost.upgrade() {
                publish_fullscreen(&lost_app, &session, grant, false);
            }
        }),
    )
    .map_err(|error| {
        if let Some(grant) = error.retained_grant {
            let _ = window_fullscreen::release(app, grant);
        }
        error.message
    })?;
    Ok(())
}
fn escape_binding(session:&Session)->Option<escape_intent::Binding> {
    if session.closed.load(Ordering::Acquire)||lock(&session.viewers).is_empty(){return None;}
    if !lock(&session.viewport).as_ref().is_some_and(|v|v.visible&&v.player.is_some()){return None;}
    if !lock(&session.status).fullscreen{return None;}
    let grant=lock(&session.fullscreen_grant).clone()?;
    let epoch=session.epoch.load(Ordering::Acquire);
    (epoch>0&&grant.document_epoch==epoch).then_some(escape_intent::Binding{epoch,grant})
}
fn publish_fullscreen(app: &AppHandle, session: &Session, grant: Grant, enabled: bool) {
    {
        let mut current = lock(&session.fullscreen_grant);
        if enabled {
            *current = Some(grant);
        } else if current.as_ref() == Some(&grant) {
            *current = None;
        } else {
            return;
        }
    }
    lock(&session.status).fullscreen = enabled;
    if let Some(view) = app.get_webview(&session.label) {
        let _ = view.eval(&format!(
            "window.__atsumiYouTubeOriginal?.fullscreen({enabled})"
        ));
    }
    let _ = app.emit(
        "youtube-original:fullscreen",
        json!({"channelId":session.channel,"enabled":enabled}),
    );
}

fn open_popup(
    app: &AppHandle,
    url: tauri::Url,
    features: tauri::webview::NewWindowFeatures,
    depth: u8,
) -> tauri::webview::NewWindowResponse<tauri::Wry> {
    use tauri::webview::NewWindowResponse;
    if depth >= 3
        || !(policy::popup_url(&url) || url.as_str() == "about:blank")
        || app
            .webview_windows()
            .keys()
            .filter(|key| key.starts_with("youtube-original-popup-"))
            .count()
            >= 6
    {
        return NewWindowResponse::Deny;
    }
    let Ok(profile) = profile(app) else {
        return NewWindowResponse::Deny;
    };
    let child = app.clone();
    let initial = Arc::new(AtomicBool::new(true));
    let result = WebviewWindowBuilder::new(
        app,
        format!("youtube-original-popup-{}", uuid::Uuid::new_v4().simple()),
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .data_directory(profile)
    .browser_extensions_enabled(false)
    .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
    .window_features(features)
    .focused(false)
    .title("YouTube")
    .inner_size(720.0, 780.0)
    .min_inner_size(360.0, 420.0)
    .on_navigation(move |next| {
        if next.as_str() == "about:blank" {
            return initial.load(Ordering::Acquire);
        }
        let allowed = policy::popup_url(next);
        if allowed {
            initial.store(false, Ordering::Release);
        }
        allowed
    })
    .on_new_window(move |url, features| open_popup(&child, url, features, depth + 1))
    .on_download(|_, _| false)
    .build();
    match result {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Deny,
    }
}
fn response(view: &Webview, id: &str, result: Result<Value, String>) {
    let body = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => {
            let diagnostic=match error.as_str(){
            STALE|UNAVAILABLE|
            "이미 중단된 압축 녹화입니다."|"다른 입력 트랙입니다."|"지원하지 않는 fragment 크기입니다."|
            "mdat가 없는 moof입니다."|"moof가 없는 mdat입니다."|"moof와 mdat 사이의 박스는 지원하지 않습니다."|
            "초기화 변경 또는 지원하지 않는 fragment 박스입니다."|"지원하지 않는 mfhd입니다."|
            "지원하지 않는 fragment 트랙 수입니다."|"절대 offset 또는 지원하지 않는 tfhd입니다."|
            "알 수 없는 fragment 트랙입니다."|"fragment 트랙이 중복되었습니다."|"잘못된 tfhd 크기입니다."|
            "지원하지 않는 tfdt flags입니다."|"지원하지 않는 tfdt입니다."|"지원하지 않는 trun flags입니다."|
            "fragment data offset이 잘못되었습니다."|"잘못된 sample 길이 또는 시각입니다."|
            "mdat에 sample이 모두 들어 있지 않습니다."|"AAC composition offset은 지원하지 않습니다."|
            "겹치거나 잘못된 sample 범위입니다."|"mdat에 알 수 없는 데이터가 있습니다."|
            "독립 재생 가능한 영상·음성 시작점을 찾지 못했습니다."|"중단된 압축 녹화의 미완료 구간입니다."|
            "마지막 MP4 fragment가 불완전합니다."|"완전한 영상·음성 시작 구간을 받지 못했습니다."|
            "잘못된 AVC NAL 경계입니다."|"지원하지 않는 AVC NAL입니다."|"키프레임 표시와 AVC IDR이 일치하지 않습니다."|
            "음성·영상 원점이 잘못되었습니다."|"키프레임 앞의 표시 시각은 지원하지 않습니다."|
            "필수 MP4 박스가 없습니다."|"압축 영상 저장 버퍼 한도를 초과했습니다."|
            "잘린 MP4 필드입니다."|"잘린 MP4 시각입니다."|"잘못된 MP4 필드 위치입니다."|
            "4바이트 AVC NAL 크기와 avcC 초기화가 필요합니다."|"AVC SPS 수가 올바르지 않습니다."|
            "잘린 AVC SPS입니다."|"AVC SPS 크기가 올바르지 않습니다."|"AVC PPS가 없습니다."|
            "AVC PPS 수가 올바르지 않습니다."|"잘린 AVC PPS입니다."|"AVC PPS 크기가 올바르지 않습니다."|
            "잘린 AAC descriptor입니다."|"잘린 AAC descriptor 길이입니다."|"잘린 AAC descriptor 내용입니다."|
            "지원하지 않는 esds 버전입니다."|"외부 AAC descriptor는 지원하지 않습니다."|
            "MPEG-4 AAC 오디오가 아닙니다."|"초기 버전은 AAC-LC만 지원합니다."|"지원하지 않는 AAC 샘플레이트 또는 채널입니다."|
            "tkhd 버전이 없습니다."|"MP4 트랙 ID 또는 sample description이 올바르지 않습니다."|
            "mdhd 버전이 없습니다."|"MP4 트랙 시간 단위가 올바르지 않습니다."|"영상·음성 외 트랙은 지원하지 않습니다."|
            "잘린 dref입니다."|"비어 있지 않은 MP4 sync sample 표는 지원하지 않습니다."|"잘린 stsd입니다."|
            "잘못된 sample 데이터 참조입니다."|"잘린 sample description입니다."|
            "최대 두 개의 영상·음성 입력만 지원합니다."|"fMP4 MIME과 명시적인 codec이 필요합니다."|
            "영상과 음성은 각각 한 트랙이어야 합니다."|"mvhd가 없습니다."|"잘린 mvhd입니다."|
            "원본 플레이어 녹화 데이터가 올바르지 않습니다. 받은 파일은 보존했습니다."|
            "이미 녹화 중이거나 마무리 중입니다."|"앱 또는 플레이어를 종료하는 중입니다."|
            "이미 처리한 녹화 요청입니다."|"진행 중인 작업이 많습니다."|
            "YouTube 원본 플레이어의 채팅 보존 경로를 아직 확인하지 못했습니다. 녹화를 시작하지 않았습니다."|
            "녹화 시작 승인이 없습니다."|"이미 녹화 중입니다."|
            "지원하지 않는 MP4 박스 또는 암호화 정보가 있습니다."|"MP4 박스 크기를 지원하지 않습니다."|
            "중복 MP4 박스입니다."|"완전한 ftyp+moov 초기화가 필요합니다."|"잘못된 ftyp입니다."|
            "지원하지 않는 mvhd입니다."|"지원하지 않는 tkhd 버전입니다."|"지원하지 않는 mdhd 버전입니다."|
            "fragmented MP4 sample 표가 아닙니다."|"비어 있지 않은 일반 MP4 표는 지원하지 않습니다."|
            "트랙별 하나의 codec description이 필요합니다."|"하나의 codec description이 필요합니다."|
            "clear H.264/AAC 이외의 codec 또는 암호화 트랙입니다."|"QuickTime 오디오 확장은 지원하지 않습니다."|
            "외부 MP4 데이터 참조를 지원하지 않습니다."|"트랙과 trex가 일치하지 않습니다."|
            "확인된 H.264 영상과 AAC 음성이 모두 필요합니다."=>Some(error.as_str()),_=>None};
            json!({"ok":false,"code":"NATIVE_REJECTED","error":error,"diagnosticMessage":diagnostic})
        }
    };
    let _ = view.eval(&format!(
        "window.__atsumiYouTubeOriginal?.reply({},{})",
        json!(id),
        body
    ));
}
fn chat_observation(descriptor: &Value) -> Option<(&str, &str, bool, Option<bool>)> {
    let availability = descriptor.get("chatAvailable")?;
    if !availability.is_boolean() && !availability.is_null() { return None; }
    Some((descriptor["videoId"].as_str()?, descriptor["channelId"].as_str()?,
        descriptor["isLive"].as_bool()?, availability.as_bool()))
}
fn process(
    host: &YouTubeOriginalHost,
    app: &AppHandle,
    session: &Arc<Session>,
    message: Envelope,
) -> Result<Value, String> {
    if session.closed.load(Ordering::Acquire)
        || message.epoch != session.epoch.load(Ordering::Acquire)
    {
        return Err(STALE.into());
    }
    let fields = &message.fields;
    match message.kind.as_str() {
        "status" => {
            let _approval = lock(&session.arm);
            if session.closed.load(Ordering::Acquire)
                || message.epoch != session.epoch.load(Ordering::Acquire) {
                return Err(STALE.into());
            }
            let mut observed_channel = None;
            if let Some(descriptor) = fields.get("chatDescriptor") {
                if let Some((video, channel, live, chat)) = chat_observation(descriptor) {
                    if video == session.channel {
                        session
                            .lease
                            .bind_youtube_chat(video, channel, live, chat)?;
                        *lock(&session.canonical_channel) = Some(channel.to_string());
                        observed_channel = Some(channel.to_string());
                    }
                }
            }
            let mut status = lock(&session.status);
            status.canonical_channel_id = observed_channel;
            if let Some(state) = fields["state"]
                .as_str()
                .filter(|s| matches!(*s, "preparing" | "loading" | "playing" | "ended" | "error"))
            {
                status.state = state.into();
            }
            status.authenticated = fields["authenticated"].as_bool();
            let chat_preparing = session.lease.youtube_chat_preparing();
            status.capture_ready = fields["captureReady"].as_bool().unwrap_or(false)
                && !session.account_refreshing.load(Ordering::Acquire)
                && !chat_preparing;
            session.automatic_ready.store(
                status.capture_ready && fields["captureStable"].as_bool() == Some(true),
                Ordering::Release,
            );
            status.capture_unavailable_reason = if chat_preparing {
                Some("녹화 채팅 연결을 준비하고 있습니다.".into())
            } else if status.capture_ready {
                None
            } else {
                Some(
                    match fields["captureReason"].as_str() {
                        Some("encrypted") => "보호된 영상은 녹화할 수 없습니다.",
                        Some("codec_unsupported" | "tracks_unsupported") => {
                            "이 영상 형식은 녹화할 수 없습니다. AVC/AAC 원본 녹화만 지원합니다."
                        }
                        Some("advertisement") => "광고가 끝나면 녹화할 수 있습니다.",
                        Some(
                            "container_unsupported"
                            | "init_unsupported"
                            | "append_unsupported"
                            | "timeline_unsupported"
                            | "timeline_changed",
                        ) => "이 영상의 조각 형식은 현재 녹화할 수 없습니다.",
                        Some(
                            "observer_changed" | "source_object_unsupported" | "mse_unavailable",
                        ) => "현재 영상 수신 방식은 녹화할 수 없습니다.",
                        _ => "원본 영상이 준비되면 녹화할 수 있습니다.",
                    }
                    .into(),
                )
            };
            if fields["error"].is_string() {
                status.error = Some("YouTube 원본 플레이어 또는 저장 상태를 확인해 주세요.".into());
            } else if status.error.as_deref()
                == Some("YouTube 원본 플레이어 또는 저장 상태를 확인해 주세요.")
            {
                status.error = None;
            }
            Ok(Value::Null)
        }
        "encoded_begin" => {
            let raw = raw_begin_format(fields)?;
            if raw && !RAW_CAPTURE_ENABLED { return Err("원본 VP9 녹화 연결을 아직 검증 중입니다.".into()); }
            let request = fields["requestId"].as_str().ok_or(STALE)?;
            let source = fields["sourceId"]
                .as_str()
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                .ok_or(STALE)?;
            let mut arm = lock(&session.arm);
            let (approval, owner) = arm.as_ref().ok_or("녹화 시작 승인이 없습니다.")?;
            if !approval.accepts(request, message.epoch, Instant::now())
                || fields["channelId"].as_str() != Some(&session.channel)
                || session.closed.load(Ordering::Acquire)
                || session.account_refreshing.load(Ordering::Acquire)
                || host.closing.load(Ordering::Acquire)
                || session.epoch.load(Ordering::Acquire) != message.epoch
                || !lock(&session.continuation).permits(owner, message.epoch)
                || matches!(owner, RecordingOwner::Scheduled { .. })
                    && !scheduled_owner_matches(
                        owner,
                        lock(&session.canonical_channel).as_deref(),
                        &session.broadcast,
                    )
                || lock(&session.continuation_cancel)
                    .as_ref()
                    .is_some_and(|cancel| cancel.load(Ordering::Acquire))
                || session.stable_arm.load(Ordering::Acquire)
                    && !session.automatic_ready.load(Ordering::Acquire)
            {
                return Err(STALE.into());
            }
            let tracks = fields["tracks"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 2)
                .ok_or(STALE)?
                .iter()
                .map(|track| {
                    let init = track["init"]
                        .as_str()
                        .filter(|v| v.len() <= 88_000)
                        .ok_or(STALE)?;
                    Ok(EncodedTrack {
                        track_index: track["trackIndex"]
                            .as_u64()
                            .filter(|i| *i < 2)
                            .ok_or(STALE)? as u32,
                        mime_type: track["mimeType"]
                            .as_str()
                            .filter(|s| s.len() <= 120)
                            .ok_or(STALE)?
                            .into(),
                        init: STANDARD.decode(init).map_err(|_| STALE)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut slot = lock(&session.archive);
            if slot.is_some() {
                return Err("이미 녹화 중입니다.".into());
            }
            let track_count = tracks.len() as u64;
            let begin = || {
                if raw {
                    let generation = session.raw_source_generation.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1)).map_err(|_| STALE)? + 1;
                    session.lease.begin_youtube_raw(source.into(), lock(&session.document_id).clone(), message.epoch,
                        generation, request.into(), tracks, session.title.clone(), session.name.clone(), owner.clone()).map(Archive::Raw)
                } else {
                    session.lease.begin_encoded(source.into(), message.epoch, request.into(), tracks,
                        session.title.clone(), session.name.clone(), owner.clone()).map(Archive::Encoded)
                }
            };
            let archive = if matches!(owner, RecordingOwner::Scheduled { .. }) {
                app.try_state::<crate::interface::commands::AppState>()
                    .ok_or(STALE)?
                    .youtube_auto_recorder()
                    .with_current_owner(owner, begin)?
            } else {
                begin()?
            };
            let id = archive.snapshot()?.job_id;
            *slot = Some(archive);
            *lock(&session.source) = Some(source.into());
            *lock(&session.owner) = Some(owner.clone());
            host.remember_scheduled(session, &id, owner);
            *lock(&session.continuation_cancel) = None;
            arm.take();
            lock(&session.status).recording_job = Some(id.clone());
            *lock(&session.first_append_deadline) = Some(Instant::now());
            *lock(&session.stopping_since) = None;
            tracing::info!(diag_stage = "youtube_capture_begin", count = track_count);
            let mut response = json!({"id":id,"mode":"encoded","nativeApproved":true,"captureChat":true});
            if raw { response["format"] = json!(RAW_FORMAT); }
            Ok(response)
        }
        "encoded_raw_init" => {
            let _approval = lock(&session.arm);
            if session.closed.load(Ordering::Acquire) || session.account_refreshing.load(Ordering::Acquire)
                || host.closing.load(Ordering::Acquire) || session.epoch.load(Ordering::Acquire) != message.epoch { return Err(STALE.into()); }
            let mut slot = lock(&session.archive);
            let Some(Archive::Raw(archive)) = slot.as_mut() else { return Err(STALE.into()); };
            if !closed_fields(fields, &["recordingId", "trackIndex", "unitIndex", "startOffset", "data"]) { return Err(STALE.into()); }
            if fields["recordingId"].as_str() != lock(&session.status).recording_job.as_deref() { return Err(STALE.into()); }
            let index = fields["trackIndex"].as_u64().filter(|value| *value < 2).ok_or(STALE)? as u32;
            let unit = fields["unitIndex"].as_u64().filter(|value| *value < 100_000).ok_or(STALE)?;
            let start = fields["startOffset"].as_u64().filter(|value| *value <= 137_438_953_472).ok_or(STALE)?;
            let data = fields["data"].as_str().filter(|value| value.len() <= 88_000).ok_or(STALE)?;
            let bytes = STANDARD.decode(data).map_err(|_| STALE)?;
            match archive.observe_init(index, unit, start, &bytes)? {
                RawInitDecision::Accepted => Ok(json!({"accepted":true,"changed":false})),
                RawInitDecision::Changed => Ok(json!({"accepted":false,"changed":true})),
            }
        }
        "encoded_append" => {
            let _approval = lock(&session.arm);
            if session.epoch.load(Ordering::Acquire) != message.epoch || session.closed.load(Ordering::Acquire) { return Err(STALE.into()); }
            let mut slot = lock(&session.archive);
            let archive = slot.as_mut().ok_or(STALE)?;
            if archive.is_raw() && (session.account_refreshing.load(Ordering::Acquire)
                || !closed_fields(fields, &["recordingId", "trackIndex", "appendIndex", "chunkIndex", "finalChunk", "data", "rawFirstUnitEnd"])) { return Err(STALE.into()); }
            let recording = fields["recordingId"].as_str().ok_or(STALE)?;
            if lock(&session.status).recording_job.as_deref() != Some(recording) {
                return Err(STALE.into());
            }
            let payload = fields["data"]
                .as_str()
                .filter(|s| s.len() <= 174_764)
                .ok_or(STALE)?;
            // Source is pinned by begin; the current archive validates exact generation and source.
            let source = lock(&session.source).clone().ok_or(STALE)?;
            let bytes = STANDARD.decode(payload).map_err(|_| STALE)?;
            let byte_count = bytes.len() as u64;
            archive.append(EncodedAppend {
                source_id: source,
                generation: message.epoch,
                track_index: fields["trackIndex"]
                    .as_u64()
                    .filter(|v| *v < 2)
                    .ok_or(STALE)? as u32,
                index: fields["appendIndex"].as_u64().ok_or(STALE)?,
                chunk: fields["chunkIndex"]
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or(STALE)?,
                final_chunk: fields["finalChunk"].as_bool().ok_or(STALE)?,
                bytes,
            }, raw_boundary(fields)?)?;
            if archive.startup_ready() && lock(&session.first_append_deadline).take().is_some() {
                tracing::info!(
                    diag_stage = "youtube_capture_first_append",
                    bytes = byte_count
                );
            }
            Ok(json!({"accepted":true}))
        }
        "encoded_finish" => {
            let mut approval = lock(&session.arm);
            if fields["recordingId"].as_str() != lock(&session.status).recording_job.as_deref() {
                return Err(STALE.into());
            }
            let archive = lock(&session.archive).take().ok_or(STALE)?;
            *lock(&session.first_append_deadline) = None;
            *lock(&session.stopping_since) = None;
            let mut interrupted = fields["interrupted"].as_bool().unwrap_or(true);
            if interrupted
                && matches!(
                    fields["reason"].as_str(),
                    Some("video_ended" | "source_changed")
                )
                && public_broadcast_ended(&session.channel)
            {
                interrupted = false;
            }
            let reason = fields["reason"]
                .as_str()
                .filter(|reason| crate::diagnostics::safe_capture_reason(reason))
                .unwrap_or("other");
            tracing::info!(
                diag_stage = "youtube_capture_finish",
                capture_reason = reason,
                success = !interrupted
            );
            if reason == "init_changed" {
                trace_init_change(&fields["initChange"]);
            }
            let owner = lock(&session.owner).clone();
            let can_continue = matches!(reason, "init_changed" | "codec_changed" | "track_changed")
                && !archive.is_cancelled()
                && !host.closing.load(Ordering::Acquire)
                && !session.closed.load(Ordering::Acquire)
                && !session.account_refreshing.load(Ordering::Acquire)
                && owner
                    .as_ref()
                    .is_some_and(|owner| lock(&session.continuation).permits(owner, message.epoch))
                && !lock(&session.status).recording_stopping;
            let cancel = if can_continue {
                archive.format_change_cancel_guard()
            } else {
                None
            };
            let result = if can_continue && cancel.is_some() {
                archive.finish_format_change()
            } else {
                archive
                    .finish(
                        interrupted.then(|| "원본 영상 수신 또는 저장이 중단되었습니다.".into()),
                    )
                    .map(|job| (job, false))
            };
            {
                let mut status = lock(&session.status);
                status.recording_job = None;
                status.recording_stopping = false;
            }
            let (_, progress) = match result {
                Ok(result) => result,
                Err(error) => {
                    lock(&session.continuation).cancel(None);
                    lock(&session.status).error = Some(error.clone());
                    return Err(error);
                }
            };
            let continuation = if let (true, Some(owner), Some(cancel)) =
                (can_continue, owner, cancel)
            {
                let valid = !cancel.load(Ordering::Acquire)
                    && !host.closing.load(Ordering::Acquire)
                    && !session.closed.load(Ordering::Acquire)
                    && !session.account_refreshing.load(Ordering::Acquire)
                    && session.epoch.load(Ordering::Acquire) == message.epoch;
                if valid
                    && lock(&session.continuation).next(
                        &owner,
                        message.epoch,
                        progress,
                        Instant::now(),
                    )
                {
                    let arm = policy::Arm {
                        id: uuid::Uuid::new_v4().to_string(),
                        epoch: message.epoch,
                        created: Instant::now(),
                    };
                    let command = json!({"requestId":arm.id,"channelId":session.channel,"rightsAcknowledged":true,"automatic":true});
                    *lock(&session.continuation_cancel) = Some(cancel);
                    session.stable_arm.store(true, Ordering::Release);
                    session.automatic_ready.store(false, Ordering::Release);
                    *approval = Some((arm, owner));
                    lock(&session.status).error = None;
                    tracing::info!(
                        diag_stage = "youtube_capture_continuation",
                        success = progress
                    );
                    Some(command)
                } else {
                    lock(&session.continuation).cancel(None);
                    lock(&session.status).error=Some("영상 형식이 반복해서 변경되어 녹화를 중단했습니다. 기존 파일은 보존했습니다.".into());
                    None
                }
            } else {
                lock(&session.continuation).cancel(None);
                None
            };
            Ok(json!({"stopped":true,"interrupted":interrupted,"continuation":continuation}))
        }
        _ => Err("지원하지 않는 YouTube 요청입니다.".into()),
    }
}
#[cfg(windows)]
fn attach_bridge(
    view: &Webview,
    session: Arc<Session>,
    host: YouTubeOriginalHost,
    is_chat: bool,
) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    let (tx, rx) = std::sync::mpsc::sync_channel::<Envelope>(4);
    let worker_view = view.clone();
    let worker_session = session.clone();
    let app = view.app_handle().clone();
    std::thread::Builder::new()
        .name("youtube-original-writer".into())
        .spawn(move || loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(message) => {
                    let id = message.id.clone();
                    let message_epoch=message.epoch;
                    let message_account_revision=worker_session.account_revision.load(Ordering::Acquire);
                    let is_status=message.kind=="status";
                    let silent = matches!(message.kind.as_str(),"status"|"chat_status"|"fullscreen"|"presentation"|"screenshot");
                    let result = if message.epoch!=worker_session.epoch.load(Ordering::Acquire)||worker_session.closed.load(Ordering::Acquire){Err(STALE.to_string())}
                    else if is_chat {lock(&worker_session.status).chat_visible=message.fields["visible"].as_bool().unwrap_or(false);Ok(Value::Null)}
                    else if message.kind=="ui_intent" {let result=dispatch_ui(&host,&app,&worker_session,&message);if let Err(error)=&result{lock(&worker_session.status).error=Some(error.clone());}result}
                    else if message.kind=="fullscreen" { message.fields["enabled"].as_bool().ok_or_else(||STALE.to_string()).and_then(|enabled|set_fullscreen(&app,&worker_session,enabled).map(|_|Value::Null)) }
                    else if message.kind=="presentation" {if let Some(opacity)=message.fields["opacity"].as_f64().filter(|v|v.is_finite()&&(0.0..=1.0).contains(v)){let _=app.emit("youtube-original:presentation",json!({"channelId":worker_session.channel,"broadcastId":worker_session.broadcast,"opacity":opacity}));}Ok(Value::Null)}
                    else if message.kind=="screenshot" {let _=app.emit("youtube-original:screenshot",json!({"channelId":worker_session.channel}));Ok(Value::Null)}
                    else {process(&host,&app,&worker_session, message)};
                    if is_status&&message_epoch==worker_session.epoch.load(Ordering::Acquire) { if let Some(value)=lock(&worker_session.status).authenticated { lock(&host.authenticated).observe(message_account_revision,value); } }
                    if !silent {
                        response(&worker_view, &id, result);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            // The chat document has no recording authority or lifecycle work.
            if is_chat {if worker_session.closed.load(Ordering::Acquire){break;}continue;}
            let raw=lock(&worker_session.archive).as_ref().is_some_and(Archive::is_raw);
            let start_expired=lock(&worker_session.first_append_deadline).as_ref().is_some_and(|since|if raw { since.elapsed()>=Duration::from_secs(15) } else { policy::ack_expired(*since,Instant::now(),false) });
            let stop_expired=lock(&worker_session.stopping_since).as_ref().is_some_and(|since|policy::ack_expired(*since,Instant::now(),true));
            if start_expired||stop_expired {
                let mut approval=lock(&worker_session.arm);approval.take();lock(&worker_session.continuation).cancel(None);
                *lock(&worker_session.first_append_deadline)=None;*lock(&worker_session.stopping_since)=None;
                if start_expired {tracing::warn!(diag_stage="youtube_capture_start_timeout");}
                else {tracing::warn!(diag_stage="youtube_capture_stop_timeout");}
                if let Some(archive)=lock(&worker_session.archive).take(){let _=archive.finish(Some("원본 플레이어의 저장 응답이 끊겨 녹화를 마무리했습니다.".into()));}
                let _=worker_view.eval("window.__atsumiYouTubeOriginal?.stop()");
                let mut status=lock(&worker_session.status);status.recording_job=None;status.recording_stopping=false;status.error=Some("녹화 응답이 끊겼습니다. 녹화 목록에서 파일을 확인해 주세요.".into());
            }
            {
                let mut approval=lock(&worker_session.arm);
                if lock(&worker_session.continuation_cancel).as_ref().is_some_and(|cancel|cancel.load(Ordering::Acquire)){
                    if let Some((pending,_))=approval.take(){cancel_pending(&app,&worker_session,&pending.id);}
                    lock(&worker_session.continuation).cancel(None);
                    *lock(&worker_session.continuation_cancel)=None;
                }
                let mut slot = lock(&worker_session.archive);
                if slot.as_ref().is_some_and(|archive| archive.is_cancelled()) {
                    approval.take();lock(&worker_session.continuation).cancel(None);
                    if let Some(archive) = slot.take() {
                        let _ = archive.finish(Some("녹화 취소 요청으로 마무리했습니다.".into()));
                    }
                    let mut status = lock(&worker_session.status);
                    status.recording_job = None;
                    status.recording_stopping = false;
                }
            }
            {
                let mut arm = lock(&worker_session.arm);
                if arm
                    .as_ref()
                    .is_some_and(|(arm, _)| arm.created.elapsed() > Duration::from_secs(25))
                {
                    if let Some((pending,_))=arm.take(){cancel_pending(&app,&worker_session,&pending.id);}
                    lock(&worker_session.continuation).cancel(None);
                    lock(&worker_session.status).error =
                        Some("원본 영상 준비 시간이 초과되었습니다.".into());
                }
            }
            if lock(&worker_session.viewers).is_empty()
                && lock(&worker_session.archive).is_none()
                && lock(&worker_session.arm).is_none()
            {
                host.dispose(&app, &worker_session);
                break;
            }
            if worker_session.closed.load(Ordering::Acquire) {
                break;
            }
        })
        .map_err(|_| UNAVAILABLE)?;
    let native_view = view.clone();
    let input_app = view.app_handle().clone();
    let root_hwnd = view
        .app_handle()
        .get_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as usize)
        .ok_or(UNAVAILABLE)?;
    view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            let mut pane_hwnd = windows::Win32::Foundation::HWND::default();
            if platform.controller().ParentWindow(&mut pane_hwnd).is_err() {
                return;
            }
            let pane_hwnd = pane_hwnd.0 as usize;
            let input_diagnostics = input_diagnostics::Budget::default();
            let escape_intent=Arc::new(escape_intent::Intent::default());
            if !is_chat {
                let weak=Arc::downgrade(&session);
                let _=player_intent::install(&platform.controller(),root_hwnd,pane_hwnd,session.ui_intent.clone(),Arc::new(move||weak.upgrade().and_then(|s|ui_binding(&s))));
                let weak=Arc::downgrade(&session);
                if escape_intent::install(&platform.controller(),root_hwnd,pane_hwnd,escape_intent.clone(),Arc::new(move||weak.upgrade().and_then(|session|escape_binding(&session)))).is_err(){
                    tracing::warn!(diag_stage="youtube_fullscreen_escape_unavailable");
                }
            }
            let mut token = 0;
            let _ = core.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut source = PWSTR::null();
                    args.Source(&mut source)?;
                    let source = CoTaskMemPWSTR::from(source).to_string();
                    let mut body = PWSTR::null();
                    if args.TryGetWebMessageAsString(&mut body).is_err() {
                        return Ok(());
                    }
                    let body = CoTaskMemPWSTR::from(body).to_string();
                    let Some(raw) = body
                        .strip_prefix(policy::PREFIX)
                        .filter(|v| v.len() <= policy::MAX_MESSAGE)
                    else {
                        return Ok(());
                    };
                    let Ok(mut message) = serde_json::from_str::<Envelope>(raw) else {
                        return Ok(());
                    };
                    if !tauri::Url::parse(&source)
                        .ok()
                        .as_ref()
                        .is_some_and(|url| policy::matches_document(url, &session.channel, is_chat))
                        || uuid::Uuid::parse_str(&message.id).is_err()
                        || message.epoch == 0
                        || message.epoch != session.epoch.load(Ordering::Acquire)
                        || native_view.label()
                            != if is_chat {
                                &session.chat_label
                            } else {
                                &session.label
                            }
                        || is_chat && message.kind != "chat_status"
                    {
                        return Ok(());
                    }
                    if message.kind=="ui_intent" {
                        if is_chat||message.fields["input"].as_str()!=Some("pointer")||!closed_fields(&message.fields,&["action","input"])||!matches!(message.fields["action"].as_str(),Some("record_start"|"record_stop"|"screenshot"|"viewer_close")){return Ok(());}
                        message.intent=player_intent::consume(&session.ui_intent,ui_binding(&session).as_ref(),root_hwnd,pane_hwnd);
                        if message.intent.is_none(){response(&native_view,&message.id,Err("버튼을 다시 눌러 주세요.".into()));return Ok(());}
                    }
                    if matches!(message.kind.as_str(), "fullscreen" | "screenshot") {
                        let key = message.fields["inputKey"].as_i64().unwrap_or(0) as i32;
                        if (message.kind == "screenshot" && key != 0x53)
                            || (message.kind == "fullscreen" && !matches!(key, 1 | 0x46 | 0x1B))
                        {
                            return Ok(());
                        }
                        let gate=platform_native_geometry::trusted_input_gate(root_hwnd,pane_hwnd,key);
                        if message.kind=="fullscreen"&&key==0x1B {
                            if message.fields["enabled"].as_bool()!=Some(false){escape_intent.clear();return Ok(());}
                            let (gate,grant,receipt_used)=escape_intent.exit(escape_binding(&session),gate,||platform_native_geometry::trusted_escape_receipt_context(root_hwnd,pane_hwnd),Instant::now());
                            input_diagnostics.record(false,key,gate,receipt_used);
                            if let Some(grant)=grant {let _=window_fullscreen::release(&input_app,grant);}
                            return Ok(());
                        }
                        if message.kind=="fullscreen" {input_diagnostics.record(false,key,gate,false);}
                        if !gate.allowed() {return Ok(());}
                    }
                    if let Err(std::sync::mpsc::TrySendError::Full(message)) = tx.try_send(message)
                    {
                        if message.kind != "status" {
                            response(
                                &native_view,
                                &message.id,
                                Err("YouTube 저장 요청이 많습니다.".into()),
                            );
                        }
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
    })
    .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
fn ui_binding(session:&Session)->Option<player_intent::Binding>{
    if session.closed.load(Ordering::Acquire)||session.account_refreshing.load(Ordering::Acquire){return None;}
    let binding=session.ui_intent.current(session.epoch.load(Ordering::Acquire))?;
    session.viewers.try_lock().ok()?.contains(&binding.token).then_some(binding)
}
fn dispatch_ui(host:&YouTubeOriginalHost,app:&AppHandle,session:&Arc<Session>,message:&Envelope)->Result<Value,String>{
    let binding=message.intent.as_ref().ok_or(STALE)?;
    if host.tray_muted.load(Ordering::Acquire)||ui_binding(session).as_ref()!=Some(binding){return Err(STALE.into());}
    let action=message.fields["action"].as_str().ok_or(STALE)?;
    match action{
        "record_start"|"record_stop"=>{
            let _sequence=lock(&session.viewport_sequences);
            if ui_binding(session).as_ref()!=Some(binding){return Err(STALE.into());}
            let snapshot=app.state::<crate::interface::commands::AppState>().begin_platform_work(||host.record(app,&binding.token,action=="record_start"))?;
            Ok(json!(snapshot))
        }
        "screenshot"=>{app.emit_to("main","youtube-original:action",json!({"token":binding.token,"action":"screenshot"})).map_err(|_|UNAVAILABLE)?;Ok(Value::Null)}
        "viewer_close" if session.close_enabled.load(Ordering::Acquire)=>{
            host.close(app,&binding.token)?;
            app.emit_to("main","youtube-original:action",json!({"token":binding.token,"action":"viewer_close"})).map_err(|_|UNAVAILABLE)?;
            Ok(Value::Null)
        }
        _=>Err(STALE.into())
    }
}
#[cfg(not(windows))]
fn attach_bridge(
    _: &Webview,
    _: Arc<Session>,
    _: YouTubeOriginalHost,
    _: bool,
) -> Result<(), String> {
    Err("YouTube 원본 플레이어는 Windows에서 사용할 수 있습니다.".into())
}
#[cfg(test)]
mod chat_availability_tests {
    use super::*;
    #[test]
    fn snapshot_projects_nullable_canonical_identity_without_changing_job() {
        let mut snapshot = OriginalSnapshot {
            token: "viewer".into(), channel_id: "abcdefghijk".into(), broadcast_id: "abcdefghijk".into(),
            canonical_channel_id: None, state: "playing".into(), authenticated: None,
            chat_visible: false, capture_ready: true, capture_unavailable_reason: None,
            recording_job: Some("job".into()), recording_stopping: false, fullscreen: false, error: None,
        };
        assert!(serde_json::to_value(&snapshot).unwrap()["canonicalChannelId"].is_null());
        snapshot.canonical_channel_id = Some("UCabcdefghijklmnopqrstuv".into());
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(encoded["canonicalChannelId"], "UCabcdefghijklmnopqrstuv");
        assert_eq!(encoded["channelId"], "abcdefghijk");
        snapshot.canonical_channel_id = None;
        assert_eq!(snapshot.recording_job.as_deref(), Some("job"));
        assert!(serde_json::to_value(&snapshot).unwrap()["canonicalChannelId"].is_null());
    }
    #[test]
    fn tri_state_requires_explicit_well_typed_descriptor() {
        let mut value = json!({"videoId":"abcdefghijk","channelId":"UCabcdefghijklmnopqrstuv","isLive":true});
        assert!(chat_observation(&value).is_none());
        for availability in [Value::Null, json!(false), json!(true)] {
            value["chatAvailable"] = availability.clone();
            assert_eq!(chat_observation(&value), Some(("abcdefghijk", "UCabcdefghijklmnopqrstuv", true, availability.as_bool())));
        }
        for invalid in [json!(0), json!("false"), json!({}), json!([])] {
            value["chatAvailable"] = invalid; assert!(chat_observation(&value).is_none());
        }
        assert!(chat_observation(&Value::Null).is_none());
        value["chatAvailable"] = Value::Null;
        value["isLive"] = json!("true"); assert!(chat_observation(&value).is_none());
    }
}
#[cfg(test)]
mod raw_transport_tests {
    use super::*;
    fn begin() -> Value {
        json!({"format":RAW_FORMAT,"rawMode":"segments","rawVideoTimestampOffset":0,
            "rawAudioTimestampOffset":0,"rawAppendWindow":"default","tracks":[{},{}]})
    }
    #[test]
    fn raw_format_is_explicit_closed_and_never_a_legacy_fallback() {
        assert!(!raw_begin_format(&json!({})).unwrap());
        assert!(raw_begin_format(&begin()).unwrap());
        for value in [Value::Null, json!("other"), json!(false)] {
            let mut fields = begin(); fields["format"] = value; assert!(raw_begin_format(&fields).is_err());
        }
        for key in ["rawMode", "rawAppendWindow", "rawVideoTimestampOffset", "rawAudioTimestampOffset"] {
            let mut fields = begin(); fields[key] = json!(1); assert!(raw_begin_format(&fields).is_err());
        }
        let mut fields = begin(); fields["extra"] = json!(true); assert!(raw_begin_format(&fields).is_err());
        let mut fields = begin(); fields["tracks"][0]["secret"] = json!(true); assert!(raw_begin_format(&fields).is_err());
    }
    #[test]
    fn raw_boundary_absent_differs_from_null_and_is_bounded_integer() {
        assert_eq!(raw_boundary(&json!({})).unwrap(), None);
        assert_eq!(raw_boundary(&json!({"rawFirstUnitEnd":1})).unwrap(), Some(1));
        for value in [Value::Null, json!(0), json!(-1), json!(1.5), json!(33_554_433u64), json!("4")] {
            assert!(raw_boundary(&json!({"rawFirstUnitEnd":value})).is_err());
        }
    }
}
