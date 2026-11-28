//! Read-only source -> transport evidence. This never creates a remote anchor.
//! Production authority is minted only by native joined-owner receipt capture.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

#[path = "relay_timing_points.rs"]
mod points;
pub(crate) use points::AcknowledgedSourcePoint;
#[path="relay_timing_soop_events.rs"]
mod soop_events;
pub(crate) use soop_events::EventPosition;
#[path="relay_timing_disk_index.rs"]
mod disk_index;

const CONTROL_BYTES: u64 = 65536;
const LINE_BYTES: usize = 4096;
type Result<T> = std::result::Result<T, TimingReadError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimingReadError { Cancelled, Io, UnsafePath, Limit, Schema, Identity, Hash, Clock, Order }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect { Soop, Chzzk }
impl Dialect {
    fn bytes(self) -> u64 { match self { Self::Soop => 256*1024*1024, Self::Chzzk => 64*1024*1024 } }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Track { Video, Audio }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceTicks { pub ticks: u64, pub timescale: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteAnchor { Unknown }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MappedPosition {
    /// Exact coordinate transform inside an acknowledged source envelope, not
    /// proof that a frame/chat event existed at this arbitrary coordinate.
    Transport90k(u64),
    OutsideAcknowledgedPrefix,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceEnvelope {
    pub timescale: u32, pub first_dts: u64, pub last_dts_end: u64,
    pub min_pts: u64, pub max_pts_end: u64, pub samples: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RelayTimingSummary {
    pub dialect: Dialect, pub records: u64, pub acknowledged_segments: u64,
    pub prefix_segments: u64, pub transport_end_90k: Option<u64>,
    pub stream_id: Option<Uuid>, pub origin: Option<SourceTicks>,
    pub video: Option<SourceEnvelope>, pub audio: Option<SourceEnvelope>,
    pub end_seen: bool, pub transport_complete: bool, pub timing_incomplete: bool,
    pub sidecar_incomplete: bool,
    pub observations: u64, pub usable_observations: u64, pub unknown_observations: u64,
    pub preroll: Option<Preroll>, pub remote_anchor: RemoteAnchor,
}

/// No Deserialize, public fields, JSON/path factory or legacy fallback. The
/// future receipt owner must mint this only from its verified native binding.
pub(crate) struct RelaySidecarReadAuthorization {
    root: PathBuf,
    owner: ExpectedOwner,
    files: [FileProof; 3],
    acknowledged_segments: u64,
    end_list_acked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(super) struct FileProof { pub(super) bytes: u64, pub(super) sha256: [u8; 32] }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag="provider", content="owner", rename_all="lowercase", deny_unknown_fields)]
enum ExpectedOwner { Soop(SoopOwner), Chzzk(ChzzkOwner) }
impl ExpectedOwner {
    fn dialect(&self) -> Dialect { match self { Self::Soop(_) => Dialect::Soop, Self::Chzzk(_) => Dialect::Chzzk } }
    fn source(&self) -> &str { match self { Self::Soop(v) => &v.binding.source_id, Self::Chzzk(v) => &v.source_id } }
    fn directory_id(&self) -> &str { match self { Self::Soop(v) => &v.sidecar_id, Self::Chzzk(v) => &v.run_id } }
}
/// Encrypted catalog provenance, saved after actual join before cancellable hash
/// work. Deserialization does not make it an active source or replay capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(super) struct RelayNativeCapture {
    root: PathBuf,
    owner: ExpectedOwner,
    source_fingerprint: String,
    acknowledged_segments: u64,
    end_list_acked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(super) struct RelayTimingProof { files: [FileProof; 3] }
impl RelayTimingProof {
    pub(super) fn valid(&self) -> bool {
        self.files[0].bytes > 0 && self.files[0].bytes <= CONTROL_BYTES
            && self.files[1].bytes > 0 && self.files[1].bytes <= CONTROL_BYTES
            && self.files[2].bytes <= Dialect::Soop.bytes()
    }
}
impl RelayNativeCapture {
    pub(super) fn from_native(root: &Path, native: &crate::streaming::relay_status::NativeSidecarOwner,
        source: &super::live_store::SourceFacts, acknowledged_segments: u64, end_list_acked: bool) -> Result<Self> {
        use crate::streaming::relay_status::NativeSidecarOwner as Native;
        let owner = match native {
            Native::Soop { sidecar_id, receiver_id, run_id, source_id, document_epoch, channel_id, broadcast_id } => ExpectedOwner::Soop(SoopOwner {
                version: 1, kind: "soop-relay-chat".into(), sidecar_id: sidecar_id.clone(),
                binding: SoopBinding { receiver_id: receiver_id.clone(), run_id: run_id.clone(), source_id: source_id.clone(),
                    document_epoch: *document_epoch, channel_id: channel_id.clone(), broadcast_id: broadcast_id.clone() },
                timeline_quality: "receive-monotonic-only".into(), media_job: false,
            }),
            Native::Chzzk { run_id, session_id, source_id, document_epoch, account_revision, channel_id, started_at_ms } => ExpectedOwner::Chzzk(ChzzkOwner {
                version: 1, provider: "chzzk".into(), run_id: run_id.clone(), session_id: session_id.clone(), source_id: source_id.clone(),
                document_epoch: *document_epoch, account_revision: *account_revision, channel_id: channel_id.clone(), started_at_ms: *started_at_ms,
                clock: "native_receipt".into(), remote_archive_mapping: "unknown".into(),
            }),
        };
        validate_expected(&owner)?;
        let _directories = crate::streaming::browser_store::cleanup::pin_directories(root).map_err(|_| TimingReadError::UnsafePath)?;
        let root = fs::canonicalize(root).map_err(|_| TimingReadError::UnsafePath)?;
        let result = Self { root, owner, source_fingerprint: super::live_archive::sha(&serde_json::to_vec(source).map_err(|_| TimingReadError::Schema)?),
            acknowledged_segments, end_list_acked };
        if !result.shape() { return Err(TimingReadError::Identity); }
        Ok(result)
    }
    fn shape(&self) -> bool {
        self.root.is_absolute() && self.root.as_os_str().len() <= 2048
            && self.root.file_name().and_then(|v| v.to_str()) == Some(self.owner.directory_id())
            && validate_expected(&self.owner).is_ok() && hex(&self.source_fingerprint)
    }
    pub(super) fn matches(&self, source: &super::live_store::SourceFacts, run: &str, sidecar: &str, acks: u64, end: bool) -> bool {
        if !self.shape() || self.owner.directory_id() != sidecar || self.acknowledged_segments != acks || self.end_list_acked != end
            || serde_json::to_vec(source).ok().map(|b| super::live_archive::sha(&b)).as_deref() != Some(self.source_fingerprint.as_str()) { return false; }
        match &self.owner {
            ExpectedOwner::Soop(o) => source.provider == "soop" && o.binding.receiver_id == source.session_id
                && o.binding.run_id == run && o.binding.document_epoch == source.epoch && o.binding.channel_id == source.channel_id
                && source.broadcast_key.as_deref() == Some(o.binding.broadcast_id.as_str()),
            ExpectedOwner::Chzzk(o) => source.provider == "chzzk" && o.session_id == source.session_id && o.run_id == run
                && o.document_epoch == source.epoch && o.account_revision == source.account_revision && o.channel_id == source.channel_id,
        }
    }
    pub(super) fn root(&self) -> &Path { &self.root }
    pub(super) fn owns_reference(&self, run: Uuid, source: Uuid) -> bool {
        if !self.shape() || run.is_nil() || source.is_nil() { return false; }
        let (expected_run, expected_source) = match &self.owner {
            ExpectedOwner::Soop(o) => (&o.binding.run_id, &o.binding.source_id),
            ExpectedOwner::Chzzk(o) => (&o.run_id, &o.source_id),
        };
        uuid(expected_run) == Ok(run) && uuid(expected_source) == Ok(source)
    }
    fn authorization(&self, proof: &RelayTimingProof) -> Result<RelaySidecarReadAuthorization> {
        if !self.shape() || !proof.valid() { return Err(TimingReadError::Identity); }
        Ok(RelaySidecarReadAuthorization { root: self.root.clone(), owner: self.owner.clone(), files: proof.files.clone(),
            acknowledged_segments: self.acknowledged_segments, end_list_acked: self.end_list_acked })
    }
    pub(super) fn seal(&self, cancel: &AtomicBool) -> Result<RelayTimingProof> {
        cancelled(cancel)?;
        if !self.shape() { return Err(TimingReadError::Identity); }
        let _directories = crate::streaming::browser_store::cleanup::pin_directories(&self.root).map_err(|_| TimingReadError::UnsafePath)?;
        let mut handles = Vec::new();
        let mut proofs = Vec::new();
        for (name, max) in [("relay-owner.json", CONTROL_BYTES), ("relay-summary.json", CONTROL_BYTES), ("relay-timing.jsonl", self.owner.dialect().bytes())] {
            let path = self.root.join(name);
            let bytes = fs::symlink_metadata(&path).map_err(|_| TimingReadError::Io)?.len();
            let mut file = pinned(&path, &FileProof { bytes, sha256: [0;32] }, max)?;
            let mut hasher = Sha256::new();
            let mut seen = 0u64;
            let mut buffer = [0;65536];
            loop {
                cancelled(cancel)?;
                let n = file.read(&mut buffer).map_err(|_| TimingReadError::Io)?;
                if n == 0 { break; }
                seen = seen.checked_add(n as u64).filter(|v| *v <= max).ok_or(TimingReadError::Limit)?;
                hasher.update(&buffer[..n]);
            }
            if seen != bytes { return Err(TimingReadError::Hash); }
            file.seek(SeekFrom::Start(0)).map_err(|_| TimingReadError::Io)?;
            proofs.push(FileProof { bytes, sha256: hasher.finalize().into() });
            handles.push(file);
        }
        let proof = RelayTimingProof { files: proofs.try_into().map_err(|_| TimingReadError::Schema)? };
        // Parse exact expected owner and durable records while original guards
        // still deny replacement. Hashing an unrelated file is not admission.
        let _lease = self.open(&proof, cancel)?;
        drop(handles);
        Ok(proof)
    }
    pub(super) fn open(&self, proof: &RelayTimingProof, cancel: &AtomicBool) -> Result<RelayTimingLease> {
        RelayTimingLease::open(&self.authorization(proof)?, cancel)
    }
}
pub(crate) struct RelayTimingLease {
    summary: RelayTimingSummary,
    owner: ExpectedOwner,
    durable: Durable,
    native_acks: u64,
    native_end: bool,
    _files: [File; 3],
    _directories: Vec<File>,
    reference_read: std::sync::Mutex<()>,
    disk_index: std::sync::OnceLock<disk_index::DiskIndex>,
}
impl RelayTimingLease {
    pub(crate) fn open(auth: &RelaySidecarReadAuthorization, cancel: &AtomicBool) -> Result<Self> {
        cancelled(cancel)?;
        validate_expected(&auth.owner)?;
        if !auth.root.is_absolute() || fs::canonicalize(&auth.root).map_err(|_| TimingReadError::UnsafePath)? != auth.root
            || auth.root.file_name().and_then(|s| s.to_str()) != Some(auth.owner.directory_id()) {
            return Err(TimingReadError::UnsafePath);
        }
        let directories = crate::streaming::browser_store::cleanup::pin_directories(&auth.root).map_err(|_| TimingReadError::UnsafePath)?;
        let mut owner = pinned(&auth.root.join("relay-owner.json"), &auth.files[0], CONTROL_BYTES)?;
        let mut summary = pinned(&auth.root.join("relay-summary.json"), &auth.files[1], CONTROL_BYTES)?;
        let mut timing = pinned(&auth.root.join("relay-timing.jsonl"), &auth.files[2], auth.owner.dialect().bytes())?;
        let owner_bytes = control(&mut owner, &auth.files[0], cancel)?;
        let summary_bytes = control(&mut summary, &auth.files[1], cancel)?;
        let durable = validate_controls(&auth.owner, &owner_bytes, &summary_bytes)?;
        let mut reader = BufReader::with_capacity(8192, &mut timing);
        let (report, digest, bytes) = scan(&mut reader, &auth.owner, durable, auth.acknowledged_segments, auth.end_list_acked, cancel)?;
        drop(reader);
        if digest != auth.files[2].sha256 || bytes != auth.files[2].bytes { return Err(TimingReadError::Hash); }
        Ok(Self { summary: report, owner: auth.owner.clone(), durable,
            native_acks: auth.acknowledged_segments, native_end: auth.end_list_acked,
            _files: [owner, summary, timing], _directories: directories,
            reference_read: std::sync::Mutex::new(()), disk_index: std::sync::OnceLock::new() })
    }
    pub(crate) fn summary(&self) -> &RelayTimingSummary { &self.summary }
    /// Cross-file admission, not a new authority factory. These files were
    /// already hash-verified and stay pinned for this lease. Only the at-most
    /// three retained windows are requested; no growing per-segment index.
    pub(super) fn verify_reference_windows(&self, windows: &[super::live_fingerprint::ReferenceWindow], cancel: &AtomicBool) -> Result<()> {
        cancelled(cancel)?;
        require(windows.len() <= 3, TimingReadError::Limit)?;
        for (index, window) in windows.iter().enumerate() {
            require(window.ordinal < self.summary.prefix_segments && hex(&window.segment_sha256)
                && (8..=72).contains(&window.frames.len())
                && (index == 0 || windows[index - 1].ordinal < window.ordinal), TimingReadError::Order)?;
            require(window.frames.windows(2).all(|pair| pair[0].pts90k < pair[1].pts90k), TimingReadError::Clock)?;
        }
        if windows.is_empty() { return Ok(()); }
        // try_clone shares the file cursor. Serialize this scan even when a
        // native caller holds more than one borrowed capability to this lease.
        let _read = disk_index::lock(&self.reference_read, cancel)?;
        let mut file = self._files[2].try_clone().map_err(|_| TimingReadError::Io)?;
        file.seek(SeekFrom::Start(0)).map_err(|_| TimingReadError::Io)?;
        let mut reader = BufReader::with_capacity(8192, file);
        let mut line = Vec::with_capacity(LINE_BYTES + 1);
        let mut total = 0u64;
        let mut matched = 0usize;
        loop {
            cancelled(cancel)?;
            line.clear();
            let count = (&mut reader).take((LINE_BYTES + 2) as u64).read_until(b'\n', &mut line).map_err(|_| TimingReadError::Io)?;
            if count == 0 { return Err(TimingReadError::Order); }
            total = total.checked_add(count as u64).ok_or(TimingReadError::Limit)?;
            let limit = if self.summary.dialect == Dialect::Soop { LINE_BYTES + 1 } else { LINE_BYTES };
            require(total <= self.summary.dialect.bytes() && count <= limit, TimingReadError::Limit)?;
            require(count > 1 && line.last() == Some(&b'\n'), TimingReadError::Schema)?;
            let segment = match self.summary.dialect {
                Dialect::Soop => {
                    let row: SoopRow = parse(&line[..count - 1])?;
                    match (row.kind.as_str(), row.data) {
                        ("segment_ack", SoopData::Segment(s)) => Some((s.stream_id, s.ordinal, s.sha256, s.timing)),
                        _ => None,
                    }
                }
                Dialect::Chzzk => match parse::<ChzzkRow>(&line[..count - 1])? {
                    ChzzkRow::Segment { stream_id, ordinal, sha256, source, .. } => Some((stream_id, ordinal, sha256, source)),
                    _ => None,
                },
            };
            let Some((stream, ordinal, sha256, timing)) = segment else { continue; };
            let window = &windows[matched];
            if ordinal < window.ordinal { continue; }
            require(ordinal == window.ordinal, TimingReadError::Order)?;
            require(Some(uuid(&stream)?) == self.summary.stream_id && sha256 == window.segment_sha256, TimingReadError::Hash)?;
            require(window.source_origin_ticks == timing.origin_ticks
                && window.source_origin_timescale == timing.origin_timescale
                && window.source_video_timescale == timing.video.timescale
                && window.frames.iter().all(|frame| frame.pts90k >= timing.video.min_pts90k
                    && frame.pts90k < timing.video.max_pts_end90k), TimingReadError::Clock)?;
            matched += 1;
            if matched == windows.len() { return Ok(()); }
        }
    }
    pub(crate) fn map_source_pts(&self, track: Track, source: SourceTicks) -> Result<MappedPosition> {
        map(&self.summary, track, source)
    }
}
pub(super) fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) { Err(TimingReadError::Cancelled) } else { Ok(()) }
}
pub(super) fn pinned(path: &Path, proof: &FileProof, max: u64) -> Result<File> {
    if proof.bytes > max { return Err(TimingReadError::Limit); }
    let file = crate::streaming::browser_store::cleanup::verification_guard(path).map_err(|_| TimingReadError::UnsafePath)?;
    if file.metadata().map_err(|_| TimingReadError::Io)?.len() != proof.bytes { return Err(TimingReadError::Hash); }
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION}};
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.map_err(|_| TimingReadError::UnsafePath)?;
        if info.nNumberOfLinks != 1 { return Err(TimingReadError::UnsafePath); }
    }
    Ok(file)
}
fn control(file: &mut File, proof: &FileProof, cancel: &AtomicBool) -> Result<Vec<u8>> {
    cancelled(cancel)?;
    let mut bytes = Vec::with_capacity(proof.bytes as usize);
    file.take(CONTROL_BYTES + 1).read_to_end(&mut bytes).map_err(|_| TimingReadError::Io)?;
    if bytes.len() as u64 != proof.bytes || bytes.len() as u64 > CONTROL_BYTES { return Err(TimingReadError::Limit); }
    if <[u8; 32]>::from(Sha256::digest(&bytes)) != proof.sha256 { return Err(TimingReadError::Hash); }
    cancelled(cancel)?; Ok(bytes)
}
fn parse<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| TimingReadError::Schema)
}
fn uuid(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).ok().filter(|v| v.to_string() == value && !v.is_nil()).ok_or(TimingReadError::Identity)
}
fn hex(value: &str) -> bool { value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn require(value: bool, error: TimingReadError) -> Result<()> { if value { Ok(()) } else { Err(error) } }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopBinding { receiver_id: String, run_id: String, source_id: String, document_epoch: u64, channel_id: String, broadcast_id: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopOwner { version: u8, kind: String, sidecar_id: String, binding: SoopBinding, timeline_quality: String, media_job: bool }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct ChzzkOwner {
    version: u8, provider: String, run_id: String, session_id: String, document_epoch: u64,
    account_revision: u64, channel_id: String, source_id: String, started_at_ms: u64,
    clock: String, remote_archive_mapping: String,
}
fn validate_expected(owner: &ExpectedOwner) -> Result<()> {
    match owner {
        ExpectedOwner::Soop(o) => {
            uuid(&o.sidecar_id)?; uuid(&o.binding.receiver_id)?; uuid(&o.binding.run_id)?; uuid(&o.binding.source_id)?;
            require(o.version == 1 && o.kind == "soop-relay-chat" && !o.media_job && o.timeline_quality == "receive-monotonic-only"
                && o.binding.document_epoch > 0 && (1..=40).contains(&o.binding.channel_id.len())
                && o.binding.channel_id.bytes().all(|v| v.is_ascii_alphanumeric() || matches!(v,b'_'|b'-'))
                && o.binding.broadcast_id.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == o.binding.broadcast_id), TimingReadError::Identity)
        }
        ExpectedOwner::Chzzk(o) => {
            uuid(&o.run_id)?; uuid(&o.session_id)?; uuid(&o.source_id)?;
            require(o.version == 1 && o.provider == "chzzk" && o.document_epoch > 0 && o.channel_id.len() == 32
                && o.channel_id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && o.clock == "native_receipt" && o.remote_archive_mapping == "unknown", TimingReadError::Identity)
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopSummary {
    sidecar_id: String, raw_rows: u64, event_rows: u64, bytes: u64, interrupted: bool,
    assets_incomplete: bool, failure: Option<String>, text_state: String, joined: bool,
    clock_quality: String, timing_records: u64, timing_incomplete: bool,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct ChzzkSummary {
    sidecar_id: String, joined: bool, raw_rows: u64, event_rows: u64, bytes: u64,
    interrupted: bool, assets_incomplete: bool, failure: bool, text_state: String,
    clock_quality: String, timing_records: u64, timing_incomplete: bool,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct ChzzkSummaryFile { version: u8, binding: ChzzkOwner, summary: ChzzkSummary, asset_expected: u64, asset_incomplete: bool, remote_archive_mapping: String }
#[derive(Clone, Copy)]
struct Durable { records: u64, incomplete: bool, sidecar_incomplete: bool }
fn validate_controls(expected: &ExpectedOwner, owner: &[u8], summary: &[u8]) -> Result<Durable> {
    match expected {
        ExpectedOwner::Soop(o) => {
            require(parse::<SoopOwner>(owner)? == *o, TimingReadError::Identity)?;
            let s: SoopSummary = parse(summary)?;
            require(s.sidecar_id == o.sidecar_id && !s.joined && s.clock_quality == "receive-monotonic-only"
                && matches!(s.text_state.as_str(), "complete" | "partial" | "unknown"), TimingReadError::Identity)?;
            Ok(Durable { records: s.timing_records, incomplete: s.timing_incomplete,
                sidecar_incomplete:s.text_state != "complete" || s.interrupted || s.assets_incomplete || s.failure.is_some() })
        }
        ExpectedOwner::Chzzk(o) => {
            require(parse::<ChzzkOwner>(owner)? == *o, TimingReadError::Identity)?;
            let s: ChzzkSummaryFile = parse(summary)?;
            require(s.version == 1 && s.binding == *o && s.summary.sidecar_id == o.run_id && !s.summary.joined
                && s.summary.clock_quality == "native_receipt_and_separate_source_observations"
                && matches!(s.summary.text_state.as_str(), "ended" | "incomplete")
                && s.remote_archive_mapping == "unknown" && s.asset_incomplete == s.summary.assets_incomplete, TimingReadError::Identity)?;
            Ok(Durable { records: s.summary.timing_records, incomplete: s.summary.timing_incomplete,
                sidecar_incomplete:s.summary.text_state != "ended" || s.summary.interrupted || s.summary.assets_incomplete || s.summary.failure })
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct TrackTiming {
    timescale: u32, first_dts: u64, last_dts_end: u64, min_pts: u64, max_pts_end: u64, samples: u64,
    first_dts90k: u64, last_dts_end90k: u64, min_pts90k: u64, max_pts_end90k: u64,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct Timing { origin_ticks: u64, origin_timescale: u32, segment_start90k: u64, segment_end90k: u64, video: TrackTiming, audio: TrackTiming }
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct EndOrigin { ticks: u64, timescale: u32, video_base: u64, audio_base: u64, video_remainder: u32, audio_remainder: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct Preroll { pub video_samples: u64, pub audio_samples: u64, pub bytes: u64 }
// Unit () accepts only JSON null. It cannot silently accept a fabricated mapping.
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopSegment { stream_id: String, ordinal: u64, native_before_ns: Option<u64>, native_after_ns: Option<u64>, bytes: u64, sha256: String, timing: Timing, archive_mapping: () }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopEnd { stream_id: String, segments: u64, native_before_ns: Option<u64>, native_after_ns: Option<u64>, origin: EndOrigin, preroll: Preroll, archive_mapping: () }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopSample { source_time_seconds: f64, performance_now_ms: f64, timestamp_offset_seconds: f64, current_time_seconds: f64, playback_rate: f64, paused: bool, seeking: bool, ready_state: u8 }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct SoopObservation { nonce: String, native_before_ns: Option<u64>, native_after_ns: Option<u64>, sample: Option<SoopSample>, usable: bool, quality: String, archive_mapping: () }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct Missing { nonce: String, native_before_ns: Option<u64>, native_after_ns: Option<u64>, reason: String }
#[derive(Deserialize)]
#[serde(untagged)]
enum SoopData { Event(soop_events::EventRow), Segment(SoopSegment), End(SoopEnd), Observation(SoopObservation), Missing(Missing) }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoopRow { version: u8, sequence: u64, kind: String, binding: SoopBinding, data: SoopData }
#[derive(Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
struct ChzzkSample {
    source_time_seconds: f64, current_time: f64, timestamp_offset: f64,
    track_index: u32, source_id: String, clock: String, performance_now_ms: f64,
    #[serde(default)] paused: Option<bool>,
    #[serde(default)] seeking: Option<bool>,
    #[serde(default)] ready_state: Option<u8>,
    #[serde(default)] ended: Option<bool>,
    #[serde(default)] playback_rate: Option<f64>,
}
impl ChzzkSample {
    // Qualification of the observed player only. It does not establish the
    // historical MSE offset range, a chat receipt mapping, or a remote origin.
    fn playback_usable(&self, bracket: Option<(u64, u64)>) -> Result<bool> {
        match (self.paused, self.seeking, self.ready_state, self.ended, self.playback_rate) {
            (None, None, None, None, None) => Ok(false),
            (Some(paused), Some(seeking), Some(ready), Some(ended), Some(rate)) => {
                require(ready <= 4 && rate.is_finite() && (0.0625..=16.).contains(&rate)
                    && bracket.is_some_and(|(a,z)| z-a <= 3_000_000_000), TimingReadError::Clock)?;
                Ok(!paused && !seeking && !ended && ready >= 2)
            }
            _ => Err(TimingReadError::Clock),
        }
    }
}
#[derive(Deserialize)]
#[serde(tag="kind", deny_unknown_fields)]
enum ChzzkRow {
    #[serde(rename="segment_ack", rename_all="camelCase")]
    Segment { version: u8, stream_id: String, ordinal: u64, sha256: String, source: Timing, native_before_ns: u64, native_after_ns: u64 },
    #[serde(rename="end_ack", rename_all="camelCase")]
    End { version: u8, stream_id: String, segments: u64, origin: EndOrigin, preroll: Preroll, native_before_ns: u64, native_after_ns: u64 },
    #[serde(rename="source_observation", rename_all="camelCase")]
    Observation { version: u8, nonce: String, native_before_ns: u64, native_after_ns: u64, sample: Option<ChzzkSample>, quality: String },
}
struct State {
    report: RelayTimingSummary, last: Option<(u64, Timing)>, prefix_open: bool,
    observation_end: Option<u64>, last_performance: Option<f64>, last_nonce: Option<Uuid>,
    ack_end: Option<u64>, native_acks: u64, native_end: bool,
    events:soop_events::ReadState,
}
impl State {
    fn new(dialect: Dialect, incomplete: bool, native_acks: u64, native_end: bool) -> Self {
        Self { report: RelayTimingSummary { dialect, records:0, acknowledged_segments:0, prefix_segments:0,
            transport_end_90k:None, stream_id:None, origin:None, video:None, audio:None, end_seen:false,
            transport_complete:false, timing_incomplete:incomplete, sidecar_incomplete:false, observations:0, usable_observations:0,
            unknown_observations:0, preroll:None, remote_anchor:RemoteAnchor::Unknown }, last:None,
            prefix_open:true, observation_end:None, last_performance:None, last_nonce:None, ack_end:None, native_acks, native_end,events:soop_events::ReadState::default() }
    }
    fn row(&mut self, bytes: &[u8], owner: &ExpectedOwner) -> Result<()> {
        match owner {
            ExpectedOwner::Soop(o) => {
                let r: SoopRow = parse(bytes)?;
                require(r.version == 1 && r.sequence == self.report.records && r.binding == o.binding, TimingReadError::Identity)?;
                match (r.kind.as_str(), r.data) {
                    ("segment_ack", SoopData::Segment(v)) => {
                        require(v.bytes > 0 && v.bytes <= 16*1024*1024 && v.bytes % 188 == 0, TimingReadError::Schema)?;
                        self.segment(&v.stream_id, v.ordinal, &v.sha256, v.timing, v.native_before_ns, v.native_after_ns)?;
                    }
                    ("end_ack", SoopData::End(v)) => self.end(&v.stream_id,v.segments,v.origin,v.preroll,v.native_before_ns,v.native_after_ns)?,
                    ("soop_event_clock", SoopData::Event(v)) => {
                        require(!self.report.end_seen,TimingReadError::Order)?;
                        let gap=self.events.observe(&v)?;
                        if gap||!v.usable {self.report.timing_incomplete=true;}
                    }
                    ("source_observation", SoopData::Observation(v)) => {
                        require(v.quality == "presentation-observation-bracket", TimingReadError::Schema)?;
                        let b = bracket(v.native_before_ns, v.native_after_ns)?;
                        let valid = if let Some(s) = &v.sample {
                            valid_clock(s.source_time_seconds,s.current_time_seconds,s.timestamp_offset_seconds,s.performance_now_ms)?;
                            require(s.playback_rate.is_finite() && (0.0625..=16.).contains(&s.playback_rate) && s.ready_state <= 4
                                && b.is_some_and(|(a,z)| z-a <= 3_000_000_000), TimingReadError::Clock)?;
                            !s.paused && !s.seeking && s.ready_state >= 2
                        } else { false };
                        require(v.usable == valid, TimingReadError::Clock)?;
                        self.observation(&v.nonce,b,v.sample.as_ref().map(|s|s.performance_now_ms),valid)?;
                    }
                    ("observation_missing", SoopData::Missing(v)) => {
                        require(matches!(v.reason.as_str(), "timeout"|"input_ended"), TimingReadError::Schema)?;
                        self.observation(&v.nonce,bracket(v.native_before_ns,v.native_after_ns)?,None,false)?;
                    }
                    _ => return Err(TimingReadError::Schema),
                }
            }
            ExpectedOwner::Chzzk(_) => match parse::<ChzzkRow>(bytes)? {
                ChzzkRow::Segment {version,stream_id,ordinal,sha256,source,native_before_ns,native_after_ns} => {
                    require(version == 1,TimingReadError::Schema)?;
                    self.segment(&stream_id,ordinal,&sha256,source,Some(native_before_ns),Some(native_after_ns))?;
                }
                ChzzkRow::End {version,stream_id,segments,origin,preroll,native_before_ns,native_after_ns} => {
                    require(version == 1,TimingReadError::Schema)?;
                    self.end(&stream_id,segments,origin,preroll,Some(native_before_ns),Some(native_after_ns))?;
                }
                ChzzkRow::Observation {version,nonce,native_before_ns,native_after_ns,sample,quality} => {
                    require(version == 1 && quality == if sample.is_some(){"source_bound_player_observation"}else{"unknown"},TimingReadError::Schema)?;
                    let b = bracket(Some(native_before_ns),Some(native_after_ns))?;
                    let usable = if let Some(s) = &sample {
                        require(s.source_id == owner.source() && s.clock == "mse_presentation_v1" && s.track_index <= 1,TimingReadError::Identity)?;
                        valid_clock(s.source_time_seconds,s.current_time,s.timestamp_offset,s.performance_now_ms)?;
                        s.playback_usable(b)?
                    } else { false };
                    // Older numeric-only samples remain unknown. A newly
                    // qualified observation alone still does not map receipts.
                    self.observation(&nonce,b,sample.as_ref().map(|s|s.performance_now_ms),usable)?;
                }
            },
        }
        self.report.records = self.report.records.checked_add(1).ok_or(TimingReadError::Limit)?; Ok(())
    }
    fn observation(&mut self, nonce: &str, b: Option<(u64,u64)>, performance: Option<f64>, usable: bool) -> Result<()> {
        let id = uuid(nonce)?;
        require(self.last_nonce != Some(id), TimingReadError::Order)?;
        self.last_nonce = Some(id);
        if let Some((before,after)) = b {
            require(self.observation_end.is_none_or(|last| before >= last),TimingReadError::Order)?;
            self.observation_end = Some(after);
        }
        if let Some(p) = performance {
            require(self.last_performance.is_none_or(|last| p > last),TimingReadError::Clock)?;
            self.last_performance = Some(p);
        }
        self.report.observations += 1;
        if usable { self.report.usable_observations += 1; } else { self.report.unknown_observations += 1; }
        if performance.is_none() || b.is_none() { self.report.timing_incomplete = true; } Ok(())
    }
    fn ack_bracket(&mut self, before: Option<u64>, after: Option<u64>) -> Result<()> {
        if let Some((a,z)) = bracket(before,after)? {
            require(self.ack_end.is_none_or(|last| a >= last),TimingReadError::Order)?; self.ack_end = Some(z);
        } else { self.report.timing_incomplete = true; } Ok(())
    }
    fn segment(&mut self, stream: &str, ordinal: u64, hash: &str, t: Timing, before: Option<u64>, after: Option<u64>) -> Result<()> {
        require(!self.report.end_seen && ordinal < self.native_acks && hex(hash),TimingReadError::Order)?;
        let id = uuid(stream)?;
        require(self.report.stream_id.is_none_or(|s|s == id),TimingReadError::Identity)?;
        self.ack_bracket(before,after)?;
        let origin = SourceTicks { ticks:t.origin_ticks,timescale:t.origin_timescale };
        require(origin.timescale > 0 && self.report.origin.is_none_or(|o|o == origin),TimingReadError::Clock)?;
        validate_track(t.video,origin)?; validate_track(t.audio,origin)?;
        require(t.segment_end90k.checked_sub(t.segment_start90k).is_some_and(|d| d > 0 && d <= 450000)
            && t.video.min_pts90k >= t.segment_start90k && t.video.min_pts90k < t.segment_end90k
            && t.video.max_pts_end90k <= t.segment_end90k.checked_add(1).ok_or(TimingReadError::Clock)?
            && t.audio.min_pts == t.audio.first_dts && t.audio.max_pts_end == t.audio.last_dts_end,TimingReadError::Clock)?;
        if let Some((old,prev)) = self.last {
            require(ordinal > old && t.video.timescale == prev.video.timescale && t.audio.timescale == prev.audio.timescale,TimingReadError::Order)?;
            let adjacent = old.checked_add(1) == Some(ordinal);
            require(if adjacent { t.video.first_dts == prev.video.last_dts_end && t.audio.first_dts == prev.audio.last_dts_end && t.segment_start90k == prev.segment_end90k }
                else { t.video.first_dts >= prev.video.last_dts_end && t.audio.first_dts >= prev.audio.last_dts_end && t.segment_start90k >= prev.segment_end90k },TimingReadError::Clock)?;
            self.prefix_open &= adjacent;
        } else {
            self.prefix_open = ordinal == 0;
            if ordinal == 0 { require(t.segment_start90k == 0,TimingReadError::Clock)?; }
        }
        self.report.stream_id = Some(id); self.report.origin = Some(origin);
        self.report.acknowledged_segments += 1;
        if self.prefix_open {
            append_envelope(&mut self.report.video,t.video)?; append_envelope(&mut self.report.audio,t.audio)?;
            self.report.prefix_segments += 1; self.report.transport_end_90k = Some(t.segment_end90k);
        } else { self.report.timing_incomplete = true; }
        self.last = Some((ordinal,t)); Ok(())
    }
    fn end(&mut self, stream: &str, count: u64, origin: EndOrigin, preroll: Preroll, before: Option<u64>, after: Option<u64>) -> Result<()> {
        require(!self.report.end_seen && self.native_end && count == self.native_acks && count > 0,TimingReadError::Order)?;
        let id = uuid(stream)?;
        require(self.report.stream_id.is_none_or(|s|s == id),TimingReadError::Identity)?;
        self.ack_bracket(before,after)?;
        require(origin.timescale > 0,TimingReadError::Clock)?;
        if let Some((last,t)) = self.last {
            require(last < count && origin.ticks == t.origin_ticks && origin.timescale == t.origin_timescale,TimingReadError::Clock)?;
            for (scale,base,remainder) in [(t.video.timescale,origin.video_base,origin.video_remainder),(t.audio.timescale,origin.audio_base,origin.audio_remainder)] {
                let product = u128::from(origin.ticks)*u128::from(scale);
                require(product/u128::from(origin.timescale) == u128::from(base) && product%u128::from(origin.timescale) == u128::from(remainder),TimingReadError::Clock)?;
            }
        }
        self.report.end_seen = true; self.report.preroll = Some(preroll);
        self.report.transport_complete = self.report.prefix_segments == count;
        Ok(())
    }
}
fn bracket(before: Option<u64>, after: Option<u64>) -> Result<Option<(u64,u64)>> {
    match (before,after) { (Some(a),Some(b)) => { require(a <= b,TimingReadError::Clock)?; Ok(Some((a,b))) }, _ => Ok(None) }
}
fn valid_clock(source: f64, current: f64, offset: f64, page: f64) -> Result<()> {
    require([source,current,offset,page].iter().all(|n|n.is_finite())
        && (0.0..=1e9).contains(&source) && (0.0..=1e9).contains(&current) && offset.abs() <= 1e9
        && (0.0..=1e12).contains(&page) && (current-offset-source).abs() <= 1e-6,TimingReadError::Clock)
}
fn to90k(ticks: u64, scale: u32, origin: SourceTicks) -> Result<u64> {
    require(scale > 0 && origin.timescale > 0,TimingReadError::Clock)?;
    let value = (u128::from(ticks)*u128::from(origin.timescale))
        .checked_sub(u128::from(origin.ticks)*u128::from(scale)).ok_or(TimingReadError::Clock)?;
    let denominator = u128::from(scale)*u128::from(origin.timescale);
    let n = value.checked_mul(90000).and_then(|n|n.checked_add(denominator/2)).ok_or(TimingReadError::Clock)?;
    u64::try_from(n/denominator).map_err(|_|TimingReadError::Clock)
}
fn validate_track(t: TrackTiming, origin: SourceTicks) -> Result<()> {
    require(t.timescale > 0 && t.samples > 0 && t.first_dts < t.last_dts_end && t.min_pts < t.max_pts_end
        && t.min_pts >= t.first_dts && t.max_pts_end >= t.last_dts_end
        && t.samples <= t.last_dts_end-t.first_dts,TimingReadError::Clock)?;
    for (source,wire) in [(t.first_dts,t.first_dts90k),(t.last_dts_end,t.last_dts_end90k),(t.min_pts,t.min_pts90k),(t.max_pts_end,t.max_pts_end90k)] {
        require(to90k(source,t.timescale,origin)? == wire,TimingReadError::Clock)?;
    } Ok(())
}
fn append_envelope(out: &mut Option<SourceEnvelope>, t: TrackTiming) -> Result<()> {
    if let Some(v) = out { v.last_dts_end=t.last_dts_end; v.min_pts=v.min_pts.min(t.min_pts); v.max_pts_end=v.max_pts_end.max(t.max_pts_end); v.samples=v.samples.checked_add(t.samples).ok_or(TimingReadError::Limit)?; }
    else { *out=Some(SourceEnvelope {timescale:t.timescale,first_dts:t.first_dts,last_dts_end:t.last_dts_end,min_pts:t.min_pts,max_pts_end:t.max_pts_end,samples:t.samples}); } Ok(())
}
fn map(report: &RelayTimingSummary, track: Track, source: SourceTicks) -> Result<MappedPosition> {
    let (Some(origin),Some(range)) = (report.origin,match track {Track::Video=>report.video,Track::Audio=>report.audio}) else { return Ok(MappedPosition::Unknown); };
    require(source.timescale > 0,TimingReadError::Clock)?;
    // Compare rational coordinates without rounding at admission boundaries.
    let coordinate=u128::from(source.ticks)*u128::from(range.timescale);
    if coordinate < u128::from(range.min_pts)*u128::from(source.timescale) || coordinate >= u128::from(range.max_pts_end)*u128::from(source.timescale) { return Ok(MappedPosition::OutsideAcknowledgedPrefix); }
    Ok(MappedPosition::Transport90k(to90k(source.ticks,source.timescale,origin)?))
}
fn scan<R: BufRead>(reader: &mut R, owner: &ExpectedOwner, durable: Durable, native_acks: u64, native_end: bool, cancel: &AtomicBool) -> Result<(RelayTimingSummary,[u8;32],u64)> {
    validate_expected(owner)?;
    let max=owner.dialect().bytes();
    require(durable.records <= max/64,TimingReadError::Limit)?;
    let mut state=State::new(owner.dialect(),durable.incomplete,native_acks,native_end);
    state.report.sidecar_incomplete=durable.sidecar_incomplete;
    let mut digest=Sha256::new(); let mut total=0u64; let mut line=Vec::with_capacity(LINE_BYTES+1);
    loop {
        cancelled(cancel)?; line.clear();
        // take bounds allocation even if a corrupt file has no newline.
        let count=(&mut *reader).take((LINE_BYTES+2) as u64).read_until(b'\n',&mut line).map_err(|_|TimingReadError::Io)?;
        if count == 0 { break; }
        total=total.checked_add(count as u64).ok_or(TimingReadError::Limit)?;
        let line_limit=if owner.dialect()==Dialect::Soop {LINE_BYTES+1}else{LINE_BYTES};
        require(total <= max && count <= line_limit && state.report.records < max/64,TimingReadError::Limit)?;
        require(line.last()==Some(&b'\n') && count>1,TimingReadError::Schema)?;
        digest.update(&line);
        state.row(&line[..count-1],owner)?;
    }
    require(state.report.records == durable.records && state.report.acknowledged_segments <= native_acks,TimingReadError::Order)?;
    if !state.report.transport_complete { state.report.timing_incomplete=true; }
    Ok((state.report,digest.finalize().into(),total))
}

#[cfg(test)]
#[path="relay_timing_read_tests.rs"]
mod tests;
