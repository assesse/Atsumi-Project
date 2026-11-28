//! Native-only bounded ingress. This module has no page/account authority,
//! archive, transport, disk, or process integration. An InputEnded report means
//! only that admitted input was validated and put into the output mailbox.
//! The native work-set MUST retain RelayRun until join() returns Joined.
use super::browser::encoded::fmp4::{
    relay::{
        ts::{TsEnd, TsMuxer, TsRejection, TsSegment},
        ContinuousChunk, ContinuousEnd, ContinuousMp4Muxer, EncodedSampleReader, ReaderEnd,
        SampleDescriptor, SampleState, ValidatedSampleBatch,
    },
    EncodedTrackInput,
};
use sha2::{Digest, Sha256};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

const CHUNK_BYTES: usize = 128 * 1024;
const QUEUE_SLOTS: usize = 8;
const INPUT_BYTES: usize = CHUNK_BYTES * QUEUE_SLOTS;
const APPEND_BYTES: usize = 16 * 1024 * 1024;
const OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const OPEN: u8 = 0;
const FINISHING: u8 = 1;
const INPUT_ENDED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelayProvider {
    Soop,
    Chzzk,
}

/// Identity, not proof of authorization. The future native receiver must bind
/// these IDs only after its existing source/origin/document checks succeed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeIdentity {
    pub provider: RelayProvider,
    pub owner: Uuid,
    pub run: Uuid,
    pub source: Uuid,
    pub document_epoch: u64,
}

#[derive(Clone)]
pub(crate) struct RelayBinding(Arc<BindingIdentity>);
struct BindingIdentity {
    identity: NativeIdentity,
    started: AtomicBool,
}
impl RelayBinding {
    pub(crate) fn new(identity: NativeIdentity) -> Result<Self, Fault> {
        if identity.owner.is_nil()
            || identity.run.is_nil()
            || identity.source.is_nil()
            || identity.document_epoch == 0
        {
            return Err(Fault::Binding);
        }
        Ok(Self(Arc::new(BindingIdentity {
            identity,
            started: AtomicBool::new(false),
        })))
    }
    pub(crate) fn identity(&self) -> NativeIdentity {
        self.0.identity
    }
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Fault {
    Cancelled = 10,
    Binding,
    Bounds,
    InputFull,
    InputClosed,
    Order,
    Codec,
    OutputFull,
    OutputClosed,
    PartialAppend,
    WorkerPanic,
    Spawn,
    Closed,
}
impl Fault {
    fn from_state(value: u8) -> Self {
        match value {
            10 => Self::Cancelled,
            11 => Self::Binding,
            12 => Self::Bounds,
            13 => Self::InputFull,
            14 => Self::InputClosed,
            15 => Self::Order,
            16 => Self::Codec,
            17 => Self::OutputFull,
            18 => Self::OutputClosed,
            19 => Self::PartialAppend,
            20 => Self::WorkerPanic,
            21 => Self::Spawn,
            _ => Self::Closed,
        }
    }
}

struct Budget {
    used: AtomicUsize,
    peak: AtomicUsize,
    limit: usize,
}
impl Budget {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            limit,
        })
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Credit> {
        let old = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|n| *n <= self.limit)
            })
            .ok()?;
        self.peak.fetch_max(old + bytes, Ordering::Relaxed);
        Some(Credit {
            budget: self.clone(),
            bytes,
        })
    }
}
struct Credit {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct Core {
    binding: RelayBinding,
    state: AtomicU8,
    enqueue_ops: AtomicUsize,
    input: Arc<Budget>,
    output: Arc<Budget>,
    output_alive: AtomicBool,
    codec_admitted: AtomicBool,
}
impl Core {
    fn new(binding: RelayBinding) -> Arc<Self> {
        Arc::new(Self {
            binding,
            state: AtomicU8::new(OPEN),
            enqueue_ops: AtomicUsize::new(0),
            input: Budget::new(INPUT_BYTES),
            output: Budget::new(OUTPUT_BYTES),
            output_alive: AtomicBool::new(true),
            codec_admitted: AtomicBool::new(false),
        })
    }
    /// First terminal state wins. A finish REQUEST is not a successful end;
    /// it can still be interrupted by a fault/cancel before end validation.
    fn interrupt(&self, fault: Fault) -> Fault {
        let prior = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state < INPUT_ENDED).then_some(fault as u8)
            });
        match prior {
            Ok(_) => fault,
            Err(state) => Fault::from_state(state),
        }
    }
    fn fault(&self) -> Option<Fault> {
        let state = self.state.load(Ordering::Acquire);
        (state > INPUT_ENDED).then(|| Fault::from_state(state))
    }
}

struct EnqueueOperation<'a>(&'a AtomicUsize);
impl Drop for EnqueueOperation<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct InputChunk {
    track: u32,
    append: u64,
    chunk: u32,
    last: bool,
    bytes: Box<[u8]>,
    _credit: Credit,
}

#[derive(Clone)]
pub(crate) struct RelayInput {
    sender: SyncSender<InputChunk>,
    core: Arc<Core>,
}
impl RelayInput {
    /// Copies only bounded admitted bytes; never waits for the worker or sink.
    /// The caller must also bound the upstream source allocation/IPC message.
    pub(crate) fn try_enqueue(
        &self,
        binding: &RelayBinding,
        track: u32,
        append: u64,
        chunk: u32,
        last: bool,
        bytes: &[u8],
    ) -> Result<(), Fault> {
        self.core.enqueue_ops.fetch_add(1, Ordering::SeqCst);
        let _operation = EnqueueOperation(&self.core.enqueue_ops);
        if self.core.state.load(Ordering::SeqCst) != OPEN {
            return Err(Fault::Closed);
        }
        if !self.core.binding.same(binding) {
            return Err(self.core.interrupt(Fault::Binding));
        }
        // Arbitrary small nonempty chunks are valid. Their count can never
        // exceed the append byte budget; this is not a128-chunk size policy.
        if track > 1
            || chunk >= APPEND_BYTES as u32
            || bytes.is_empty()
            || bytes.len() > CHUNK_BYTES
        {
            return Err(self.core.interrupt(Fault::Bounds));
        }
        let credit = self
            .core
            .input
            .reserve(bytes.len())
            .ok_or_else(|| self.core.interrupt(Fault::InputFull))?;
        let value = InputChunk {
            track,
            append,
            chunk,
            last,
            bytes: bytes.into(),
            _credit: credit,
        };
        if let Some(fault) = self.core.fault() {
            return Err(fault);
        }
        match self.sender.try_send(value) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(self.core.interrupt(Fault::InputFull)),
            Err(TrySendError::Disconnected(_)) => Err(self.core.interrupt(Fault::InputClosed)),
        }
    }
    pub(crate) fn finish(&self) {
        let _ =
            self.core
                .state
                .compare_exchange(OPEN, FINISHING, Ordering::SeqCst, Ordering::SeqCst);
    }
    pub(crate) fn cancel(&self) {
        self.core.interrupt(Fault::Cancelled);
    }
    pub(crate) fn codec_admitted(&self) -> bool {
        self.core.codec_admitted.load(Ordering::Acquire)
    }
}

/// Byte credit is held until this owner drops, including after dequeue. No
/// into_bytes is exposed: moving out the Vec would evade downstream accounting.
pub(crate) struct RelayOutput {
    payload: OutputPayload,
    binding: RelayBinding,
    _credit: Credit,
}
enum OutputPayload {
    Mp4(ContinuousChunk),
    Ts(TsSegment),
}
impl OutputPayload {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Mp4(chunk) => &chunk.bytes,
            Self::Ts(segment) => segment.bytes(),
        }
    }
}
impl From<ContinuousChunk> for OutputPayload {
    fn from(chunk: ContinuousChunk) -> Self {
        Self::Mp4(chunk)
    }
}
impl RelayOutput {
    pub(crate) fn chunk(&self) -> Option<&ContinuousChunk> {
        match &self.payload {
            OutputPayload::Mp4(chunk) => Some(chunk),
            OutputPayload::Ts(_) => None,
        }
    }
    /// Borrow while this guard owns the byte credit, including throughout a
    /// synchronous delivery call. A dequeue is not a memory-budget release.
    pub(crate) fn ts_segment(&self) -> Option<&TsSegment> {
        match &self.payload {
            OutputPayload::Ts(segment) => Some(segment),
            OutputPayload::Mp4(_) => None,
        }
    }
    pub(crate) fn binding(&self) -> &RelayBinding {
        &self.binding
    }
}
pub(crate) struct RelayOutputReceiver {
    receiver: Receiver<RelayOutput>,
    core: Arc<Core>,
}
impl RelayOutputReceiver {
    pub(crate) fn try_recv(&self) -> Result<RelayOutput, TryRecvError> {
        self.receiver.try_recv()
    }
    pub(crate) fn held_bytes(&self) -> usize {
        self.core.output.used.load(Ordering::Acquire)
    }
}
impl Drop for RelayOutputReceiver {
    fn drop(&mut self) {
        // Linearize disconnection against the final InputEnded CAS; a receiver
        // dropped before end cannot race past the worker's last alive check.
        self.core.interrupt(Fault::OutputClosed);
        self.core.output_alive.store(false, Ordering::Release);
    }
}
struct BoundedOutput {
    sender: SyncSender<RelayOutput>,
    core: Arc<Core>,
}
impl BoundedOutput {
    fn try_accept(&self, payload: impl Into<OutputPayload>) -> Result<(), Fault> {
        if let Some(fault) = self.core.fault() {
            return Err(fault);
        }
        let payload = payload.into();
        let credit = self
            .core
            .output
            .reserve(payload.bytes().len())
            .ok_or(Fault::OutputFull)?;
        let output = RelayOutput {
            payload,
            binding: self.core.binding.clone(),
            _credit: credit,
        };
        match self.sender.try_send(output) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(Fault::OutputFull),
            Err(TrySendError::Disconnected(_)) => Err(Fault::OutputClosed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorkerCounts {
    pub consumed_chunks: u64,
    pub duplicate_chunks: u64,
    pub validated_appends: u64,
    pub output_chunks: u64,
    pub output_bytes: u64,
    pub partial_append_peak_bytes: usize,
    pub codec_diagnostic: Option<CodecDiagnostic>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodecStage {
    ReaderInit,
    MuxInit,
    ReaderPush,
    MuxPush,
    ReaderFinish,
    MuxFinish,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodecDiagnostic {
    pub stage: CodecStage,
    pub error_code: &'static str,
    pub reader_before_call: Option<SampleState>,
    pub mux_before_call: Option<SampleState>,
    pub ts_rejection: Option<TsRejection>,
}
fn codec_fault(
    counts: &mut WorkerCounts,
    stage: CodecStage,
    error: super::model::StreamError,
    reader: Option<SampleState>,
    mux: Option<SampleState>,
) -> Fault {
    let error_code = match error.code.as_str() {
        "ENCODED_UNSUPPORTED" => "ENCODED_UNSUPPORTED",
        "ENCODED_TS_CLOCK_UNSUPPORTED" => "ENCODED_TS_CLOCK_UNSUPPORTED",
        _ => "ENCODED_OTHER",
    };
    counts.codec_diagnostic.get_or_insert(CodecDiagnostic {
        stage,
        error_code,
        reader_before_call: reader,
        mux_before_call: mux,
        ts_rejection: None,
    });
    Fault::Codec
}
fn codec_mux_fault(
    counts: &mut WorkerCounts,
    stage: CodecStage,
    error: super::model::StreamError,
    reader: Option<SampleState>,
    mux: Option<SampleState>,
    rejection: Option<TsRejection>,
) -> Fault {
    let first = counts.codec_diagnostic.is_none();
    let fault = codec_fault(counts, stage, error, reader, mux);
    if first {
        if let Some(diagnostic) = counts.codec_diagnostic.as_mut() {
            diagnostic.ts_rejection = rejection;
        }
    }
    fault
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum InputEnd {
    Ended(ContinuousEnd),
    TsEnded(TsEnd),
    Interrupted(Fault),
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerReport {
    pub input_end: InputEnd,
    pub counts: WorkerCounts,
    pub input_credit_bytes: usize,
    pub output_held_bytes: usize,
    pub input_peak_bytes: usize,
    pub output_peak_bytes: usize,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum JoinResult {
    Pending,
    Joined(WorkerReport),
}

pub(crate) struct RelayRun {
    core: Arc<Core>,
    handle: Option<JoinHandle<WorkerReport>>,
    joined: Option<WorkerReport>,
}
#[derive(Clone, Copy)]
enum OutputMode {
    Mp4,
    Ts,
}
impl RelayRun {
    pub(crate) fn start(
        binding: RelayBinding,
        tracks: Vec<EncodedTrackInput>,
    ) -> Result<(Self, RelayInput, RelayOutputReceiver), Fault> {
        Self::start_mode(binding, tracks, OutputMode::Mp4)
    }
    /// Native callers select this fixed mode before codec admission. It does
    /// not expose a new page command, transport target, or upload authority.
    pub(crate) fn start_ts(
        binding: RelayBinding,
        tracks: Vec<EncodedTrackInput>,
    ) -> Result<(Self, RelayInput, RelayOutputReceiver), Fault> {
        Self::start_mode(binding, tracks, OutputMode::Ts)
    }
    fn start_mode(
        binding: RelayBinding,
        tracks: Vec<EncodedTrackInput>,
        mode: OutputMode,
    ) -> Result<(Self, RelayInput, RelayOutputReceiver), Fault> {
        // Admit only bounded original codec inputs here; full codec parsing is
        // done by the worker. Never expose a prevalidated descriptor to callers.
        if tracks.is_empty()
            || tracks.len() > 2
            || tracks.iter().any(|t| {
                t.track_index > 1
                    || t.init.is_empty()
                    || t.init.len() > 64 * 1024
                    || t.mime_type.is_empty()
                    || t.mime_type.len() > 512
            })
            || (tracks.len() == 2 && tracks[0].track_index == tracks[1].track_index)
        {
            return Err(Fault::Bounds);
        }
        if binding.0.started.swap(true, Ordering::AcqRel) {
            return Err(Fault::Binding);
        }
        let (core, input, receiver, output, output_receiver) = channels(binding);
        let worker_core = core.clone();
        let handle = thread::Builder::new()
            .name("relay-ingress".into())
            .spawn(move || {
                guard_worker(&worker_core, || {
                    worker_mode(&worker_core, tracks, receiver, output, mode)
                })
            })
            .map_err(|_| Fault::Spawn)?;
        Ok((
            Self {
                core,
                handle: Some(handle),
                joined: None,
            },
            input,
            output_receiver,
        ))
    }
    pub(crate) fn cancel(&self) {
        self.core.interrupt(Fault::Cancelled);
    }
    /// Timeout leaves the handle and owner intact. Future active-work removal
    /// must use Joined, never Drop, a terminal atomic flag, or InputEnded alone.
    pub(crate) fn join(&mut self, timeout: Duration) -> JoinResult {
        if let Some(value) = self.joined {
            return JoinResult::Joined(value);
        }
        let start = Instant::now();
        while let Some(handle) = self.handle.as_ref() {
            if handle.is_finished() {
                let value = self.handle.take().unwrap().join().unwrap_or_else(|_| {
                    let fault = self.core.interrupt(Fault::WorkerPanic);
                    report(
                        &self.core,
                        InputEnd::Interrupted(fault),
                        WorkerCounts::default(),
                    )
                });
                self.joined = Some(value);
                return JoinResult::Joined(value);
            }
            if start.elapsed() >= timeout {
                return JoinResult::Pending;
            }
            thread::sleep(Duration::from_millis(1).min(timeout.saturating_sub(start.elapsed())));
        }
        JoinResult::Pending
    }
}
fn guard_worker(core: &Core, action: impl FnOnce() -> WorkerReport) -> WorkerReport {
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(report) => report,
        Err(_) => {
            let fault = core.interrupt(Fault::WorkerPanic);
            report(core, InputEnd::Interrupted(fault), WorkerCounts::default())
        }
    }
}
impl Drop for RelayRun {
    fn drop(&mut self) {
        // Detaching is only a last-resort cleanup aid, never join evidence.
        if self.handle.is_some() {
            self.cancel();
        }
    }
}

fn channels(
    binding: RelayBinding,
) -> (
    Arc<Core>,
    RelayInput,
    Receiver<InputChunk>,
    BoundedOutput,
    RelayOutputReceiver,
) {
    let core = Core::new(binding);
    let (sender, receiver) = mpsc::sync_channel(QUEUE_SLOTS);
    let (out_sender, out_receiver) = mpsc::sync_channel(QUEUE_SLOTS);
    (
        core.clone(),
        RelayInput {
            sender,
            core: core.clone(),
        },
        receiver,
        BoundedOutput {
            sender: out_sender,
            core: core.clone(),
        },
        RelayOutputReceiver {
            receiver: out_receiver,
            core,
        },
    )
}
fn report(core: &Core, input_end: InputEnd, counts: WorkerCounts) -> WorkerReport {
    WorkerReport {
        input_end,
        counts,
        input_credit_bytes: core.input.used.load(Ordering::Acquire),
        output_held_bytes: core.output.used.load(Ordering::Acquire),
        input_peak_bytes: core.input.peak.load(Ordering::Acquire),
        output_peak_bytes: core.output.peak.load(Ordering::Acquire),
    }
}

#[derive(PartialEq, Eq)]
struct Receipt {
    track: u32,
    append: u64,
    chunk: u32,
    last: bool,
    size: usize,
    hash: [u8; 32],
}
impl Receipt {
    fn of(input: &InputChunk) -> Self {
        Self {
            track: input.track,
            append: input.append,
            chunk: input.chunk,
            last: input.last,
            size: input.bytes.len(),
            hash: Sha256::digest(&input.bytes).into(),
        }
    }
}
struct PartialAppend {
    track: u32,
    append: u64,
    next_chunk: u32,
    bytes: Vec<u8>,
}
struct Assembly {
    tracks: [bool; 2],
    next_append: [u64; 2],
    last: Option<Receipt>,
    partial: Option<PartialAppend>,
}
impl Assembly {
    fn new(tracks: &[EncodedTrackInput]) -> Self {
        let mut present = [false; 2];
        for track in tracks {
            present[track.track_index as usize] = true;
        }
        Self {
            tracks: present,
            next_append: [0; 2],
            last: None,
            partial: None,
        }
    }
    fn push(
        &mut self,
        input: &InputChunk,
        counts: &mut WorkerCounts,
    ) -> Result<Option<(u32, Vec<u8>)>, Fault> {
        let receipt = Receipt::of(input);
        if self.last.as_ref() == Some(&receipt) {
            counts.duplicate_chunks += 1;
            return Ok(None);
        }
        let track = input.track as usize;
        if track >= self.tracks.len()
            || !self.tracks[track]
            || input.append != self.next_append[track]
        {
            return Err(Fault::Order);
        }
        if self.partial.is_none() {
            if input.chunk != 0 {
                return Err(Fault::Order);
            }
            self.partial = Some(PartialAppend {
                track: input.track,
                append: input.append,
                next_chunk: 0,
                bytes: Vec::new(),
            });
        }
        let partial = self.partial.as_mut().unwrap();
        if partial.track != input.track
            || partial.append != input.append
            || partial.next_chunk != input.chunk
        {
            return Err(Fault::Order);
        }
        let size = partial
            .bytes
            .len()
            .checked_add(input.bytes.len())
            .filter(|n| *n <= APPEND_BYTES)
            .ok_or(Fault::Bounds)?;
        partial
            .bytes
            .try_reserve_exact(input.bytes.len())
            .map_err(|_| Fault::Bounds)?;
        partial.bytes.extend_from_slice(&input.bytes);
        partial.next_chunk = partial.next_chunk.checked_add(1).ok_or(Fault::Bounds)?;
        counts.partial_append_peak_bytes = counts.partial_append_peak_bytes.max(size);
        self.last = Some(receipt);
        if !input.last {
            return Ok(None);
        }
        self.next_append[track] = self.next_append[track]
            .checked_add(1)
            .ok_or(Fault::Bounds)?;
        let done = self.partial.take().unwrap();
        Ok(Some((done.track, done.bytes)))
    }
}

enum OutputMuxer {
    Mp4(ContinuousMp4Muxer),
    Ts(TsMuxer),
}
impl OutputMuxer {
    fn rejection(&self) -> Option<TsRejection> {
        match self { Self::Mp4(_) => None, Self::Ts(mux) => mux.rejection() }
    }
    fn new(
        mode: OutputMode,
        descriptor: SampleDescriptor,
    ) -> Result<Self, super::model::StreamError> {
        match mode {
            OutputMode::Mp4 => Ok(Self::Mp4(ContinuousMp4Muxer::new(descriptor))),
            OutputMode::Ts => TsMuxer::new(descriptor).map(Self::Ts),
        }
    }
    fn sample_state(&self) -> SampleState {
        match self {
            Self::Mp4(mux) => mux.sample_state(),
            Self::Ts(mux) => mux.sample_state(),
        }
    }
    fn push(
        &mut self,
        batch: ValidatedSampleBatch,
    ) -> Result<Vec<OutputPayload>, super::model::StreamError> {
        match self {
            Self::Mp4(mux) => mux
                .push(batch)
                .map(|c| c.into_iter().map(OutputPayload::Mp4).collect()),
            Self::Ts(mux) => mux
                .push(batch)
                .map(|s| s.into_iter().map(OutputPayload::Ts).collect()),
        }
    }
    fn finish(
        &mut self,
        end: ReaderEnd,
    ) -> Result<(Vec<OutputPayload>, InputEnd), super::model::StreamError> {
        match self {
            Self::Mp4(mux) => mux
                .finish(end)
                .map(|end| (Vec::new(), InputEnd::Ended(end))),
            Self::Ts(mux) => mux.finish(end).map(|(segments, end)| {
                (
                    segments.into_iter().map(OutputPayload::Ts).collect(),
                    InputEnd::TsEnded(end),
                )
            }),
        }
    }
}
fn publish(
    payloads: Vec<OutputPayload>,
    counts: &mut WorkerCounts,
    output: &BoundedOutput,
) -> Result<(), Fault> {
    for payload in payloads {
        let bytes = payload.bytes().len() as u64;
        output.try_accept(payload)?;
        counts.output_chunks += 1;
        counts.output_bytes += bytes;
    }
    Ok(())
}
fn consume(
    input: InputChunk,
    assembly: &mut Assembly,
    reader: &mut EncodedSampleReader,
    mux: &mut OutputMuxer,
    counts: &mut WorkerCounts,
    core: &Core,
    output: &BoundedOutput,
) -> Result<(), Fault> {
    counts.consumed_chunks += 1;
    let assembled = assembly.push(&input, counts)?;
    // In-flight envelope credit includes its copy into the independently
    // bounded16MiB assembly; do not release on dequeue.
    drop(input);
    if let Some((track, bytes)) = assembled {
        let reader_before = reader.sample_state();
        let mux_before = mux.sample_state();
        let batch = reader.push(track, &bytes).map_err(|error| {
            codec_fault(
                counts,
                CodecStage::ReaderPush,
                error,
                Some(reader_before),
                Some(mux_before),
            )
        })?;
        drop(bytes);
        if let Some(fault) = core.fault() {
            return Err(fault);
        }
        // Empty batches carry reader ordinal, so never skip them.
        let reader_before = reader.sample_state();
        let payloads = mux.push(batch).map_err(|error| {
            codec_mux_fault(
                counts,
                CodecStage::MuxPush,
                error,
                Some(reader_before),
                Some(mux_before),
                mux.rejection(),
            )
        })?;
        counts.validated_appends += 1;
        publish(payloads, counts, output)?;
    }
    Ok(())
}

#[cfg(test)]
fn worker(
    core: &Core,
    tracks: Vec<EncodedTrackInput>,
    receiver: Receiver<InputChunk>,
    output: BoundedOutput,
) -> WorkerReport {
    worker_mode(core, tracks, receiver, output, OutputMode::Mp4)
}
fn worker_mode(
    core: &Core,
    tracks: Vec<EncodedTrackInput>,
    receiver: Receiver<InputChunk>,
    output: BoundedOutput,
    mode: OutputMode,
) -> WorkerReport {
    let mut counts = WorkerCounts::default();
    let outcome = (|| -> Result<InputEnd, Fault> {
        if let Some(fault) = core.fault() {
            return Err(fault);
        }
        let mut assembly = Assembly::new(&tracks);
        let mut reader = EncodedSampleReader::new(tracks)
            .map_err(|error| codec_fault(&mut counts, CodecStage::ReaderInit, error, None, None))?;
        let mut mux = OutputMuxer::new(mode, reader.descriptor()).map_err(|error| {
            codec_fault(
                &mut counts,
                CodecStage::MuxInit,
                error,
                Some(reader.sample_state()),
                None,
            )
        })?;
        core.codec_admitted.store(true, Ordering::Release);
        loop {
            if let Some(fault) = core.fault() {
                return Err(fault);
            }
            if !core.output_alive.load(Ordering::Acquire) {
                return Err(Fault::OutputClosed);
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(input) => {
                    consume(
                        input,
                        &mut assembly,
                        &mut reader,
                        &mut mux,
                        &mut counts,
                        core,
                        &output,
                    )?;
                }
                Err(receive_error) => {
                    let state = core.state.load(Ordering::SeqCst);
                    if state == FINISHING && core.enqueue_ops.load(Ordering::SeqCst) == 0 {
                        // An operation could have sent after recv_timeout returned
                        // but before its count reached zero; recheck the mailbox.
                        match receiver.try_recv() {
                            Ok(input) => {
                                consume(
                                    input,
                                    &mut assembly,
                                    &mut reader,
                                    &mut mux,
                                    &mut counts,
                                    core,
                                    &output,
                                )?;
                                continue;
                            }
                            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
                        }
                        if assembly.partial.is_some() {
                            return Err(Fault::PartialAppend);
                        }
                        let reader_before = reader.sample_state();
                        let mux_before = mux.sample_state();
                        let reader_end = reader.finish().map_err(|error| {
                            codec_fault(
                                &mut counts,
                                CodecStage::ReaderFinish,
                                error,
                                Some(reader_before),
                                Some(mux_before),
                            )
                        })?;
                        let (tail, end) = mux.finish(reader_end).map_err(|error| {
                            codec_mux_fault(
                                &mut counts,
                                CodecStage::MuxFinish,
                                error,
                                Some(reader_before),
                                Some(mux_before),
                                mux.rejection(),
                            )
                        })?;
                        // Last TS segment uses the same mailbox/credit checks.
                        // Never mint end proof before every output was accepted.
                        publish(tail, &mut counts, &output)?;
                        core.state
                            .compare_exchange(
                                FINISHING,
                                INPUT_ENDED,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .map_err(Fault::from_state)?;
                        return Ok(end);
                    }
                    if matches!(receive_error, RecvTimeoutError::Disconnected) {
                        return Err(Fault::InputClosed);
                    }
                }
            }
        }
    })();
    let input_end = match outcome {
        Ok(end) => end,
        Err(fault) => InputEnd::Interrupted(core.interrupt(fault)),
    };
    // Release queued input credits before the worker's joined report. A producer
    // already admitted before cancellation can still be finishing its bounded
    // copy; receiver Drop rejects its send and its owning credit then drops.
    drop(receiver);
    drop(output);
    report(core, input_end, counts)
}

#[cfg(test)]
mod tests {
    use super::super::browser::encoded::fmp4::{fixtures, relay::SourceOrigin};
    use super::*;

    fn binding() -> RelayBinding {
        RelayBinding::new(NativeIdentity {
            provider: RelayProvider::Soop,
            owner: Uuid::new_v4(),
            run: Uuid::new_v4(),
            source: Uuid::new_v4(),
            document_epoch: 1,
        })
        .unwrap()
    }
    fn send(input: &RelayInput, binding: &RelayBinding, track: u32, append: u64) {
        let scale = if track == 0 {
            fixtures::VIDEO_SCALE
        } else {
            fixtures::AUDIO_SCALE
        };
        input
            .try_enqueue(
                binding,
                track,
                append,
                0,
                true,
                &fixtures::fragment(track, u64::from(append) * u64::from(scale), 2, true),
            )
            .unwrap();
    }
    fn interrupted(report: WorkerReport, expected: Fault) {
        assert!(
            matches!(report.input_end, InputEnd::Interrupted(f) if f == expected),
            "{report:?}"
        );
    }
    fn fake_chunk(bytes: usize) -> ContinuousChunk {
        // Mailbox pressure fixture only, never parsed or reported as media.
        ContinuousChunk {
            bytes: vec![0; bytes],
            includes_init: false,
            origin: SourceOrigin {
                ticks: 0,
                timescale: 1,
                video_base: 0,
                audio_base: 0,
                video_remainder: 0,
                audio_remainder: 0,
            },
            video: None,
            audio: None,
        }
    }

    #[test]
    fn native_binding_is_opaque_even_with_identical_identity_and_single_use() {
        let b = binding();
        let other = RelayBinding::new(b.identity()).unwrap();
        let (core, input, rx, out, _consumer) = channels(b.clone());
        assert_eq!(
            input.try_enqueue(&other, 0, 0, 0, true, b"x"),
            Err(Fault::Binding)
        );
        interrupted(worker(&core, fixtures::inputs(), rx, out), Fault::Binding);
        let (mut run, _input, _out) = RelayRun::start(b.clone(), fixtures::inputs()).unwrap();
        assert!(matches!(
            RelayRun::start(b, fixtures::inputs()),
            Err(Fault::Binding)
        ));
        run.cancel();
        assert!(matches!(
            run.join(Duration::from_secs(2)),
            JoinResult::Joined(_)
        ));
    }

    #[test]
    fn enqueue_bounds_precede_copy_and_codec_validation_runs_in_worker() {
        for (track, index, chunk, bytes) in [
            (2, 0, 0, vec![0]),
            (0, 0, APPEND_BYTES as u32, vec![0]),
            (0, 0, 0, vec![]),
            (0, 0, 0, vec![0; CHUNK_BYTES + 1]),
        ] {
            let b = binding();
            let (core, input, _rx, _out, _consumer) = channels(b.clone());
            assert_eq!(
                input.try_enqueue(&b, track, index, chunk, true, &bytes),
                Err(Fault::Bounds)
            );
            assert_eq!(core.input.used.load(Ordering::Acquire), 0);
        }
        let b = binding();
        let (core, _input, rx, out, _consumer) = channels(b);
        let mut tracks = fixtures::inputs();
        tracks[0].init = vec![0; 8];
        interrupted(worker(&core, tracks, rx, out), Fault::Codec);
        assert!(!core.codec_admitted.load(Ordering::Acquire));
    }

    #[test]
    fn input_credit_includes_popped_in_flight_chunk_and_full_queue_can_cancel() {
        let b = binding();
        let (core, input, rx, out, _consumer) = channels(b.clone());
        let bytes = vec![0; CHUNK_BYTES];
        for n in 0..8 {
            input.try_enqueue(&b, 0, 0, n, false, &bytes).unwrap();
        }
        let held = rx.try_recv().unwrap();
        assert_eq!(core.input.used.load(Ordering::Acquire), INPUT_BYTES);
        assert_eq!(
            input.try_enqueue(&b, 0, 0, 8, true, b"x"),
            Err(Fault::InputFull)
        );
        input.cancel();
        input.finish();
        assert_eq!(core.fault(), Some(Fault::InputFull));
        drop(held);
        let result = worker(&core, fixtures::inputs(), rx, out);
        interrupted(result, Fault::InputFull);
        assert_eq!(result.input_credit_bytes, 0);
        assert_eq!(result.counts.output_chunks, 0);
    }

    #[test]
    fn finish_is_out_of_band_and_drains_exactly_full_input_mailbox() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        for n in 0..4 {
            send(&input, &b, 0, n);
            send(&input, &b, 1, n);
        }
        input.finish();
        assert_eq!(
            input.try_enqueue(&b, 0, 4, 0, true, b"x"),
            Err(Fault::Closed)
        );
        let result = worker(&core, fixtures::inputs(), rx, out);
        let InputEnd::Ended(end) = result.input_end else {
            panic!("{result:?}")
        };
        assert_eq!(end.video_end, 4 * u64::from(fixtures::VIDEO_SCALE));
        assert_eq!(end.audio_end, 4 * u64::from(fixtures::AUDIO_SCALE));
        assert_eq!(result.counts.validated_appends, 8);
        assert_eq!(result.counts.consumed_chunks, 8);
        assert_eq!(result.input_credit_bytes, 0);
        assert!(result.output_held_bytes > 0); // Input end is not sink drain.
        input.cancel();
        assert_eq!(core.state.load(Ordering::Acquire), INPUT_ENDED);
        let mut count = 0;
        while let Ok(value) = consumer.try_recv() {
            assert!(value.binding().same(&b));
            assert!(!value.chunk().unwrap().bytes.is_empty());
            count += 1;
        }
        assert_eq!(count, result.counts.output_chunks);
        assert_eq!(consumer.held_bytes(), 0);
    }

    #[test]
    fn split_append_exact_duplicate_and_empty_batches_preserve_reader_order() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        // A permitted free atom emits an empty sample batch. Repeated init is
        // intentionally unsupported by the unchanged production parser.
        input
            .try_enqueue(&b, 0, 0, 0, true, b"\0\0\0\x08free")
            .unwrap();
        let bytes = fixtures::fragment(0, 0, 2, true);
        input.try_enqueue(&b, 0, 1, 0, false, &bytes[..17]).unwrap();
        input.try_enqueue(&b, 0, 1, 0, false, &bytes[..17]).unwrap();
        input.try_enqueue(&b, 0, 1, 1, true, &bytes[17..]).unwrap();
        input.try_enqueue(&b, 0, 1, 1, true, &bytes[17..]).unwrap();
        send(&input, &b, 1, 0);
        input.finish();
        let result = worker(&core, fixtures::inputs(), rx, out);
        assert!(matches!(result.input_end, InputEnd::Ended(_)), "{result:?}");
        assert_eq!(result.counts.duplicate_chunks, 2);
        assert_eq!(result.counts.validated_appends, 3);
        assert_eq!(result.counts.output_chunks, 1);
        assert!(consumer.try_recv().unwrap().chunk().unwrap().includes_init);
    }

    #[test]
    fn changed_duplicate_gap_and_interleaved_partial_interrupt() {
        for scenario in 0..4 {
            let b = binding();
            let (core, input, rx, out, _consumer) = channels(b.clone());
            input.try_enqueue(&b, 0, 0, 0, false, b"prefix").unwrap();
            match scenario {
                0 => input.try_enqueue(&b, 0, 0, 0, false, b"changed").unwrap(),
                1 => input.try_enqueue(&b, 0, 0, 2, true, b"gap").unwrap(),
                2 => input.try_enqueue(&b, 1, 0, 0, true, b"other").unwrap(),
                _ => input.try_enqueue(&b, 0, 1, 0, true, b"next").unwrap(),
            }
            input.finish();
            let result = worker(&core, fixtures::inputs(), rx, out);
            interrupted(result, Fault::Order);
            assert_eq!(result.counts.output_chunks, 0);
        }
    }

    #[test]
    fn incomplete_append_finish_is_interrupted_without_publication() {
        let b = binding();
        let (core, input, rx, out, _consumer) = channels(b.clone());
        input.try_enqueue(&b, 0, 0, 0, false, b"partial").unwrap();
        input.finish();
        let result = worker(&core, fixtures::inputs(), rx, out);
        interrupted(result, Fault::PartialAppend);
        assert_eq!(result.counts.output_chunks, 0);
    }

    #[test]
    fn failed_whole_append_emits_nothing_and_keeps_earlier_good_output() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        send(&input, &b, 0, 0);
        send(&input, &b, 1, 0);
        let mut malformed = fixtures::fragment(0, u64::from(fixtures::VIDEO_SCALE), 2, true);
        malformed.extend_from_slice(b"\0\0\0\x08zzzz");
        input.try_enqueue(&b, 0, 1, 0, true, &malformed).unwrap();
        input.finish();
        let result = worker(&core, fixtures::inputs(), rx, out);
        interrupted(result, Fault::Codec);
        let diagnostic = result.counts.codec_diagnostic.unwrap();
        assert_eq!(diagnostic.stage, CodecStage::ReaderPush);
        assert_eq!(diagnostic.error_code, "ENCODED_UNSUPPORTED");
        assert_eq!(result.counts.validated_appends, 2);
        assert_eq!(result.counts.output_chunks, 1);
        assert!(consumer.try_recv().is_ok());
        assert!(matches!(
            consumer.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn output_credit_survives_pop_and_receiver_drop_until_owner_drops() {
        let (core, _input, _rx, out, consumer) = channels(binding());
        out.try_accept(fake_chunk(OUTPUT_BYTES)).unwrap();
        let held = consumer.try_recv().unwrap();
        assert_eq!(consumer.held_bytes(), OUTPUT_BYTES);
        assert_eq!(out.try_accept(fake_chunk(1)), Err(Fault::OutputFull));
        drop(consumer);
        assert_eq!(core.output.used.load(Ordering::Acquire), OUTPUT_BYTES);
        drop(held);
        assert_eq!(core.output.used.load(Ordering::Acquire), 0);
        assert_eq!(out.try_accept(fake_chunk(1)), Err(Fault::OutputClosed));
        assert_eq!(core.output.used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn full_output_mailbox_interrupts_worker_and_releases_rejected_credit() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        for _ in 0..QUEUE_SLOTS {
            out.try_accept(fake_chunk(1)).unwrap();
        }
        send(&input, &b, 0, 0);
        send(&input, &b, 1, 0);
        input.finish();
        let result = worker(&core, fixtures::inputs(), rx, out);
        interrupted(result, Fault::OutputFull);
        assert_eq!(result.counts.output_chunks, 0);
        assert_eq!(consumer.held_bytes(), QUEUE_SLOTS);
        drop(consumer);
        assert_eq!(core.output.used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn terminal_race_keeps_first_fault_and_finish_cannot_override_it() {
        for _ in 0..16 {
            let (core, input, _rx, _out, _consumer) = channels(binding());
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let b = barrier.clone();
            let i = input.clone();
            let a = thread::spawn(move || {
                b.wait();
                i.cancel();
            });
            let b = barrier.clone();
            let c = core.clone();
            let other = thread::spawn(move || {
                b.wait();
                c.interrupt(Fault::Order);
            });
            barrier.wait();
            input.finish();
            a.join().unwrap();
            other.join().unwrap();
            let first = core.fault().unwrap();
            assert!(matches!(first, Fault::Cancelled | Fault::Order));
            input.finish();
            core.interrupt(Fault::Codec);
            input.cancel();
            assert_eq!(core.fault(), Some(first));
        }
    }

    #[test]
    fn pending_join_retains_handle_and_joined_is_not_an_output_drain_claim() {
        let (core, _input, _rx, _out, _consumer) = channels(binding());
        let (release, wait) = mpsc::channel();
        let c = core.clone();
        // Test-only held thread; no production callback or blocking sink exists.
        let handle = thread::spawn(move || {
            let _ = wait.recv();
            report(
                &c,
                InputEnd::Interrupted(Fault::Cancelled),
                WorkerCounts::default(),
            )
        });
        let mut run = RelayRun {
            core,
            handle: Some(handle),
            joined: None,
        };
        assert!(matches!(run.join(Duration::ZERO), JoinResult::Pending));
        assert!(run.handle.is_some());
        assert!(run.joined.is_none());
        run.cancel();
        release.send(()).unwrap();
        assert!(matches!(
            run.join(Duration::from_secs(2)),
            JoinResult::Joined(_)
        ));
        assert!(run.handle.is_none());
        assert!(run.joined.is_some());
    }

    #[test]
    fn actual_worker_thread_admits_and_joins_without_any_sink_or_account_io() {
        let b = binding();
        let (mut run, input, consumer) = RelayRun::start(b.clone(), fixtures::inputs()).unwrap();
        send(&input, &b, 0, 0);
        send(&input, &b, 1, 0);
        input.finish();
        let JoinResult::Joined(result) = run.join(Duration::from_secs(2)) else {
            panic!("worker not joined")
        };
        assert!(input.codec_admitted());
        assert!(matches!(result.input_end, InputEnd::Ended(_)), "{result:?}");
        assert_eq!(result.counts.output_chunks, 1);
        assert!(consumer.held_bytes() > 0);
        let value = consumer.try_recv().unwrap();
        let bytes = value.chunk().unwrap().bytes.len();
        assert_eq!(consumer.held_bytes(), bytes);
        drop(value);
        assert_eq!(consumer.held_bytes(), 0);
    }

    #[test]
    fn cancelled_full_mailbox_needs_no_control_slot_and_first_cause_sticks() {
        let b = binding();
        let (core, input, rx, out, _consumer) = channels(b.clone());
        for n in 0..8 {
            input.try_enqueue(&b, 0, 0, n, false, b"x").unwrap();
        }
        input.cancel();
        input.finish();
        assert_eq!(
            input.try_enqueue(&b, 0, 0, 8, true, b"x"),
            Err(Fault::Closed)
        );
        let result = worker(&core, fixtures::inputs(), rx, out);
        interrupted(result, Fault::Cancelled);
        assert_eq!(result.input_credit_bytes, 0);
        assert_eq!(result.counts.consumed_chunks, 0);
    }

    #[test]
    fn finish_waits_for_already_admitted_enqueue_before_final_mailbox_check() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        send(&input, &b, 0, 0);
        // Model a producer already past OPEN and copying its bounded payload.
        core.enqueue_ops.fetch_add(1, Ordering::SeqCst);
        let bytes = fixtures::fragment(1, 0, 2, true).into_boxed_slice();
        let credit = core.input.reserve(bytes.len()).unwrap();
        let audio = InputChunk {
            track: 1,
            append: 0,
            chunk: 0,
            last: true,
            bytes,
            _credit: credit,
        };
        input.finish();
        let c = core.clone();
        let handle = thread::spawn(move || worker(&c, fixtures::inputs(), rx, out));
        thread::sleep(Duration::from_millis(25));
        assert!(!handle.is_finished());
        assert!(input.sender.try_send(audio).is_ok());
        core.enqueue_ops.fetch_sub(1, Ordering::SeqCst);
        let result = handle.join().unwrap();
        assert!(matches!(result.input_end, InputEnd::Ended(_)), "{result:?}");
        assert_eq!(result.counts.validated_appends, 2);
        assert_eq!(result.counts.output_chunks, 1);
        assert!(consumer.try_recv().is_ok());
    }

    #[test]
    fn append_ordinal_has_no_runtime_limit_and_overflow_fails_without_wrapping() {
        let mut assembly = Assembly::new(&fixtures::inputs());
        let budget = Budget::new(2);
        let mut counts = WorkerCounts::default();
        for index in [1_000_000, u64::MAX] {
            assembly.next_append[0] = index;
            assembly.partial = None;
            let input = InputChunk {
                track: 0,
                append: index,
                chunk: 0,
                last: true,
                bytes: vec![1].into_boxed_slice(),
                _credit: budget.reserve(1).unwrap(),
            };
            if index == u64::MAX {
                assert!(matches!(
                    assembly.push(&input, &mut counts),
                    Err(Fault::Bounds)
                ));
            } else {
                assert!(assembly.push(&input, &mut counts).unwrap().is_some());
            }
        }
        assert_eq!(assembly.next_append[0], u64::MAX);
    }

    #[test]
    fn receiver_drop_linearizes_before_end_but_does_not_rewrite_prior_end() {
        let (core, input, _rx, _out, consumer) = channels(binding());
        input.finish();
        drop(consumer);
        assert_eq!(
            core.state.compare_exchange(
                FINISHING,
                INPUT_ENDED,
                Ordering::AcqRel,
                Ordering::Acquire
            ),
            Err(Fault::OutputClosed as u8)
        );
        assert_eq!(core.fault(), Some(Fault::OutputClosed));
        let (core, input, _rx, _out, consumer) = channels(binding());
        input.finish();
        core.state
            .compare_exchange(FINISHING, INPUT_ENDED, Ordering::AcqRel, Ordering::Acquire)
            .unwrap();
        drop(consumer);
        assert_eq!(core.state.load(Ordering::Acquire), INPUT_ENDED);
    }

    // Structural AVCC/AAC fixture, not a codec decode claim. Keep the existing
    // parser fixture payload and give it30fps AVC/1024-sample AAC source clocks.
    fn ts_fragment(track: u32, append: u64) -> Vec<u8> {
        let (count, duration) = if track == 0 {
            (60u32, 3000u32)
        } else {
            (94, 1024)
        };
        let start = append * u64::from(count) * u64::from(duration);
        let mut bytes = fixtures::fragment(track, start, count, true);
        let trun = bytes.windows(4).position(|b| b == b"trun").unwrap();
        for n in 0..count as usize {
            let at = trun + 16 + n * 16;
            bytes[at..at + 4].copy_from_slice(&duration.to_be_bytes());
        }
        bytes
    }
    fn send_ts(input: &RelayInput, b: &RelayBinding, track: u32, append: u64) {
        input
            .try_enqueue(b, track, append, 0, true, &ts_fragment(track, append))
            .unwrap();
    }
    fn ts_segment_fixture() -> TsSegment {
        let mut reader = EncodedSampleReader::new(fixtures::inputs()).unwrap();
        let mut mux = TsMuxer::new(reader.descriptor()).unwrap();
        for track in 0..2 {
            assert!(mux
                .push(reader.push(track, &ts_fragment(track, 0)).unwrap())
                .unwrap()
                .is_empty());
        }
        let (mut segments, _) = mux.finish(reader.finish().unwrap()).unwrap();
        assert_eq!(segments.len(), 1);
        segments.remove(0)
    }

    #[test]
    fn ts_mode_joins_with_sealed_end_after_last_segment_keeps_dequeued_credit() {
        let b = binding();
        let (mut run, input, consumer) = RelayRun::start_ts(b.clone(), fixtures::inputs()).unwrap();
        for append in 0..2 {
            for track in 0..2 {
                send_ts(&input, &b, track, append);
            }
        }
        input.finish();
        let JoinResult::Joined(result) = run.join(Duration::from_secs(2)) else {
            panic!("TS worker did not join");
        };
        let InputEnd::TsEnded(end) = result.input_end else {
            panic!("{result:?}");
        };
        assert!(input.codec_admitted());
        assert!(result.counts.codec_diagnostic.is_none());
        assert_eq!(end.segments(), 2);
        assert_eq!(result.counts.output_chunks, 2);
        assert_eq!(result.counts.validated_appends, 4);
        assert_eq!(result.input_credit_bytes, 0);
        let mut held = Vec::new();
        let mut sum = 0;
        while let Ok(guard) = consumer.try_recv() {
            assert!(guard.binding().same(&b));
            assert!(guard.chunk().is_none());
            let segment = guard.ts_segment().unwrap();
            assert_eq!(segment.stream_id(), end.stream_id());
            assert_eq!(segment.ordinal(), held.len() as u64);
            assert_eq!(segment.bytes().len() % 188, 0);
            assert_eq!(segment.bytes()[0], 0x47);
            sum += segment.bytes().len();
            held.push(guard);
        }
        assert_eq!(held.len(), 2);
        assert_eq!(consumer.held_bytes(), sum);
        assert_eq!(result.output_held_bytes, sum);
        assert_eq!(result.counts.output_bytes, sum as u64);
        drop(consumer);
        assert_eq!(run.core.output.used.load(Ordering::Acquire), sum);
        let first = held.remove(0);
        let first_bytes = first.ts_segment().unwrap().bytes().len();
        drop(first);
        assert_eq!(
            run.core.output.used.load(Ordering::Acquire),
            sum - first_bytes
        );
        drop(held);
        assert_eq!(run.core.output.used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn ts_mode_binding_is_opaque_and_cannot_restart_as_mp4() {
        let b = binding();
        let other = RelayBinding::new(b.identity()).unwrap();
        let (mut run, input, _output) = RelayRun::start_ts(b.clone(), fixtures::inputs()).unwrap();
        assert_eq!(
            input.try_enqueue(&other, 0, 0, 0, true, &ts_fragment(0, 0)),
            Err(Fault::Binding)
        );
        input.finish();
        let JoinResult::Joined(result) = run.join(Duration::from_secs(2)) else {
            panic!("join");
        };
        interrupted(result, Fault::Binding);
        assert_eq!(result.counts.output_chunks, 0);
        assert!(matches!(
            RelayRun::start(b, fixtures::inputs()),
            Err(Fault::Binding)
        ));
    }

    #[test]
    fn ts_failed_append_keeps_prior_segment_but_never_mints_clean_end() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        for append in 0..2 {
            for track in 0..2 {
                send_ts(&input, &b, track, append);
            }
        }
        let mut bad = ts_fragment(0, 2);
        bad.extend_from_slice(b"\0\0\0\x08zzzz");
        input.try_enqueue(&b, 0, 2, 0, true, &bad).unwrap();
        input.finish();
        let result = worker_mode(&core, fixtures::inputs(), rx, out, OutputMode::Ts);
        interrupted(result, Fault::Codec);
        assert_eq!(result.counts.validated_appends, 4);
        assert_eq!(result.counts.output_chunks, 1);
        let accepted = consumer.try_recv().unwrap();
        assert_eq!(accepted.ts_segment().unwrap().ordinal(), 0);
        assert!(matches!(
            consumer.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn codec_diagnostic_reader_finish_reports_partial_container_without_payload() {
        let b = binding();
        let (core, input, rx, out, _consumer) = channels(b.clone());
        input
            .try_enqueue(&b, 0, 0, 0, true, b"\0\0\0\x20free")
            .unwrap();
        input.finish();
        let result = worker(&core, fixtures::inputs(), rx, out);
        interrupted(result, Fault::Codec);
        assert_eq!(result.counts.validated_appends, 1);
        let d = result.counts.codec_diagnostic.unwrap();
        assert_eq!(d.stage, CodecStage::ReaderFinish);
        assert_eq!(d.reader_before_call.unwrap().container_pending_bytes, 8);
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("free"));
        assert!(!json.contains("message"));
    }
    #[test]
    fn codec_diagnostic_mux_finish_retains_exact_pre_failure_sample_ends() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        send_ts(&input, &b, 0, 0);
        send_ts(&input, &b, 1, 0);
        send_ts(&input, &b, 0, 1);
        let mut short_audio = fixtures::fragment(1, 94 * 1024, 90, true);
        let trun = short_audio.windows(4).position(|s| s == b"trun").unwrap();
        for n in 0..90 {
            let at = trun + 16 + n * 16;
            short_audio[at..at + 4].copy_from_slice(&1024u32.to_be_bytes());
        }
        input.try_enqueue(&b, 1, 1, 0, true, &short_audio).unwrap();
        input.finish();
        let result = worker_mode(&core, fixtures::inputs(), rx, out, OutputMode::Ts);
        interrupted(result, Fault::Codec);
        let d = result.counts.codec_diagnostic.unwrap();
        assert_eq!(d.stage, CodecStage::MuxFinish);
        assert_eq!(d.error_code, "ENCODED_UNSUPPORTED");
        let s = d.mux_before_call.unwrap();
        assert_eq!(s.video_end, Some(360000));
        assert_eq!(s.audio_end, Some(184 * 1024));
        assert_eq!(s.video_pending_samples, 60);
        assert_eq!(s.audio_pending_samples, 90);
        assert_eq!(d.reader_before_call.unwrap().audio_end, s.audio_end);
        assert_eq!(result.counts.validated_appends, 4);
        assert_eq!(result.counts.output_chunks, 1);
        assert!(consumer.try_recv().unwrap().ts_segment().is_some());
    }
    #[test]
    fn codec_diagnostic_is_closed_and_first_failure_is_not_replaced() {
        let mut counts = WorkerCounts::default();
        assert_eq!(
            codec_fault(
                &mut counts,
                CodecStage::MuxPush,
                super::super::model::StreamError::new(
                    "ENCODED_TS_CLOCK_UNSUPPORTED",
                    "secret https://example.invalid/token",
                    false
                ),
                None,
                None
            ),
            Fault::Codec
        );
        codec_fault(
            &mut counts,
            CodecStage::ReaderFinish,
            super::super::model::StreamError::new("arbitrary-secret-code", "other", false),
            None,
            None,
        );
        let d = counts.codec_diagnostic.unwrap();
        assert_eq!(d.stage, CodecStage::MuxPush);
        assert_eq!(d.error_code, "ENCODED_TS_CLOCK_UNSUPPORTED");
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("https"));
        let mut counts = WorkerCounts::default();
        codec_fault(
            &mut counts,
            CodecStage::ReaderInit,
            super::super::model::StreamError::new("arbitrary-secret-code", "other", false),
            None,
            None,
        );
        assert_eq!(counts.codec_diagnostic.unwrap().error_code, "ENCODED_OTHER");
    }

    #[test]
    fn ts_final_segment_output_full_cannot_report_end_or_leak_rejected_credit() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        // Eight real sealed outputs fill the common mailbox. The final TS
        // segment from this worker must not be discarded and called ended.
        let mut held_bytes = 0;
        for _ in 0..QUEUE_SLOTS {
            let segment = ts_segment_fixture();
            held_bytes += segment.bytes().len();
            out.try_accept(OutputPayload::Ts(segment)).unwrap();
        }
        send_ts(&input, &b, 0, 0);
        send_ts(&input, &b, 1, 0);
        input.finish();
        let result = worker_mode(&core, fixtures::inputs(), rx, out, OutputMode::Ts);
        interrupted(result, Fault::OutputFull);
        assert_eq!(result.counts.validated_appends, 2);
        assert_eq!(result.counts.output_chunks, 0);
        assert_eq!(consumer.held_bytes(), held_bytes);
        drop(consumer);
        assert_eq!(core.output.used.load(Ordering::Acquire), 0);
    }

    #[test]
    fn ts_credit_budget_remains_held_through_borrow_and_cancel() {
        let (core, input, _rx, out, consumer) = channels(binding());
        let segment = ts_segment_fixture();
        let bytes = segment.bytes().len();
        out.try_accept(OutputPayload::Ts(segment)).unwrap();
        let held = consumer.try_recv().unwrap();
        let reservation = core.output.reserve(OUTPUT_BYTES - bytes).unwrap();
        assert_eq!(
            out.try_accept(OutputPayload::Ts(ts_segment_fixture())),
            Err(Fault::OutputFull)
        );
        input.cancel();
        assert_eq!(held.ts_segment().unwrap().bytes().len(), bytes);
        assert_eq!(core.output.used.load(Ordering::Acquire), OUTPUT_BYTES);
        drop(reservation);
        drop(held);
        assert_eq!(core.output.used.load(Ordering::Acquire), 0);
        assert_eq!(
            out.try_accept(OutputPayload::Ts(ts_segment_fixture())),
            Err(Fault::Cancelled)
        );
    }

    #[test]
    fn ts_cancel_and_partial_input_end_never_return_sealed_end() {
        for cancelled in [false, true] {
            let b = binding();
            let (core, input, rx, out, consumer) = channels(b.clone());
            input
                .try_enqueue(&b, 0, 0, 0, false, &ts_fragment(0, 0)[..17])
                .unwrap();
            if cancelled {
                input.cancel();
            }
            input.finish();
            let result = worker_mode(&core, fixtures::inputs(), rx, out, OutputMode::Ts);
            interrupted(
                result,
                if cancelled {
                    Fault::Cancelled
                } else {
                    Fault::PartialAppend
                },
            );
            assert_eq!(result.counts.output_chunks, 0);
            assert_eq!(result.input_credit_bytes, 0);
            assert_eq!(consumer.held_bytes(), 0);
        }
    }

    #[test]
    fn ts_panic_drops_input_but_preserves_accepted_output_ownership() {
        let b = binding();
        let (core, input, rx, out, consumer) = channels(b.clone());
        send_ts(&input, &b, 0, 0);
        let result = guard_worker(&core, move || {
            let _input_held = rx.try_recv().unwrap();
            let _receiver_owned = rx;
            out.try_accept(OutputPayload::Ts(ts_segment_fixture()))
                .unwrap();
            panic!("test worker failure after accepted output");
        });
        interrupted(result, Fault::WorkerPanic);
        assert_eq!(result.input_credit_bytes, 0);
        assert!(result.output_held_bytes > 0);
        input.finish();
        assert_eq!(core.fault(), Some(Fault::WorkerPanic));
        let held = consumer.try_recv().unwrap();
        assert!(held.ts_segment().is_some());
        drop(consumer);
        assert!(core.output.used.load(Ordering::Acquire) > 0);
        drop(held);
        assert_eq!(core.output.used.load(Ordering::Acquire), 0);
    }
}
