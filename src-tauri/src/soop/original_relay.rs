//! Independent native lane for the already selected official SOOP document.
//! No new receiver, MediaJob, page-supplied target or account authority lives here.
//! Native HLS mode owns delivery and a source-bound lightweight chat lease.
#[path = "original_relay_delivery.rs"]
mod delivery;
use super::{lock, Envelope, Session, SoopOriginalHost, STALE, UNAVAILABLE};
use crate::interface::commands::RelayWorkRegistration;
use crate::platform_media::encoded::OfficialReceiverLease;
use crate::platform_media::relay_chat::timing::{ClockSample, RelayTimingSink};
use crate::streaming::relay_hls::{DeliveryFault, DeliveryProgress, HlsTarget};
use crate::streaming::{
    browser::encoded::fmp4::EncodedTrackInput,
    relay_ingress::{
        InputEnd, JoinResult, NativeIdentity, RelayBinding, RelayInput, RelayProvider, RelayRun,
        WorkerReport,
    },
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, Webview};
use uuid::Uuid;

const ARM_TIMEOUT: Duration = Duration::from_secs(25);
const STOP_TIMEOUT: Duration = Duration::from_secs(4);
const JS_INTEGER: u64 = 9_007_199_254_740_991;
const CHUNK_BYTES: usize = 128 * 1024;

/// Native selection of an already open source. No Deserialize/Serialize: a page
/// or IPC caller cannot reconstruct this authority from public status fields.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SoopRelaySource {
    session_id: String,
    epoch: u64,
    account_revision: u64,
    channel: String,
    broadcast: String,
}
impl SoopRelaySource {
    fn from_session(session: &Session) -> Self {
        Self {
            session_id: session.id.clone(), epoch: session.epoch.load(Ordering::Acquire),
            account_revision: session.account_revision.load(Ordering::Acquire),
            channel: session.channel.clone(), broadcast: session.broadcast.clone(),
        }
    }
    pub(crate) fn channel(&self) -> &str { &self.channel }
    pub(crate) fn broadcast(&self) -> &str { &self.broadcast }
    pub(crate) fn session_id(&self) -> &str { &self.session_id }
    pub(crate) fn epoch(&self) -> u64 { self.epoch }
    pub(crate) fn account_revision(&self) -> u64 { self.account_revision }
    #[cfg(test)]
    pub(crate) fn test_fixture() -> Self {
        // No session or receiver is created; tests cannot admit this at a host.
        Self { session_id: Uuid::new_v4().to_string(), epoch: 1, account_revision: 1,
            channel: "fixture".into(), broadcast: "1".into() }
    }
}
fn selection_matches(expected: &SoopRelaySource, current: &SoopRelaySource, available: bool, viewers: bool) -> bool {
    expected == current && current.epoch != 0 && available && viewers
}

/// Exact-run observation survives token removal and a later session run. The
/// manager work registration, not this handle's Drop, proves resource cleanup.
pub(crate) struct SelectedRelayHandle {
    app: AppHandle,
    // Host registry/work registration own the live Session through join. A
    // receipt observer must not extend OfficialReceiverLease into app shutdown.
    session: std::sync::Weak<Session>,
    source: SoopRelaySource,
    run: Uuid,
    progress: Arc<Mutex<Inspection>>,
}
impl SelectedRelayHandle {
    pub(crate) fn source_current(&self) -> bool {
        self.session.upgrade().is_some_and(|session|
            self.source == SoopRelaySource::from_session(&session)
                && !session.closed.load(Ordering::Acquire))
    }
    pub(crate) fn inspection(&self) -> Inspection {
        if let Some(session) = self.session.upgrade() { session.relay.reap(); }
        lock(&self.progress).clone()
    }
    pub(crate) fn request_stop(&self, interrupted: bool) {
        if let Some(session) = self.session.upgrade() {
            session.relay.request_stop_bound(Some(self.run), self.app.get_webview(&session.label), interrupted);
        }
    }
}
impl Drop for SelectedRelayHandle {
    fn drop(&mut self) {
        // No WebView call or join on Drop, and never cancel a newer run.
        let Some(session) = self.session.upgrade() else { return; };
        let owner = lock(&session.relay.owner);
        if let Some(owner) = owner.as_ref().filter(|owner| owner.context.run == self.run) {
            lock(&owner.shared).stop(true);
        }
    }
}

#[derive(Default)]
pub(super) struct Slot {
    owner: Mutex<Option<Owner>>,
    last: Mutex<Option<Inspection>>,
}
struct Owner {
    context: Arc<Context>,
    shared: Arc<Mutex<Admission>>,
    begin: SyncSender<BeginJob>,
    handle: JoinHandle<Inspection>,
    registration: Option<RelayWorkRegistration>,
    cleanup_proof: Arc<AtomicBool>,
    graceful_shutdown: bool,
    progress: Arc<Mutex<Inspection>>,
}
#[derive(Clone)]
struct Context {
    owner: Uuid,
    run: Uuid,
    epoch: u64,
    channel: String,
    broadcast: String,
}
#[derive(Default)]
struct Admission {
    source: Option<Uuid>,
    begin_sent: bool,
    admitted: bool,
    stopped: bool,
    interrupted: bool,
    stop_deadline: Option<Instant>,
    transport_cancel: Arc<AtomicBool>,
    input: Option<(RelayInput, RelayBinding)>,
    timing: Option<RelayTimingSink>,
}
impl Admission {
    fn stop(&mut self, interrupted: bool) {
        self.stopped = true;
        self.interrupted |= interrupted || self.input.is_none();
        if self.interrupted {
            self.transport_cancel.store(true, Ordering::Release);
        }
        if let Some((input, _)) = &self.input {
            if self.interrupted {
                input.cancel();
            } else {
                input.finish();
            }
        }
    }
}
enum ManagerMode {
    #[cfg(all(test, windows))]
    Inspector,
    Hls {
        target: HlsTarget,
        receiver: Option<OfficialReceiverLease>,
    },
}
struct BeginJob {
    source: Uuid,
    tracks: Vec<EncodedTrackInput>,
    reply_id: String,
}
#[derive(Clone, Default)]
struct Replies {
    view: Option<Webview>,
    epoch: u64,
}
impl Replies {
    fn clock_probe(&self, run:Uuid, nonce:Uuid) {
        if let Some(view)=&self.view {
            let _=view.eval(&format!("window.__atsumiSoopOriginal?.relayClockProbe({},{},{})",json!(nonce.to_string()),json!(run.to_string()),self.epoch));
        }
    }
    fn send(&self, id: &str, result: Result<Value, String>) {
        if let Some(view) = &self.view {
            reply(view, id, self.epoch, result);
        }
    }
    fn stop(&self, run: Uuid) {
        if let Some(view) = &self.view {
            let _ = view.eval(&format!(
                "window.__atsumiSoopOriginal?.relayStop({},{})",
                json!(run.to_string()),
                self.epoch
            ));
        }
    }
}

pub(crate) use crate::streaming::relay_status::{Inspection, DeliveryInspection, SidecarInspection};
impl DeliveryInspection {
    fn observe(&mut self, p: DeliveryProgress) {
        self.segment_acks = p.segment_acks;
        self.playlist_acks = p.playlist_acks;
        self.requests = p.requests;
        self.acknowledged_bytes = p.acknowledged_bytes;
        self.end_list_acked = p.end_list_acked;
    }
}
impl Inspection {
    fn fail(&mut self, fault: &str) {
        self.interrupted = true;
        self.fault.get_or_insert_with(|| fault.to_owned());
    }
    fn delivery_failed(&mut self, fault: DeliveryFault) {
        let text = format!("{fault:?}");
        self.delivery
            .get_or_insert_with(Default::default)
            .fault
            .get_or_insert(text.clone());
        self.fail(&format!("Delivery:{text}"));
    }
    fn worker(&mut self, report: WorkerReport) {
        if self.codec_diagnostic.is_none() {
            self.codec_diagnostic = report.counts.codec_diagnostic;
        }
        self.worker_joined = true;
        self.input_ended = matches!(report.input_end, InputEnd::Ended(_) | InputEnd::TsEnded(_));
        self.interrupted |= !self.input_ended;
        if let InputEnd::Interrupted(fault) = report.input_end {
            self.input_fault = Some(format!("{fault:?}"));
            self.fail(&format!("{fault:?}"));
        }
        self.consumed_chunks = report.counts.consumed_chunks;
        self.validated_appends = report.counts.validated_appends;
        self.duplicate_chunks = report.counts.duplicate_chunks;
        self.input_peak_bytes = report.input_peak_bytes;
        self.output_peak_bytes = report.output_peak_bytes;
    }
}

impl Slot {
    pub(super) fn active(&self) -> bool {
        lock(&self.owner).is_some()
    }
    pub(super) fn cancel(&self) {
        if let Some(owner) = lock(&self.owner).as_ref() {
            lock(&owner.shared).stop(true);
        }
    }
    fn request_stop(&self, view: Option<Webview>, interrupted: bool) {
        self.request_stop_bound(None, view, interrupted);
    }
    fn request_stop_bound(&self, expected: Option<Uuid>, view: Option<Webview>, interrupted: bool) {
        let command = {
            let slot = lock(&self.owner);
            let Some(owner) = slot.as_ref() else {
                return;
            };
            if expected.is_some_and(|run| owner.context.run != run) { return; }
            let mut state = lock(&owner.shared);
            if interrupted {
                state.stop(true);
            } else if !state.stopped {
                state
                    .stop_deadline
                    .get_or_insert(Instant::now() + STOP_TIMEOUT);
            }
            (owner.context.run, owner.context.epoch, owner.shared.clone())
        };
        if !view.is_some_and(|view| {
            view.eval(&format!(
                "window.__atsumiSoopOriginal?.relayStop({},{})",
                json!(command.0.to_string()),
                command.1
            ))
            .is_ok()
        }) {
            lock(&command.2).stop(true);
        }
    }
    pub(super) fn request_shutdown(&self, view: Option<Webview>) {
        let graceful = lock(&self.owner)
            .as_ref()
            .is_some_and(|o| o.graceful_shutdown);
        self.request_stop(view, !graceful);
    }
    /// Nonblocking reap. Registration is released only AFTER the manager joined;
    /// the manager in turn cannot return while a common worker remains unjoined.
    pub(super) fn reap(&self) {
        let mut slot = lock(&self.owner);
        if let Some(owner) = slot.as_ref() {
            let mut state = lock(&owner.shared);
            if !state.stopped && state.stop_deadline.is_some_and(|d| Instant::now() >= d) {
                state.stop(true);
            }
        }
        if !slot
            .as_ref()
            .is_some_and(|o| o.handle.is_finished() && o.cleanup_proof.load(Ordering::Acquire))
        {
            return;
        }
        let owner = slot.take().unwrap();
        let mut result = owner.handle.join().unwrap_or_else(|_| {
            let mut result = lock(&owner.progress).clone();
            result.fail("ManagerPanic");
            result
        });
        result.manager_joined = true;
        // Keep the exact run's final observation alive for selected owners even
        // after registry disposal or another run replacing Slot.last.
        *lock(&owner.progress) = result.clone();
        if let Some(registration) = owner.registration {
            registration.release_after_join();
        }
        *lock(&self.last) = Some(result);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Begin {
    request_id: String,
    source_id: String,
    channel_id: String,
    actual_channel: String,
    actual_broadcast: String,
    tracks: Vec<Track>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Track {
    track_index: u32,
    mime_type: String,
    init: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Append {
    run_id: String,
    source_id: String,
    actual_channel: String,
    actual_broadcast: String,
    track_index: u32,
    append_index: u64,
    chunk_index: u32,
    final_chunk: bool,
    data: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Finish {
    run_id: String,
    source_id: String,
    actual_channel: String,
    actual_broadcast: String,
    interrupted: bool,
    reason: String,
}
fn uuid(value: &str) -> Result<Uuid, String> {
    let value = Uuid::parse_str(value).map_err(|_| STALE)?;
    if value.is_nil() {
        Err(STALE.into())
    } else {
        Ok(value)
    }
}
fn identity(context: &Context, run: &str, channel: &str, broadcast: &str) -> Result<(), String> {
    if uuid(run)? != context.run || channel != context.channel || broadcast != context.broadcast {
        return Err(STALE.into());
    }
    Ok(())
}
fn decode_tracks(tracks: Vec<Track>) -> Result<Vec<EncodedTrackInput>, String> {
    if tracks.is_empty() || tracks.len() > 2 {
        return Err(STALE.into());
    }
    let mut decoded = Vec::with_capacity(tracks.len());
    for track in tracks {
        if track.track_index > 1
            || track.mime_type.is_empty()
            || track.mime_type.len() > 120
            || track.init.is_empty()
            || track.init.len() > 88_000
            || decoded
                .iter()
                .any(|t: &EncodedTrackInput| t.track_index == track.track_index)
        {
            return Err(STALE.into());
        }
        let init = STANDARD.decode(track.init).map_err(|_| STALE)?;
        if init.is_empty() || init.len() > 64 * 1024 {
            return Err(STALE.into());
        }
        decoded.push(EncodedTrackInput {
            track_index: track.track_index,
            mime_type: track.mime_type,
            init,
        });
    }
    Ok(decoded)
}

pub(super) fn is_message(kind: &str) -> bool {
    kind.starts_with("relay_")
}
pub(super) fn reply(view: &Webview, id: &str, epoch: u64, result: Result<Value, String>) {
    let body = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error,"code":"relay_unavailable"}),
    };
    let _ = view.eval(&format!(
        "window.__atsumiSoopOriginal?.relayReply({},{},{})",
        json!(id),
        body,
        epoch
    ));
}
/// Called only after the existing exact native URL, WebView label and epoch gate.
pub(super) fn dispatch(session: &Session, view: &Webview, message: Envelope) {
    let clock = matches!(message.kind.as_str(), "relay_clock" | "relay_chat_clock");
    let id = message.id.clone();
    let epoch = message.epoch;
    let result = dispatch_fields(
        &session.relay,
        &session.channel,
        &session.broadcast,
        session.epoch.load(std::sync::atomic::Ordering::Acquire),
        session.closed.load(std::sync::atomic::Ordering::Acquire),
        message,
    );
    // A begin reply is asynchronous: codec admission happens on the common worker.
    if !clock { if let Some(result) = result {
        reply(view, &id, epoch, result);
    }}
}
fn dispatch_fields(
    slot: &Slot,
    channel: &str,
    broadcast: &str,
    epoch: u64,
    closed: bool,
    message: Envelope,
) -> Option<Result<Value, String>> {
    let result = (|| {
        if closed || message.epoch != epoch {
            return Err(STALE.into());
        }
        let owner_guard = lock(&slot.owner);
        let owner = owner_guard.as_ref().ok_or(UNAVAILABLE)?;
        let context = &owner.context;
        if context.epoch != epoch || context.channel != channel || context.broadcast != broadcast {
            return Err(STALE.into());
        }
        match message.kind.as_str() {
            "relay_chat_clock" => {
                #[derive(Deserialize)]
                #[serde(rename_all="camelCase",deny_unknown_fields)]
                struct Reply {run_id:String,source_id:String,actual_channel:String,actual_broadcast:String,
                    event:crate::platform_media::relay_chat::timing::EventClock}
                let reply:Reply=serde_json::from_value(message.fields).map_err(|_|STALE)?;
                identity(context,&reply.run_id,&reply.actual_channel,&reply.actual_broadcast)?;
                let state=lock(&owner.shared);
                if state.stopped||!state.admitted||state.source!=Some(uuid(&reply.source_id)?){return Err(STALE.into());}
                let timing=state.timing.as_ref().ok_or(STALE)?;
                if !timing.event_clock(reply.event,Instant::now()){return Err(STALE.into());}
                Ok(None)
            }
            "relay_clock" => {
                #[derive(Deserialize)]
                #[serde(rename_all="camelCase",deny_unknown_fields)]
                struct Reply {nonce:String,run_id:String,source_id:String,actual_channel:String,actual_broadcast:String,sample:Option<ClockSample>}
                let reply:Reply=serde_json::from_value(message.fields).map_err(|_|STALE)?;
                identity(context,&reply.run_id,&reply.actual_channel,&reply.actual_broadcast)?;
                let state=lock(&owner.shared);
                if state.stopped||!state.admitted||state.source!=Some(uuid(&reply.source_id)?){return Err(STALE.into());}
                let timing=state.timing.as_ref().ok_or(STALE)?;
                if !timing.observe(uuid(&reply.nonce)?,reply.sample,Instant::now()){return Err(STALE.into());}
                Ok(None)
            }
            "relay_begin" => {
                let begin: Begin = serde_json::from_value(message.fields).map_err(|_| STALE)?;
                identity(
                    context,
                    &begin.request_id,
                    &begin.actual_channel,
                    &begin.actual_broadcast,
                )?;
                if begin.channel_id != context.channel {
                    return Err(STALE.into());
                }
                let source = uuid(&begin.source_id)?;
                let tracks = decode_tracks(begin.tracks)?;
                let mut state = lock(&owner.shared);
                if state.stopped || state.begin_sent {
                    return Err(STALE.into());
                }
                state.source = Some(source);
                state.begin_sent = true;
                if owner
                    .begin
                    .try_send(BeginJob {
                        source,
                        tracks,
                        reply_id: message.id,
                    })
                    .is_err()
                {
                    state.stop(true);
                    return Err(UNAVAILABLE.into());
                }
                Ok(None)
            }
            "relay_append" => {
                let append: Append = serde_json::from_value(message.fields).map_err(|_| STALE)?;
                identity(
                    context,
                    &append.run_id,
                    &append.actual_channel,
                    &append.actual_broadcast,
                )?;
                let source = uuid(&append.source_id)?;
                let mut state = lock(&owner.shared);
                if state.source != Some(source) || !state.admitted || state.stopped {
                    return Err(STALE.into());
                }
                if append.track_index > 1
                    || append.append_index > JS_INTEGER
                    || append.data.len() > 174_764
                {
                    state.stop(true);
                    return Err(STALE.into());
                }
                let bytes = match STANDARD.decode(append.data) {
                    Ok(bytes) if !bytes.is_empty() && bytes.len() <= CHUNK_BYTES => bytes,
                    _ => {
                        state.stop(true);
                        return Err(STALE.into());
                    }
                };
                let (input, binding) = state.input.as_ref().ok_or(UNAVAILABLE)?;
                if input
                    .try_enqueue(
                        binding,
                        append.track_index,
                        append.append_index,
                        append.chunk_index,
                        append.final_chunk,
                        &bytes,
                    )
                    .is_err()
                {
                    state.stop(true);
                    return Err(UNAVAILABLE.into());
                }
                Ok(Some(json!({"acceptedInMemory":true})))
            }
            "relay_finish" => {
                let finish: Finish = serde_json::from_value(message.fields).map_err(|_| STALE)?;
                identity(
                    context,
                    &finish.run_id,
                    &finish.actual_channel,
                    &finish.actual_broadcast,
                )?;
                let source = uuid(&finish.source_id)?;
                if finish.reason.len() > 64
                    || !finish
                        .reason
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err(STALE.into());
                }
                let mut state = lock(&owner.shared);
                // The native request UUID is known even when the begin ACK was lost.
                // If begin itself never arrived, only interruption is admissible.
                if state.source.is_some_and(|pinned| pinned != source)
                    || (!state.begin_sent && !finish.interrupted)
                {
                    return Err(STALE.into());
                }
                state.stop(finish.interrupted);
                Ok(Some(
                    json!({"acceptedInMemory":true,"stopRequested":true,"interrupted":state.interrupted}),
                ))
            }
            _ => Err(STALE.into()),
        }
    })();
    match result {
        Ok(None) => None,
        Ok(Some(value)) => Some(Ok(value)),
        Err(error) => Some(Err(error)),
    }
}

// A concrete inspector, sealed behind native test construction. There is no
// production start method and no caller-supplied blocking callback or sink.
struct JoinedWorker {
    run: RelayRun,
    proof: Arc<AtomicBool>,
}
impl Drop for JoinedWorker {
    fn drop(&mut self) {
        // Also runs on manager unwind. Never detach a child and then release
        // its managed-work registration. Pending keeps this manager alive.
        self.run.cancel();
        while matches!(
            self.run.join(Duration::from_millis(10)),
            JoinResult::Pending
        ) {}
        self.proof.store(true, Ordering::Release);
    }
}
#[cfg(all(test, windows))]
fn inspect_manager(
    context: Arc<Context>,
    shared: Arc<Mutex<Admission>>,
    begin: Receiver<BeginJob>,
    replies: Replies,
    cleanup_proof: Arc<AtomicBool>,
    progress: Arc<Mutex<Inspection>>,
) -> Inspection {
    let mut result = Inspection {
        run_id: context.run.to_string(),
        ..Inspection::default()
    };
    let deadline = Instant::now() + ARM_TIMEOUT;
    let job = loop {
        {
            let mut state = lock(&shared);
            if state.stop_deadline.is_some_and(|d| Instant::now() >= d) {
                state.stop(true);
            }
        }
        if lock(&shared).stopped || Instant::now() >= deadline {
            lock(&shared).stop(true);
            result.interrupted = true;
            result.fault = Some("ArmStopped".into());
            replies.stop(context.run);
            return result;
        }
        match begin.recv_timeout(Duration::from_millis(10)) {
            Ok(job) => break job,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                result.interrupted = true;
                return result;
            }
        }
    };
    let binding = RelayBinding::new(NativeIdentity {
        provider: RelayProvider::Soop,
        owner: context.owner,
        run: context.run,
        source: job.source,
        document_epoch: context.epoch,
    });
    cleanup_proof.store(false, Ordering::Release);
    let started = binding.and_then(|binding| {
        RelayRun::start(binding.clone(), job.tracks).map(|parts| (binding, parts))
    });
    let (binding, (run, input, output)) = match started {
        Ok(value) => value,
        Err(fault) => {
            cleanup_proof.store(true, Ordering::Release);
            result.interrupted = true;
            result.fault = Some(format!("{fault:?}"));
            lock(&shared).stop(true);
            replies.send(&job.reply_id, Err(UNAVAILABLE.into()));
            replies.stop(context.run);
            return result;
        }
    };
    let mut worker = JoinedWorker {
        run,
        proof: cleanup_proof,
    };
    {
        let mut state = lock(&shared);
        state.input = Some((input.clone(), binding.clone()));
        if state.stopped {
            input.cancel();
        }
    }
    let mut hash = Sha256::new();
    let mut begin_replied = false;
    let mut published_chunks = 0;
    let mut published_admission = false;
    loop {
        let send_begin = {
            let mut state = lock(&shared);
            if state.stop_deadline.is_some_and(|d| Instant::now() >= d) && !state.stopped {
                state.stop(true);
            }
            if state.interrupted {
                worker.run.cancel();
            }
            if !begin_replied && input.codec_admitted() && !state.stopped {
                state.admitted = true;
                result.codec_admitted = true;
                begin_replied = true;
                true
            } else {
                false
            }
        };
        if send_begin {
            // Never schedule WebView work while holding the admission mutex.
            replies.send(&job.reply_id, Ok(json!({"runId":context.run.to_string(),"mode":"relay","nativeApproved":true,"acceptedInMemory":true,"captureChat":false})));
        }
        // Each guard is consumed and dropped before the next receive. Bytes
        // never escape their common-worker credit; only fixed-size facts remain.
        for _ in 0..8 {
            match output.try_recv() {
                Ok(guard) => {
                    if guard.binding().identity() != binding.identity() {
                        worker.run.cancel();
                    }
                    let Some(chunk) = guard.chunk() else {
                        worker.run.cancel();
                        result.fault = Some("unexpected_ts_output".into());
                        continue;
                    };
                    result.output_chunks += 1;
                    result.output_bytes += chunk.bytes.len() as u64;
                    hash.update(&chunk.bytes);
                    if let Some(v) = chunk.video {
                        result.video_samples += v.samples;
                        result.video_end = Some(v.last_end);
                        result.video_timescale = Some(v.timescale);
                    }
                    if let Some(a) = chunk.audio {
                        result.audio_samples += a.samples;
                        result.audio_end = Some(a.last_end);
                        result.audio_timescale = Some(a.timescale);
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if published_chunks != result.output_chunks || published_admission != result.codec_admitted
        {
            // Test-only fixed-size facts. No media payload and no live digest.
            // manager_joined stays false; only reap() can publish final proof.
            *lock(&progress) = result.clone();
            published_chunks = result.output_chunks;
            published_admission = result.codec_admitted;
        }
        if let JoinResult::Joined(report) = worker.run.join(Duration::ZERO) {
            // Worker termination can enqueue its last output between the drain
            // and join. Drain that finite mailbox before dropping the receiver.
            while let Ok(guard) = output.try_recv() {
                let Some(chunk) = guard.chunk() else {
                    worker.run.cancel();
                    result.fault = Some("unexpected_ts_output".into());
                    continue;
                };
                result.output_chunks += 1;
                result.output_bytes += chunk.bytes.len() as u64;
                hash.update(&chunk.bytes);
                if let Some(v) = chunk.video {
                    result.video_samples += v.samples;
                    result.video_end = Some(v.last_end);
                    result.video_timescale = Some(v.timescale);
                }
                if let Some(a) = chunk.audio {
                    result.audio_samples += a.samples;
                    result.audio_end = Some(a.last_end);
                    result.audio_timescale = Some(a.timescale);
                }
            }
            result.worker(report);
            {
                let mut state = lock(&shared);
                state.stopped = true;
                state.input.take();
            }
            if !begin_replied {
                replies.send(&job.reply_id, Err(UNAVAILABLE.into()));
            }
            result.output_sha256 = format!("{:x}", hash.finalize());
            replies.stop(context.run);
            return result;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(all(test, windows))]
fn install_inspector(
    slot: &Slot,
    context: Context,
    replies: Replies,
    registration: Option<RelayWorkRegistration>,
) -> Result<(), String> {
    install_manager(slot, context, replies, registration, ManagerMode::Inspector).map(|_| ())
}
fn install_manager(
    slot: &Slot,
    context: Context,
    replies: Replies,
    registration: Option<RelayWorkRegistration>,
    mode: ManagerMode,
) -> Result<Arc<Mutex<Inspection>>, String> {
    let mut owner_slot = lock(&slot.owner);
    if owner_slot.is_some() {
        return Err("SOOP 전송 작업을 마무리하는 중입니다.".into());
    }
    let context = Arc::new(context);
    let shared = Arc::new(Mutex::new(Admission::default()));
    let cleanup_proof = Arc::new(AtomicBool::new(true));
    let worker_proof = cleanup_proof.clone();
    let progress = Arc::new(Mutex::new(Inspection {
        run_id: context.run.to_string(),
        ..Inspection::default()
    }));
    let worker_progress = progress.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_context = context.clone();
    let worker_shared = shared.clone();
    let graceful_shutdown = matches!(&mode, ManagerMode::Hls { .. });
    let handle = thread::Builder::new()
        .name("soop-relay-manager".into())
        .spawn(move || match mode {
            #[cfg(all(test, windows))]
            ManagerMode::Inspector => inspect_manager(
                worker_context,
                worker_shared,
                rx,
                replies,
                worker_proof,
                worker_progress,
            ),
            ManagerMode::Hls { target, receiver } => delivery::run_manager(
                worker_context,
                worker_shared,
                rx,
                replies,
                worker_proof,
                worker_progress,
                target,
                receiver,
            ),
        })
        .map_err(|_| UNAVAILABLE)?;
    let mut registration = registration;
    if let Some(registration) = registration.as_mut() {
        registration.mark_started();
    }
    *lock(&slot.last) = None;
    *owner_slot = Some(Owner {
        context,
        shared,
        begin: tx,
        handle,
        registration,
        cleanup_proof,
        graceful_shutdown,
        progress: progress.clone(),
    });
    Ok(progress)
}

impl SoopOriginalHost {
    pub(crate) fn relay_available_channels(&self) -> Vec<String> {
        let channels = lock(&self.registry).sessions.values().map(|s| s.channel.clone()).collect::<std::collections::BTreeSet<_>>();
        channels.into_iter().filter(|channel| self.select_relay_source_by_channel(channel).is_ok()).collect()
    }
    /// Select only an already open, uniquely bound native source.
    pub(crate) fn select_relay_source_by_channel(&self, channel: &str) -> Result<SoopRelaySource, String> {
        let sessions = lock(&self.registry).sessions.values()
            .filter(|session| session.channel == channel).cloned().collect::<Vec<_>>();
        let [session] = sessions.as_slice() else { return Err(STALE.into()); };
        let _flow = lock(&self.account_flow);
        let _approval = lock(&session.arm);
        let source = SoopRelaySource::from_session(session);
        if !self.relay_source_ready(&source, session) { return Err(STALE.into()); }
        Ok(source)
    }
    pub(crate) fn select_relay_source(&self, token: &str) -> Result<SoopRelaySource, String> {
        let session = self.session(token)?;
        let _flow = lock(&self.account_flow);
        let _approval = lock(&session.arm);
        let source = SoopRelaySource::from_session(&session);
        if !self.relay_source_ready(&source, &session) { return Err(STALE.into()); }
        Ok(source)
    }
    fn relay_source_ready(&self, source: &SoopRelaySource, session: &Session) -> bool {
        selection_matches(source, &SoopRelaySource::from_session(session),
            !self.closing.load(Ordering::Acquire)
                && !session.closed.load(Ordering::Acquire)
                && !session.account_refreshing.load(Ordering::Acquire)
                && lock(&session.status).capture_ready,
            !lock(&session.viewers).is_empty())
    }
    pub(crate) fn relay_source_current(&self, source: &SoopRelaySource) -> bool {
        let session = lock(&self.registry).sessions.get(&source.session_id).cloned();
        let Some(session) = session else { return false; };
        let _flow = lock(&self.account_flow);
        let _approval = lock(&session.arm);
        self.relay_source_ready(source, &session)
    }
    pub(crate) fn start_selected_hls_relay(
        &self, app: &AppHandle, source: &SoopRelaySource, run: Uuid, target: HlsTarget,
    ) -> Result<SelectedRelayHandle, String> {
        if run.is_nil() { return Err(STALE.into()); }
        let session = lock(&self.registry).sessions.get(&source.session_id).cloned().ok_or(STALE)?;
        let (_, progress) = self.start_relay_owned(app, session.clone(), Some(target), Some(source), run)?;
        Ok(SelectedRelayHandle { app: app.clone(), session: Arc::downgrade(&session), source: source.clone(), run, progress })
    }
    /// Called by normal-exit pending work checks as well as idle housekeeping.
    pub(crate) fn poll_relay_join(&self, app: &AppHandle) {
        let sessions = lock(&self.registry)
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            session.relay.reap();
            if !session.relay.active()
                && self.closing.load(std::sync::atomic::Ordering::Acquire)
                && lock(&session.archive).is_none()
                && lock(&session.arm).is_none()
            {
                self.dispose(app, &session);
            }
        }
    }
    #[cfg(all(test, windows))]
    pub(crate) fn start_relay_inspector(
        &self,
        app: &AppHandle,
        token: &str,
    ) -> Result<String, String> {
        self.start_relay(app, token, None)
    }
    pub(crate) fn start_hls_relay(
        &self,
        app: &AppHandle,
        token: &str,
        target: HlsTarget,
    ) -> Result<String, String> {
        self.start_relay(app, token, Some(target))
    }
    fn start_relay(
        &self,
        app: &AppHandle,
        token: &str,
        target: Option<HlsTarget>,
    ) -> Result<String, String> {
        let session = self.session(token)?;
        let run = Uuid::new_v4();
        self.start_relay_owned(app, session, target, None, run).map(|(run, _)| run)
    }
    fn start_relay_owned(
        &self, app: &AppHandle, session: Arc<Session>, target: Option<HlsTarget>,
        expected: Option<&SoopRelaySource>, run: Uuid,
    ) -> Result<(String, Arc<Mutex<Inspection>>), String> {
        let run_id = run.to_string();
        // Enter managed-work authority without holding ANY host/session lock.
        let state = app.state::<crate::interface::commands::AppState>();
        state.begin_relay_work(&session.id, &run_id, |registration| {
            let _flow = lock(&self.account_flow);
            let _archive_approval = lock(&session.arm);
            if self.closing.load(std::sync::atomic::Ordering::Acquire)
                || session.closed.load(std::sync::atomic::Ordering::Acquire)
                || session
                    .account_refreshing
                    .load(std::sync::atomic::Ordering::Acquire)
                || !lock(&session.status).capture_ready
                || lock(&session.viewers).is_empty()
            {
                return Err(STALE.into());
            }
            let epoch = session.epoch.load(std::sync::atomic::Ordering::Acquire);
            if epoch == 0 || expected.is_some_and(|source| !self.relay_source_ready(source, &session)) {
                return Err(STALE.into());
            }
            let view = app.get_webview(&session.label).ok_or(UNAVAILABLE)?;
            let mode = match target {
                Some(target) => ManagerMode::Hls {
                    target,
                    receiver: Some(session.lease.clone()),
                },
                #[cfg(all(test, windows))]
                None => ManagerMode::Inspector,
                #[cfg(not(all(test, windows)))]
                None => return Err(UNAVAILABLE.into()),
            };
            let progress = install_manager(
                &session.relay,
                Context {
                    owner: uuid(&session.id)?,
                    run,
                    epoch,
                    channel: session.channel.clone(),
                    broadcast: session.broadcast.clone(),
                },
                Replies {
                    view: Some(view.clone()),
                    epoch,
                },
                Some(registration),
                mode,
            )?;
            drop(_archive_approval);
            drop(_flow);
            if view
                .eval(&format!(
                    "window.__atsumiSoopOriginal?.relayStart({},{})",
                    json!(run_id),
                    epoch
                ))
                .is_err()
            {
                session.relay.cancel();
                // A selected destination has already been consumed and owns a
                // manager. Return its exact handle so failure remains joinable.
                if expected.is_none() { return Err(UNAVAILABLE.into()); }
            }
            Ok((run_id.clone(), progress))
        })
    }
    #[cfg(all(test, windows))]
    pub(crate) fn stop_relay_inspector(
        &self,
        app: &AppHandle,
        token: &str,
        interrupted: bool,
    ) -> Result<(), String> {
        self.stop_hls_relay(app, token, interrupted)
    }
    pub(crate) fn stop_hls_relay(
        &self,
        app: &AppHandle,
        token: &str,
        interrupted: bool,
    ) -> Result<(), String> {
        let session = self.session(token)?;
        session
            .relay
            .request_stop(app.get_webview(&session.label), interrupted);
        Ok(())
    }
    #[cfg(all(test, windows))]
    pub(crate) fn relay_inspection(&self, token: &str) -> Result<Option<Inspection>, String> {
        self.relay_delivery_status(token)
    }
    pub(crate) fn relay_delivery_status(&self, token: &str) -> Result<Option<Inspection>, String> {
        let session = self.session(token)?;
        session.relay.reap();
        if let Some(owner) = lock(&session.relay.owner).as_ref() {
            return Ok(Some(lock(&owner.progress).clone()));
        }
        let result = lock(&session.relay.last).clone();
        Ok(result)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::streaming::browser::encoded::fmp4::fixtures;

    #[test]
    fn selected_source_rejects_changed_document_account_or_missing_viewer() {
        let source = SoopRelaySource::test_fixture();
        assert!(selection_matches(&source, &source, true, true));
        assert!(!selection_matches(&source, &source, true, false));
        assert!(!selection_matches(&source, &source, false, true));
        for field in 0..5 {
            let mut current = source.clone();
            match field {
                0 => current.session_id = Uuid::new_v4().to_string(),
                1 => current.epoch += 1,
                2 => current.account_revision += 1,
                3 => current.channel = "different".into(),
                _ => current.broadcast = "2".into(),
            }
            assert!(!selection_matches(&source, &current, true, true));
        }
    }

    #[test]
    fn selected_stale_stop_cannot_cancel_new_run_and_join_updates_exact_progress() {
        let h = Harness::new();
        let progress = lock(&h.slot.owner).as_ref().unwrap().progress.clone();
        h.slot.request_stop_bound(Some(Uuid::new_v4()), None, true);
        assert!(!lock(&lock(&h.slot.owner).as_ref().unwrap().shared).stopped);
        h.slot.request_stop_bound(Some(h.context.run), None, true);
        let result = h.result();
        assert!(result.manager_joined && result.interrupted);
        let saved = lock(&progress).clone();
        assert!(saved.manager_joined && saved.run_id == h.context.run.to_string());
        *lock(&h.slot.last) = None;
        assert!(lock(&progress).manager_joined, "exact owned receipt outlives slot replacement");
    }

    struct Harness {
        slot: Slot,
        context: Context,
        source: Uuid,
    }
    impl Harness {
        fn new() -> Self {
            let h = Self {
                slot: Slot::default(),
                context: Context {
                    owner: Uuid::new_v4(),
                    run: Uuid::new_v4(),
                    epoch: 7,
                    channel: "testchannel".into(),
                    broadcast: "123456789".into(),
                },
                source: Uuid::new_v4(),
            };
            install_inspector(&h.slot, h.context.clone(), Replies::default(), None).unwrap();
            h
        }
        fn call(&self, kind: &str, fields: Value) -> Option<Result<Value, String>> {
            dispatch_fields(
                &self.slot,
                &self.context.channel,
                &self.context.broadcast,
                self.context.epoch,
                false,
                Envelope {
                    intent: None,
                    id: Uuid::new_v4().to_string(),
                    epoch: self.context.epoch,
                    kind: kind.into(),
                    fields,
                },
            )
        }
        fn begin_fields(&self) -> Value {
            let tracks:Vec<_>=fixtures::inputs().into_iter().map(|t|json!({"trackIndex":t.track_index,"mimeType":t.mime_type,"init":STANDARD.encode(t.init)})).collect();
            json!({"requestId":self.context.run.to_string(),"sourceId":self.source.to_string(),"channelId":self.context.channel,"actualChannel":self.context.channel,"actualBroadcast":self.context.broadcast,"tracks":tracks})
        }
        fn begin(&self) {
            assert!(self.call("relay_begin", self.begin_fields()).is_none());
        }
        fn wait_ready(&self) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if lock(&self.slot.owner)
                    .as_ref()
                    .is_some_and(|o| lock(&o.shared).admitted)
                {
                    return;
                }
                thread::sleep(Duration::from_millis(1));
            }
            panic!("codec admission deadline");
        }
        fn append_fields(
            &self,
            track: u32,
            index: u64,
            chunk: u32,
            last: bool,
            bytes: &[u8],
        ) -> Value {
            json!({"runId":self.context.run.to_string(),"sourceId":self.source.to_string(),"actualChannel":self.context.channel,"actualBroadcast":self.context.broadcast,"trackIndex":track,"appendIndex":index,"chunkIndex":chunk,"finalChunk":last,"data":STANDARD.encode(bytes)})
        }
        fn append(&self, track: u32, index: u64, chunk: u32, last: bool, bytes: &[u8]) {
            assert!(self
                .call(
                    "relay_append",
                    self.append_fields(track, index, chunk, last, bytes)
                )
                .unwrap()
                .unwrap()["acceptedInMemory"]
                .as_bool()
                .unwrap());
        }
        fn finish(&self, interrupted: bool) -> Value {
            self.call("relay_finish",json!({"runId":self.context.run.to_string(),"sourceId":self.source.to_string(),"actualChannel":self.context.channel,"actualBroadcast":self.context.broadcast,"interrupted":interrupted,"reason":"test_stop"})).unwrap().unwrap()
        }
        fn result(&self) -> Inspection {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                self.slot.reap();
                if let Some(value) = lock(&self.slot.last).clone() {
                    return value;
                }
                thread::sleep(Duration::from_millis(1));
            }
            panic!("manager join deadline");
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.slot.cancel();
            let until = Instant::now() + Duration::from_secs(3);
            while self.slot.active() && Instant::now() < until {
                self.slot.reap();
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    #[test]
    fn page_cannot_start_an_unarmed_receiver() {
        let h = Harness::new();
        let slot = Slot::default();
        let result = dispatch_fields(
            &slot,
            &h.context.channel,
            &h.context.broadcast,
            7,
            false,
            Envelope {
                intent: None,
                id: Uuid::new_v4().to_string(),
                epoch: 7,
                kind: "relay_begin".into(),
                fields: h.begin_fields(),
            },
        );
        assert!(result.unwrap().is_err());
        assert!(!slot.active());
    }
    #[test]
    fn exact_run_source_and_document_identity_are_required() {
        let h = Harness::new();
        let mut fields = h.begin_fields();
        fields["requestId"] = json!(Uuid::new_v4().to_string());
        assert!(h.call("relay_begin", fields).unwrap().is_err());
        let mut fields = h.begin_fields();
        fields["actualBroadcast"] = json!("987654321");
        assert!(h.call("relay_begin", fields).unwrap().is_err());
        let mut fields = h.begin_fields();
        fields["extra"] = json!(true);
        assert!(h.call("relay_begin", fields).unwrap().is_err());
        let result = dispatch_fields(
            &h.slot,
            &h.context.channel,
            &h.context.broadcast,
            8,
            false,
            Envelope {
                intent: None,
                id: Uuid::new_v4().to_string(),
                epoch: 7,
                kind: "relay_begin".into(),
                fields: h.begin_fields(),
            },
        );
        assert!(result.unwrap().is_err());
        h.begin();
        h.wait_ready();
        let mut append = h.append_fields(0, 0, 0, true, &fixtures::fragment(0, 0, 4, true));
        append["sourceId"] = json!(Uuid::new_v4().to_string());
        assert!(h.call("relay_append", append).unwrap().is_err());
        h.append(0, 0, 0, true, &fixtures::fragment(0, 0, 4, true));
        h.append(1, 0, 0, true, &fixtures::fragment(1, 0, 4, true));
        h.finish(false);
        let result = h.result();
        assert!(result.input_ended);
        assert_eq!(result.video_samples, 4);
        assert_eq!(result.audio_samples, 4);
    }
    #[test]
    fn guarded_output_is_consumed_and_finish_means_stop_requested_until_join() {
        let h = Harness::new();
        h.begin();
        h.wait_ready();
        // Legal empty sample batch remains in reader/mux order.
        h.append(0, 0, 0, true, b"\0\0\0\x08free");
        let video = fixtures::fragment(0, 0, 4, true);
        let split = video.len() / 2;
        h.append(0, 1, 0, false, &video[..split]);
        h.append(0, 1, 0, false, &video[..split]);
        h.append(0, 1, 1, true, &video[split..]);
        h.append(1, 0, 0, true, &fixtures::fragment(1, 0, 4, true));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let progress = {
                let owner = lock(&h.slot.owner);
                let snapshot = lock(&owner.as_ref().unwrap().progress).clone();
                snapshot
            };
            if progress.output_bytes > 0 {
                assert!(progress.codec_admitted);
                assert!(!progress.manager_joined);
                assert!(!progress.worker_joined);
                assert!(progress.output_sha256.is_empty());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "live inspector progress deadline"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let ack = h.finish(false);
        assert_eq!(ack["stopRequested"], true);
        assert!(ack.get("stopped").is_none());
        assert!(ack.get("joined").is_none());
        let result = h.result();
        assert!(
            result.codec_admitted
                && result.worker_joined
                && result.manager_joined
                && result.input_ended
        );
        assert_eq!(result.video_samples, 4);
        assert_eq!(result.audio_samples, 4);
        assert_eq!(result.validated_appends, 3);
        assert_eq!(result.duplicate_chunks, 1);
        assert!(result.output_bytes > 0);
        assert_eq!(result.output_sha256.len(), 64);
        assert!(!h.slot.active());
    }
    #[test]
    fn interrupted_finish_before_begin_ack_cannot_revive_owner() {
        let h = Harness::new();
        assert_eq!(h.finish(true)["interrupted"], true);
        assert!(h.call("relay_begin", h.begin_fields()).unwrap().is_err());
        let result = h.result();
        assert!(result.manager_joined && result.interrupted);
        assert!(!result.codec_admitted);
        assert!(!result.worker_joined);
    }
    #[test]
    fn invalid_codec_never_becomes_native_approved() {
        let h = Harness::new();
        let mut fields = h.begin_fields();
        fields["tracks"][0]["init"] = json!(STANDARD.encode(b"\0\0\0\x08free"));
        assert!(h.call("relay_begin", fields).is_none());
        let result = h.result();
        assert!(result.worker_joined && result.manager_joined && result.interrupted);
        assert!(!result.codec_admitted);
        assert_eq!(result.output_chunks, 0);
    }
    #[test]
    fn partial_append_and_unsafe_js_ordinal_interrupt_only_relay() {
        let h = Harness::new();
        h.begin();
        h.wait_ready();
        h.append(0, 0, 0, false, b"abc");
        h.finish(false);
        let result = h.result();
        assert!(result.interrupted);
        assert_eq!(result.fault.as_deref(), Some("PartialAppend"));
        assert_eq!(result.output_chunks, 0);
        let h = Harness::new();
        h.begin();
        h.wait_ready();
        let fields = h.append_fields(0, JS_INTEGER + 1, 0, true, b"\0\0\0\x08free");
        assert!(h.call("relay_append", fields).unwrap().is_err());
        assert!(h.result().interrupted);
    }
    #[test]
    fn cancellation_keeps_owner_until_actual_reap_and_rejects_replacement() {
        let h = Harness::new();
        h.begin();
        h.wait_ready();
        h.slot.cancel();
        assert!(h.slot.active());
        assert!(install_inspector(&h.slot, h.context.clone(), Replies::default(), None).is_err());
        let result = h.result();
        assert!(result.worker_joined && result.manager_joined && result.interrupted);
        assert!(!h.slot.active());
        assert!(h.call("relay_begin", h.begin_fields()).unwrap().is_err());
    }
    #[test]
    fn manager_unwind_cancels_and_actually_joins_child_before_cleanup_proof() {
        let proof = Arc::new(AtomicBool::new(false));
        let proof_for_thread = proof.clone();
        let handle = thread::spawn(move || {
            let binding = RelayBinding::new(NativeIdentity {
                provider: RelayProvider::Soop,
                owner: Uuid::new_v4(),
                run: Uuid::new_v4(),
                source: Uuid::new_v4(),
                document_epoch: 1,
            })
            .unwrap();
            let (run, _input, _output) = RelayRun::start(binding, fixtures::inputs()).unwrap();
            let _worker = JoinedWorker {
                run,
                proof: proof_for_thread,
            };
            panic!("test manager unwind");
        });
        assert!(handle.join().is_err());
        assert!(proof.load(Ordering::Acquire));
    }
    #[test]
    fn lost_finish_before_begin_obeys_stop_deadline_instead_of_arm_deadline() {
        let h = Harness::new();
        {
            let owner = lock(&h.slot.owner);
            lock(&owner.as_ref().unwrap().shared).stop_deadline = Some(Instant::now());
        }
        let result = h.result();
        assert!(result.manager_joined && result.interrupted);
        assert!(!result.worker_joined);
    }
}
