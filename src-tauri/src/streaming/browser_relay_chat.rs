//! CHZZK light sidecar, independent of recording and SOOP domain state.
use super::*;
use crate::streaming::{relay_status::SidecarInspection, replay_assets};
use encoded::fmp4::relay::ts::{TsEnd, TsSegment};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
};
const MAX_TEXT: u64 = 64 * 1024 * 1024;
struct Batch {
    id: u64,
    events: Vec<Value>,
    bytes: Vec<u8>,
    received: u64,
    receipt: Instant,
    reply: Option<(Webview, String)>,
}
struct Operation<'a>(&'a std::sync::atomic::AtomicUsize);
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct State {
    #[cfg(test)]
    pause_after_timing_drain: Mutex<Option<(SyncSender<()>, Receiver<()>)>>,
    operations: std::sync::atomic::AtomicUsize,
    ended: AtomicBool,
    interrupted: AtomicBool,
    partial: AtomicBool,
    timing_incomplete: AtomicBool,
    started: Instant,
}
#[derive(Clone)]
pub(super) struct Input {
    sender: SyncSender<Batch>,
    timing: SyncSender<Vec<u8>>,
    state: Arc<State>,
}
impl Input {
    pub(super) fn timing_missing(&self) {
        self.state.timing_incomplete.store(true, Ordering::Release);
    }
    pub(super) fn partial(&self) {
        self.state.partial.store(true, Ordering::Release);
    }
    pub(super) fn batch(
        &self,
        id: u64,
        events: Vec<Value>,
        window: Webview,
        reply: String,
    ) -> Result<(), StreamError> {
        self.state.operations.fetch_add(1, Ordering::SeqCst);
        let _operation = Operation(&self.state.operations);
        let bytes = serde_json::to_vec(&events).map_err(|_| fault())?;
        if self.state.ended.load(Ordering::Acquire)
            || id == 0
            || id > 9_007_199_254_740_991
            || events.len() > 32
            || bytes.len() > 128 * 1024
        {
            self.partial();
            return Err(fault());
        }
        self.sender
            .try_send(Batch {
                id,
                events,
                bytes,
                received: now_ms(),
                receipt: Instant::now(),
                reply: Some((window, reply)),
            })
            .map_err(|_| {
                self.partial();
                fault()
            })
    }
    fn ns(&self, t: Instant) -> u64 {
        t.saturating_duration_since(self.state.started)
            .as_nanos()
            .min(u64::MAX as u128) as u64
    }
    fn record(&self, value: Value) {
        self.state.operations.fetch_add(1, Ordering::SeqCst);
        let _operation = Operation(&self.state.operations);
        let Ok(mut bytes) = serde_json::to_vec(&value) else {
            self.state.timing_incomplete.store(true, Ordering::Release);
            return;
        };
        bytes.push(b'\n');
        if bytes.len() > 4096
            || self.state.ended.load(Ordering::Acquire)
            || self.timing.try_send(bytes).is_err()
        {
            self.state.timing_incomplete.store(true, Ordering::Release);
        }
    }
    pub(super) fn clock(
        &self,
        nonce: Uuid,
        before: Instant,
        after: Instant,
        sample: Option<&Clock>,
    ) {
        self.record(json!({"version":1,"kind":"source_observation","nonce":nonce,"nativeBeforeNs":self.ns(before),"nativeAfterNs":self.ns(after),"sample":sample,"quality":if sample.is_some(){"source_bound_player_observation"}else{"unknown"}}));
    }
}
pub(super) struct Owner {
    pub root: PathBuf,
    pub native_owner: crate::streaming::relay_status::NativeSidecarOwner,
    input: Input,
    handle: Option<thread::JoinHandle<SidecarInspection>>,
    result: Option<SidecarInspection>,
}
fn create_file(root: &Path, name: &str) -> Result<File, StreamError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .map_err(|_| fault())
}
fn owned_root(data: &Path, run: Uuid) -> Result<PathBuf, StreamError> {
    if !data.is_absolute() {
        return Err(fault());
    }
    for path in data.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() {
                return Err(fault());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(fault());
                }
            }
        }
    }
    let base = data.join("streaming").join("relay-sidecars");
    fs::create_dir_all(&base).map_err(|_| fault())?;
    for path in base.ancestors() {
        let m = fs::symlink_metadata(path).map_err(|_| fault())?;
        if m.file_type().is_symlink() {
            return Err(fault());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(fault());
            }
        }
    }
    let root = base.join(run.to_string());
    fs::create_dir(&root).map_err(|_| fault())?;
    fs::canonicalize(root).map_err(|_| fault())
}
impl Owner {
    pub(super) fn start(
        inner: &Inner,
        run: Uuid,
        source: &ChzzkRelaySource,
        media: Uuid,
    ) -> Result<Self, StreamError> {
        let root = owned_root(&inner.data_dir, run)?;
        let started = now_ms();
        let native_owner = crate::streaming::relay_status::NativeSidecarOwner::Chzzk {
            run_id: run.to_string(), session_id: source.session.clone(), source_id: media.to_string(),
            document_epoch: source.epoch, account_revision: source.revision,
            channel_id: source.channel.clone(), started_at_ms: started,
        };
        let native = Instant::now();
        let binding = json!({"version":1,"provider":"chzzk","runId":run,"sessionId":source.session,"documentEpoch":source.epoch,"accountRevision":source.revision,"channelId":source.channel,"sourceId":media,"startedAtMs":started,"clock":"native_receipt","remoteArchiveMapping":"unknown"});
        let mut owner = create_file(&root, "relay-owner.json")?;
        serde_json::to_writer(&mut owner, &binding).map_err(|_| fault())?;
        owner.sync_all().map_err(|_| fault())?;
        let log = ChatStore::create(&root)?;
        let raw = create_file(&root, "chat.events.jsonl")?;
        let timing = create_file(&root, "relay-timing.jsonl")?;
        let (tx, rx) = mpsc::sync_channel(8);
        let (tt, tr) = mpsc::sync_channel(8);
        let state = Arc::new(State {
            #[cfg(test)]
            pause_after_timing_drain: Mutex::new(None),
            operations: std::sync::atomic::AtomicUsize::new(0),
            ended: AtomicBool::new(false),
            interrupted: AtomicBool::new(false),
            partial: AtomicBool::new(false),
            timing_incomplete: AtomicBool::new(false),
            started: native,
        });
        let input = Input {
            sender: tx,
            timing: tt,
            state: state.clone(),
        };
        let root2 = root.clone();
        let assets = inner.replay_assets.clone();
        let source_id = media.to_string();
        let id = run.to_string();
        let handle = thread::Builder::new()
            .name("chzzk-relay-chat".into())
            .spawn(move || {
                write_loop(
                    root2, id, started, source_id, log, raw, timing, rx, tr, state, assets, binding,
                )
            })
            .map_err(|_| fault())?;
        Ok(Self {
            root,
            native_owner,
            input,
            handle: Some(handle),
            result: None,
        })
    }
    pub(super) fn input(&self) -> Input {
        self.input.clone()
    }
    pub(super) fn finish(&self, interrupted: bool) {
        if interrupted {
            self.input.state.interrupted.store(true, Ordering::Release);
        }
        self.input.state.ended.store(true, Ordering::Release);
    }
    pub(super) fn segment(&self, s: &TsSegment, b: Instant, a: Instant) {
        self.input.record(json!({"version":1,"kind":"segment_ack","streamId":s.stream_id(),"ordinal":s.ordinal(),"sha256":format!("{:x}",Sha256::digest(s.bytes())),"source":s.timing(),"nativeBeforeNs":self.input.ns(b),"nativeAfterNs":self.input.ns(a)}));
    }
    pub(super) fn end(&self, e: &TsEnd, b: Instant, a: Instant) {
        self.input.record(json!({"version":1,"kind":"end_ack","streamId":e.stream_id(),"segments":e.segments(),"origin":{"ticks":e.origin().ticks,"timescale":e.origin().timescale,"videoBase":e.origin().video_base,"audioBase":e.origin().audio_base,"videoRemainder":e.origin().video_remainder,"audioRemainder":e.origin().audio_remainder},"preroll":{"videoSamples":e.preroll().video_samples,"audioSamples":e.preroll().audio_samples,"bytes":e.preroll().bytes},"nativeBeforeNs":self.input.ns(b),"nativeAfterNs":self.input.ns(a)}));
    }
    pub(super) fn poll_join(&mut self) -> Option<SidecarInspection> {
        if self.result.is_some() || self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            Some(self.join())
        } else {
            None
        }
    }
    pub(super) fn join(&mut self) -> SidecarInspection {
        if self.result.is_none() {
            self.result =
                Some(
                    self.handle
                        .take()
                        .unwrap()
                        .join()
                        .unwrap_or_else(|_| SidecarInspection {
                            joined: true,
                            failure: true,
                            text_state: "incomplete".into(),
                            ..Default::default()
                        }),
                );
        }
        self.result.clone().unwrap()
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.finish(true);
            self.join();
        }
    }
}
// A deadline qualifies failure; it never manufactures a drained target. The
// native owner remains pending until all admitted asset tickets actually settle.
fn wait_fence<T>(
    deadline: Instant,
    mut acquire: impl FnMut() -> Option<T>,
    incomplete: &mut bool,
) -> T {
    loop {
        if let Some(fence) = acquire() {
            return fence;
        }
        if Instant::now() >= deadline {
            *incomplete = true;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn write_loop(
    root: PathBuf,
    id: String,
    started: u64,
    source: String,
    mut log: ChatStore,
    mut raw: File,
    mut timing: File,
    rx: Receiver<Batch>,
    tr: Receiver<Vec<u8>>,
    state: Arc<State>,
    assets: crate::streaming::replay_assets::ReplayAssetCache,
    binding: Value,
) -> SidecarInspection {
    let mut r = SidecarInspection {
        sidecar_id: id.clone(),
        clock_quality: "native_receipt_and_separate_source_observations".into(),
        ..Default::default()
    };
    let mut prior: Option<(u64, Vec<u8>)> = None;
    let mut expected = HashSet::new();
    let mut last_clock = None;
    let mut timing_bytes = 0u64;
    loop {
        while let Ok(bytes) = tr.try_recv() {
            if timing_bytes + bytes.len() as u64 > MAX_TEXT || timing.write_all(&bytes).is_err() {
                state.timing_incomplete.store(true, Ordering::Release);
            } else {
                timing_bytes += bytes.len() as u64;
                r.timing_records += 1;
            }
        }
        #[cfg(test)]
        if let Some((entered, resume)) = lock(&state.pause_after_timing_drain).take() {
            let _ = entered.send(());
            let _ = resume.recv_timeout(Duration::from_secs(1));
        }
        let batch = match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(b) => b,
            Err(mpsc::RecvTimeoutError::Timeout)
                if !state.ended.load(Ordering::Acquire)
                    || state.operations.load(Ordering::SeqCst) > 0 =>
            {
                continue
            }
            Err(_) => match rx.try_recv() {
                Ok(batch) => batch,
                Err(_) => break,
            },
        };
        let result = (|| -> Result<Value, StreamError> {
            if let Some((old, bytes)) = &prior {
                if batch.id == *old && batch.bytes == *bytes {
                    return Ok(json!({"stored":true}));
                }
                if batch.id != old.checked_add(1).ok_or_else(fault)? {
                    return Err(fault());
                }
            } else if batch.id != 1 {
                return Err(fault());
            }
            if r.failure || r.bytes + batch.bytes.len() as u64 > MAX_TEXT {
                return Err(fault());
            }
            for value in batch.events {
                if value.get("atsumiViewerSample").is_some() {
                    continue;
                }
                let Some(ChatEvent::Message {
                    sender,
                    text,
                    server_time,
                    rich,
                }) = crate::streaming::chat::parse_message_with_chat_channel(&value, None)
                else {
                    continue;
                };
                let mut clock = value
                    .get("replayClock")
                    .and_then(|v| serde_json::from_value::<ChatReplayClock>(v.clone()).ok())
                    .and_then(ChatReplayClock::bounded);
                if clock.as_ref().is_some_and(|c| {
                    c.received_at_ms < started.saturating_sub(60000)
                        || c.received_at_ms > batch.received.saturating_add(60000)
                        || last_clock.is_some_and(|(m, g)| {
                            c.observed_monotonic_ms < m || c.source_generation < g
                        })
                }) {
                    clock = None;
                }
                if let Some(c) = clock.as_mut() {
                    last_clock = Some((c.observed_monotonic_ms, c.source_generation));
                    if c.source_id.as_deref() != Some(source.as_str())
                        || c.clock != "mse_presentation_v1"
                    {
                        c.observation_only();
                    }
                }
                let received = clock.as_ref().map_or(batch.received, |c| c.received_at_ms);
                let message = ChatMessage {
                    sequence: r.event_rows + 1,
                    sender,
                    text,
                    server_time,
                    received_at: received,
                    offset_seconds: batch
                        .receipt
                        .saturating_duration_since(state.started)
                        .as_secs_f64(),
                    broadcast_offset_seconds: None,
                    replay_clock: clock,
                    sender_key: value
                        .get("senderKey")
                        .and_then(Value::as_str)
                        .and_then(crate::streaming::model::bounded_sender_key),
                    rich: rich.map(|r| *r),
                };
                // Native projection is the persistence boundary; page-added private
                // fields never enter this public event journal. Offsets remain per sink.
                let event = json!({"sender":message.sender,"text":message.text,"serverTime":message.server_time,"receivedAt":message.received_at,"replayClock":message.replay_clock,"senderKey":message.sender_key,"rich":message.rich});
                let bytes = serde_json::to_vec(&event).map_err(|_| fault())?;
                if bytes.len() > 64 * 1024 || r.bytes + bytes.len() as u64 + 1 > MAX_TEXT {
                    return Err(fault());
                }
                raw.write_all(&bytes)
                    .and_then(|_| raw.write_all(b"\n"))
                    .map_err(|_| fault())?;
                r.bytes += bytes.len() as u64 + 1;
                r.raw_rows += 1;
                if let Some(rich) = &message.rich {
                    for url in rich.asset_urls().take(25) {
                        if let Some(asset) = replay_assets::asset_id(url) {
                            if expected.len() < 4096 {
                                expected.insert(asset);
                            } else {
                                r.assets_incomplete = true;
                            }
                        }
                    }
                }
                log.append_batched(&message)?;
                assets.submit_recording(&root, message.rich.as_ref());
                r.event_rows += 1;
            }
            log.sync()?;
            raw.sync_data().map_err(|_| fault())?;
            prior = Some((batch.id, batch.bytes));
            Ok(json!({"stored":true}))
        })();
        if result.is_err() {
            r.failure = true;
            state.partial.store(true, Ordering::Release);
        }
        if let Some((window, id)) = batch.reply {
            reply(&window, &id, result);
        }
    }
    // Closing admission plus operations==0 means no producer can append after
    // these final queue drains, including an ENDLIST receipt accepted during recv.
    while let Ok(bytes) = tr.try_recv() {
        if timing_bytes + bytes.len() as u64 > MAX_TEXT || timing.write_all(&bytes).is_err() {
            state.timing_incomplete.store(true, Ordering::Release);
        } else {
            timing_bytes += bytes.len() as u64;
            r.timing_records += 1;
        }
    }
    // Producer admission is detached; preserve target-owned tickets through all
    // outstanding cache jobs. Global ReplayAssetCache is never stopped here.
    let deadline = Instant::now() + Duration::from_secs(2000);
    let fence = wait_fence(
        deadline,
        || replay_assets::try_fence(&root),
        &mut r.assets_incomplete,
    );
    r.assets_incomplete |= expected
        .iter()
        .any(|asset| replay_assets::read_recording(&root, asset).is_err());
    r.failure |= log.sync().is_err() || raw.sync_all().is_err();
    if timing.sync_all().is_err() {
        state.timing_incomplete.store(true, Ordering::Release);
    }
    r.interrupted = state.interrupted.load(Ordering::Acquire);
    r.timing_incomplete = state.timing_incomplete.load(Ordering::Acquire);
    r.text_state =
        if r.failure || r.interrupted || r.event_rows == 0 || state.partial.load(Ordering::Acquire)
        {
            "incomplete"
        } else {
            "ended"
        }
        .into();
    let proof = json!({"version":1,"binding":binding,"summary":r,"assetExpected":expected.len(),"assetIncomplete":r.assets_incomplete,"remoteArchiveMapping":"unknown"});
    match create_file(&root, "relay-summary.json") {
        Ok(mut file) => {
            if serde_json::to_writer(&mut file, &proof).is_err() || file.sync_all().is_err() {
                r.failure = true;
                r.text_state = "incomplete".into();
            }
        }
        Err(_) => {
            r.failure = true;
            r.text_state = "incomplete".into();
        }
    }
    drop(fence);
    r.joined = true;
    r
}

#[cfg(test)]
#[path = "browser_relay_chat_tests.rs"]
mod tests;
