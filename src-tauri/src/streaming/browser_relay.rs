//! Selected existing CHZZK receiver. No source open, MediaJob or video file.
use super::*;
use crate::interface::commands::RelayWorkRegistration;
use crate::streaming::{
    relay_hls::{HlsDelivery, HlsTarget},
    relay_ingress::{
        InputEnd, JoinResult, NativeIdentity, RelayBinding, RelayInput, RelayProvider, RelayRun,
        WorkerReport,
    },
    relay_status::{DeliveryInspection, Inspection},
};
use encoded::fmp4::EncodedTrackInput;
use sha2::{Digest, Sha256};
use std::sync::{
    mpsc::{self, Receiver, SyncSender, TryRecvError},
    Weak,
};
use uuid::Uuid;
#[path = "browser_relay_chat.rs"]
mod sidecar;
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
fn fault() -> StreamError {
    error(
        "CHZZK_RELAY_STALE",
        "선택한 방송의 전송 상태가 변경됐습니다.",
    )
}
fn fail(r: &mut Inspection, code: &str) {
    r.interrupted = true;
    r.fault.get_or_insert_with(|| code.to_owned());
}

#[derive(Clone)]
pub(crate) struct ChzzkRelaySource {
    inner: Weak<Inner>,
    session: String,
    epoch: u64,
    revision: u64,
    channel: String,
}
impl ChzzkRelaySource {
    pub(crate) fn channel(&self) -> &str {
        &self.channel
    }
    pub(crate) fn broadcast(&self) -> Option<&str> {
        None
    }
    pub(crate) fn session_id(&self) -> &str {
        &self.session
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
    pub(crate) fn account_revision(&self) -> u64 {
        self.revision
    }
    fn matches(&self, inner: &Inner, v: &ViewState) -> bool {
        self.session == inner.relay_id
            && v.channel.as_deref() == Some(&self.channel)
            && v.page_generation == self.epoch
            && v.auth_generation == self.revision
            && v.open
            && !v.account_busy
            && !inner.closing.load(Ordering::Acquire)
            && !inner.detached.load(Ordering::Acquire)
    }
    pub(crate) fn current(&self) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|i| self.matches(&i, &lock(&i.view)))
    }
    pub(crate) fn start(
        &self,
        app: &AppHandle,
        run: Uuid,
        target: HlsTarget,
    ) -> Result<ChzzkSelectedRelayHandle, StreamError> {
        let inner = self.inner.upgrade().ok_or_else(fault)?;
        OfficialBrowser { inner }.start_selected_hls_relay(app, self, run, target)
    }
}
pub(crate) struct ChzzkSelectedRelayHandle {
    source: ChzzkRelaySource,
    run: Uuid,
    app: AppHandle,
    progress: Arc<Mutex<Inspection>>,
}
impl ChzzkSelectedRelayHandle {
    pub(crate) fn source_current(&self) -> bool {
        self.source.current()
    }
    pub(crate) fn inspection(&self) -> Inspection {
        if let Some(i) = self.source.inner.upgrade() {
            i.relay.reap();
        }
        lock(&self.progress).clone()
    }
    pub(crate) fn request_stop(&self, interrupted: bool) {
        if let Some(i) = self.source.inner.upgrade() {
            i.relay
                .request_stop(&self.app, &i.label, Some(self.run), interrupted);
        }
    }
}
impl Drop for ChzzkSelectedRelayHandle {
    fn drop(&mut self) {
        let joined = lock(&self.progress).manager_joined;
        if !joined {
            self.request_stop(true);
        }
    }
}

// ContextGroup normally holds Weak viewer references. A detached pane still
// owns its relay manager until real join; this native registry preserves that
// owner without changing recording or pane-disposal state.
static RETAINED: std::sync::OnceLock<Mutex<std::collections::HashMap<Uuid, Arc<Inner>>>> =
    std::sync::OnceLock::new();
fn retained() -> &'static Mutex<std::collections::HashMap<Uuid, Arc<Inner>>> {
    RETAINED.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}
fn relay_hosts(root: &OfficialBrowser) -> Vec<Arc<Inner>> {
    let mut result: Vec<_> = root
        .inner
        .contexts
        .hosts()
        .into_iter()
        .map(|h| h.inner)
        .collect();
    for inner in lock(retained()).values() {
        if Arc::ptr_eq(&inner.contexts, &root.inner.contexts)
            && !result.iter().any(|i| Arc::ptr_eq(i, inner))
        {
            result.push(inner.clone());
        }
    }
    result
}

#[derive(Default)]
pub(super) struct Slot {
    owner: Mutex<Option<Owner>>,
}
struct Owner {
    run: Uuid,
    epoch: u64,
    revision: u64,
    shared: Arc<Mutex<Admission>>,
    begin: SyncSender<BeginJob>,
    thread: thread::JoinHandle<Inspection>,
    registration: Option<RelayWorkRegistration>,
    proof: Arc<AtomicBool>,
    progress: Arc<Mutex<Inspection>>,
}
struct Admission {
    source: Option<Uuid>,
    video_track: Option<u32>,
    input: Option<(RelayInput, RelayBinding)>,
    stopped: bool,
    interrupted: bool,
    deadline: Option<Instant>,
    cancel: Arc<AtomicBool>,
    chat: Option<sidecar::Input>,
    probe: Option<(Uuid, Instant)>,
    last_performance: Option<f64>,
}
impl Admission {
    fn new() -> Self {
        Self {
            source: None,
            video_track: None,
            input: None,
            stopped: false,
            interrupted: false,
            deadline: None,
            cancel: Arc::new(AtomicBool::new(false)),
            chat: None,
            probe: None,
            last_performance: None,
        }
    }
    fn stop(&mut self, interrupted: bool) {
        self.stopped = true;
        self.interrupted |= interrupted;
        if self.interrupted {
            self.cancel.store(true, Ordering::Release);
        }
        if let Some((i, _)) = &self.input {
            if self.interrupted {
                i.cancel();
            } else {
                i.finish();
            }
        }
    }
    fn expire(&mut self) {
        if self.deadline.is_some_and(|d| Instant::now() >= d) && !self.stopped {
            self.stop(true);
        }
    }
}
impl Slot {
    pub(super) fn active(&self) -> bool {
        lock(&self.owner).is_some()
    }
    fn request_stop(&self, app: &AppHandle, label: &str, run: Option<Uuid>, interrupted: bool) {
        let command = {
            let owner = lock(&self.owner);
            let Some(o) = owner.as_ref().filter(|o| run.is_none_or(|r| r == o.run)) else {
                return;
            };
            let mut s = lock(&o.shared);
            if interrupted {
                s.stop(true);
            } else if !s.stopped {
                s.deadline
                    .get_or_insert(Instant::now() + Duration::from_secs(4));
            }
            json!({"kind":"stop","runId":o.run,"epoch":o.epoch,"interrupted":interrupted})
        };
        send_command(app, label, &command);
    }
    /// Only a finished manager with child-join proof releases the work-set.
    pub(super) fn reap(&self) {
        let mut slot = lock(&self.owner);
        if !slot
            .as_ref()
            .is_some_and(|o| o.thread.is_finished() && o.proof.load(Ordering::Acquire))
        {
            return;
        }
        let mut o = slot.take().unwrap();
        let mut r = o.thread.join().unwrap_or_else(|_| {
            let mut r = lock(&o.progress).clone();
            fail(&mut r, "ManagerPanic");
            r
        });
        r.manager_joined = true;
        *lock(&o.progress) = r;
        if let Some(reg) = o.registration.take() {
            reg.release_after_join();
        }
        lock(retained()).remove(&o.run);
    }
    pub(super) fn cancel_busy(&self, run: &str) {
        if let Ok(slot) = self.owner.try_lock() {
            if let Some(o) = slot.as_ref().filter(|o| o.run.to_string() == run) {
                if let Ok(mut s) = o.shared.try_lock() {
                    s.stop(true);
                }
            }
        }
    }
    pub(super) fn clock_lost(&self, run: &str) {
        // No waiting on the UI callback. If either lock is busy, the still
        // outstanding native nonce records an Unknown timeout on its next tick.
        if let Ok(slot) = self.owner.try_lock() {
            if let Some(owner) = slot.as_ref().filter(|o| o.run.to_string() == run) {
                if let Ok(state) = owner.shared.try_lock() {
                    if let Some(chat) = &state.chat {
                        chat.timing_missing();
                    }
                }
            }
        }
    }
}
fn send_command(app: &AppHandle, label: &str, value: &Value) {
    if let Some(w) = app.get_webview(label) {
        let _=w.eval(format!("window.dispatchEvent(new CustomEvent('atsumi-browser-relay-command',{{detail:{value}}}));"));
    }
}
impl OfficialBrowser {
    pub(crate) fn relay_available_channels(&self) -> Vec<String> {
        let mut result = Vec::new();
        for h in self.inner.contexts.hosts() {
            let v = lock(&h.inner.view);
            if v.open
                && v.ready
                && !v.account_busy
                && !h.inner.detached.load(Ordering::Acquire)
                && !h.inner.closing.load(Ordering::Acquire)
            {
                if let Some(c) = &v.channel {
                    result.push(c.clone());
                }
            }
        }
        result.sort();
        result.dedup();
        result.retain(|channel| self.select_relay_source_by_channel(channel).is_ok());
        result
    }
    pub(crate) fn select_relay_source_by_channel(
        &self,
        channel: &str,
    ) -> Result<ChzzkRelaySource, StreamError> {
        if channel.len() != 32
            || !channel
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(fault());
        }
        let mut found = None;
        for host in self.inner.contexts.hosts() {
            let v = lock(&host.inner.view);
            let source = ChzzkRelaySource {
                inner: Arc::downgrade(&host.inner),
                session: host.inner.relay_id.clone(),
                epoch: v.page_generation,
                revision: v.auth_generation,
                channel: channel.into(),
            };
            if v.ready && source.epoch > 0 && source.matches(&host.inner, &v) {
                if found.is_some() {
                    return Err(fault());
                }
                found = Some(source);
            }
        }
        found.ok_or_else(fault)
    }
    pub(crate) fn relay_source_current(&self, source: &ChzzkRelaySource) -> bool {
        source.current()
    }
    pub(crate) fn start_selected_hls_relay(
        &self,
        app: &AppHandle,
        source: &ChzzkRelaySource,
        run: Uuid,
        target: HlsTarget,
    ) -> Result<ChzzkSelectedRelayHandle, StreamError> {
        if run.is_nil() || !source.current() {
            return Err(fault());
        }
        let inner = source.inner.upgrade().ok_or_else(fault)?;
        inner.relay.reap();
        let label = inner.label.clone();
        let appstate = app.try_state::<AppState>().ok_or_else(fault)?;
        let app2 = app.clone();
        let selected = source.clone();
        let progress = appstate
            .begin_relay_work(
                &source.session,
                &run.to_string(),
                move |mut registration| {
                    let v = lock(&inner.view);
                    if !v.ready || !selected.matches(&inner, &v) {
                        return Err("CHZZK_RELAY_STALE".into());
                    }
                    let mut slot = lock(&inner.relay.owner);
                    if lock(retained())
                        .values()
                        .filter(|i| Arc::ptr_eq(&i.contexts, &inner.contexts))
                        .count()
                        >= 9
                    {
                        return Err("CHZZK_RELAY_CAPACITY".into());
                    }
                    if slot.is_some() {
                        return Err("CHZZK_RELAY_ACTIVE".into());
                    }
                    let shared = Arc::new(Mutex::new(Admission::new()));
                    let proof = Arc::new(AtomicBool::new(false));
                    let progress = Arc::new(Mutex::new(Inspection {
                        run_id: run.to_string(),
                        delivery: Some(Default::default()),
                        ..Default::default()
                    }));
                    let (tx, rx) = mpsc::sync_channel(1);
                    let worker_inner = inner.clone();
                    let state = shared.clone();
                    let done = proof.clone();
                    let p = progress.clone();
                    let worker_source = selected.clone();
                    let worker_app = app2.clone();
                    let thread = thread::Builder::new()
                        .name("chzzk-relay".into())
                        .spawn(move || {
                            run_manager(
                                worker_inner,
                                worker_source,
                                run,
                                state,
                                rx,
                                worker_app,
                                target,
                                done,
                                p,
                            )
                        })
                        .map_err(|_| "CHZZK_RELAY_SPAWN".to_string())?;
                    registration.mark_started();
                    *slot = Some(Owner {
                        run,
                        epoch: selected.epoch,
                        revision: selected.revision,
                        shared,
                        begin: tx,
                        thread,
                        registration: Some(registration),
                        proof,
                        progress: progress.clone(),
                    });
                    lock(retained()).insert(run, inner.clone());
                    Ok(progress)
                },
            )
            .map_err(|_| fault())?;
        send_command(
            app,
            &label,
            &json!({"kind":"start","runId":run,"epoch":source.epoch,"channelId":source.channel}),
        );
        Ok(ChzzkSelectedRelayHandle {
            source: source.clone(),
            run,
            app: app.clone(),
            progress,
        })
    }
    pub(crate) fn request_relay_shutdown_all(&self, app: &AppHandle) {
        for inner in relay_hosts(self) {
            inner.relay.request_stop(app, &inner.label, None, false);
        }
    }
    pub(crate) fn poll_relay_join_all(&self, _app: &AppHandle) {
        for inner in relay_hosts(self) {
            inner.relay.reap();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Wire {
    pub run_id: String,
    pub epoch: u64,
    pub source_id: Option<String>,
    pub packet: Packet,
}
#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum Packet {
    Begin {
        tracks: Vec<Track>,
    },
    Append {
        track_index: u32,
        append_index: u64,
        chunk_index: u32,
        final_chunk: bool,
        data: String,
    },
    Finish {
        interrupted: bool,
    },
    Chat {
        batch_id: u64,
        events: Vec<Value>,
    },
    ChatGap,
    Clock {
        nonce: String,
        sample: Option<Clock>,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Track {
    track_index: u32,
    mime_type: String,
    init: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Clock {
    clock: String,
    source_id: String,
    source_time_seconds: f64,
    current_time: f64,
    timestamp_offset: f64,
    track_index: u32,
    performance_now_ms: f64,
    // Old sidecars and in-flight old documents lack these fields. Absence is
    // preserved as unknown; it must not be upgraded to a playing observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seeking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ready_state: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    playback_rate: Option<f64>,
}
impl Clock {
    fn valid(&self, source: Uuid) -> bool {
        self.clock == "mse_presentation_v1"
            && self.source_id == source.to_string()
            && self.track_index <= 1
            && [
                self.source_time_seconds,
                self.current_time,
                self.timestamp_offset,
                self.performance_now_ms,
            ]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e12)
            && self.source_time_seconds >= 0.
            && self.source_time_seconds <= 1e9
            && self.current_time >= 0.
            && self.current_time <= 1e9
            && self.timestamp_offset.abs() <= 1e9
            && self.performance_now_ms >= 0.
            && (self.current_time - self.timestamp_offset - self.source_time_seconds).abs() <= 1e-6
            && self.playback_state_valid()
    }
    fn playback_state_valid(&self) -> bool {
        match (self.paused, self.seeking, self.ready_state, self.ended, self.playback_rate) {
            (None, None, None, None, None) => true,
            (Some(_), Some(_), Some(ready), Some(_), Some(rate)) =>
                ready <= 4 && rate.is_finite() && (0.0625..=16.).contains(&rate),
            _ => false,
        }
    }
}
struct BeginJob {
    source: Uuid,
    tracks: Vec<EncodedTrackInput>,
    reply_id: String,
    window: Webview,
}
fn tracks(values: Vec<Track>) -> Result<Vec<EncodedTrackInput>, StreamError> {
    if values.is_empty() || values.len() > 2 {
        return Err(fault());
    }
    let mut out: Vec<EncodedTrackInput> = Vec::new();
    for v in values {
        if v.track_index > 1
            || v.mime_type.len() > 120
            || v.init.len() > 88_000
            || out.iter().any(|t| t.track_index == v.track_index)
        {
            return Err(fault());
        }
        let bytes = STANDARD.decode(v.init).map_err(|_| fault())?;
        if bytes.is_empty() || bytes.len() > 65536 {
            return Err(fault());
        }
        out.push(EncodedTrackInput {
            track_index: v.track_index,
            mime_type: v.mime_type,
            init: bytes,
        });
    }
    Ok(out)
}
/// Dedicated bounded native bridge worker calls this; begin/chat ACK may be deferred.
pub(super) fn dispatch(
    host: &OfficialBrowser,
    window: &Webview,
    channel: &str,
    id: String,
    wire: Wire,
) {
    let media = matches!(
        &wire.packet,
        Packet::Begin { .. } | Packet::Append { .. } | Packet::Finish { .. }
    );
    let result = (|| -> Result<Option<Value>, StreamError> {
        let v = lock(&host.inner.view);
        if host.inner.detached.load(Ordering::Acquire)
            || host.inner.closing.load(Ordering::Acquire)
            || v.page_generation != wire.epoch
            || v.channel.as_deref() != Some(channel)
            || v.account_busy
        {
            return Err(fault());
        }
        let owner = lock(&host.inner.relay.owner);
        let o = owner
            .as_ref()
            .filter(|o| {
                o.run.to_string() == wire.run_id
                    && o.epoch == wire.epoch
                    && o.revision == v.auth_generation
            })
            .ok_or_else(fault)?;
        let mut s = lock(&o.shared);
        drop(v);
        s.expire();
        let supplied = wire
            .source_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| fault())?;
        if s.source.is_some() && s.source != supplied {
            return Err(fault());
        }
        match wire.packet {
            Packet::Begin { tracks: raw } => {
                if s.stopped || s.source.is_some() {
                    return Err(fault());
                }
                let source = supplied.filter(|s| !s.is_nil()).ok_or_else(fault)?;
                let tracks = tracks(raw)?;
                s.video_track = tracks
                    .iter()
                    .find(|t| t.mime_type.to_ascii_lowercase().contains("avc"))
                    .map(|t| t.track_index);
                s.source = Some(source);
                o.begin
                    .try_send(BeginJob {
                        source,
                        tracks,
                        reply_id: id.clone(),
                        window: window.clone(),
                    })
                    .map_err(|_| fault())?;
                Ok(None)
            }
            Packet::Append {
                track_index,
                append_index,
                chunk_index,
                final_chunk,
                data,
            } => {
                if s.stopped || data.len() > 174_764 || append_index > 9_007_199_254_740_991 {
                    return Err(fault());
                }
                let bytes = STANDARD.decode(data).map_err(|_| fault())?;
                let (input, binding) = s.input.as_ref().ok_or_else(fault)?;
                input
                    .try_enqueue(
                        binding,
                        track_index,
                        append_index,
                        chunk_index,
                        final_chunk,
                        &bytes,
                    )
                    .map_err(|_| fault())?;
                Ok(Some(json!({"acceptedInMemory":true})))
            }
            Packet::Finish { interrupted } => {
                s.stop(interrupted);
                Ok(Some(
                    json!({"acceptedInMemory":true,"stopRequested":true,"interrupted":s.interrupted}),
                ))
            }
            Packet::Chat { batch_id, events } => {
                let chat = s.chat.as_ref().ok_or_else(fault)?;
                chat.batch(batch_id, events, window.clone(), id.clone())?;
                Ok(None)
            }
            Packet::ChatGap => {
                if let Some(c) = &s.chat {
                    c.partial();
                }
                Ok(Some(json!({"acceptedInMemory":true})))
            }
            Packet::Clock { nonce, sample } => {
                let Some((expected, before)) = s.probe else {
                    return Err(fault());
                };
                if nonce != expected.to_string() {
                    return Err(fault());
                }
                s.probe = None;
                let now = Instant::now();
                let valid = sample.as_ref().filter(|c| {
                    s.source.is_some_and(|source| c.valid(source))
                        && s.video_track == Some(c.track_index)
                        && s.last_performance.is_none_or(|p| c.performance_now_ms >= p)
                        && now.duration_since(before) <= Duration::from_secs(3)
                });
                if let Some(c) = valid {
                    s.last_performance = Some(c.performance_now_ms);
                }
                if let Some(chat) = &s.chat {
                    chat.clock(expected, before, now, valid);
                }
                Ok(Some(json!({"acceptedInMemory":true})))
            }
        }
    })();
    match result {
        Ok(None) => {}
        Ok(Some(v)) => reply(window, &id, Ok(v)),
        Err(e) => {
            if media {
                host.inner.relay.cancel_busy(&wire.run_id);
            }
            reply(window, &id, Err(e));
        }
    }
}

struct Resources {
    worker: Option<RelayRun>,
    chat: Option<sidecar::Owner>,
    frames: Option<crate::cloud_recording::live_fingerprint::FingerprintWorker>,
    shared: Arc<Mutex<Admission>>,
    proof: Arc<AtomicBool>,
    settled: bool,
}
impl Drop for Resources {
    fn drop(&mut self) {
        if !self.settled {
            lock(&self.shared).stop(true);
        }
        if let Some(w) = self.worker.as_mut() {
            w.cancel();
            while matches!(w.join(Duration::from_millis(10)), JoinResult::Pending) {}
        }
        self.frames.take(); // Optional decoder still owns real child/worker joins.
        if let Some(c) = self.chat.as_mut() {
            if !self.settled {
                c.finish(true);
            }
            c.join();
        }
        lock(&self.shared).input.take();
        lock(&self.shared).chat.take();
        self.proof.store(true, Ordering::Release);
    }
}
fn worker_facts(r: &mut Inspection, w: WorkerReport) {
    r.worker_joined = true;
    r.consumed_chunks = w.counts.consumed_chunks;
    r.validated_appends = w.counts.validated_appends;
    r.duplicate_chunks = w.counts.duplicate_chunks;
    r.input_peak_bytes = w.input_peak_bytes;
    r.output_peak_bytes = w.output_peak_bytes;
    r.codec_diagnostic = w.counts.codec_diagnostic;
    if let InputEnd::Interrupted(f) = w.input_end {
        r.input_fault = Some(format!("{f:?}"));
        fail(r, "Input");
    } else {
        r.input_ended = true;
    }
}
fn delivery_facts(r: &mut Inspection, d: &HlsDelivery) {
    let p = d.progress();
    r.delivery = Some(DeliveryInspection {
        segment_acks: p.segment_acks,
        playlist_acks: p.playlist_acks,
        requests: p.requests,
        acknowledged_bytes: p.acknowledged_bytes,
        end_list_acked: p.end_list_acked,
        fault: r.delivery.as_ref().and_then(|d| d.fault.clone()),
    });
}
fn segment_facts(
    r: &mut Inspection,
    segment: &encoded::fmp4::relay::ts::TsSegment,
) -> Result<(), StreamError> {
    // TsTiming is sealed by the muxer. This is the same exact integer source
    // fact object persisted in the ACK ledger, never page-reported statistics.
    let facts = serde_json::to_value(segment.timing()).map_err(|_| fault())?;
    let mut next = r.clone();
    for (name, count, end, scale) in [
        (
            "video",
            &mut next.video_samples,
            &mut next.video_end,
            &mut next.video_timescale,
        ),
        (
            "audio",
            &mut next.audio_samples,
            &mut next.audio_end,
            &mut next.audio_timescale,
        ),
    ] {
        let value = &facts[name];
        let samples = value["samples"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(fault)?;
        let first = value["firstDts"].as_u64().ok_or_else(fault)?;
        let last = value["lastDtsEnd"]
            .as_u64()
            .filter(|n| *n > first)
            .ok_or_else(fault)?;
        let timescale = value["timescale"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(fault)?;
        if end.is_some_and(|n| n != first) || scale.is_some_and(|n| n != timescale) {
            return Err(fault());
        }
        *count = count.checked_add(samples).ok_or_else(fault)?;
        *end = Some(last);
        *scale = Some(timescale);
    }
    *r = next;
    Ok(())
}
fn settle_pending_probe(shared: &Mutex<Admission>) {
    let mut state = lock(shared);
    if let Some((nonce, before)) = state.probe.take() {
        if let Some(chat) = &state.chat {
            chat.timing_missing();
            chat.clock(nonce, before, Instant::now(), None);
        }
    }
}
fn run_manager(
    inner: Arc<Inner>,
    source: ChzzkRelaySource,
    run: Uuid,
    shared: Arc<Mutex<Admission>>,
    begin: Receiver<BeginJob>,
    app: AppHandle,
    target: HlsTarget,
    proof: Arc<AtomicBool>,
    progress: Arc<Mutex<Inspection>>,
) -> Inspection {
    let mut r = lock(&progress).clone();
    r.frame_reference = Some(crate::streaming::relay_status::FrameReferenceInspection::unavailable("tools_unavailable"));
    let mut resources = Resources {
        worker: None,
        chat: None,
        frames: None,
        shared: shared.clone(),
        proof,
        settled: false,
    };
    let deadline = Instant::now() + Duration::from_secs(25);
    let job = loop {
        lock(&shared).expire();
        let stopped = lock(&shared).stopped;
        if stopped || !source.current() || Instant::now() >= deadline {
            fail(&mut r, "ArmStopped");
            return r;
        }
        match begin.recv_timeout(Duration::from_millis(10)) {
            Ok(j) => break j,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => {
                fail(&mut r, "BeginClosed");
                return r;
            }
        }
    };
    let setup = (|| -> Result<_, StreamError> {
        let chat = sidecar::Owner::start(&inner, run, &source, job.source)?;
        lock(&shared).chat = Some(chat.input());
        resources.chat = Some(chat);
        let binding = RelayBinding::new(NativeIdentity {
            provider: RelayProvider::Chzzk,
            owner: Uuid::parse_str(&source.session).map_err(|_| fault())?,
            run,
            source: job.source,
            document_epoch: source.epoch,
        })
        .map_err(|_| fault())?;
        let parts = RelayRun::start_ts(binding.clone(), job.tracks).map_err(|_| fault())?;
        Ok((binding, parts))
    })();
    let (binding, (worker, input, output)) = match setup {
        Ok(x) => x,
        Err(e) => {
            reply(&job.window, &job.reply_id, Err(e));
            fail(&mut r, "Start");
            return r;
        }
    };
    resources.worker = Some(worker);
    resources.frames = inner.merges.relay_media_tools().and_then(|tools|
        match crate::cloud_recording::live_fingerprint::FingerprintWorker::start(tools, run, job.source) {
            Ok(worker) => Some(worker),
            Err(_) => { r.frame_reference = Some(crate::streaming::relay_status::FrameReferenceInspection::unavailable("reference_worker_unavailable")); None }
        });
    if resources.frames.is_some() {
        r.frame_reference = Some(crate::streaming::relay_status::FrameReferenceInspection::sampling());
    }
    lock(&shared).input = Some((input.clone(), binding.clone()));
    let cancel = lock(&shared).cancel.clone();
    let mut delivery = match HlsDelivery::new(target, cancel) {
        Ok(d) => if let Some(frames) = &resources.frames { d.with_frame_sampler(frames.sink()) } else { d },
        Err(_) => {
            reply(&job.window, &job.reply_id, Err(fault()));
            fail(&mut r, "Target");
            return r;
        }
    };
    let mut admitted = false;
    let mut joined = None;
    let mut hash = Sha256::new();
    let mut next_probe = Instant::now();
    loop {
        {
            let current = source.current();
            let mut s = lock(&shared);
            s.expire();
            if !current {
                s.stop(true);
            }
            if s.interrupted {
                fail(&mut r, "Cancelled");
                resources.worker.as_mut().unwrap().cancel();
            }
        }
        if !admitted && input.codec_admitted() && !r.interrupted {
            admitted = true;
            r.codec_admitted = true;
            reply(
                &job.window,
                &job.reply_id,
                Ok(
                    json!({"runId":run,"mode":"relay","nativeApproved":true,"acceptedInMemory":true,"captureChat":true}),
                ),
            );
        }
        if admitted && Instant::now() >= next_probe {
            let now = Instant::now();
            let command = {
                let mut s = lock(&shared);
                if let Some((nonce, before)) = s.probe.take() {
                    if let Some(c) = &s.chat {
                        c.clock(nonce, before, now, None);
                    }
                }
                if !s.stopped {
                    let nonce = Uuid::new_v4();
                    s.probe = Some((nonce, now));
                    Some(json!({"kind":"clock","runId":run,"epoch":source.epoch,"nonce":nonce}))
                } else {
                    None
                }
            };
            if let Some(command) = command {
                send_command(&app, &inner.label, &command);
            }
            next_probe = now + Duration::from_secs(3);
        }
        if joined.is_none() {
            if let JoinResult::Joined(report) =
                resources.worker.as_mut().unwrap().join(Duration::ZERO)
            {
                worker_facts(&mut r, report);
                joined = Some(report);
            }
        }
        match output.try_recv() {
            Ok(guard) => {
                if guard.binding().identity() != binding.identity() {
                    fail(&mut r, "Binding");
                } else if let Some(segment) = guard.ts_segment() {
                    if segment_facts(&mut r, segment).is_err() {
                        fail(&mut r, "TimingFacts");
                    }
                    r.output_chunks += 1;
                    r.output_bytes += segment.bytes().len() as u64;
                    hash.update(segment.bytes());
                    if !r.interrupted {
                        let before = Instant::now();
                        match delivery.deliver(segment) {
                            Ok(_) => {
                                if let Some(c) = resources.chat.as_ref() {
                                    c.segment(segment, before, Instant::now());
                                }
                            }
                            Err(_) => fail(&mut r, "Delivery"),
                        };
                        delivery_facts(&mut r, &delivery);
                    }
                } else {
                    fail(&mut r, "Payload");
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) if joined.is_some() => break,
            Err(_) => thread::sleep(Duration::from_millis(1)),
        }
        if r.interrupted {
            if let Some(frames) = &resources.frames { frames.begin_finish(true); }
            lock(&shared).stop(true);
        }
        *lock(&progress) = r.clone();
    }
    if !admitted {
        reply(&job.window, &job.reply_id, Err(fault()));
    }
    if !r.interrupted {
        if let InputEnd::TsEnded(end) = joined.unwrap().input_end {
            let before = Instant::now();
            match delivery.finish(&end) {
                Ok(_) => {
                    if let Some(c) = resources.chat.as_ref() {
                        c.end(&end, before, Instant::now());
                    }
                }
                Err(_) => fail(&mut r, "EndList"),
            };
            delivery_facts(&mut r, &delivery);
        } else {
            fail(&mut r, "Unfinished");
        }
    }
    r.output_sha256 = format!("{:x}", hash.finalize());
    settle_pending_probe(&shared);
    if let Some(c) = resources.chat.as_mut() {
        c.finish(r.interrupted);
        let facts = loop {
            if lock(&shared).interrupted {
                c.finish(true);
            }
            if let Some(facts) = c.poll_join() {
                break facts;
            }
            *lock(&progress) = r.clone();
            thread::sleep(Duration::from_millis(10));
        };
        r.sidecar_directory = Some(c.root.clone());
        r.sidecar_owner = Some(c.native_owner.clone());
        if let Some(frames) = resources.frames.as_mut() {
            frames.begin_finish(r.interrupted);
            let mut publish_failures = 0;
            let state = loop {
                if lock(&shared).interrupted { frames.begin_finish(true); }
                match frames.poll_finish_into(&c.root) {
                    Ok(Some(reference)) => break crate::streaming::relay_status::FrameReferenceInspection::settled(&reference),
                    Ok(None) => { *lock(&progress) = r.clone(); thread::sleep(Duration::from_millis(10)); },
                    Err(_) => {
                        publish_failures += 1;
                        if publish_failures >= 3 { break crate::streaming::relay_status::FrameReferenceInspection::failed_joined(); }
                        thread::sleep(Duration::from_millis(100));
                    },
                }
            };
            // Drop also joins on the error path. No manager can publish its
            // joined sidecar while an optional decoder is still running.
            resources.frames.take();
            r.frame_reference = Some(state);
        }
        if facts.failure || !facts.joined {
            fail(&mut r, "Sidecar");
        }
        r.interrupted |= facts.interrupted;
        r.sidecar = Some(facts);
    }
    send_command(
        &app,
        &inner.label,
        &json!({"kind":"stop","runId":run,"epoch":source.epoch,"interrupted":r.interrupted}),
    );
    resources.settled = true;
    *lock(&progress) = r.clone();
    r
}

#[cfg(test)]
#[path = "browser_relay_tests.rs"]
mod tests;
