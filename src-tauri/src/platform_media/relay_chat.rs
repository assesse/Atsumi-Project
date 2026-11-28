//! Independent text/image sidecar for one native-approved SOOP relay epoch.
//! No video job, account, remote clock or network authority is created here.
use super::{
    chat_assets::ArchiveAssets,
    chat_parts::{Stream, Writer},
    chat_status::{self, Marker, Reason},
    encoded::OfficialReceiverLease,
    live_chat::SharedChat,
    storage,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

const ERROR: &str = "릴레이 채팅 보존을 마무리하지 못했습니다. 받은 자료는 보존했습니다.";
const QUEUE_ITEMS: usize = 32;
const QUEUE_BYTES: usize = 4 * 1024 * 1024;
const LEDGER_BYTES: u64 = 256 * 1024 * 1024;
#[path = "relay_chat_timing.rs"]
pub(crate) mod timing;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelayChatSummary {
    pub sidecar_id: String,
    #[serde(skip_serializing)]
    pub directory: PathBuf,
    pub raw_rows: u64,
    pub event_rows: u64,
    pub bytes: u64,
    pub interrupted: bool,
    pub assets_incomplete: bool,
    pub failure: Option<String>,
    pub text_state: String,
    pub joined: bool,
    pub clock_quality: &'static str,
    pub timing_records: u64,
    pub timing_incomplete: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Binding {
    pub receiver_id: String,
    pub run_id: String,
    pub source_id: String,
    pub document_epoch: u64,
    pub channel_id: String,
    pub broadcast_id: String,
}
impl Binding {
    fn valid(&self) -> bool {
        [&self.receiver_id, &self.run_id, &self.source_id]
            .into_iter()
            .all(|id| uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == *id))
            && self.document_epoch > 0
            && super::MediaRef {
                provider: "soop".into(),
                kind: "live".into(),
                external_id: self.channel_id.clone(),
            }
            .canonical_url()
            .is_ok()
            && !self.broadcast_id.is_empty()
            && self.broadcast_id.len() <= 20
            && self.broadcast_id.bytes().all(|b| b.is_ascii_digit())
            && self
                .broadcast_id
                .parse::<u64>()
                .is_ok_and(|v| v > 0 && v.to_string() == self.broadcast_id)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Receipt {
    sequence: u64,
    monotonic_ns: Option<u64>,
}
struct Record {
    bytes: Vec<u8>,
    raw: bool,
    display_control: bool,
    receipt: Receipt,
    timing: bool,
    event_clock: bool,
}
struct Queue {
    records: VecDeque<Record>,
    // Includes a record while the writer owns it, not only queued records.
    bytes: usize,
    items: usize,
    control_items: usize,
    control_bytes: usize,
    timing_items: usize,
    timing_bytes: usize,
    event_clock_items: usize,
    event_clock_bytes: usize,
    next: u64,
    finish: bool,
    sealed: bool,
    interrupted: bool,
    failure: Option<&'static str>,
    partial: Option<Reason>,
}
struct Core {
    id: String,
    started: Instant,
    assets: Arc<ArchiveAssets>,
    state: Mutex<Queue>,
    wake: Condvar,
    handle: Mutex<Option<JoinHandle<RelayChatSummary>>>,
    joined: AtomicBool,
    timing_incomplete: AtomicBool,
    #[cfg(test)]
    fail_write: AtomicBool,
}
#[derive(Clone)]
pub(super) struct Sink(Arc<Core>);
impl Sink {
    pub(super) fn display(&self, bytes:&[u8], received:Instant)->bool {
        let mut q=self.0.state.lock().unwrap_or_else(|p|p.into_inner());
        if q.finish||bytes.len()>super::soop_display::MAX_CONTROL_BYTES{return false;}
        if q.control_items>=64 {
            // Preserve ordinary chat capacity. The last queued control becomes
            // explicit Unknown instead of ending the whole public chat sink.
            let mut sizes=None;
            if let Some(row)=q.records.iter_mut().rev().find(|r|r.display_control) {
                if let Ok(value)=serde_json::from_slice::<serde_json::Value>(&row.bytes) {
                    if let Some(mut control)=super::soop_display::Control::parse(&value) {
                        control.clear();
                        if let Ok(unknown)=serde_json::to_vec(&control){sizes=Some((row.bytes.len(),unknown.len()));row.bytes=unknown;}
                    }
                }
            }
            if let Some((before,after))=sizes{q.control_bytes=q.control_bytes-before+after;}
            return false;
        }
        let Some(next)=q.next.checked_add(1) else {fail(&mut q,"sequence_limit",Reason::ArchiveLimit);self.0.wake.notify_one();return false;};
        let receipt=Receipt{sequence:q.next,monotonic_ns:received.checked_duration_since(self.0.started).and_then(|d|u64::try_from(d.as_nanos()).ok())};
        q.next=next;q.control_items+=1;q.control_bytes+=bytes.len();
        q.records.push_back(Record{bytes:bytes.to_vec(),raw:false,display_control:true,receipt,timing:false,event_clock:false});
        self.0.wake.notify_one();true
    }
    pub(super) fn id(&self) -> &str {
        &self.0.id
    }
    pub(super) fn assets(&self) -> &Arc<ArchiveAssets> {
        &self.0.assets
    }
    pub(super) fn joined(&self) -> bool {
        self.0.joined.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub(super) fn fail_next_write(&self) {
        self.0.fail_write.store(true, Ordering::Release);
    }
    pub(super) fn raw(&self, bytes: &[u8], received: Instant) -> Option<Receipt> {
        let mut q = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if q.finish {
            return None;
        }
        let Some(next) = q.next.checked_add(1) else {
            fail(&mut q, "sequence_limit", Reason::ArchiveLimit);
            self.0.wake.notify_one();
            return None;
        };
        let receipt = Receipt {
            sequence: q.next,
            monotonic_ns: received
                .checked_duration_since(self.0.started)
                .and_then(|d| u64::try_from(d.as_nanos()).ok()),
        };
        if !enqueue(&mut q, bytes, true, receipt) {
            self.0.wake.notify_one();
            return None;
        }
        q.next = next;
        self.0.wake.notify_one();
        Some(receipt)
    }
    pub(super) fn event(&self, bytes: &[u8], receipt: Receipt) -> bool {
        let mut q = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        let accepted = enqueue(&mut q, bytes, false, receipt);
        self.0.wake.notify_one();
        accepted
    }
    pub(super) fn partial(&self, reason: Reason) {
        let mut q = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if q.sealed {
            return;
        }
        q.partial.get_or_insert(reason);
    }
    pub(super) fn finish(&self, interrupted: bool) {
        let mut q = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if q.sealed {
            return;
        }
        q.finish = true;
        q.interrupted |= interrupted;
        if interrupted {
            q.partial.get_or_insert(Reason::Interrupted);
        }
        self.0.wake.notify_one();
    }
    fn poll_join(&self) -> Result<Option<RelayChatSummary>, String> {
        let mut handle = self.0.handle.lock().unwrap_or_else(|p| p.into_inner());
        if !handle.as_ref().is_some_and(JoinHandle::is_finished) {
            return Ok(None);
        }
        let outcome = handle.take().expect("checked owned handle").join();
        self.0.joined.store(true, Ordering::Release);
        outcome
            .map(|mut report| {
                report.joined = true;
                Some(report)
            })
            .map_err(|_| ERROR.into())
    }
}
fn fail(q: &mut Queue, code: &'static str, reason: Reason) {
    q.failure.get_or_insert(code);
    q.partial.get_or_insert(reason);
    q.finish = true;
}
fn enqueue(q: &mut Queue, bytes: &[u8], raw: bool, receipt: Receipt) -> bool {
    if q.finish {
        return false;
    }
    let limit = if raw { 512 * 1024 } else { 64 * 1024 };
    if bytes.is_empty()
        || bytes.len() > limit
        || q.items >= QUEUE_ITEMS
        || q.bytes
            .checked_add(bytes.len())
            .is_none_or(|n| n > QUEUE_BYTES)
    {
        fail(q, "queue_limit", Reason::ArchiveLimit);
        return false;
    }
    q.bytes += bytes.len();
    q.items += 1;
    q.records.push_back(Record {
        bytes: bytes.to_vec(),
        raw,
        display_control: false,
        receipt,
        timing: false,
        event_clock: false,
    });
    true
}

/// Manager retains this owner through actual join. SharedChat also retains the
/// exact Sink/JoinHandle until poll_join removes it; Drop never detaches a worker.
pub(crate) struct RelayChatLease {
    chat: Arc<SharedChat>,
    sink: Option<Sink>,
    outcome: Option<Result<RelayChatSummary, String>>,
    _receiver: Option<OfficialReceiverLease>,
    timing: timing::RelayTimingSink,
    native_owner: crate::streaming::relay_status::NativeSidecarOwner,
}
impl RelayChatLease {
    pub(super) fn attach(
        chat: Arc<SharedChat>,
        receiver: Option<OfficialReceiverLease>,
        binding: Binding,
        root: &Path,
    ) -> Result<Self, String> {
        if !binding.valid() {
            return Err(ERROR.into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let parent = root.join("SOOP").join("Relays");
        storage::safe_path(&parent)?;
        fs::create_dir_all(&parent).map_err(|_| ERROR)?;
        let directory = parent.join(&id);
        let mut _parents = chat_status::pin_parents(&directory)?;
        fs::create_dir(&directory).map_err(|_| ERROR)?;
        _parents.extend(chat_status::pin_parents(
            &directory.join("relay-owner.json"),
        )?);
        // Keep the already validated/pinned ancestors while fixing the spelling
        // of this newly created owned directory. Downstream immutable-history
        // readers require the same canonical root, including the Windows prefix.
        let directory = fs::canonicalize(&directory).map_err(|_| ERROR)?;
        #[cfg(windows)]
        _parents.extend(crate::streaming::browser_store::cleanup::pin_directories(&directory)
            .map_err(|_| ERROR)?);
        let mut proof = create(&directory.join("relay-owner.json"))?;
        let owner = serde_json::json!({"version":1,"kind":"soop-relay-chat","sidecarId":id,
            "binding":binding,"timelineQuality":"receive-monotonic-only","mediaJob":false});
        proof
            .write_all(owner.to_string().as_bytes())
            .and_then(|_| proof.sync_all())
            .map_err(|_| ERROR)?;
        let marker = chat_status::begin_multipart(&directory, "soop", &id)?;
        let writer = Writer::create(&directory, &marker)?;
        let assets = ArchiveAssets::new(&directory)?;
        let ledger = create(&directory.join("relay-receive.jsonl"))?;
        let timing_file = create(&directory.join("relay-timing.jsonl"))?;
        let core = Arc::new(Core {
            id: id.clone(),
            started: Instant::now(),
            assets,
            state: Mutex::new(Queue {
                records: VecDeque::new(),
                bytes: 0,
                items: 0,
                control_items: 0,
                control_bytes: 0,
                timing_items: 0,
                timing_bytes: 0,
                event_clock_items: 0, event_clock_bytes: 0,
                next: 1,
                finish: false,
                sealed: false,
                interrupted: false,
                failure: None,
                partial: None,
            }),
            wake: Condvar::new(),
            handle: Mutex::new(None),
            joined: AtomicBool::new(false),
            timing_incomplete: AtomicBool::new(false),
            #[cfg(test)]
            fail_write: AtomicBool::new(false),
        });
        let native_owner = crate::streaming::relay_status::NativeSidecarOwner::Soop {
            sidecar_id: id.clone(), receiver_id: binding.receiver_id.clone(),
            run_id: binding.run_id.clone(), source_id: binding.source_id.clone(),
            document_epoch: binding.document_epoch, channel_id: binding.channel_id.clone(),
            broadcast_id: binding.broadcast_id.clone(),
        };
        let timing = timing::RelayTimingSink::new(core.clone(), binding);
        let worker_core = core.clone();
        let handle = thread::Builder::new()
            .name("relay-chat-writer".into())
            .spawn(move || write_loop(worker_core, directory, marker, writer, ledger, timing_file, _parents))
            .map_err(|_| ERROR.to_string())?;
        *core.handle.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
        let sink = Sink(core);
        if let Err(error) = chat.attach_relay_sink(sink.clone()) {
            sink.finish(true);
            // This is rejected setup before a manager receives ownership.
            while matches!(sink.poll_join(), Ok(None)) {
                thread::sleep(std::time::Duration::from_millis(2));
            }
            return Err(error);
        }
        Ok(Self {
            chat,
            sink: Some(sink),
            outcome: None,
            _receiver: receiver,
            timing,
            native_owner,
        })
    }
    pub(crate) fn native_owner(&self) -> &crate::streaming::relay_status::NativeSidecarOwner { &self.native_owner }
    pub(crate) fn timing(&self) -> timing::RelayTimingSink { self.timing.clone() }
    pub(crate) fn begin_finish(&mut self, interrupted: bool) {
        if let Some(sink) = &self.sink {
            self.chat.finish_relay_sink(sink.id(), interrupted);
        }
    }
    pub(crate) fn poll_join(&mut self) -> Result<Option<RelayChatSummary>, String> {
        if let Some(outcome) = &self.outcome {
            return outcome.clone().map(Some);
        }
        let Some(sink) = &self.sink else {
            return Err(ERROR.into());
        };
        let outcome = match sink.poll_join() {
            Ok(None) => return Ok(None),
            Ok(Some(r)) => Ok(r),
            Err(e) => Err(e),
        };
        self.chat.release_relay_sink(sink.id());
        self.sink.take();
        self.outcome = Some(outcome.clone());
        outcome.map(Some)
    }
}
impl Drop for RelayChatLease {
    fn drop(&mut self) {
        self.begin_finish(true);
    }
}
fn create(path: &Path) -> Result<File, String> {
    storage::safe_path(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    chat_status::checked_handle(&file, LEDGER_BYTES)?;
    Ok(file)
}
/// Isolated in-process owner for native manager unwind tests; no producer or I/O
/// beyond this new temporary sidecar. Not a source admission/receiver proof.
#[cfg(test)]
pub(crate) fn test_fixture() -> (RelayChatLease, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let binding = Binding {
        receiver_id: uuid::Uuid::new_v4().to_string(),
        run_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        channel_id: "fixture".into(),
        broadcast_id: "123".into(),
    };
    let lease = RelayChatLease::attach(SharedChat::new(), None, binding, root.path()).unwrap();
    (lease, root)
}
fn write_loop(
    core: Arc<Core>,
    directory: PathBuf,
    marker: Marker,
    mut writer: Writer,
    mut ledger: File,
    mut timing_file: File,
    _parents: Vec<File>,
) -> RelayChatSummary {
    struct CloseTarget(Arc<ArchiveAssets>);
    impl Drop for CloseTarget {
        fn drop(&mut self) {
            self.0.close();
        }
    }
    let _close_on_unwind = CloseTarget(core.assets.clone());
    let mut report = RelayChatSummary {
        sidecar_id: core.id.clone(),
        directory: directory.clone(),
        raw_rows: 0,
        event_rows: 0,
        bytes: 0,
        interrupted: false,
        assets_incomplete: false,
        failure: None,
        joined: false,
        text_state: "unknown".into(),
        clock_quality: "receive-monotonic-only",
        timing_records: 0,
        timing_incomplete: false,
    };
    let mut ledger_bytes = 0u64;
    let mut event_records = 0u64;
    let mut timing_bytes = 0u64;
    let mut timing_failed = false;
    loop {
        let row = {
            let mut q = core.state.lock().unwrap_or_else(|p| p.into_inner());
            while q.records.is_empty() && !q.finish {
                q = core.wake.wait(q).unwrap_or_else(|p| p.into_inner());
            }
            match q.records.pop_front() {
                Some(row) => row,
                None => break,
            }
        };
        if row.timing {
            if timing_failed
                || timing_bytes + row.bytes.len() as u64 + 1 > LEDGER_BYTES
                || timing_file.write_all(&row.bytes).and_then(|_|timing_file.write_all(b"\n")).is_err() {
                core.timing_incomplete.store(true, Ordering::Release);
                timing_failed = true;
            } else {
                timing_bytes += row.bytes.len() as u64 + 1;
                report.timing_records += 1;
            }
            let mut q=core.state.lock().unwrap_or_else(|p|p.into_inner());
            if row.event_clock {q.event_clock_items-=1;q.event_clock_bytes-=row.bytes.len();}
            else {q.timing_items-=1;q.timing_bytes-=row.bytes.len();}
            continue;
        }
        let result = (|| {
            #[cfg(test)]
            if core.fail_write.swap(false, Ordering::AcqRel) {
                return Err(Reason::WriteFailed);
            }
            let evidence = serde_json::json!({"version":1,"receipt":row.receipt.sequence,
                "receiveMonotonicNs":row.receipt.monotonic_ns,
                "stream":if row.raw {"source"} else {"events"},
                "row":if row.raw {report.raw_rows} else {event_records},
                "displayControl":row.display_control,
                "sha256":format!("{:x}",Sha256::digest(&row.bytes))});
            let evidence = serde_json::to_vec(&evidence).map_err(|_| Reason::WriteFailed)?;
            if ledger_bytes + evidence.len() as u64 + 1 > LEDGER_BYTES {
                return Err(Reason::ArchiveLimit);
            }
            writer.append(
                if row.raw {
                    Stream::Source
                } else {
                    Stream::Events
                },
                &row.bytes,
            )?;
            ledger
                .write_all(&evidence)
                .and_then(|_| ledger.write_all(b"\n"))
                .map_err(|_| Reason::WriteFailed)?;
            ledger_bytes += evidence.len() as u64 + 1;
            if row.raw {
                report.raw_rows += 1;
            } else {
                event_records += 1;
                if !row.display_control {report.event_rows += 1;}
            }
            report.bytes += row.bytes.len() as u64 + 1;
            Ok(())
        })();
        let mut q = core.state.lock().unwrap_or_else(|p| p.into_inner());
        if row.display_control {q.control_bytes-=row.bytes.len();q.control_items-=1;}
        else {q.bytes -= row.bytes.len();q.items -= 1;}
        if let Err(reason) = result {
            fail(&mut q, "write_failed", reason);
            // Previously durable bytes remain. Do not pretend queued rows were saved.
            q.records.clear();
            q.bytes = 0;
            q.items = 0;
            q.control_items=0;q.control_bytes=0;
            q.timing_items=0;q.timing_bytes=0;q.event_clock_items=0;q.event_clock_bytes=0;
            core.timing_incomplete.store(true,Ordering::Release);
            break;
        }
    }
    let ledger_failed = ledger.sync_all().is_err();
    if timing_file.sync_all().is_err(){core.timing_incomplete.store(true,Ordering::Release);}
    // At most64 queued +1 active fetch, each bounded5s by fetch_source. New
    // events cannot add this target after finish. This is not an OS I/O SLA.
    let asset_deadline = Instant::now() + std::time::Duration::from_secs(330);
    while core.assets.pending() && Instant::now() < asset_deadline {
        thread::sleep(std::time::Duration::from_millis(10));
    }
    report.assets_incomplete = core.assets.close();
    // Final input outcome linearizes only after target drain. Cancellation or
    // source failure during that wait must not be hidden by an earlier snapshot.
    let mut reason = {
        let mut q = core.state.lock().unwrap_or_else(|p| p.into_inner());
        q.sealed = true;
        report.failure = q.failure.map(str::to_owned);
        report.interrupted = q.interrupted;
        report.timing_incomplete = core.timing_incomplete.load(Ordering::Acquire);
        q.partial.clone()
    };
    if ledger_failed {
        reason.get_or_insert(Reason::FlushFailed);
        report.failure.get_or_insert("ledger_flush_failed".into());
    }
    let parts = writer.close();
    match chat_status::finish_multipart(
        &directory,
        &marker,
        parts,
        reason,
        report.assets_incomplete,
        &AtomicBool::new(false),
    ) {
        Ok(summary) => {
            report.text_state = match summary.state {
                chat_status::TextState::Complete => "complete",
                chat_status::TextState::Partial => "partial",
                chat_status::TextState::Unknown => "unknown",
            }
            .into()
        }
        Err(_) => {
            report.failure.get_or_insert("status_failed".into());
        }
    }
    // This is a storage summary, never an uploaded/remote-synchronized receipt.
    if create(&directory.join("relay-summary.json"))
        .and_then(|mut file| {
            let bytes = serde_json::to_vec(&report).map_err(|_| ERROR)?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| ERROR.into())
        })
        .is_err()
    {
        report.failure.get_or_insert("summary_failed".into());
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    fn queue() -> Queue {
        Queue {
            records: VecDeque::new(),
            bytes: 0,
            items: 0,
            control_items: 0,
            control_bytes: 0,
            timing_items: 0,
            timing_bytes: 0,
                event_clock_items: 0, event_clock_bytes: 0,
            next: 1,
            finish: false,
            sealed: false,
            interrupted: false,
            failure: None,
            partial: None,
        }
    }
    #[test]
    fn stalled_display_writer_does_not_consume_public_chat_credits_or_end_sink() {
        use super::super::soop_display::{Control,Values,Evidence,Basis,FORMAT};
        let root=tempfile::tempdir().unwrap();
        // The real queue is deliberately held without its writer consuming it.
        // An in-flight control still owns its credit after it leaves the deque.
        let sink=Sink(Arc::new(Core{id:uuid::Uuid::new_v4().to_string(),started:Instant::now(),assets:ArchiveAssets::new(root.path()).unwrap(),
            state:Mutex::new(queue()),wake:Condvar::new(),handle:Mutex::new(None),joined:AtomicBool::new(false),timing_incomplete:AtomicBool::new(false),fail_write:AtomicBool::new(false)}));
        let control=Control{format:FORMAT.into(),receiver_id:uuid::Uuid::new_v4().to_string(),document_epoch:1,sequence:1,received_at_ms:1_700_000_000_000,observed_at_ms:Some(1_700_000_000_000),initial:false,
            values:Values{show_emoticons:Some(true),..Default::default()},basis:Evidence{show_emoticons:Some(Basis::Dom),..Default::default()},palette:None,palette_incomplete:false,surface:None,surface_incomplete:false};
        let mut value=serde_json::to_value(&control).unwrap();
        value["palette"]=serde_json::json!({"version":1,"basis":"computed-css-variable","nickname":vec![Some("#123456");20],"roles":{"normal":"#123456","streamer":null,"manager":null,"vip":null,"subscribe":null,"fan":null,"support":null,"nightbot":null}});
        let control=Control::parse(&value).unwrap();assert!(control.palette.is_some());
        let bytes=serde_json::to_vec(&control).unwrap();
        for _ in 0..64 {assert!(sink.display(&bytes,Instant::now()));}
        let held=sink.0.state.lock().unwrap().records.pop_front().unwrap();
        assert!(!sink.display(&bytes,Instant::now()));
        for _ in 0..QUEUE_ITEMS {assert!(sink.raw(b"{}",Instant::now()).is_some());}
        let q=sink.0.state.lock().unwrap();
        assert!(!q.finish);assert!(q.failure.is_none());assert_eq!(q.items,QUEUE_ITEMS);assert_eq!(q.control_items,64);assert!(q.control_bytes<=64*2048);
        let last=q.records.iter().rev().find(|r|r.display_control).unwrap();let value:serde_json::Value=serde_json::from_slice(&last.bytes).unwrap();
        let unknown=Control::parse(&value).unwrap();assert_eq!(unknown.values,Values::default());assert!(unknown.palette.is_none());assert!(!unknown.palette_incomplete);assert!(held.display_control);
    }
    #[test]
    fn dequeued_payload_keeps_credit_until_writer_finishes() {
        let mut q = queue();
        let receipt = Receipt {
            sequence: 1,
            monotonic_ns: Some(1),
        };
        let b = vec![1; 512 * 1024];
        for _ in 0..8 {
            assert!(enqueue(&mut q, &b, true, receipt));
        }
        let held = q.records.pop_front().unwrap();
        assert_eq!(q.bytes, QUEUE_BYTES);
        assert!(!enqueue(&mut q, b"x", true, receipt));
        assert_eq!(q.failure, Some("queue_limit"));
        assert!(q.finish);
        assert_eq!(held.bytes.len(), 512 * 1024);
    }
    #[test]
    fn full_queue_failure_cannot_be_cleared_by_later_stop() {
        let mut q = queue();
        let r = Receipt {
            sequence: 1,
            monotonic_ns: None,
        };
        for _ in 0..QUEUE_ITEMS {
            assert!(enqueue(&mut q, b"{}", true, r));
        }
        assert!(!enqueue(&mut q, b"{}", true, r));
        fail(&mut q, "later", Reason::Interrupted);
        assert_eq!(q.failure, Some("queue_limit"));
        assert_eq!(q.partial, Some(Reason::ArchiveLimit));
        assert!(!enqueue(&mut q, b"{}", false, r));
    }
    #[test]
    fn forged_native_identity_rejected_before_directory_creation() {
        let root = tempfile::tempdir().unwrap();
        let binding = Binding {
            receiver_id: "bad".into(),
            run_id: uuid::Uuid::new_v4().to_string(),
            source_id: uuid::Uuid::new_v4().to_string(),
            document_epoch: 1,
            channel_id: "fixture".into(),
            broadcast_id: "123".into(),
        };
        assert!(RelayChatLease::attach(SharedChat::new(), None, binding, root.path()).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[test]
    fn ordinary_windows_source_root_yields_canonical_pinned_native_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let ordinary = PathBuf::from(root.path().to_string_lossy().trim_start_matches(r"\\?\").to_owned());
        assert_ne!(ordinary, fs::canonicalize(&ordinary).unwrap());
        let binding = Binding {
            receiver_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            source_id: uuid::Uuid::new_v4().to_string(),
            document_epoch: 1, channel_id: "fixture".into(), broadcast_id: "123".into(),
        };
        let mut lease = RelayChatLease::attach(SharedChat::new(), None, binding.clone(), &ordinary).unwrap();
        lease.begin_finish(false);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let summary = loop {
            if let Some(summary) = lease.poll_join().unwrap() { break summary; }
            assert!(Instant::now() < deadline, "sidecar writer did not join");
            thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(summary.joined && summary.failure.is_none());
        assert_eq!(summary.directory, fs::canonicalize(&summary.directory).unwrap());
        let _pins = crate::streaming::browser_store::cleanup::pin_directories(&summary.directory).unwrap();
        let _owner = crate::streaming::browser_store::cleanup::verification_guard(
            &summary.directory.join("relay-owner.json")).unwrap();
        let owner: serde_json::Value = serde_json::from_slice(&fs::read(summary.directory.join("relay-owner.json")).unwrap()).unwrap();
        assert_eq!(owner["binding"], serde_json::to_value(&binding).unwrap());
        assert_eq!(summary.directory.file_name().unwrap().to_str(), Some(summary.sidecar_id.as_str()));
        assert!(summary.directory.starts_with(fs::canonicalize(ordinary).unwrap()));
    }
}
