//! Isolated, pull-only raw-container evidence. Never enables production recording.
//! Contract: docs/requirements/youtube-webm-raw-pair-qa-20261015.json.
//! No tool invocation, account/chat access or ordinary application IPC.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, AtomicI32, Ordering}, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{Webview, WebviewBuilder, WebviewUrl};

const VIDEO_ID: &str = "T4XZmMPQ9Kw";
const MAX_TRACK: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 32 * 1024 * 1024;
const MAX_CHUNK: usize = 128 * 1024;
const MAX_METADATA: usize = 16 * 1024;
const MAX_CHUNK_REPLY: usize = 176 * 1024;
const STARTUP_MS: u32 = 15000;
const STABLE_LOAD_MS: u32 = 2000;
static RESULT: AtomicI32 = AtomicI32::new(2);

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Boundary {
    EvaluateBeforeUrlUnavailable, EvaluateBeforeUrlRejected,
    EvaluateAfterUrlUnavailable, EvaluateAfterUrlRejected,
    PinnedDocumentChanged, PinnedSourceChanged,
    SelectedBeforeUnitChanged, SelectedAfterUnitsChanged, TerminalMetadataChanged,
}
#[derive(Clone, Copy, Debug)]
enum Failure {
    InvalidMetadata, InvalidMetadataAt(Boundary), InvalidChunk, StorageRejected, SourceChanged(Boundary),
    Deadline, StartupTimeout, NotReady, CaptureFailed(Reason, CaptureDiagnostics), EvalFailed,
}
impl Failure {
    fn label(self) -> &'static str {
        match self {
            Self::InvalidMetadata | Self::InvalidMetadataAt(_) => "invalid_metadata", Self::InvalidChunk => "invalid_chunk",
            Self::StorageRejected => "storage_rejected", Self::SourceChanged(_) => "source_changed",
            Self::Deadline => "deadline", Self::StartupTimeout => "startup_timeout", Self::NotReady => "not_ready",
            Self::CaptureFailed(_, _) => "capture_failed", Self::EvalFailed => "eval_failed",
        }
    }
    fn capture_reason(self) -> Option<Reason> { if let Self::CaptureFailed(reason, _) = self { Some(reason) } else { None } }
    fn diagnostics(self) -> Option<CaptureDiagnostics> { if let Self::CaptureFailed(_, value) = self { Some(value) } else { None } }
    fn boundary(self) -> Option<Boundary> {
        match self { Self::SourceChanged(boundary) | Self::InvalidMetadataAt(boundary) => Some(boundary), _ => None }
    }
}
type Result<T> = std::result::Result<T, Failure>;

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CapturePhase { #[default] Waiting, Polling, BeforeUnit, UnitTransfer, AfterUnits, Manifest }
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LoadPhase { #[default] Unobserved, Started, Finished, Limit }
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct LoadPoint {
    elapsed_ms: u32, capture_phase: CapturePhase, load_phase: LoadPhase,
    started: u8, finished: u8, event_limit_reached: bool,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadTrace {
    first_pin: Option<LoadPoint>, current: LoadPoint,
    #[serde(skip)]
    last_event_ms: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct StartupPermit { started: u8, finished: u8 }
impl LoadTrace {
    fn time(&mut self, elapsed: u32) { self.current.elapsed_ms=self.current.elapsed_ms.max(elapsed.min(70000)); }
    fn load(&mut self, started: bool, elapsed: u32) {
        self.time(elapsed);
        self.last_event_ms=self.current.elapsed_ms;
        if self.current.started + self.current.finished >= 16 {
            self.current.event_limit_reached=true; self.current.load_phase=LoadPhase::Limit; return;
        }
        if started { self.current.started+=1; self.current.load_phase=LoadPhase::Started; }
        else { self.current.finished+=1; self.current.load_phase=LoadPhase::Finished; }
    }
    fn phase(&mut self, phase: CapturePhase, elapsed: u32) { self.time(elapsed); self.current.capture_phase=phase; }
    fn pin(&mut self, elapsed: u32) { self.time(elapsed); if self.first_pin.is_none() { self.first_pin=Some(self.current); } }
    fn startup_permit(&mut self, elapsed: u32) -> Result<Option<StartupPermit>> {
        self.time(elapsed);
        if self.current.elapsed_ms>=STARTUP_MS { return Err(Failure::StartupTimeout); }
        Ok((self.current.load_phase==LoadPhase::Finished && !self.current.event_limit_reached
            && self.current.started>0 && self.current.finished>0
            && self.current.elapsed_ms.saturating_sub(self.last_event_ms)>=STABLE_LOAD_MS)
            .then_some(StartupPermit { started:self.current.started, finished:self.current.finished }))
    }
    fn accept_startup(&mut self, permit: StartupPermit, pin: &mut Pin, snapshot: &Snapshot, elapsed: u32) -> Result<bool> {
        if self.startup_permit(elapsed)?!=Some(permit) { return Ok(false); }
        pin.accept(snapshot)?;
        self.pin(elapsed);
        Ok(true)
    }
}
struct LoadDiagnostics { start: Instant, trace: Mutex<LoadTrace> }
impl LoadDiagnostics {
    fn new(start: Instant) -> Self { Self { start, trace: Mutex::new(LoadTrace::default()) } }
    fn update<T>(&self, f: impl FnOnce(&mut LoadTrace,u32)->T) -> T {
        let elapsed=self.start.elapsed().as_millis().min(70000) as u32;
        let mut trace=self.trace.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut trace,elapsed)
    }
    fn load(&self, started: bool) { self.update(|trace,elapsed| trace.load(started,elapsed)); }
    fn phase(&self, phase: CapturePhase) { self.update(|trace,elapsed| trace.phase(phase,elapsed)); }
    fn pin(&self) { self.update(LoadTrace::pin); }
    fn snapshot(&self) -> LoadTrace { self.update(|trace,elapsed| { trace.time(elapsed); trace.clone() }) }
}

fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where D: Deserializer<'de>, T: Deserialize<'de> { Option::<T>::deserialize(deserializer) }
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum State { Collecting, Ready, Stopped }
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Reason {
    Observing, Complete, Deadline, Limit, SourceChanged, ConfigurationChanged,
    Excluded, Malformed, Unsupported, NoCandidate, ObserverError, Closed,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Track { Video, Audio }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
enum Configuration {
    Vp9Webm {
        track_number: u32, timestamp_scale_ns: u64,
        #[serde(deserialize_with = "nullable")]
        default_duration_ns: Option<u64>,
        width: u32, height: u32, profile: u8, bit_depth: u8,
    },
    AacMp4 { track_id: u32, timescale: u32 },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Unit {
    unit: u8, track: Track, generation: u8, start_offset: u64, end_offset: u64,
    byte_length: u64, init_length: u64, start_seconds: f64, end_seconds: f64,
    configuration: Configuration,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct TrackBytes { video: u64, audio: u64 }
/// Fixed scalar observations only; these cannot relax the selection checks.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrackDiagnostics {
    units: u8,
    adjacent_pairs: u8,
    adjacent_triples: u8,
    init_differences: u8,
    configuration_differences: u8,
    byte_generation_breaks: u8,
    clock_order_breaks: u8,
    gaps: u8,
}
impl TrackDiagnostics {
    fn valid(self) -> bool {
        self.units <= 16 && self.adjacent_pairs <= 15 && self.adjacent_triples <= 14
            && self.init_differences <= 15 && self.configuration_differences <= 15
            && self.byte_generation_breaks <= 15 && self.clock_order_breaks <= 15 && self.gaps <= 15
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureDiagnostics { video: TrackDiagnostics, audio: TrackDiagnostics }
impl CaptureDiagnostics { fn valid(self) -> bool { self.video.valid() && self.audio.valid() } }
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    version: u8, capture_id: String, document_id: String, state: State, reason: Reason,
    #[serde(deserialize_with = "nullable")]
    source_ordinal: Option<u8>,
    elapsed_ms: u32, total_bytes: u64, track_bytes: TrackBytes,
    excluded_partial_units: u8, requires_audio_join: bool, units: Vec<Unit>,
    diagnostics: CaptureDiagnostics,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chunk {
    version: u8, capture_id: String, document_id: String, unit: u8,
    generation: u8, offset: u64, byte_length: u64,
    #[serde(rename = "final")]
    final_chunk: bool,
    base64: String,
}
fn canonical_uuid(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).ok().is_some_and(|id| id.to_string() == value)
}
fn finite(value: f64) -> bool { value.is_finite() && (0.0..=1_000_000_000.0).contains(&value) }
fn valid_configuration(unit: &Unit) -> bool {
    match (&unit.track, &unit.configuration) {
        (Track::Video, Configuration::Vp9Webm { track_number, timestamp_scale_ns, default_duration_ns, width, height, profile, bit_depth }) => {
            (1..=2_147_483_647).contains(track_number) && (1..=1_000_000_000).contains(timestamp_scale_ns)
                && default_duration_ns.is_none_or(|n| (1..=1_000_000_000).contains(&n))
                && (1..=16384).contains(width) && (1..=16384).contains(height)
                && *profile <= 3 && matches!(*bit_depth, 8 | 10 | 12)
        },
        (Track::Audio, Configuration::AacMp4 { track_id, timescale }) => {
            (1..=2_147_483_647).contains(track_id) && (1..=768000).contains(timescale)
        },
        _ => false,
    }
}
fn contiguous(units: &[Unit]) -> bool {
    units.windows(2).all(|pair| {
        let (previous, next) = (&pair[0], &pair[1]);
        previous.end_offset == next.start_offset
            && previous.generation.checked_add(1) == Some(next.generation)
            && previous.configuration == next.configuration
            && previous.start_seconds < next.start_seconds
            && previous.end_seconds < next.end_seconds
    })
}
fn parse_snapshot(raw: &str, capture_id: &str) -> Result<Snapshot> {
    if raw.len() > MAX_METADATA { return Err(Failure::InvalidMetadata); }
    let snapshot: Snapshot = serde_json::from_str(raw).map_err(|_| Failure::InvalidMetadata)?;
    if snapshot.version != 3 || snapshot.capture_id != capture_id || !canonical_uuid(capture_id)
        || !canonical_uuid(&snapshot.document_id) || snapshot.elapsed_ms > 15000
        || snapshot.source_ordinal.is_some_and(|n| !(1..=8).contains(&n))
        || snapshot.track_bytes.video > MAX_TRACK || snapshot.track_bytes.audio > MAX_TRACK
        || snapshot.total_bytes != snapshot.track_bytes.video + snapshot.track_bytes.audio
        || snapshot.total_bytes > MAX_TOTAL || snapshot.excluded_partial_units > 32 || !snapshot.diagnostics.valid() {
        return Err(Failure::InvalidMetadata);
    }
    match snapshot.state {
        State::Collecting if snapshot.reason == Reason::Observing && snapshot.units.is_empty() && !snapshot.requires_audio_join => return Ok(snapshot),
        State::Stopped if !matches!(snapshot.reason, Reason::Observing | Reason::Complete) && snapshot.units.is_empty() && !snapshot.requires_audio_join => return Ok(snapshot),
        State::Ready if snapshot.reason == Reason::Complete && snapshot.source_ordinal.is_some()
            && (4..=6).contains(&snapshot.units.len()) && snapshot.requires_audio_join => (),
        _ => return Err(Failure::InvalidMetadata),
    }
    let video_count = snapshot.units.iter().take_while(|unit| unit.track == Track::Video).count();
    let audio_count = snapshot.units.len() - video_count;
    if !(2..=3).contains(&video_count) || !(2..=3).contains(&audio_count)
        || (video_count == 2 && snapshot.elapsed_ms != 15000) {
        return Err(Failure::InvalidMetadata);
    }
    let mut video_bytes = 0u64;
    let mut audio_bytes = 0u64;
    for (index, unit) in snapshot.units.iter().enumerate() {
        let track_limit = if index < video_count { snapshot.track_bytes.video } else { snapshot.track_bytes.audio };
        if unit.unit as usize != index + 1 || unit.track != if index < video_count { Track::Video } else { Track::Audio }
            || !(1..=16).contains(&unit.generation) || unit.start_offset >= unit.end_offset
            || unit.end_offset > track_limit || unit.byte_length != unit.end_offset - unit.start_offset
            || unit.byte_length > MAX_TRACK || !(1..=65536).contains(&unit.init_length)
            || unit.init_length > unit.byte_length || !finite(unit.start_seconds) || !finite(unit.end_seconds)
            || unit.end_seconds <= unit.start_seconds || !valid_configuration(unit) { return Err(Failure::InvalidMetadata); }
        match unit.track { Track::Video => video_bytes += unit.byte_length, Track::Audio => audio_bytes += unit.byte_length }
    }
    if video_bytes > snapshot.track_bytes.video || audio_bytes > snapshot.track_bytes.audio { return Err(Failure::InvalidMetadata); }
    let (video, audio) = snapshot.units.split_at(video_count);
    let video_start = video[0].start_seconds;
    let video_end = video[video.len()-1].end_seconds;
    if !contiguous(video) || !contiguous(audio)
        || audio[0].start_seconds > video_start || audio[audio.len()-1].end_seconds < video_end
        || audio.iter().any(|unit| unit.end_seconds <= video_start || unit.start_seconds >= video_end)
        || audio.windows(2).any(|pair| pair[0].end_seconds < pair[1].start_seconds) {
        return Err(Failure::InvalidMetadata);
    }
    Ok(snapshot)
}

#[derive(Default)]
struct Pin { document: Option<String>, source: Option<u8>, latest: Option<Snapshot> }
impl Pin {
    fn accept(&mut self, snapshot: &Snapshot) -> Result<()> {
        if self.document.as_ref().is_some_and(|id| id != &snapshot.document_id) {
            return Err(Failure::SourceChanged(Boundary::PinnedDocumentChanged));
        }
        if self.source.is_some_and(|source| Some(source) != snapshot.source_ordinal) {
            return Err(Failure::SourceChanged(Boundary::PinnedSourceChanged));
        }
        if let Some(previous) = &self.latest {
            if previous.state != State::Collecting && previous != snapshot {
                return Err(Failure::InvalidMetadataAt(Boundary::TerminalMetadataChanged));
            }
            if snapshot.elapsed_ms < previous.elapsed_ms || snapshot.track_bytes.video < previous.track_bytes.video
                || snapshot.track_bytes.audio < previous.track_bytes.audio { return Err(Failure::InvalidMetadata); }
        }
        self.document.get_or_insert_with(|| snapshot.document_id.clone());
        if let Some(source) = snapshot.source_ordinal { self.source = Some(source); }
        self.latest = Some(snapshot.clone());
        Ok(())
    }
}
fn parse_chunk(raw: &str, snapshot: &Snapshot, unit: &Unit, offset: u64) -> Result<Vec<u8>> {
    if raw.len() > MAX_CHUNK_REPLY || offset >= unit.byte_length { return Err(Failure::InvalidChunk); }
    let chunk: Chunk = serde_json::from_str(raw).map_err(|_| Failure::InvalidChunk)?;
    let expected = (unit.byte_length - offset).min(MAX_CHUNK as u64);
    if chunk.version != 3 || chunk.capture_id != snapshot.capture_id || chunk.document_id != snapshot.document_id
        || chunk.unit != unit.unit || chunk.generation != unit.generation || chunk.offset != offset
        || chunk.byte_length != expected || chunk.final_chunk != (offset + expected == unit.byte_length)
        || chunk.base64.len() > MAX_CHUNK.div_ceil(3) * 4 { return Err(Failure::InvalidChunk); }
    let bytes = STANDARD.decode(&chunk.base64).map_err(|_| Failure::InvalidChunk)?;
    if bytes.len() as u64 != expected || STANDARD.encode(&bytes) != chunk.base64 { return Err(Failure::InvalidChunk); }
    Ok(bytes)
}

fn safe_ancestors(path: &Path) -> Result<()> {
    if !path.is_absolute() { return Err(Failure::StorageRejected); }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                if meta.file_type().is_symlink() { return Err(Failure::StorageRejected); }
                #[cfg(windows)] {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 { return Err(Failure::StorageRejected); }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(Failure::StorageRejected),
        }
    }
    Ok(())
}

fn inspect_handle(file: &File, directory: bool) -> Result<()> {
    let metadata = file.metadata().map_err(|_| Failure::StorageRejected)?;
    if metadata.is_dir() != directory || (!directory && !metadata.is_file()) {
        return Err(Failure::StorageRejected);
    }
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION}};
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| Failure::StorageRejected)?;
        if info.dwFileAttributes & 0x400 != 0 || (!directory && info.nNumberOfLinks != 1) {
            return Err(Failure::StorageRejected);
        }
    }
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if !directory && metadata.nlink() != 1 { return Err(Failure::StorageRejected); }
    }
    Ok(())
}

fn directory_guard(path: &Path) -> Result<File> {
    safe_ancestors(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        // Directory handle permits children but denies replacement/deletion of the directory.
        options.share_mode(3).custom_flags(0x02000000 | 0x00200000);
    }
    let file = options.open(path).map_err(|_| Failure::StorageRejected)?;
    inspect_handle(&file, true)?;
    Ok(file)
}

struct OwnedRoot {
    id: String,
    path: PathBuf,
    _guards: Vec<File>,
}
impl OwnedRoot {
    fn create() -> Result<Self> {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent()
            .ok_or(Failure::StorageRejected)?.to_owned();
        Self::under(&workspace)
    }
    fn under(workspace: &Path) -> Result<Self> {
        safe_ancestors(workspace)?;
        let mut guards = vec![directory_guard(workspace)?];
        let mut base = workspace.to_owned();
        for component in [".runtime", "youtube-webm-raw-pair-qa"] {
            base = base.join(component);
            match fs::create_dir(&base) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(_) => return Err(Failure::StorageRejected),
            }
            guards.push(directory_guard(&base)?);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let path = base.join(&id);
        fs::create_dir(&path).map_err(|_| Failure::StorageRejected)?;
        guards.push(directory_guard(&path)?);
        Ok(Self { id, path, _guards: guards })
    }
    fn file(&self, name: &'static str) -> Result<File> {
        if !matches!(name, "video-1.webm" | "video-2.webm" | "video-3.webm"
            | "audio-1.mp4" | "audio-2.mp4" | "audio-3.mp4" | "manifest.json") {
            return Err(Failure::StorageRejected);
        }
        safe_ancestors(&self.path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(windows)] {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let file = options.open(self.path.join(name)).map_err(|_| Failure::StorageRejected)?;
        inspect_handle(&file, false)?;
        Ok(file)
    }
}

fn hash_file(file: &mut File, length: u64) -> Result<String> {
    inspect_handle(file, false)?;
    if file.metadata().map_err(|_| Failure::StorageRejected)?.len() != length { return Err(Failure::StorageRejected); }
    file.seek(SeekFrom::Start(0)).map_err(|_| Failure::StorageRejected)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; MAX_CHUNK];
    let mut read = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| Failure::StorageRejected)?;
        if count == 0 { break; }
        read = read.checked_add(count as u64).ok_or(Failure::StorageRejected)?;
        if read > length { return Err(Failure::StorageRejected); }
        hash.update(&buffer[..count]);
    }
    if read != length { return Err(Failure::StorageRejected); }
    Ok(format!("{:x}", hash.finalize()))
}

fn approved(url: &tauri::Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("www.youtube.com") && url.path() == "/watch"
        && url.port().is_none() && url.username().is_empty() && url.password().is_none() && url.fragment().is_none()
        && url.query_pairs().filter(|(key, _)| key == "v").count() == 1
        && url.query_pairs().any(|(key, value)| key == "v" && value == VIDEO_ID)
}
fn evaluation_boundary(url: Option<&tauri::Url>, after: bool) -> Result<()> {
    match url {
        Some(url) if approved(url) => Ok(()),
        Some(_) => Err(Failure::SourceChanged(if after { Boundary::EvaluateAfterUrlRejected } else { Boundary::EvaluateBeforeUrlRejected })),
        None => Err(Failure::SourceChanged(if after { Boundary::EvaluateAfterUrlUnavailable } else { Boundary::EvaluateBeforeUrlUnavailable })),
    }
}

fn evaluate(view: &Webview, expression: &str, maximum: usize, deadline: Instant, done: &AtomicBool) -> Result<String> {
    if done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
    evaluation_boundary(view.url().ok().as_ref(), false)?;
    let script = format!("(()=>{{if(window!==window.top||location.protocol!=='https:'||location.hostname!=='www.youtube.com'||location.pathname!=='/watch'||new URLSearchParams(location.search).getAll('v').length!==1||new URLSearchParams(location.search).get('v')!=='{VIDEO_ID}')return null;return ({expression});}})()");
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(&script, move |raw| {
        let bounded = (raw.len() <= maximum).then_some(raw);
        let _ = tx.send(bounded);
    }).map_err(|_| Failure::EvalFailed)?;
    let timeout = deadline.saturating_duration_since(Instant::now()).min(Duration::from_secs(3));
    let raw = rx.recv_timeout(timeout).map_err(|_| Failure::EvalFailed)?.ok_or(Failure::InvalidMetadata)?;
    if done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
    evaluation_boundary(view.url().ok().as_ref(), true)?;
    Ok(raw)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileProof { unit: u8, filename: &'static str, byte_length: u64, sha256: String }
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
struct InitializationsIdentical { video: bool, audio: bool }
struct Completion { units: usize, video_units: usize, audio_units: usize, bytes: u64, requires_audio_join: bool,
    initializations_identical: InitializationsIdentical, manifest_sha256: String }
fn snapshot(view: &Webview, root: &OwnedRoot, pin: &mut Pin, deadline: Instant, done: &AtomicBool, diagnostics: &LoadDiagnostics) -> Result<Option<Snapshot>> {
    evaluation_boundary(view.url().ok().as_ref(), false)?;
    let startup=if pin.document.is_none() {
        match diagnostics.update(LoadTrace::startup_permit)? { Some(permit)=>Some(permit), None=>return Ok(None) }
    } else { None };
    let evaluation_deadline=if startup.is_some() { deadline.min(diagnostics.start+Duration::from_millis(STARTUP_MS as u64)) } else { deadline };
    let raw = match evaluate(view, "window.__atsumiYouTubeRawPairQa?.snapshot()??null", MAX_METADATA, evaluation_deadline, done) {
        Err(Failure::Deadline | Failure::EvalFailed) if startup.is_some() && diagnostics.start.elapsed()>=Duration::from_millis(STARTUP_MS as u64) => return Err(Failure::StartupTimeout),
        result=>result?,
    };
    if raw == "null" && pin.document.is_none() { return Ok(None); }
    let value = parse_snapshot(&raw, &root.id)?;
    if let Some(permit)=startup {
        if !diagnostics.update(|trace,elapsed| trace.accept_startup(permit,pin,&value,elapsed))? { return Ok(None); }
    } else {
        pin.accept(&value)?;
        diagnostics.pin();
    }
    Ok(Some(value))
}
fn write_and_verify(file: &mut File, bytes: &[u8]) -> Result<String> {
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|_| Failure::StorageRejected)?;
    let expected = format!("{:x}", Sha256::digest(bytes));
    let actual = hash_file(file, bytes.len() as u64)?;
    if actual != expected { return Err(Failure::StorageRejected); }
    Ok(actual)
}
fn unit_filename(track: Track, ordinal: usize) -> Result<&'static str> {
    match (track, ordinal) {
        (Track::Video, 0) => Ok("video-1.webm"), (Track::Video, 1) => Ok("video-2.webm"),
        (Track::Video, 2) => Ok("video-3.webm"), (Track::Audio, 0) => Ok("audio-1.mp4"),
        (Track::Audio, 1) => Ok("audio-2.mp4"), (Track::Audio, 2) => Ok("audio-3.mp4"),
        _ => Err(Failure::InvalidMetadata),
    }
}
fn init_prefix(file: &mut File, unit: &Unit) -> Result<Vec<u8>> {
    inspect_handle(file, false)?;
    if !(1..=65536).contains(&unit.init_length) || unit.init_length > unit.byte_length
        || file.metadata().map_err(|_| Failure::StorageRejected)?.len() != unit.byte_length {
        return Err(Failure::StorageRejected);
    }
    let cursor = file.stream_position().map_err(|_| Failure::StorageRejected)?;
    let result = (|| {
        file.seek(SeekFrom::Start(0)).map_err(|_| Failure::StorageRejected)?;
        let mut bytes = vec![0; unit.init_length as usize];
        file.read_exact(&mut bytes).map_err(|_| Failure::StorageRejected)?;
        Ok(bytes)
    })();
    // Restore even after a read error; never hand back a silently moved cursor.
    file.seek(SeekFrom::Start(cursor)).map_err(|_| Failure::StorageRejected)?;
    inspect_handle(file, false)?;
    if file.metadata().map_err(|_| Failure::StorageRejected)?.len() != unit.byte_length {
        return Err(Failure::StorageRejected);
    }
    result
}
fn verify_initializations(files: &mut [File], units: &[Unit], deadline: Instant, done: &AtomicBool) -> Result<InitializationsIdentical> {
    if files.len() != units.len() || !(4..=6).contains(&units.len()) { return Err(Failure::InvalidMetadata); }
    let mut identical=InitializationsIdentical { video:true, audio:true };
    for track in [Track::Video, Track::Audio] {
        let mut expected: Option<Vec<u8>> = None;
        let mut count = 0;
        for (file, unit) in files.iter_mut().zip(units).filter(|(_, unit)| unit.track == track) {
            if done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
            let prefix = init_prefix(file, unit)?;
            if expected.as_ref().is_some_and(|bytes| bytes != &prefix) {
                match track { Track::Video=>identical.video=false, Track::Audio=>identical.audio=false }
            }
            if expected.is_none() { expected = Some(prefix); }
            count += 1;
        }
        if !(2..=3).contains(&count) { return Err(Failure::InvalidMetadata); }
    }
    if done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
    // Preserve differing complete original initializations as raw evidence.
    // Read/check every prefix even after a difference; this is not join approval.
    Ok(identical)
}
fn capture(view: &Webview, root: &OwnedRoot, deadline: Instant, done: &AtomicBool, diagnostics: &LoadDiagnostics) -> Result<Completion> {
    diagnostics.phase(CapturePhase::Polling);
    let mut pin = Pin::default();
    let mut selected = None;
    for _ in 0..350 {
        if done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
        if pin.document.is_none() && diagnostics.start.elapsed()>=Duration::from_millis(STARTUP_MS as u64) { return Err(Failure::StartupTimeout); }
        // A brand-new view can briefly be about:blank. Never evaluate there, and
        // never allow this startup exception after pinning a real document.
        let url = view.url().map_err(|_| Failure::EvalFailed)?;
        if url.as_str() == "about:blank" && pin.document.is_none() {
            thread::sleep(Duration::from_millis(100)); continue;
        }
        if let Some(value) = snapshot(view, root, &mut pin, deadline, done, diagnostics)? {
            match value.state {
                State::Ready => { selected = Some(value); break; },
                State::Stopped => return Err(Failure::CaptureFailed(value.reason, value.diagnostics)),
                State::Collecting => (),
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    let selected = selected.ok_or(Failure::NotReady)?;
    let mut files = Vec::new();
    let mut held_files = Vec::new();
    let mut total_written = 0u64;
    let mut video_units = 0;
    let mut audio_units = 0;
    for unit in &selected.units {
        diagnostics.phase(CapturePhase::BeforeUnit);
        if snapshot(view, root, &mut pin, deadline, done, diagnostics)?.as_ref() != Some(&selected) {
            return Err(Failure::SourceChanged(Boundary::SelectedBeforeUnitChanged));
        }
        let ordinal = match unit.track { Track::Video => video_units, Track::Audio => audio_units };
        let filename = unit_filename(unit.track, ordinal)?;
        match unit.track { Track::Video => video_units += 1, Track::Audio => audio_units += 1 }
        let mut file = root.file(filename)?;
        let mut offset = 0u64;
        let mut expected_hash = Sha256::new();
        diagnostics.phase(CapturePhase::UnitTransfer);
        while offset < unit.byte_length {
            let expression = format!("window.__atsumiYouTubeRawPairQa?.readChunk({},{},{},{})??null",
                serde_json::to_string(&selected.capture_id).map_err(|_| Failure::InvalidMetadata)?,
                serde_json::to_string(&selected.document_id).map_err(|_| Failure::InvalidMetadata)?, unit.unit, offset);
            let raw = evaluate(view, &expression, MAX_CHUNK_REPLY, deadline, done)?;
            let bytes = parse_chunk(&raw, &selected, unit, offset)?;
            if done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
            file.write_all(&bytes).map_err(|_| Failure::StorageRejected)?;
            expected_hash.update(&bytes);
            offset = offset.checked_add(bytes.len() as u64).ok_or(Failure::InvalidChunk)?;
            total_written = total_written.checked_add(bytes.len() as u64).ok_or(Failure::InvalidChunk)?;
            if offset > unit.byte_length || total_written > MAX_TOTAL { return Err(Failure::InvalidChunk); }
        }
        file.sync_all().map_err(|_| Failure::StorageRejected)?;
        let sha256 = hash_file(&mut file, unit.byte_length)?;
        if sha256 != format!("{:x}", expected_hash.finalize()) { return Err(Failure::StorageRejected); }
        files.push(FileProof { unit: unit.unit, filename, byte_length: unit.byte_length, sha256 });
        // Keep write/delete sharing denied through manifest publication.
        held_files.push(file);
    }
    let initializations_identical=verify_initializations(&mut held_files, &selected.units, deadline, done)?;
    diagnostics.phase(CapturePhase::AfterUnits);
    if snapshot(view, root, &mut pin, deadline, done, diagnostics)?.as_ref() != Some(&selected) {
        return Err(Failure::SourceChanged(Boundary::SelectedAfterUnitsChanged));
    }
    let expected_total: u64 = selected.units.iter().map(|unit| unit.byte_length).sum();
    if total_written != expected_total { return Err(Failure::InvalidChunk); }
    diagnostics.phase(CapturePhase::Manifest);
    let manifest = json!({
        "version":3, "kind":"youtube_webm_raw_pair_qa", "captureCompleted":true,
        "packetVerified":false, "mediaVerified":false, "recordingVerified":false, "localMediaToolsInvoked":false,
        "initializationsIdentical":initializations_identical, "observations":selected, "files":files
    });
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|_| Failure::InvalidMetadata)?;
    if bytes.len() > 32 * 1024 || done.load(Ordering::Acquire) || Instant::now() >= deadline { return Err(Failure::Deadline); }
    let mut manifest_file = root.file("manifest.json")?;
    let manifest_sha256 = write_and_verify(&mut manifest_file, &bytes)?;
    Ok(Completion { units: files.len(), video_units, audio_units, bytes: total_written, requires_audio_join: selected.requires_audio_join,
        initializations_identical, manifest_sha256 })
}

fn summary(id: &str, result: &Result<Completion>) -> serde_json::Value {
    match result {
        Ok(value) => json!({"kind":"youtube_webm_raw_pair_qa", "captureCompleted":true,
            "outcome":if !value.initializations_identical.video || !value.initializations_identical.audio {"needs_initialization_review"} else {"needs_audio_unit_join"},
            "units":value.units, "bytes":value.bytes, "requiresAudioJoin":value.requires_audio_join,
            "videoUnits":value.video_units, "audioUnits":value.audio_units,
            "initializationsIdentical":value.initializations_identical,
            "manifestSha256":value.manifest_sha256,
            "directory":format!(".runtime/youtube-webm-raw-pair-qa/{id}"),
            "packetVerified":false, "mediaVerified":false, "recordingVerified":false, "localMediaToolsInvoked":false}),
        Err(reason) => {
            let mut value=json!({"kind":"youtube_webm_raw_pair_qa", "captureCompleted":false, "outcome":reason.label(),
            "captureReason":reason.capture_reason(), "boundary":reason.boundary(),
            "directory":format!(".runtime/youtube-webm-raw-pair-qa/{id}"),
            "mediaVerified":false, "recordingVerified":false, "localMediaToolsInvoked":false});
            if let Some(diagnostics)=reason.diagnostics() { value["diagnostics"]=json!(diagnostics); }
            value
        },
    }
}
fn summary_with_load(id: &str, result: &Result<Completion>, diagnostics: &LoadDiagnostics) -> serde_json::Value {
    let mut value=summary(id,result);
    value["pageLoad"]=json!(diagnostics.snapshot());
    value
}

fn run() -> Result<()> {
    let start=Instant::now();
    let deadline = start + Duration::from_secs(70);
    let diagnostics=Arc::new(LoadDiagnostics::new(start));
    let root = OwnedRoot::create()?;
    let profile = tempfile::Builder::new().prefix("atsumi-youtube-raw-pair-").tempdir().map_err(|_| Failure::StorageRejected)?;
    let profile_path = profile.path().join("webview-profile");
    let done = Arc::new(AtomicBool::new(false));
    let final_done = done.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = format!("local.atsumi.youtubewebmrawqa.{}", root.id.replace('-', ""));
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().setup(move |app| {
        let handle = app.handle().clone();
        let deadline_handle = handle.clone();
        let deadline_done = done.clone();
        let deadline_id = root.id.clone();
        let deadline_diagnostics=diagnostics.clone();
        thread::spawn(move || {
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
            if deadline_done.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                RESULT.store(3, Ordering::Release);
                println!("{}", summary_with_load(&deadline_id, &Err(Failure::Deadline), &deadline_diagnostics));
                deadline_handle.exit(3);
            }
        });
        let window = tauri::window::WindowBuilder::new(app, "youtube-webm-raw-pair")
            .title("Isolated YouTube raw-container QA").inner_size(1280.0, 720.0)
            .position(-16000.0, -16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
        let script = format!("window.__atsumiYouTubeRawPairQaCaptureId={};\n{}",
            serde_json::to_string(&root.id)?, include_str!("youtube_webm_raw_pair_qa.js"));
        let page_diagnostics=diagnostics.clone();
        let view = window.add_child(WebviewBuilder::new("youtube-webm-raw-watch", WebviewUrl::External(
            format!("https://www.youtube.com/watch?v={VIDEO_ID}&autoplay=1&mute=1").parse()?))
            .focused(false).data_directory(profile_path).initialization_script(script)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(approved).on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_page_load(move |_,payload| page_diagnostics.load(matches!(payload.event(),tauri::webview::PageLoadEvent::Started)))
            .on_download(|_, _| false),
            tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(1280.0, 720.0))?;
        thread::spawn(move || {
            let result = capture(&view, &root, deadline, &done, &diagnostics);
            if done.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                let code = if result.is_ok() { 0 } else { 2 };
                RESULT.store(code, Ordering::Release);
                println!("{}", summary_with_load(&root.id, &result, &diagnostics));
                handle.exit(code);
            }
        });
        Ok(())
    }).build(context).map_err(|_| Failure::EvalFailed)?;
    app.run_return(|_, _| {});
    final_done.store(true, Ordering::Release);
    drop(profile);
    Ok(())
}
fn main() {
    if !cfg!(debug_assertions) {
        println!("{}", json!({"kind":"youtube_webm_raw_pair_qa", "outcome":"debug_only", "captureCompleted":false}));
        std::process::exit(2);
    }
    if let Err(reason) = run() {
        let mut value=json!({"kind":"youtube_webm_raw_pair_qa", "outcome":reason.label(), "captureReason":reason.capture_reason(), "boundary":reason.boundary(), "captureCompleted":false,
            "mediaVerified":false, "recordingVerified":false, "localMediaToolsInvoked":false});
        if let Some(diagnostics)=reason.diagnostics() { value["diagnostics"]=json!(diagnostics); }
        println!("{value}");
        std::process::exit(2);
    }
    std::process::exit(RESULT.load(Ordering::Acquire));
}

#[cfg(test)]
mod tests {
    use super::*;
    const CAPTURE: &str = "132833d3-133a-4f97-99a3-7e0ba8c9f4bb";
    const DOCUMENT: &str = "87021c59-0dfc-4f0e-8c18-c49876dc5531";
    fn fixture_counts(video: usize, audio: usize) -> serde_json::Value {
        let mut units = Vec::new();
        for i in 0..video {
            units.push(json!({"unit":i+1,"track":"video","generation":4+i,"startOffset":i*8,"endOffset":(i+1)*8,"byteLength":8,"initLength":4,
                "startSeconds":10.0+i as f64*5.0,"endSeconds":15.0+i as f64*5.0,
                "configuration":{"kind":"vp9_webm","trackNumber":1,"timestampScaleNs":1000000,
                    "defaultDurationNs":33333333,"width":1280,"height":720,"profile":0,"bitDepth":8}}));
        }
        let step = (10.0 + video as f64*5.0 + 0.01 - 9.98) / audio as f64;
        for i in 0..audio {
            units.push(json!({"unit":video+i+1,"track":"audio","generation":7+i,"startOffset":i*8,"endOffset":(i+1)*8,"byteLength":8,"initLength":4,
                "startSeconds":9.98+i as f64*step,"endSeconds":9.98+(i+1) as f64*step,
                "configuration":{"kind":"aac_mp4","trackId":2,"timescale":44100}}));
        }
        json!({"version":3,"captureId":CAPTURE,"documentId":DOCUMENT,"state":"ready","reason":"complete",
            "sourceOrdinal":1,"elapsedMs":15000,"totalBytes":(video+audio)*8,"trackBytes":{"video":video*8,"audio":audio*8},
            "excludedPartialUnits":2,"requiresAudioJoin":true,"units":units,"diagnostics":CaptureDiagnostics::default()})
    }
    fn fixture() -> serde_json::Value { fixture_counts(2,2) }
    fn valid(value: &serde_json::Value) -> bool { parse_snapshot(&value.to_string(), CAPTURE).is_ok() }
    fn ready() -> Snapshot { parse_snapshot(&fixture().to_string(), CAPTURE).unwrap() }
    fn chunk(snapshot: &Snapshot, unit: &Unit, offset: u64, bytes: &[u8]) -> serde_json::Value {
        json!({"version":3,"captureId":snapshot.capture_id,"documentId":snapshot.document_id,"unit":unit.unit,
            "generation":unit.generation,"offset":offset,"byteLength":bytes.len(),
            "final":offset+bytes.len() as u64==unit.byte_length,"base64":STANDARD.encode(bytes)})
    }
    #[test]
    fn closed_metadata_accepts_independent_generation_not_same_generation_short_audio() {
        assert!(valid(&fixture()));
        let mut value=fixture(); value["units"][2]["generation"]=json!(4); value["units"][3]["generation"]=json!(5);
        value["units"][3]["endSeconds"]=json!(19.98); assert!(!valid(&value));
        value["units"][3]["endSeconds"]=json!(20.01); assert!(valid(&value));
        for key in ["url","path","cookie","filename","account"] {
            let mut value=fixture(); value[key]=json!("private"); assert!(!valid(&value));
            let mut value=fixture(); value["units"][0]["configuration"][key]=json!("private"); assert!(!valid(&value));
        }
    }
    #[test]
    fn metadata_bounds_required_nullable_and_state_rules_are_strict() {
        for (key,bad) in [("version",json!(1)),("version",json!(2)),("elapsedMs",json!(15001)),("sourceOrdinal",json!(9)),
            ("state",json!("private")),("reason",json!("observing")),("totalBytes",json!(33)),
            ("captureId",json!(DOCUMENT)),("documentId",json!("../private")),("excludedPartialUnits",json!(33))] {
            let mut value=fixture(); value[key]=bad; assert!(!valid(&value),"{key}");
        }
        for (key,bad) in [("byteLength",json!(9)),("generation",json!(17)),("endOffset",json!(MAX_TRACK+1)),
            ("startOffset",json!(8)),("initLength",json!(9)),("startSeconds",json!(-1)),("endSeconds",json!(10))] {
            let mut value=fixture(); value["units"][0][key]=bad; assert!(!valid(&value),"{key}");
        }
        let mut value=fixture(); value["units"][0]["configuration"].as_object_mut().unwrap().remove("defaultDurationNs"); assert!(!valid(&value));
        value["units"][0]["configuration"]["defaultDurationNs"]=serde_json::Value::Null;
        assert!(!valid(&value)); value["units"][1]["configuration"]["defaultDurationNs"]=serde_json::Value::Null; assert!(valid(&value));
        let mut value=fixture(); value.as_object_mut().unwrap().remove("sourceOrdinal"); assert!(!valid(&value));
        let mut value=fixture(); value["state"]=json!("collecting"); value["reason"]=json!("observing");
        value["units"]=json!([]); value["requiresAudioJoin"]=json!(false); value["sourceOrdinal"]=serde_json::Value::Null; assert!(valid(&value));
        value["state"]=json!("stopped"); assert!(!valid(&value)); value["reason"]=json!("no_candidate"); assert!(valid(&value));
        assert!(parse_snapshot(&" ".repeat(MAX_METADATA+1), CAPTURE).is_err());
    }
    #[test]
    fn two_and_three_unit_runs_preserve_deadline_and_independent_audio_coverage() {
        for (video,audio) in [(2,2),(2,3),(3,2),(3,3)] {
            let mut value=fixture_counts(video,audio); assert!(valid(&value));
            value["elapsedMs"]=json!(14999); assert_eq!(valid(&value),video==3);
            value["requiresAudioJoin"]=json!(false); assert!(!valid(&value));
        }
        for (video,audio) in [(1,2),(2,1),(4,2),(2,4)] { assert!(!valid(&fixture_counts(video,audio))); }
        let mut gap=fixture(); gap["units"][3]["startSeconds"]=json!(15.1); assert!(!valid(&gap));
        let mut overlap=fixture(); overlap["units"][3]["startSeconds"]=json!(14.9); assert!(valid(&overlap));
        overlap["units"][3]["endSeconds"]=json!(19.9); assert!(!valid(&overlap));
        let mut late=fixture(); late["units"][2]["startSeconds"]=json!(10.1); assert!(!valid(&late));
    }
    #[test]
    fn every_track_requires_adjacent_bytes_generations_config_and_ordered_clocks() {
        for first in [0,2] {
            for (key,value) in [("startOffset",json!(7)),("endOffset",json!(15)),("generation",json!(12)),
                ("startSeconds",json!(9.0)),("endSeconds",json!(14.0)),("initLength",json!(0))] {
                let mut changed=fixture(); changed["units"][first+1][key]=value; assert!(!valid(&changed),"{first}:{key}");
            }
            let mut changed=fixture();
            changed["units"][first+1]["generation"]=changed["units"][first]["generation"].clone(); assert!(!valid(&changed));
            let mut changed=fixture();
            changed["units"][first+1]["configuration"][if first==0 {"width"} else {"timescale"}]=json!(1234); assert!(!valid(&changed));
            let mut gap=fixture();
            gap["units"][first+1]["startOffset"]=json!(9); gap["units"][first+1]["endOffset"]=json!(17);
            gap["trackBytes"][if first==0 {"video"} else {"audio"}]=json!(17); gap["totalBytes"]=json!(33);
            assert!(!valid(&gap)); // All individual extents fit; the missing byte is still forbidden.
        }
        let mut mixed=fixture(); mixed["units"].as_array_mut().unwrap().swap(1,2); assert!(!valid(&mixed));
        let mut duplicate=fixture(); duplicate["units"][1]=duplicate["units"][0].clone(); assert!(!valid(&duplicate));
        let mut partial=fixture(); partial["units"][1]["byteLength"]=json!(7); assert!(!valid(&partial));
        let mut extra=fixture_counts(3,3); let last=extra["units"][5].clone(); extra["units"].as_array_mut().unwrap().push(last); assert!(!valid(&extra));
    }
    #[test]
    fn document_source_terminal_and_monotonic_pins_reject_replacement() {
        let ready=ready(); let mut initial=ready.clone(); initial.state=State::Collecting; initial.reason=Reason::Observing;
        initial.source_ordinal=None; initial.units.clear(); initial.elapsed_ms=0; initial.total_bytes=0;
        initial.track_bytes=TrackBytes{video:0,audio:0}; initial.excluded_partial_units=0; initial.requires_audio_join=false;
        let mut pin=Pin::default(); pin.accept(&initial).unwrap(); pin.accept(&ready).unwrap(); pin.accept(&ready).unwrap();
        let mut changed=ready.clone(); changed.document_id=CAPTURE.into();
        assert!(matches!(pin.accept(&changed), Err(Failure::SourceChanged(Boundary::PinnedDocumentChanged))));
        changed=ready.clone(); changed.source_ordinal=Some(2);
        assert!(matches!(pin.accept(&changed), Err(Failure::SourceChanged(Boundary::PinnedSourceChanged))));
        changed=ready.clone(); changed.units[0].end_seconds+=0.01;
        let failure=pin.accept(&changed).unwrap_err();
        assert_eq!(failure.label(),"invalid_metadata");
        assert_eq!(failure.boundary(),Some(Boundary::TerminalMetadataChanged));
        assert!(pin.accept(&initial).is_err());
        assert_eq!(pin.latest.as_ref(),Some(&ready));
    }
    #[test]
    fn evaluation_diagnostics_distinguish_phase_and_unavailable_without_relaxing_url_policy() {
        let approved_url: tauri::Url=format!("https://www.youtube.com/watch?v={VIDEO_ID}&autoplay=1&mute=1").parse().unwrap();
        let rejected: tauri::Url="https://accounts.google.com/private?token=never-logged".parse().unwrap();
        for after in [false,true] {
            assert!(evaluation_boundary(Some(&approved_url),after).is_ok());
            let unavailable=evaluation_boundary(None,after).unwrap_err();
            let invalid=evaluation_boundary(Some(&rejected),after).unwrap_err();
            assert_eq!(unavailable.label(),"source_changed");
            assert_eq!(invalid.label(),"source_changed");
            assert_eq!(unavailable.boundary(),Some(if after { Boundary::EvaluateAfterUrlUnavailable } else { Boundary::EvaluateBeforeUrlUnavailable }));
            assert_eq!(invalid.boundary(),Some(if after { Boundary::EvaluateAfterUrlRejected } else { Boundary::EvaluateBeforeUrlRejected }));
            let printed=summary(CAPTURE,&Err(invalid)).to_string();
            for sensitive in ["accounts.google.com","private","token","never-logged"] { assert!(!printed.contains(sensitive)); }
        }
    }
    #[test]
    fn boundary_summary_is_a_closed_fixed_scalar_and_never_changes_success_claims() {
        let boundaries=[
            Boundary::EvaluateBeforeUrlUnavailable, Boundary::EvaluateBeforeUrlRejected,
            Boundary::EvaluateAfterUrlUnavailable, Boundary::EvaluateAfterUrlRejected,
            Boundary::PinnedDocumentChanged, Boundary::PinnedSourceChanged,
            Boundary::SelectedBeforeUnitChanged, Boundary::SelectedAfterUnitsChanged, Boundary::TerminalMetadataChanged,
        ];
        for boundary in boundaries {
            let failure=if boundary==Boundary::TerminalMetadataChanged { Failure::InvalidMetadataAt(boundary) } else { Failure::SourceChanged(boundary) };
            let value=summary(CAPTURE,&Err(failure));
            assert_eq!(value["boundary"],serde_json::to_value(boundary).unwrap());
            assert_eq!(value["outcome"],failure.label());
            assert!(value["captureReason"].is_null());
            for key in ["captureCompleted","mediaVerified","recordingVerified","localMediaToolsInvoked"] { assert_eq!(value[key],false); }
            for key in ["base64","observations","documentId","sourceOrdinal","url","cookie","host","path"] { assert!(value.get(key).is_none()); }
            assert!(value.to_string().len()<1024);
        }
        assert!(summary(CAPTURE,&Err(Failure::CaptureFailed(Reason::SourceChanged,CaptureDiagnostics::default())))["boundary"].is_null());
    }
    #[test]
    fn page_load_points_preserve_first_pin_and_bound_monotonic_observations() {
        let mut trace=LoadTrace::default();
        trace.phase(CapturePhase::Polling,10); trace.load(true,20); trace.pin(25);
        let first=trace.first_pin.unwrap();
        assert_eq!(first.load_phase,LoadPhase::Started); assert_eq!(first.started,1); assert_eq!(first.finished,0);
        trace.load(false,30); trace.phase(CapturePhase::AfterUnits,40); trace.load(true,45); trace.pin(50);
        assert_eq!(trace.first_pin,Some(first));
        assert_eq!(trace.current.capture_phase,CapturePhase::AfterUnits); assert_eq!(trace.current.started,2);
        assert_eq!(trace.current.finished,1); assert_eq!(trace.current.load_phase,LoadPhase::Started);
        trace.time(1); assert_eq!(trace.current.elapsed_ms,50);
        for index in 0..100 { trace.load(index%2==0,100+index); }
        assert_eq!(trace.current.started+trace.current.finished,16);
        assert!(trace.current.event_limit_reached); assert_eq!(trace.current.load_phase,LoadPhase::Limit);
        trace.time(u32::MAX); assert_eq!(trace.current.elapsed_ms,70000);
        assert_eq!(trace.first_pin,Some(first));
    }
    #[test]
    fn page_load_summary_contains_only_fixed_phases_and_bounded_numeric_observations() {
        let diagnostics=LoadDiagnostics::new(Instant::now());
        diagnostics.phase(CapturePhase::Polling); diagnostics.load(true); diagnostics.pin();
        diagnostics.load(false); diagnostics.phase(CapturePhase::BeforeUnit);
        let value=summary_with_load(CAPTURE,&Err(Failure::SourceChanged(Boundary::PinnedDocumentChanged)),&diagnostics);
        assert_eq!(value["boundary"],"pinned_document_changed"); assert_eq!(value["captureCompleted"],false);
        let load=&value["pageLoad"];
        assert_eq!(load.as_object().unwrap().len(),2);
        for point in [&load["firstPin"],&load["current"]] {
            assert_eq!(point.as_object().unwrap().len(),6);
            for key in ["elapsedMs","capturePhase","loadPhase","started","finished","eventLimitReached"] { assert!(point.get(key).is_some()); }
            assert!(point["elapsedMs"].as_u64().unwrap()<=70000);
        }
        assert_eq!(load["firstPin"]["loadPhase"],"started");
        assert_eq!(load["current"]["loadPhase"],"finished");
        assert_eq!(load["current"]["capturePhase"],"before_unit");
        let printed=load.to_string();
        for private in ["documentId","captureId","sourceOrdinal","url","cookie",CAPTURE,DOCUMENT] { assert!(!printed.contains(private)); }
        assert!(value.to_string().len()<2048);
    }
    #[test]
    fn startup_waits_for_finished_and_two_unchanged_seconds_before_first_snapshot() {
        let mut trace=LoadTrace::default();
        assert_eq!(trace.startup_permit(0).unwrap(),None);
        trace.load(true,100); assert_eq!(trace.startup_permit(2100).unwrap(),None);
        trace.load(false,2200); assert_eq!(trace.startup_permit(4199).unwrap(),None);
        assert_eq!(trace.startup_permit(4200).unwrap(),Some(StartupPermit {started:1,finished:1}));
        assert!(trace.first_pin.is_none());
    }
    #[test]
    fn prepin_page_replacement_discards_only_unpinned_response_and_never_relaxes_pinned_identity() {
        let mut trace=LoadTrace::default(); let mut pin=Pin::default(); let selected=ready();
        trace.load(true,100); trace.load(false,500);
        let old=trace.startup_permit(2500).unwrap().unwrap();
        trace.load(true,2501);
        assert!(!trace.accept_startup(old,&mut pin,&selected,2502).unwrap());
        assert!(pin.document.is_none()); assert!(trace.first_pin.is_none());
        trace.load(false,3000); assert_eq!(trace.startup_permit(4999).unwrap(),None);
        let current=trace.startup_permit(5000).unwrap().unwrap(); assert_ne!(old,current);
        assert!(!trace.accept_startup(old,&mut pin,&selected,5001).unwrap());
        assert!(trace.accept_startup(current,&mut pin,&selected,5002).unwrap());
        trace.load(true,5003); trace.load(false,5100);
        let mut replacement=selected.clone(); replacement.document_id=CAPTURE.into();
        assert!(matches!(pin.accept(&replacement),Err(Failure::SourceChanged(Boundary::PinnedDocumentChanged))));
        assert_eq!(pin.latest.as_ref(),Some(&selected)); assert_eq!(trace.first_pin.unwrap().elapsed_ms,5002);
    }
    #[test]
    fn startup_deadline_and_lifecycle_overflow_fail_without_a_first_pin_or_restart() {
        let mut trace=LoadTrace::default(); trace.load(true,1); trace.load(false,2);
        assert!(trace.startup_permit(14999).unwrap().is_some());
        assert!(matches!(trace.startup_permit(15000),Err(Failure::StartupTimeout)));
        let mut trace=LoadTrace::default();
        for index in 0..17 { trace.load(index%2==0,index); }
        assert_eq!(trace.startup_permit(5000).unwrap(),None);
        assert!(trace.first_pin.is_none());
        let summary=summary(CAPTURE,&Err(trace.startup_permit(15000).unwrap_err()));
        assert_eq!(summary["outcome"],"startup_timeout"); assert_eq!(summary["captureCompleted"],false);
    }
    #[test]
    fn chunks_require_exact_ids_generation_offsets_canonical_base64_and_final_size() {
        let snapshot=ready(); let unit=&snapshot.units[0]; let value=chunk(&snapshot,unit,0,b"12345678");
        assert_eq!(parse_chunk(&value.to_string(),&snapshot,unit,0).unwrap(),b"12345678");
        for (key,bad) in [("version",json!(1)),("version",json!(2)),("captureId",json!(DOCUMENT)),("documentId",json!(CAPTURE)),("generation",json!(5)),
            ("unit",json!(2)),("offset",json!(1)),("byteLength",json!(7)),("final",json!(false)),
            ("base64",json!("MTIzNDU2Nzg=\n")),("path",json!("private"))] {
            let mut bad_value=value.clone(); bad_value[key]=bad;
            assert!(parse_chunk(&bad_value.to_string(),&snapshot,unit,0).is_err(),"{key}");
        }
        assert!(parse_chunk(&value.to_string(),&snapshot,unit,1).is_err());
        assert!(parse_chunk(&value.to_string(),&snapshot,unit,unit.byte_length).is_err());
        assert!(parse_chunk(&" ".repeat(MAX_CHUNK_REPLY+1),&snapshot,unit,0).is_err());
        let mut big=unit.clone(); big.byte_length=MAX_CHUNK as u64+7;
        let first=vec![0u8;MAX_CHUNK]; let value=chunk(&snapshot,&big,0,&first);
        assert!(parse_chunk(&value.to_string(),&snapshot,&big,0).is_ok());
        let value=chunk(&snapshot,&big,MAX_CHUNK as u64,b"1234567");
        assert!(parse_chunk(&value.to_string(),&snapshot,&big,MAX_CHUNK as u64).is_ok());
    }
    #[test]
    fn owned_files_are_fixed_create_new_synced_hashed_and_never_overwritten() {
        let workspace=tempfile::tempdir().unwrap(); let root=OwnedRoot::under(workspace.path()).unwrap();
        assert!(canonical_uuid(&root.id)); assert!(root.path.starts_with(workspace.path()));
        assert!(root.file("../private").is_err()); assert!(root.file("wrong.mp4").is_err());
        let mut file=root.file("video-1.webm").unwrap();
        let digest=write_and_verify(&mut file,b"owned-test").unwrap();
        assert_eq!(digest,format!("{:x}",Sha256::digest(b"owned-test")));
        assert!(root.file("video-1.webm").is_err()); assert!(root.file("unit-1.webm").is_err()); assert!(hash_file(&mut file,9).is_err());
        drop(file);
        let linked=root.path.join("linked-fixture"); fs::hard_link(root.path.join("video-1.webm"),&linked).unwrap();
        let file=File::open(&linked).unwrap(); assert!(inspect_handle(&file,false).is_err());
    }
    fn transferred_files(root: &OwnedRoot, snapshot: &Snapshot) -> Vec<File> {
        let mut ordinals = [0,0];
        snapshot.units.iter().map(|unit| {
            let index=if unit.track==Track::Video {0} else {1};
            let mut file=root.file(unit_filename(unit.track,ordinals[index]).unwrap()).unwrap();
            ordinals[index]+=1;
            write_and_verify(&mut file,if index==0 {b"VID0aaaa"} else {b"AUD0bbbb"}).unwrap();
            file.seek(SeekFrom::Start(2)).unwrap(); file
        }).collect()
    }
    #[test]
    fn transferred_init_bytes_are_equal_per_track_not_inferred_from_metadata() {
        let workspace=tempfile::tempdir().unwrap(); let root=OwnedRoot::under(workspace.path()).unwrap();
        let snapshot=ready(); let mut files=transferred_files(&root,&snapshot);
        let done=AtomicBool::new(false); let deadline=Instant::now()+Duration::from_secs(10);
        assert_eq!(verify_initializations(&mut files,&snapshot.units,deadline,&done).unwrap(),InitializationsIdentical{video:true,audio:true});
        assert!(files.iter_mut().all(|file| file.stream_position().unwrap()==2));
        // Equal declared configuration and length are insufficient: raw init differs.
        files[1].seek(SeekFrom::Start(0)).unwrap(); files[1].write_all(b"VID1").unwrap(); files[1].sync_all().unwrap();
        files[1].seek(SeekFrom::Start(3)).unwrap();
        assert_eq!(verify_initializations(&mut files,&snapshot.units,deadline,&done).unwrap(),InitializationsIdentical{video:false,audio:true});
        assert_eq!(files[1].stream_position().unwrap(),3);
        assert_eq!(hash_file(&mut files[1],8).unwrap(),format!("{:x}",Sha256::digest(b"VID1aaaa")));
        assert!(!root.path.join("manifest.json").exists());
    }
    #[test]
    fn exact_init_verification_handles_all_six_files_and_no_cross_track_equivalence() {
        let workspace=tempfile::tempdir().unwrap(); let root=OwnedRoot::under(workspace.path()).unwrap();
        let snapshot=parse_snapshot(&fixture_counts(3,3).to_string(),CAPTURE).unwrap();
        let mut files=transferred_files(&root,&snapshot);
        assert_eq!(verify_initializations(&mut files,&snapshot.units,Instant::now()+Duration::from_secs(10),&AtomicBool::new(false)).unwrap(),InitializationsIdentical{video:true,audio:true});
        for (track,stem,extension) in [(Track::Video,"video","webm"),(Track::Audio,"audio","mp4")] {
            for ordinal in 0..3 {
                assert_eq!(unit_filename(track,ordinal).unwrap(),format!("{stem}-{}.{extension}",ordinal+1));
            }
            assert!(unit_filename(track,3).is_err());
        }
        assert!(files.iter_mut().all(|file| file.stream_position().unwrap()==2));
        assert!(!root.path.join("manifest.json").exists());
    }
    #[test]
    fn init_verification_rejects_short_files_length_changes_cancel_and_expired_deadline() {
        let workspace=tempfile::tempdir().unwrap(); let root=OwnedRoot::under(workspace.path()).unwrap();
        let snapshot=ready(); let mut files=transferred_files(&root,&snapshot);
        let future=Instant::now()+Duration::from_secs(10);
        assert!(matches!(verify_initializations(&mut files,&snapshot.units,future,&AtomicBool::new(true)),Err(Failure::Deadline)));
        assert!(matches!(verify_initializations(&mut files,&snapshot.units,Instant::now(),&AtomicBool::new(false)),Err(Failure::Deadline)));
        let mut units=snapshot.units.clone(); units[1].init_length=3;
        assert_eq!(verify_initializations(&mut files,&units,future,&AtomicBool::new(false)).unwrap(),InitializationsIdentical{video:false,audio:true});
        files[1].set_len(7).unwrap();
        assert!(matches!(verify_initializations(&mut files,&snapshot.units,future,&AtomicBool::new(false)),Err(Failure::StorageRejected)));
        assert!(!root.path.join("manifest.json").exists());
    }
    #[test]
    fn init_read_failure_still_restores_original_cursor() {
        let directory=tempfile::tempdir().unwrap();
        let mut file=OpenOptions::new().write(true).create_new(true).open(directory.path().join("write-only-fixture")).unwrap();
        file.write_all(b"VID0aaaa").unwrap(); file.seek(SeekFrom::Start(2)).unwrap();
        assert!(matches!(init_prefix(&mut file,&ready().units[0]),Err(Failure::StorageRejected)));
        assert_eq!(file.stream_position().unwrap(),2);
    }
    #[test]
    fn v3_keeps_different_init_lengths_and_bytes_without_rewriting_or_js_equality_claims() {
        let mut value=fixture(); value["units"][1]["initLength"]=json!(3); value["units"][3]["initLength"]=json!(5);
        assert!(valid(&value));
        let snapshot=parse_snapshot(&value.to_string(),CAPTURE).unwrap();
        let workspace=tempfile::tempdir().unwrap(); let root=OwnedRoot::under(workspace.path()).unwrap();
        let mut files=transferred_files(&root,&snapshot);
        let before=files.iter_mut().map(|file|hash_file(file,8).unwrap()).collect::<Vec<_>>();
        for file in &mut files { file.seek(SeekFrom::Start(2)).unwrap(); }
        let actual=verify_initializations(&mut files,&snapshot.units,Instant::now()+Duration::from_secs(10),&AtomicBool::new(false)).unwrap();
        assert_eq!(actual,InitializationsIdentical{video:false,audio:false});
        assert!(files.iter_mut().all(|file|file.stream_position().unwrap()==2));
        let after=files.iter_mut().map(|file|hash_file(file,8).unwrap()).collect::<Vec<_>>();
        assert_eq!(before,after);
        value["initializationsIdentical"]=json!({"video":true,"audio":true}); assert!(!valid(&value));
        assert!(!root.path.join("manifest.json").exists());
    }
    #[test]
    fn differing_prefix_does_not_skip_later_file_guards() {
        let workspace=tempfile::tempdir().unwrap(); let root=OwnedRoot::under(workspace.path()).unwrap();
        let snapshot=ready(); let mut files=transferred_files(&root,&snapshot);
        files[1].seek(SeekFrom::Start(0)).unwrap(); files[1].write_all(b"VID1").unwrap(); files[1].sync_all().unwrap();
        files[3].set_len(7).unwrap();
        assert!(matches!(verify_initializations(&mut files,&snapshot.units,Instant::now()+Duration::from_secs(10),&AtomicBool::new(false)),Err(Failure::StorageRejected)));
        assert!(!root.path.join("manifest.json").exists());
    }
    #[test]
    fn initialization_review_is_raw_capture_only_and_reports_both_native_flags() {
        for (video,audio) in [(true,true),(false,true),(true,false),(false,false)] {
            let value=summary(CAPTURE,&Ok(Completion{units:4,video_units:2,audio_units:2,bytes:32,requires_audio_join:true,
                initializations_identical:InitializationsIdentical{video,audio},manifest_sha256:"0".repeat(64)}));
            assert_eq!(value["initializationsIdentical"],json!({"video":video,"audio":audio}));
            assert_eq!(value["initializationsIdentical"].as_object().unwrap().len(),2);
            assert_eq!(value["outcome"],if video&&audio {"needs_audio_unit_join"} else {"needs_initialization_review"});
            assert_eq!(value["captureCompleted"],true);
            for key in ["packetVerified","mediaVerified","recordingVerified","localMediaToolsInvoked"] { assert_eq!(value[key],false); }
        }
    }
    #[test]
    fn exact_public_document_and_summary_cannot_expose_page_payloads() {
        assert!(approved(&format!("https://www.youtube.com/watch?v={VIDEO_ID}&autoplay=1&mute=1").parse().unwrap()));
        for url in ["https://accounts.google.com/","http://www.youtube.com/watch?v=T4XZmMPQ9Kw",
            "https://www.youtube.com/watch?v=cswFQYam8Ic","https://www.youtube.com/watch?v=T4XZmMPQ9Kw&v=T4XZmMPQ9Kw",
            "https://www.youtube.com/live_chat?v=T4XZmMPQ9Kw","https://www.youtube.com/watch?v=T4XZmMPQ9Kw#x"] {
            assert!(!approved(&url.parse().unwrap()));
        }
        let value=summary(CAPTURE,&Ok(Completion{units:4,video_units:2,audio_units:2,bytes:32,requires_audio_join:true,
            initializations_identical:InitializationsIdentical{video:true,audio:true},manifest_sha256:"0".repeat(64)}));
        assert_eq!(value["captureCompleted"],true); assert_eq!(value["mediaVerified"],false);
        assert_eq!(value["videoUnits"],2); assert_eq!(value["audioUnits"],2); assert_eq!(value["outcome"],"needs_audio_unit_join");
        assert_eq!(value["recordingVerified"],false); assert_eq!(value["localMediaToolsInvoked"],false);
        for key in ["base64","observations","documentId","unitsMetadata","url","cookie"] { assert!(value.get(key).is_none()); }
        let failed=summary(CAPTURE,&Err(Failure::CaptureFailed(Reason::NoCandidate,CaptureDiagnostics::default())));
        assert_eq!(failed["outcome"],"capture_failed"); assert_eq!(failed["captureReason"],"no_candidate");
        assert_eq!(failed["captureCompleted"],false);
    }
    #[test]
    fn diagnostics_require_closed_present_integer_objects_and_exact_bounds() {
        let fields=[("units",16), ("adjacentPairs",15), ("adjacentTriples",14), ("initDifferences",15),
            ("configurationDifferences",15), ("byteGenerationBreaks",15), ("clockOrderBreaks",15), ("gaps",15)];
        let mut missing=fixture(); missing.as_object_mut().unwrap().remove("diagnostics"); assert!(!valid(&missing));
        for invalid in [serde_json::Value::Null,json!([]),json!(false),json!(0)] {
            let mut value=fixture(); value["diagnostics"]=invalid; assert!(!valid(&value));
        }
        for track in ["video","audio"] {
            let mut missing=fixture(); missing["diagnostics"].as_object_mut().unwrap().remove(track); assert!(!valid(&missing));
            for (field,maximum) in fields {
                let mut upper=fixture(); upper["diagnostics"][track][field]=json!(maximum); assert!(valid(&upper));
                for invalid in [json!(maximum+1),json!(-1),json!(0.5),json!("0"),serde_json::Value::Null,json!(false),json!([]),json!({})] {
                    let mut value=fixture(); value["diagnostics"][track][field]=invalid; assert!(!valid(&value),"{track}:{field}");
                }
                let mut missing=fixture(); missing["diagnostics"][track].as_object_mut().unwrap().remove(field); assert!(!valid(&missing));
            }
            let mut extra=fixture(); extra["diagnostics"][track]["url"]=json!("never-allowed"); assert!(!valid(&extra));
        }
        let mut extra=fixture(); extra["diagnostics"]["track"]=json!({}); assert!(!valid(&extra));
        let duplicate=fixture().to_string().replacen("\"adjacentPairs\":0","\"adjacentPairs\":0,\"adjacentPairs\":0",1);
        assert!(parse_snapshot(&duplicate,CAPTURE).is_err());
    }
    #[test]
    fn diagnostics_are_terminal_immutable_observations_not_selection_authority() {
        let mut selected=ready(); let mut pin=Pin::default(); pin.accept(&selected).unwrap();
        selected.diagnostics.video.init_differences=1;
        assert!(matches!(pin.accept(&selected),Err(Failure::InvalidMetadataAt(Boundary::TerminalMetadataChanged))));
        let mut invalid=fixture(); invalid["diagnostics"]["video"]["adjacentPairs"]=json!(15);
        invalid["units"][1]["generation"]=json!(12); assert!(!valid(&invalid));
    }
    #[test]
    fn only_capture_failure_outputs_fixed_counter_diagnostics_without_snapshot() {
        let diagnostics=CaptureDiagnostics { video:TrackDiagnostics {units:4,init_differences:3,..TrackDiagnostics::default()},
            audio:TrackDiagnostics {units:3,gaps:1,..TrackDiagnostics::default()} };
        let value=summary(CAPTURE,&Err(Failure::CaptureFailed(Reason::NoCandidate,diagnostics)));
        assert_eq!(value["diagnostics"],serde_json::to_value(diagnostics).unwrap());
        assert_eq!(value["diagnostics"].as_object().unwrap().len(),2);
        for track in ["video","audio"] {
            let object=value["diagnostics"][track].as_object().unwrap(); assert_eq!(object.len(),8);
            assert!(object.values().all(|counter|counter.as_u64().is_some_and(|n|n<=16)));
        }
        for key in ["captureCompleted","mediaVerified","recordingVerified","localMediaToolsInvoked"] { assert_eq!(value[key],false); }
        for failure in [Failure::Deadline,Failure::InvalidMetadata,Failure::InvalidChunk,Failure::EvalFailed,
            Failure::SourceChanged(Boundary::PinnedDocumentChanged),Failure::StorageRejected] {
            assert!(summary(CAPTURE,&Err(failure)).get("diagnostics").is_none());
        }
        let completed=summary(CAPTURE,&Ok(Completion {units:4,video_units:2,audio_units:2,bytes:32,requires_audio_join:true,
            initializations_identical:InitializationsIdentical{video:true,audio:true},manifest_sha256:"0".repeat(64)}));
        assert!(completed.get("diagnostics").is_none());
        let text=value.to_string();
        for forbidden in ["startSeconds","documentId","sourceOrdinal","configuration\"","base64","url","cookie",DOCUMENT] { assert!(!text.contains(forbidden)); }
        assert!(text.len()<2048);
    }
}
