//! YouTube-local, source-preserving finalization. A checked candidate is not a
//! published library item; source admission and exact publication remain gates.
pub(super) mod aac_selection;
mod publication;
pub(in crate::platform_media) use publication::{PublicationExclusion, PublicationFacts, PublicationHandoff, PublicationSourceFacts, PublicationState};
use super::{
    codec_facts,
    files::{Directories, GuardedFile},
    index::{self, Clock, Configuration, Format},
    managed_probe::ManagedProbe,
    packet_facts::{self, PacketFact, PairComparison, PairReport, Rational, Times, TrackFold},
    packet_probe, packet_tape,
    reconstruct::TrackBuilder,
    recovery::{selection::{self, CompletedSelection, SelectionBinding, SelectionFacts}, PrefixState, SequentialSource, UnitSpan},
    stream_facts::{ProbeFacts, StreamFacts},
    vp9_admission::{SelectedAdmission, SourceAdmission},
    ClockDescriptor, Error, SealCause, SpoolIdentity, Track, TrackExtent,
};
use sha2::{Digest, Sha256};
use aac_selection::{AacRetention, Partition, RetainedComparison};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const CHUNK: usize = 131_072;
const MAX_FILE: u64 = 137_438_953_472;
const MAX_PACKETS: u64 = 20_000_000;
const RESERVE: u64 = 1_073_741_824;
const HEADROOM: u64 = 16_384;
const OVERHEAD: u64 = 67_108_864;
const MAX_TIME: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media) enum Fault {
    Source(Error),
    Probe(packet_probe::Fault),
    Tape(packet_tape::Error),
    Codec(codec_facts::Fault),
    Packet(packet_facts::Fault),
    Identity,
    Path,
    Io,
    Space,
    Bounds,
    Deadline,
    Cancelled,
    UnsupportedAdmission,
    Coverage,
    Changed,
    Terminal,
}
pub(super) type Result<T> = std::result::Result<T, Fault>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnedKind {
    DerivedVideo,
    DerivedAudio,
    Candidate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnedSelection {
    DerivedVideo0,
    DerivedAudio0,
    CandidateVideo0,
    CandidateAudio1,
}

fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(Fault::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Fault::Deadline);
    }
    Ok(())
}
fn space(path: &Path, pending: u64) -> Result<()> {
    let needed = RESERVE.checked_add(pending).ok_or(Fault::Bounds)?;
    if fs2::available_space(path).map_err(|_| Fault::Space)? < needed {
        return Err(Fault::Space);
    }
    Ok(())
}
fn hash_file(file: &File, length: u64, cancel: &AtomicBool, deadline: Instant) -> Result<[u8; 32]> {
    if length == 0 || length > MAX_FILE {
        return Err(Fault::Bounds);
    }
    let mut reader = file;
    let position = reader.stream_position().map_err(|_| Fault::Io)?;
    let result = (|| {
        reader.seek(SeekFrom::Start(0)).map_err(|_| Fault::Io)?;
        let mut hash = Sha256::new();
        let mut scratch = vec![0; CHUNK];
        let mut remaining = length;
        while remaining != 0 {
            checkpoint(cancel, deadline)?;
            let n = remaining.min(CHUNK as u64) as usize;
            reader
                .read_exact(&mut scratch[..n])
                .map_err(|_| Fault::Io)?;
            hash.update(&scratch[..n]);
            remaining -= n as u64;
        }
        let mut extra = [0];
        if reader.read(&mut extra).map_err(|_| Fault::Io)? != 0 {
            return Err(Fault::Changed);
        }
        checkpoint(cancel, deadline)?;
        Ok(hash.finalize().into())
    })();
    reader
        .seek(SeekFrom::Start(position))
        .map_err(|_| Fault::Io)?;
    result
}

/// Native private constructor: never a deserialized path or caller-made file.
pub(super) struct HeldInput {
    path: PathBuf,
    kind: OwnedKind,
    file: GuardedFile,
    directories: Arc<Directories>,
    length: u64,
    hash: [u8; 32],
}
impl HeldInput {
    fn open_owned(
        path: PathBuf,
        kind: OwnedKind,
        directories: Arc<Directories>,
        expected: Option<(u64, [u8; 32])>,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Self> {
        checkpoint(cancel, deadline)?;
        directories.check().map_err(Fault::Source)?;
        let file = GuardedFile::open(&path, false).map_err(Fault::Source)?;
        let length = file.check().map_err(Fault::Source)?;
        let hash = hash_file(&file.file, length, cancel, deadline)?;
        if expected.is_some_and(|value| value != (length, hash)) {
            return Err(Fault::Changed);
        }
        let result = Self {
            path,
            kind,
            file,
            directories,
            length,
            hash,
        };
        result.check(cancel, deadline)?;
        Ok(result)
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn kind(&self) -> OwnedKind {
        self.kind
    }
    pub(super) fn len(&self) -> u64 {
        self.length
    }
    pub(super) fn sha256(&self) -> [u8; 32] {
        self.hash
    }
    pub(super) fn check(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        self.directories.check().map_err(Fault::Source)?;
        if self.file.check().map_err(Fault::Source)? != self.length {
            return Err(Fault::Changed);
        }
        checkpoint(cancel, deadline)
    }
    pub(super) fn rehash(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        self.check(cancel, deadline)?;
        if hash_file(&self.file.file, self.length, cancel, deadline)? != self.hash {
            return Err(Fault::Changed);
        }
        self.check(cancel, deadline)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileKey {
    volume: u64,
    index: u64,
}
fn file_key(file: &File) -> Result<(FileKey, u64)> {
    let meta = file.metadata().map_err(|_| Fault::Io)?;
    if !meta.is_file() {
        return Err(Fault::Path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| Fault::Path)?;
        if info.dwFileAttributes & 0x400 != 0 || info.nNumberOfLinks != 1 {
            return Err(Fault::Path);
        }
        Ok((
            FileKey {
                volume: u64::from(info.dwVolumeSerialNumber),
                index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            },
            meta.len(),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(Fault::Path);
        }
        Ok((
            FileKey {
                volume: meta.dev(),
                index: meta.ino(),
            },
            meta.len(),
        ))
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(Fault::Path)
    }
}
fn attributes(path: &Path, writable: bool) -> Result<File> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Fault::Path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Fault::Path);
    }
    let mut options = OpenOptions::new();
    options.read(true).write(writable);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(if writable { 1 } else { 3 })
            .custom_flags(0x00200000);
        if !writable {
            options.access_mode(0x80);
        }
    }
    let file = options.open(path).map_err(|_| Fault::Path)?;
    file_key(&file)?;
    Ok(file)
}
struct ObservedOutput {
    file: File,
    key: FileKey,
}

/// Exactly one fresh merge UUID in the owned job directory. The runner creates
/// it with -n; this slot never grants overwrite or arbitrary output authority.
pub(super) struct OutputSlot {
    path: PathBuf,
    directories: Arc<Directories>,
    cap: u64,
    begun: AtomicBool,
    observed: Mutex<Option<ObservedOutput>>,
}
impl OutputSlot {
    fn new(job: &Path, directories: Arc<Directories>, cap: u64) -> Result<Self> {
        if cap == 0 || cap > MAX_FILE {
            return Err(Fault::Bounds);
        }
        directories.check().map_err(Fault::Source)?;
        let path = job.join(format!("merge-{}.mp4", uuid::Uuid::new_v4()));
        if path.exists() {
            return Err(Fault::Path);
        }
        Ok(Self {
            path,
            directories,
            cap,
            begun: AtomicBool::new(false),
            observed: Mutex::new(None),
        })
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn begin(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        if self.begun.swap(true, Ordering::AcqRel) {
            return Err(Fault::Terminal);
        }
        self.directories.check().map_err(Fault::Source)?;
        match fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(Fault::Path),
        }
        space(self.path.parent().ok_or(Fault::Path)?, self.cap)?;
        checkpoint(cancel, deadline)
    }
    pub(super) fn check(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        if !self.begun.load(Ordering::Acquire) {
            return Err(Fault::Terminal);
        }
        self.directories.check().map_err(Fault::Source)?;
        let mut observed = self.observed.lock().map_err(|_| Fault::Terminal)?;
        let file = match attributes(&self.path, false) {
            Ok(file) => file,
            Err(error) => {
                if observed.is_none()
                    && fs::symlink_metadata(&self.path)
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    space(self.path.parent().ok_or(Fault::Path)?, self.cap)?;
                    return checkpoint(cancel, deadline);
                }
                return Err(error);
            }
        };
        let (key, length) = file_key(&file)?;
        if length > self.cap {
            return Err(Fault::Bounds);
        }
        if let Some(previous) = observed.as_ref() {
            if previous.key != key || file_key(&previous.file)?.0 != key {
                return Err(Fault::Changed);
            }
        } else {
            *observed = Some(ObservedOutput { file, key });
        }
        space(self.path.parent().ok_or(Fault::Path)?, self.cap - length)?;
        checkpoint(cancel, deadline)
    }
    pub(super) fn finish(self, cancel: &AtomicBool, deadline: Instant) -> Result<HeldInput> {
        self.check(cancel, deadline)?;
        let observed = self.observed.lock().map_err(|_| Fault::Terminal)?;
        let expected = observed.as_ref().ok_or(Fault::Changed)?.key;
        // The owned child is joined before this method. Sync its exact file via
        // a nontruncating handle, then acquire the immutable read holding.
        let sync = attributes(&self.path, true)?;
        if file_key(&sync)?.0 != expected {
            return Err(Fault::Changed);
        }
        sync.sync_all().map_err(|_| Fault::Io)?;
        drop(sync);
        let held = HeldInput::open_owned(
            self.path.clone(),
            OwnedKind::Candidate,
            self.directories.clone(),
            None,
            cancel,
            deadline,
        )?;
        if file_key(&held.file.file)?.0 != expected || held.len() > self.cap {
            return Err(Fault::Changed);
        }
        Ok(held)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceScope {
    identity: SpoolIdentity,
    clock: ClockDescriptor,
    tracks: [TrackExtent; 2],
    receipts: u64,
    state: PrefixState,
    seal_cause: Option<SealCause>,
    incomplete_append: bool,
    torn_journal: bool,
    uncommitted_bytes: [u64; 2],
}
impl SourceScope {
    fn from_span(span: &UnitSpan<'_>) -> Self {
        let f = span.facts();
        Self {
            identity: f.identity.clone(),
            clock: f.clock.clone(),
            tracks: (*f.tracks).clone(),
            receipts: f.receipts,
            state: f.state,
            seal_cause: f.seal_cause,
            incomplete_append: f.incomplete_append,
            torn_journal: f.torn_journal,
            uncommitted_bytes: f.uncommitted_bytes,
        }
    }
}

fn selection_error(error: selection::Fault) -> Fault {
    match error {
        selection::Fault::Source(Error::Cancelled) => Fault::Cancelled,
        selection::Fault::Source(error) => Fault::Source(error),
        selection::Fault::Overflow => Fault::Bounds,
        selection::Fault::Poisoned => Fault::Terminal,
        selection::Fault::Changed => Fault::Changed,
        _ => Fault::Coverage,
    }
}

// These two local cases share the verifier, not their admission authority.
enum FinalizerSource {
    Whole(SequentialSource),
    Selected(CompletedSelection),
}
impl FinalizerSource {
    fn check(&self, cancel: &AtomicBool) -> Result<()> {
        match self {
            Self::Whole(source) => source.check(cancel).map_err(Fault::Source),
            Self::Selected(source) => source.check(cancel).map_err(selection_error),
        }
    }
    fn binding(&self, cancel: &AtomicBool) -> Result<Option<SelectionBinding>> {
        match self {
            Self::Whole(source) => { source.check(cancel).map_err(Fault::Source)?; Ok(None) }
            Self::Selected(source) => source.binding(cancel).map(Some).map_err(selection_error),
        }
    }
}
enum FinalizerAdmission<'a> {
    Whole(&'a SourceAdmission),
    Selected(&'a SelectedAdmission),
}
impl FinalizerAdmission<'_> {
    fn check_source(&self, source: &FinalizerSource, cancel: &AtomicBool) -> Result<()> {
        match (self, source) {
            (Self::Whole(_), FinalizerSource::Whole(source)) => source.check(cancel).map_err(Fault::Source),
            (Self::Selected(admission), FinalizerSource::Selected(source)) => admission.check_source(source, cancel).map_err(|_| Fault::UnsupportedAdmission),
            _ => Err(Fault::UnsupportedAdmission),
        }
    }
    fn check_span(&self, span: &UnitSpan<'_>) -> Result<()> {
        match self {
            Self::Whole(admission) => admission.check_span(span),
            Self::Selected(admission) => admission.check_span(span),
        }.map_err(|_| Fault::UnsupportedAdmission)
    }
}

/// Keeps historical source exclusions/partial state distinct from proof of the
/// selected candidate. Neither pending flag on the native selection is erased.
pub(super) struct SelectedEvidence {
    binding: SelectionBinding,
    facts: SelectionFacts,
    audio: AacRetention,
}
impl SelectedEvidence {
    pub(super) fn facts(&self) -> &SelectionFacts { &self.facts }
    pub(super) fn audio(&self) -> &AacRetention { &self.audio }
}

pub(super) struct ExpectedCandidate<'a> {
    pub identity: &'a SpoolIdentity,
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub origin: Rational,
    pub comparisons: &'a [PairReport; 2],
    pub state: PrefixState,
    pub seal_cause: Option<SealCause>,
    pub incomplete_append: bool,
    pub torn_journal: bool,
    pub uncommitted_bytes: [u64; 2],
    pub selected: Option<&'a SelectedEvidence>,
}
/// Not serializable/Clone and not a production admission constructor. A caller
/// must consume a separately approved publication handoff, never a bare path.
pub(in crate::platform_media) struct VerifiedCandidate {
    source: FinalizerSource,
    scope: SourceScope,
    candidate: HeldInput,
    origin: Rational,
    comparisons: [PairReport; 2],
    duration: Rational,
    selected: Option<SelectedEvidence>,
    _attempt: Arc<Directories>,
}
impl VerifiedCandidate {
    pub(in crate::platform_media) fn raw_timeline(
        &self, cancel: &AtomicBool,
    ) -> Option<super::raw_anchor::RawTimelineFacts> {
        self.check_source(cancel).ok()?;
        match &self.source {
            FinalizerSource::Selected(source) => source.raw_timeline(self.origin, cancel),
            FinalizerSource::Whole(_) => None,
        }
    }
    pub(super) fn with_verified_path<T>(
        &self,
        cancel: &AtomicBool,
        deadline: Instant,
        work: impl FnOnce(&Path, ExpectedCandidate<'_>) -> Result<T>,
    ) -> Result<T> {
        checkpoint(cancel, deadline)?;
        self.check_source(cancel)?;
        self.candidate.rehash(cancel, deadline)?;
        let value = work(
            self.candidate.path(),
            ExpectedCandidate {
                identity: &self.scope.identity,
                bytes: self.candidate.len(),
                sha256: self.candidate.sha256(),
                origin: self.origin,
                comparisons: &self.comparisons,
                state: self.scope.state,
                seal_cause: self.scope.seal_cause,
                incomplete_append: self.scope.incomplete_append,
                torn_journal: self.scope.torn_journal,
                uncommitted_bytes: self.scope.uncommitted_bytes,
                selected: self.selected.as_ref(),
            },
        )?;
        self.candidate.rehash(cancel, deadline)?;
        self.check_source(cancel)?;
        checkpoint(cancel, deadline)?;
        Ok(value)
    }
    fn check_source(&self, cancel: &AtomicBool) -> Result<()> {
        check_source_evidence(&self.source, &self.selected, cancel)
    }
}
fn check_source_evidence(source: &FinalizerSource, selected: &Option<SelectedEvidence>, cancel: &AtomicBool) -> Result<()> {
        source.check(cancel)?;
        match (source, selected) {
            (FinalizerSource::Whole(_), None) => Ok(()),
            (FinalizerSource::Selected(source), Some(evidence)) => {
                if source.binding(cancel).map_err(selection_error)? != evidence.binding
                    || source.facts() != &evidence.facts { return Err(Fault::Changed); }
                evidence.audio.check_binding(&evidence.binding)
            }
            _ => Err(Fault::Identity),
        }
}

fn visit(
    source: &mut FinalizerSource,
    track: Track,
    cancel: &AtomicBool,
    mut work: impl for<'a> FnMut(UnitSpan<'a>) -> Result<()>,
) -> Result<()> {
    let mut failure = None;
    let result = match source {
        FinalizerSource::Whole(source) => source.visit(track, cancel, |span| match work(span) {
            Ok(()) => Ok(()),
            Err(error) => { failure = Some(error); Err(Error::Proof) }
        }).map_err(Fault::Source),
        FinalizerSource::Selected(source) => source.visit(track, cancel, |span| match work(span) {
            Ok(()) => Ok(()),
            Err(error) => { failure = Some(error); Err(Error::Proof) }
        }).map_err(selection_error),
    };
    if let Some(error) = failure {
        return Err(error);
    }
    result
}

#[derive(Clone)]
struct TrackPlan {
    configuration: Configuration,
    clock: Clock,
    units: u64,
    items: u64,
}
fn preflight(
    source: &mut FinalizerSource,
    identity: &SpoolIdentity,
    admission: &FinalizerAdmission<'_>,
    job: &Path,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(SourceScope, [TrackPlan; 2])> {
    admission.check_source(source, cancel)?;
    let job_directories = Directories::hold(job).map_err(Fault::Source)?;
    let raw_keys = [attributes(&job.join(super::schema::RAW[0]), false)?, attributes(&job.join(super::schema::RAW[1]), false)?];
    let mut scope = None;
    let mut plans: [Option<TrackPlan>; 2] = [None, None];
    for track in [Track::Video, Track::Audio] {
        let mut root_checked = false;
        visit(source, track, cancel, |mut span| {
            checkpoint(cancel, deadline)?;
            if !root_checked {
                check_span_root(&mut span, &raw_keys[track.index()], &job_directories)?;
                root_checked = true;
            }
            admission
                .check_span(&span)
                .map_err(|_| Fault::UnsupportedAdmission)?;
            let candidate = SourceScope::from_span(&span);
            if &candidate.identity != identity
                || scope.as_ref().is_some_and(|known| known != &candidate)
            {
                return Err(Fault::Identity);
            }
            if scope.is_none() {
                scope = Some(candidate);
            }
            let unit = &span.facts().unit.facts;
            let entry = &mut plans[track.index()];
            if let Some(plan) = entry {
                if plan.configuration != unit.configuration
                    || plan.clock.end != unit.clock.first
                    || plan.clock.ticks_per_second != unit.clock.ticks_per_second
                {
                    return Err(Fault::Coverage);
                }
                plan.clock.last = unit.clock.last;
                plan.clock.end = unit.clock.end;
                plan.units = plan.units.checked_add(1).ok_or(Fault::Bounds)?;
                plan.items = plan.items.checked_add(unit.items).ok_or(Fault::Bounds)?;
            } else {
                *entry = Some(TrackPlan {
                    configuration: unit.configuration.clone(),
                    clock: unit.clock,
                    units: 1,
                    items: unit.items,
                });
            }
            let plan = entry.as_ref().ok_or(Fault::Coverage)?;
            if plan.units > 100_000 || plan.items == 0 || plan.items > MAX_PACKETS {
                return Err(Fault::Bounds);
            }
            Ok(())
        })?;
    }
    let scope = scope.ok_or(Fault::Coverage)?;
    let [video, audio] = plans;
    let plans = [video.ok_or(Fault::Coverage)?, audio.ok_or(Fault::Coverage)?];
    if let Some(binding) = source.binding(cancel)? {
        binding.check_coverage(plans[0].clock, plans[1].clock).map_err(selection_error)?;
    } else {
        coverage(plans[0].clock, plans[1].clock)?;
    }
    job_directories.check().map_err(Fault::Source)?;
    Ok((scope, plans))
}

fn check_span_root(span: &mut UnitSpan<'_>, expected: &File, directories: &Directories) -> Result<()> {
    directories.check().map_err(Fault::Source)?;
    span.check().map_err(Fault::Source)?;
    // Metadata only: no second data reader, clone or shared-cursor movement.
    if file_key(span.take_reader().map_err(Fault::Source)?)? != file_key(expected)? {
        return Err(Fault::Identity);
    }
    span.check().map_err(Fault::Source)?;
    directories.check().map_err(Fault::Source)
}

fn rational_cmp(a: Rational, b: Rational) -> Result<std::cmp::Ordering> {
    let lhs = a
        .numerator
        .checked_mul(b.denominator)
        .ok_or(Fault::Bounds)?;
    let rhs = b
        .numerator
        .checked_mul(a.denominator)
        .ok_or(Fault::Bounds)?;
    Ok(lhs.cmp(&rhs))
}
fn seconds(value: i128, scale: u64) -> Result<Rational> {
    Rational::new(value, i128::from(scale)).map_err(Fault::Packet)
}
fn coverage(video: Clock, audio: Clock) -> Result<()> {
    if video.first > video.last
        || video.last >= video.end
        || audio.first > audio.last
        || audio.last >= audio.end
        || video.ticks_per_second == 0
        || audio.ticks_per_second == 0
    {
        return Err(Fault::Coverage);
    }
    if rational_cmp(
        seconds(audio.first, audio.ticks_per_second)?,
        seconds(video.first, video.ticks_per_second)?,
    )?
    .is_gt()
        || rational_cmp(
            seconds(audio.end, audio.ticks_per_second)?,
            seconds(video.end, video.ticks_per_second)?,
        )?
        .is_lt()
    {
        return Err(Fault::Coverage);
    }
    Ok(())
}

fn tape_size(items: u64) -> Result<u64> {
    if items == 0 || items > MAX_PACKETS {
        return Err(Fault::Bounds);
    }
    items
        .checked_mul(80)
        .and_then(|n| n.checked_add(8))
        .ok_or(Fault::Bounds)
}
fn budgets(scope: &SourceScope, plans: &[TrackPlan; 2]) -> Result<([u64; 2], u64, u64)> {
    let raw = scope.tracks[0]
        .bytes
        .checked_add(scope.tracks[1].bytes)
        .ok_or(Fault::Bounds)?;
    if raw == 0 || raw > MAX_FILE {
        return Err(Fault::Bounds);
    }
    let rows = plans[0]
        .items
        .checked_add(plans[1].items)
        .ok_or(Fault::Bounds)?;
    let cap = raw
        .checked_add(rows.checked_mul(32).ok_or(Fault::Bounds)?)
        .and_then(|n| n.checked_add(OVERHEAD))
        .filter(|n| *n <= MAX_FILE)
        .ok_or(Fault::Bounds)?;
    let tapes = [tape_size(plans[0].items)?, tape_size(plans[1].items)?];
    let pending = raw
        .checked_add(cap)
        .and_then(|n| n.checked_add(tapes[0]))
        .and_then(|n| n.checked_add(tapes[1]))
        .and_then(|n| n.checked_add(HEADROOM))
        .ok_or(Fault::Bounds)?;
    Ok((tapes, cap, pending))
}

/// Bounded sequential writes into a new file only. An I/O failure is sticky;
/// it never becomes a successful flush or a reusable partial output.
struct BudgetWriter<'a> {
    file: &'a GuardedFile,
    directories: &'a Directories,
    directory: &'a Path,
    cancel: &'a AtomicBool,
    deadline: Instant,
    cap: u64,
    written: u64,
    future: u64,
    checked_at: u64,
    checked_time: Instant,
    failure: Option<Fault>,
}
impl<'a> BudgetWriter<'a> {
    fn new(
        file: &'a GuardedFile,
        directories: &'a Directories,
        directory: &'a Path,
        cancel: &'a AtomicBool,
        deadline: Instant,
        cap: u64,
        future: u64,
    ) -> Result<Self> {
        if cap == 0 || cap > MAX_FILE || file.check().map_err(Fault::Source)? != 0 {
            return Err(Fault::Bounds);
        }
        let mut value = Self {
            file,
            directories,
            directory,
            cancel,
            deadline,
            cap,
            written: 0,
            future,
            checked_at: 0,
            checked_time: Instant::now(),
            failure: None,
        };
        value.inspect()?;
        Ok(value)
    }
    fn inspect(&mut self) -> Result<()> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = (|| {
            checkpoint(self.cancel, self.deadline)?;
            self.directories.check().map_err(Fault::Source)?;
            if self.file.check().map_err(Fault::Source)? != self.written {
                return Err(Fault::Changed);
            }
            space(
                self.directory,
                (self.cap - self.written)
                    .checked_add(self.future)
                    .ok_or(Fault::Bounds)?,
            )?;
            self.checked_at = self.written;
            self.checked_time = Instant::now();
            Ok(())
        })();
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }
    fn complete(&mut self) -> Result<()> {
        self.inspect()?;
        if let Err(_) = self.file.file.sync_all() {
            self.failure = Some(Fault::Io);
            return Err(Fault::Io);
        }
        self.inspect()
    }
    fn error(&self) -> Option<Fault> {
        self.failure
    }
}
impl Write for BudgetWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let result = (|| {
            if let Some(error) = self.failure {
                return Err(error);
            }
            checkpoint(self.cancel, self.deadline)?;
            let next = self
                .written
                .checked_add(bytes.len() as u64)
                .filter(|n| *n <= self.cap)
                .ok_or(Fault::Bounds)?;
            if self.written - self.checked_at >= 1_048_576
                || self.checked_time.elapsed() >= Duration::from_millis(250)
            {
                self.inspect()?;
            }
            let mut writer = &self.file.file;
            for part in bytes.chunks(CHUNK) {
                checkpoint(self.cancel, self.deadline)?;
                writer.write_all(part).map_err(|_| Fault::Io)?;
            }
            self.written = next;
            checkpoint(self.cancel, self.deadline)?;
            Ok(bytes.len())
        })();
        match result {
            Ok(n) => Ok(n),
            Err(error) => {
                self.failure = Some(error);
                Err(std::io::Error::other("owned finalizer write failed"))
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inspect()
            .map_err(|_| std::io::Error::other("owned finalizer flush failed"))
    }
}

struct Tape {
    file: GuardedFile,
    directories: Arc<Directories>,
    length: u64,
    hash: [u8; 32],
    summary: packet_tape::Summary,
}
impl Tape {
    fn check(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        self.directories.check().map_err(Fault::Source)?;
        if self.file.check().map_err(Fault::Source)? != self.length {
            return Err(Fault::Changed);
        }
        Ok(())
    }
    fn rehash(&self, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        self.check(cancel, deadline)?;
        if hash_file(&self.file.file, self.length, cancel, deadline)? != self.hash {
            return Err(Fault::Changed);
        }
        self.check(cancel, deadline)
    }
}

/// Constructed only at the finalizer's all-packet derived equality boundary.
/// The runner can inspect this proof, but cannot mint or swap its held input.
pub(super) struct SelectedRemux<'a> {
    audio_key: FileKey,
    audio_length: u64,
    audio_hash: [u8; 32],
    binding: &'a SelectionBinding,
    summary: &'a packet_tape::Summary,
    plan: &'a AacRetention,
    origin: Rational,
}
impl<'a> SelectedRemux<'a> {
    fn new(
        audio: &HeldInput,
        binding: &'a SelectionBinding,
        summary: &'a packet_tape::Summary,
        plan: &'a AacRetention,
        origin: Rational,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Self> {
        audio.check(cancel, deadline)?;
        let value = Self {
            audio_key: file_key(&audio.file.file)?.0,
            audio_length: audio.len(),
            audio_hash: audio.sha256(),
            binding, summary, plan, origin,
        };
        value.check_audio(audio, cancel, deadline)?;
        Ok(value)
    }
    pub(super) fn check_audio(&self, audio: &HeldInput, cancel: &AtomicBool, deadline: Instant) -> Result<()> {
        checkpoint(cancel, deadline)?;
        audio.check(cancel, deadline)?;
        self.plan.check_binding(self.binding)?;
        let (first, end) = self.plan.ordinals();
        let count = self.summary.facts().packets.count;
        if audio.kind() != OwnedKind::DerivedAudio
            || file_key(&audio.file.file)? != (self.audio_key, self.audio_length)
            || audio.len() != self.audio_length || audio.sha256() != self.audio_hash
            || self.plan.input() != self.summary || count == 0 || count > MAX_PACKETS
            || first >= end || end > count
            || end.checked_sub(first) != Some(self.plan.retained().count)
            || self.origin.numerator < 0 || self.origin.denominator <= 0
        { return Err(Fault::Changed); }
        checkpoint(cancel, deadline)
    }
    pub(super) fn origin(&self) -> Rational { self.origin }
    pub(super) fn ordinals(&self) -> (u64, u64) { self.plan.ordinals() }
}

fn codecs(
    source: &mut FinalizerSource,
    admission: &FinalizerAdmission<'_>,
    track: Track,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<codec_facts::SourceFacts> {
    let mut result = None;
    visit(source, track, cancel, |span| {
        checkpoint(cancel, deadline)?;
        admission
            .check_span(&span)
            .map_err(|_| Fault::UnsupportedAdmission)?;
        let evidence =
            packet_probe::probe_codec_span(span, tool, cancel, deadline).map_err(Fault::Probe)?;
        if result
            .as_ref()
            .is_some_and(|previous| previous != &evidence.codec)
        {
            return Err(Fault::Changed);
        }
        if result.is_none() {
            result = Some(evidence.codec);
        }
        Ok(())
    })?;
    result.ok_or(Fault::Coverage)
}

/// Known non-reordered subset. Video duration quantization is derived from the
/// exact native DefaultDuration/TimestampScale, not a blanket timing epsilon.
struct PacketClock {
    track: Track,
    native: Clock,
    tick: u64,
    duration: u64,
    items: u64,
    count: u64,
    first: Option<Times>,
    last: Option<Times>,
    selected_audio_first: Option<u32>,
}
impl PacketClock {
    fn new(track: Track, unit: &index::UnitDraft) -> Result<Self> {
        let tick = if track == Track::Video {
            unit.configuration.timebase
        } else {
            1
        };
        let duration = if track == Track::Video {
            unit.configuration
                .default_duration_ns
                .ok_or(Fault::Coverage)?
        } else {
            0
        };
        if tick == 0 || (track == Track::Video && duration < tick) {
            return Err(Fault::Coverage);
        }
        Ok(Self {
            track,
            native: unit.clock,
            tick,
            duration,
            items: unit.items,
            count: 0,
            first: None,
            last: None,
            selected_audio_first: None,
        })
    }
    fn require_selected_audio_first(&mut self, unit: &index::UnitDraft) -> Result<()> {
        let index::ReconstructionFacts::Aac(native) = unit.reconstruction else { return Err(Fault::Coverage); };
        if self.track != Track::Audio || self.count != 0 || native.first_sample_duration == 0 {
            return Err(Fault::Coverage);
        }
        self.selected_audio_first = Some(native.first_sample_duration);
        Ok(())
    }
    fn push(&mut self, packet: PacketFact) -> Result<()> {
        let record = packet.encode_record().map_err(Fault::Packet)?;
        let time = Times::from(packet);
        if record[0] != self.track.index() as u8
            || time.pts < 0
            || time.pts != time.dts
            || time.duration <= 0
            || record[68..78].iter().any(|b| *b != 0)
        {
            return Err(Fault::Coverage);
        }
        if self.count == 0 && self.selected_audio_first.is_some_and(|native| i64::from(native) != time.duration) {
            return Err(Fault::Coverage);
        }
        if self.track == Track::Video {
            let expected = if self.count.checked_add(1) == Some(self.items) {
                self.tail_ticks()?
            } else {
                i128::from(self.duration / self.tick)
            };
            if i128::from(time.duration) != expected {
                return Err(Fault::Coverage);
            }
        }
        if let Some(last) = self.last {
            let advance = time.pts.checked_sub(last.pts).ok_or(Fault::Bounds)?;
            if self.track == Track::Audio {
                if advance != last.duration {
                    return Err(Fault::Coverage);
                }
            } else {
                let lower = self.duration / self.tick;
                let upper = lower + u64::from(self.duration % self.tick != 0);
                if advance < 0 || !(lower..=upper).contains(&(advance as u64)) {
                    return Err(Fault::Coverage);
                }
            }
        }
        self.count = self
            .count
            .checked_add(1)
            .filter(|n| *n <= self.items)
            .ok_or(Fault::Bounds)?;
        self.first.get_or_insert(time);
        self.last = Some(time);
        Ok(())
    }
    fn tail_ticks(&self) -> Result<i128> {
        let native = self
            .native
            .end
            .checked_sub(self.native.last)
            .filter(|n| *n > 0)
            .ok_or(Fault::Coverage)?;
        let ticks = native / i128::from(self.tick);
        if ticks <= 0 {
            return Err(Fault::Coverage);
        }
        Ok(ticks)
    }
    fn finish(&self) -> Result<()> {
        let first = self.first.ok_or(Fault::Coverage)?;
        let last = self.last.ok_or(Fault::Coverage)?;
        let native_tick = i128::from(self.tick);
        if self.count != self.items
            || i128::from(first.pts).checked_mul(native_tick) != Some(self.native.first)
            || i128::from(last.pts).checked_mul(native_tick) != Some(self.native.last)
        {
            return Err(Fault::Coverage);
        }
        if i128::from(last.duration) != self.tail_ticks()? {
            return Err(Fault::Coverage);
        }
        Ok(())
    }
}

fn write_tape(
    source: &mut FinalizerSource,
    admission: &FinalizerAdmission<'_>,
    track: Track,
    path: &Path,
    directories: Arc<Directories>,
    cap: u64,
    future: u64,
    expected: &codec_facts::SourceFacts,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
    mut partition: Option<&mut Partition<'_>>,
) -> Result<Tape> {
    if partition.is_some() && track != Track::Audio { return Err(Fault::Identity); }
    let file = GuardedFile::open(path, true).map_err(Fault::Source)?;
    let mut output = BudgetWriter::new(
        &file,
        &directories,
        path.parent().ok_or(Fault::Path)?,
        cancel,
        deadline,
        cap,
        future,
    )?;
    let mut tape = packet_tape::Writer::new(&mut output, track).map_err(Fault::Tape)?;
    let mut fold = TrackFold::new(track);
    let mut stream = None;
    let mut previous_end = None;
    let result = visit(source, track, cancel, |span| {
        checkpoint(cancel, deadline)?;
        admission
            .check_span(&span)
            .map_err(|_| Fault::UnsupportedAdmission)?;
        let mut clock = PacketClock::new(track, &span.facts().unit.facts)?;
        if partition.is_some() {
            clock.require_selected_audio_first(&span.facts().unit.facts)?;
        }
        let mut callback_error = None;
        let probe = packet_probe::probe_span(span, tool, cancel, deadline, |packet| {
            let result = (|| {
                clock.push(packet)?;
                if track == Track::Audio {
                    let time = Times::from(packet);
                    if previous_end.is_some_and(|end| end != time.pts) {
                        return Err(Fault::Coverage);
                    }
                    previous_end = Some(time.pts.checked_add(time.duration).ok_or(Fault::Bounds)?);
                }
                fold.push(packet).map_err(Fault::Packet)?;
                if let Some(partition) = partition.as_deref_mut() { partition.push(packet)?; }
                tape.push(packet).map_err(Fault::Tape)
            })();
            if let Err(error) = result {
                callback_error = Some(error);
                return Err(packet_facts::Fault::Sink);
            }
            Ok(())
        });
        if let Some(error) = callback_error {
            return Err(error);
        }
        let probe = probe.map_err(Fault::Probe)?;
        clock.finish()?;
        if probe.stream.time_base != expected.time_base()
            || stream.as_ref().is_some_and(|old| old != &probe.stream)
        {
            return Err(Fault::Changed);
        }
        if stream.is_none() {
            stream = Some(probe.stream);
        }
        Ok(())
    });
    if let Err(error) = result {
        drop(tape);
        return Err(output.error().unwrap_or(error));
    }
    let facts = ProbeFacts {
        packets: fold.finish().map_err(Fault::Packet)?,
        stream: stream.ok_or(Fault::Coverage)?,
    };
    let summary = tape.finish(&facts).map_err(Fault::Tape)?;
    drop(tape);
    output.complete()?;
    let length = output.written;
    drop(output);
    if length != cap {
        return Err(Fault::Changed);
    }
    let hash = hash_file(&file.file, length, cancel, deadline)?;
    drop(file);
    let file = GuardedFile::open(path, false).map_err(Fault::Source)?;
    let result = Tape {
        file,
        directories,
        length,
        hash,
        summary,
    };
    result.rehash(cancel, deadline)?;
    Ok(result)
}

fn write_derived(
    source: &mut FinalizerSource,
    admission: &FinalizerAdmission<'_>,
    track: Track,
    path: &Path,
    directories: Arc<Directories>,
    cap: u64,
    future: u64,
    plan: &TrackPlan,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<HeldInput> {
    let file = GuardedFile::open(path, true).map_err(Fault::Source)?;
    let mut output = BudgetWriter::new(
        &file,
        &directories,
        path.parent().ok_or(Fault::Path)?,
        cancel,
        deadline,
        cap,
        future,
    )?;
    let binding = source.binding(cancel)?;
    let mut builder = match binding.as_ref() {
        Some(binding) => TrackBuilder::for_selection(track, &mut output, binding),
        None => TrackBuilder::new(track, &mut output),
    };
    let result = visit(source, track, cancel, |span| {
        checkpoint(cancel, deadline)?;
        admission
            .check_span(&span)
            .map_err(|_| Fault::UnsupportedAdmission)?;
        builder.push(span, cancel).map_err(Fault::Source)
    });
    if let Err(error) = result {
        drop(builder);
        return Err(output.error().unwrap_or(error));
    }
    let summary = builder.finish(cancel).map_err(Fault::Source)?;
    drop(builder);
    output.complete()?;
    if output.written != summary.bytes || summary.units != plan.units || summary.clock != plan.clock
    {
        return Err(Fault::Changed);
    }
    drop(output);
    drop(file);
    let held = HeldInput::open_owned(
        path.to_owned(),
        if track == Track::Video {
            OwnedKind::DerivedVideo
        } else {
            OwnedKind::DerivedAudio
        },
        directories,
        Some((summary.bytes, summary.sha256)),
        cancel,
        deadline,
    )?;
    let mut reader = &held.file.file;
    reader.seek(SeekFrom::Start(0)).map_err(|_| Fault::Io)?;
    let mut count = 0;
    let mut mismatch = false;
    let outcome = index::scan(
        plan.configuration.format,
        &mut reader,
        held.length,
        plan.configuration.timebase,
        cancel,
        &mut |unit| {
            count += 1;
            if count != 1
                || unit.facts.configuration != plan.configuration
                || unit.facts.clock != plan.clock
                || unit.facts.items != plan.items
            {
                mismatch = true;
                return Err(());
            }
            Ok(())
        },
    );
    if mismatch || count != 1 || outcome.terminal != index::Terminal::Complete {
        return Err(Fault::Changed);
    }
    held.rehash(cancel, deadline)?;
    Ok(held)
}

fn add_rational(a: Rational, b: Rational) -> Result<Rational> {
    Rational::new(
        a.numerator
            .checked_mul(b.denominator)
            .and_then(|v| {
                b.numerator
                    .checked_mul(a.denominator)
                    .and_then(|w| v.checked_add(w))
            })
            .ok_or(Fault::Bounds)?,
        a.denominator
            .checked_mul(b.denominator)
            .ok_or(Fault::Bounds)?,
    )
    .map_err(Fault::Packet)
}
fn check_report(report: &PairReport, exact: bool) -> Result<()> {
    let bound = if exact {
        seconds(0, 1)?
    } else {
        add_rational(report.source_tick, report.output_tick)?
    };
    for value in [
        report.max_pts_error,
        report.max_dts_error,
        report.max_duration_error,
    ] {
        if value.numerator < 0 || rational_cmp(value, bound)?.is_gt() {
            return Err(Fault::Coverage);
        }
    }
    Ok(())
}
struct Comparison {
    report: PairReport,
    end: Rational,
}
impl Comparison {
    fn new(report: PairReport, last: Times) -> Result<Self> {
        let ticks = i128::from(last.pts).checked_add(i128::from(last.duration)).ok_or(Fault::Bounds)?;
        let end = Rational::new(
            ticks.checked_mul(report.output_tick.numerator).ok_or(Fault::Bounds)?,
            report.output_tick.denominator,
        ).map_err(Fault::Packet)?;
        Ok(Self { report, end })
    }
}
fn candidate_duration(ends: [Rational; 2]) -> Result<Rational> {
    for end in ends {
        if end.numerator <= 0 || end.denominator <= 0
            || rational_cmp(end, seconds(MAX_TIME.as_secs().into(), 1)?)?.is_gt() {
            return Err(Fault::Coverage);
        }
    }
    Ok(if rational_cmp(ends[0], ends[1])?.is_ge() { ends[0] } else { ends[1] })
}
fn compare_tape(
    input: &HeldInput,
    tape: &Tape,
    selection: OwnedSelection,
    output_timebase: packet_facts::TimeBase,
    origin: Rational,
    exact: bool,
    selected_candidate: bool,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Comparison> {
    tape.check(cancel, deadline)?;
    let mut file = &tape.file.file;
    file.seek(SeekFrom::Start(0)).map_err(|_| Fault::Io)?;
    let mut reader = packet_tape::Reader::new(&mut file, &tape.summary).map_err(Fault::Tape)?;
    let source = tape.summary.facts();
    let mut compare = PairComparison::new(
        source.stream.track,
        source.stream.time_base,
        output_timebase,
        origin,
    )
    .map_err(Fault::Packet)?;
    let mut callback_error = None;
    let consume = |packet| {
        let result = (|| {
            checkpoint(cancel, deadline)?;
            let original = reader
                .next_fact()
                .map_err(Fault::Tape)?
                .ok_or(Fault::Changed)?;
            compare.push(original, packet).map_err(Fault::Packet)
        })();
        if let Err(error) = result {
            callback_error = Some(error);
            return Err(packet_facts::Fault::Sink);
        }
        Ok(())
    };
    let probe = if selected_candidate {
        packet_probe::probe_selected_owned(input, tool, selection, cancel, deadline, consume)
    } else {
        packet_probe::probe_owned(input, tool, selection, cancel, deadline, consume)
    };
    if let Some(error) = callback_error {
        return Err(error);
    }
    let probe = probe.map_err(Fault::Probe)?;
    if probe.stream.time_base != output_timebase
        || probe.stream.track != source.stream.track
        || probe.packets.count != source.packets.count
        || probe.packets.digest != source.packets.digest
        || reader.next_fact().map_err(Fault::Tape)?.is_some()
    {
        #[cfg(test)]
        eprintln!("finalizer compare_tape changed: selection={selection:?} timebase={} track={} count={}/{} digest={}", probe.stream.time_base == output_timebase, probe.stream.track == source.stream.track, probe.packets.count, source.packets.count, probe.packets.digest == source.packets.digest);
        return Err(Fault::Changed);
    }
    if exact && probe.stream != source.stream {
        return Err(Fault::Changed);
    }
    let report = compare.finish().map_err(Fault::Packet)?;
    check_report(&report, exact)?;
    tape.rehash(cancel, deadline)?;
    Comparison::new(report, probe.packets.last)
}

fn origin(tapes: &[Tape; 2], plans: &[TrackPlan; 2]) -> Result<Rational> {
    let video = tapes[0].summary.facts().packets.first;
    let audio = tapes[1].summary.facts().packets.first;
    origin_times(video, audio, plans)
}
fn origin_times(video: Times, audio: Times, plans: &[TrackPlan; 2]) -> Result<Rational> {
    if video.pts != video.dts || audio.pts != audio.dts {
        return Err(Fault::Coverage);
    }
    let a = Rational::new(
        i128::from(video.pts)
            .checked_mul(i128::from(plans[0].configuration.timebase))
            .ok_or(Fault::Bounds)?,
        1_000_000_000,
    )
    .map_err(Fault::Packet)?;
    let b = seconds(i128::from(audio.pts), plans[1].configuration.timebase)?;
    Ok(if rational_cmp(a, b)?.is_le() { a } else { b })
}

fn compare_retained(
    input: &HeldInput,
    tape: &Tape,
    plan: &AacRetention,
    binding: &SelectionBinding,
    output_timebase: packet_facts::TimeBase,
    origin: Rational,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<Comparison> {
    tape.check(cancel, deadline)?;
    let mut file = &tape.file.file;
    let position = file.stream_position().map_err(|_| Fault::Io)?;
    let result = (|| {
        file.seek(SeekFrom::Start(0)).map_err(|_| Fault::Io)?;
        let mut compare = RetainedComparison::new(
            &mut file, &tape.summary, plan, binding, output_timebase, origin, cancel, deadline,
        )?;
        let mut callback_error = None;
        let probe = packet_probe::probe_selected_owned(input, tool, OwnedSelection::CandidateAudio1, cancel, deadline, |packet| {
            if let Err(error) = compare.push(packet) {
                callback_error = Some(error);
                return Err(packet_facts::Fault::Sink);
            }
            Ok(())
        });
        if let Some(error) = callback_error { return Err(error); }
        // Physical process success precedes promotion of borrowed tape evidence.
        let probe = probe.map_err(Fault::Probe)?;
        let report = compare.finish(&probe)?;
        drop(compare);
        tape.rehash(cancel, deadline)?;
        Comparison::new(report, probe.packets.last)
    })();
    file.seek(SeekFrom::Start(position)).map_err(|_| Fault::Io)?;
    result
}

pub(super) fn prepare(
    root: &Path,
    identity: &SpoolIdentity,
    admission: &SourceAdmission,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedCandidate> {
    let deadline = deadline.min(Instant::now() + MAX_TIME);
    checkpoint(cancel, deadline)?;
    identity.validate().map_err(Fault::Source)?;
    let source = SequentialSource::prepare(root, identity, cancel).map_err(Fault::Source)?;
    prepare_inner(root, identity, FinalizerSource::Whole(source), FinalizerAdmission::Whole(admission), tool, cancel, deadline)
}

pub(super) fn prepare_selected(
    root: &Path,
    source: CompletedSelection,
    admission: &SelectedAdmission,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedCandidate> {
    let deadline = deadline.min(Instant::now() + MAX_TIME);
    checkpoint(cancel, deadline)?;
    let identity = source.facts().identity.clone();
    identity.validate().map_err(Fault::Source)?;
    admission.check_source(&source, cancel).map_err(|_| Fault::UnsupportedAdmission)?;
    prepare_inner(root, &identity, FinalizerSource::Selected(source), FinalizerAdmission::Selected(admission), tool, cancel, deadline)
}

fn prepare_inner(
    root: &Path,
    identity: &SpoolIdentity,
    mut source: FinalizerSource,
    admission: FinalizerAdmission<'_>,
    tool: &ManagedProbe,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedCandidate> {
    checkpoint(cancel, deadline)?;
    admission.check_source(&source, cancel)?;
    let job = super::super::storage::job_dir(root, "youtube", &identity.job_id).map_err(|_| Fault::Path)?;
    let binding = source.binding(cancel)?;
    let (scope, plans) = preflight(&mut source, identity, &admission, &job, cancel, deadline)?;
    let (tape_caps, candidate_cap, pending) = budgets(&scope, &plans)?;
    space(&job, pending)?;
    let mut directories = Directories::hold(&job).map_err(Fault::Source)?;
    let attempt = job.join(format!("youtube-finalize-{}", uuid::Uuid::new_v4()));
    directories.create_child(&attempt).map_err(Fault::Source)?;
    let directories = Arc::new(directories);
    let video_codec = codecs(&mut source, &admission, Track::Video, tool, cancel, deadline)?;
    let audio_codec = codecs(&mut source, &admission, Track::Audio, tool, cancel, deadline)?;
    let raw = scope.tracks[0]
        .bytes
        .checked_add(scope.tracks[1].bytes)
        .ok_or(Fault::Bounds)?;
    let after_tapes = raw
        .checked_add(candidate_cap)
        .and_then(|n| n.checked_add(HEADROOM))
        .ok_or(Fault::Bounds)?;
    let video_tape = write_tape(
        &mut source,
        &admission,
        Track::Video,
        &attempt.join("video.packets"),
        directories.clone(),
        tape_caps[0],
        after_tapes.checked_add(tape_caps[1]).ok_or(Fault::Bounds)?,
        &video_codec,
        tool,
        cancel,
        deadline,
        None,
    )?;
    let mut partition = binding.as_ref().map(|binding| Partition::new(binding, audio_codec.time_base(), cancel, deadline)).transpose()?;
    let audio_tape = write_tape(
        &mut source,
        &admission,
        Track::Audio,
        &attempt.join("audio.packets"),
        directories.clone(),
        tape_caps[1],
        after_tapes,
        &audio_codec,
        tool,
        cancel,
        deadline,
        partition.as_mut(),
    )?;
    let retention = partition.as_mut().map(|partition| partition.finish(&audio_tape.summary)).transpose()?;
    drop(partition);
    let tapes = [video_tape, audio_tape];
    let common_origin = match retention.as_ref() {
        Some(plan) => origin_times(tapes[0].summary.facts().packets.first, plan.first_retained(), &plans)?,
        None => origin(&tapes, &plans)?,
    };
    let after_derived = candidate_cap.checked_add(HEADROOM).ok_or(Fault::Bounds)?;
    let video = write_derived(
        &mut source,
        &admission,
        Track::Video,
        &attempt.join("video.webm"),
        directories.clone(),
        scope.tracks[0].bytes,
        after_derived
            .checked_add(scope.tracks[1].bytes)
            .ok_or(Fault::Bounds)?,
        &plans[0],
        cancel,
        deadline,
    )?;
    let audio = write_derived(
        &mut source,
        &admission,
        Track::Audio,
        &attempt.join("audio.mp4"),
        directories.clone(),
        scope.tracks[1].bytes,
        after_derived,
        &plans[1],
        cancel,
        deadline,
    )?;
    for (input, expected, plan, tape, selection) in [
        (
            &video,
            &video_codec,
            &plans[0],
            &tapes[0],
            OwnedSelection::DerivedVideo0,
        ),
        (
            &audio,
            &audio_codec,
            &plans[1],
            &tapes[1],
            OwnedSelection::DerivedAudio0,
        ),
    ] {
        let bytes =
            packet_probe::metadata_owned(input, tool, cancel, deadline).map_err(Fault::Probe)?;
        if codec_facts::source(&bytes, &plan.configuration).map_err(Fault::Codec)? != *expected {
            return Err(Fault::Changed);
        }
        compare_tape(
            input,
            tape,
            selection,
            expected.time_base(),
            seconds(0, 1)?,
            true,
            false,
            tool,
            cancel,
            deadline,
        )?;
        source.check(cancel)?;
    }
    let output = OutputSlot::new(&job, directories.clone(), candidate_cap)?;
    match (&binding, &retention) {
        (Some(binding), Some(plan)) => {
            // Only this point mints the runner evidence: both complete derived
            // inputs have already matched every selected source packet exactly.
            let selected = SelectedRemux::new(&audio, binding, &tapes[1].summary, plan, common_origin, cancel, deadline)?;
            packet_probe::remux_selected_owned(&video, &audio, &output, &selected, tool, cancel, deadline).map_err(Fault::Probe)?;
        }
        (None, None) => packet_probe::remux_owned(&video, &audio, &output, common_origin, tool, cancel, deadline).map_err(Fault::Probe)?,
        _ => return Err(Fault::Identity),
    }
    #[cfg(test)]
    eprintln!("finalizer stage: remux completed");
    let candidate = output.finish(cancel, deadline)?;
    source.check(cancel)?;
    #[cfg(test)]
    eprintln!("finalizer stage: output/source held");
    let bytes =
        packet_probe::metadata_owned(&candidate, tool, cancel, deadline).map_err(Fault::Probe)?;
    let candidate_codec =
        codec_facts::candidate(&bytes, &video_codec, &audio_codec).map_err(Fault::Codec)?;
    #[cfg(test)]
    eprintln!("finalizer stage: candidate codecs checked");
    let comparisons = [
        compare_tape(
            &candidate,
            &tapes[0],
            OwnedSelection::CandidateVideo0,
            candidate_codec.video_time_base(),
            common_origin,
            false,
            binding.is_some(),
            tool,
            cancel,
            deadline,
        )?,
        match (&binding, &retention) {
            (Some(binding), Some(plan)) => compare_retained(&candidate, &tapes[1], plan, binding, candidate_codec.audio_time_base(), common_origin, tool, cancel, deadline)?,
            (None, None) => compare_tape(
            &candidate,
            &tapes[1],
            OwnedSelection::CandidateAudio1,
            candidate_codec.audio_time_base(),
            common_origin,
            false,
            false,
            tool,
            cancel,
            deadline,
            )?,
            _ => return Err(Fault::Identity),
        },
    ];
    let duration = candidate_duration([comparisons[0].end, comparisons[1].end])?;
    #[cfg(test)]
    eprintln!("finalizer stage: both packet comparisons checked");
    let comparisons = comparisons.map(|value| value.report);
    packet_probe::decode_owned(&candidate, tool, cancel, deadline).map_err(Fault::Probe)?;
    #[cfg(test)]
    eprintln!("finalizer stage: full decode checked");
    for tape in &tapes {
        tape.rehash(cancel, deadline)?;
    }
    video.rehash(cancel, deadline)?;
    audio.rehash(cancel, deadline)?;
    candidate.rehash(cancel, deadline)?;
    source.check(cancel)?;
    checkpoint(cancel, deadline)?;
    #[cfg(test)]
    eprintln!("finalizer stage: input/output hashes rechecked");
    let selected = match (&source, binding, retention) {
        (FinalizerSource::Selected(source), Some(binding), Some(audio)) => {
            if source.binding(cancel).map_err(selection_error)? != binding { return Err(Fault::Changed); }
            audio.check_binding(&binding)?;
            Some(SelectedEvidence { binding, facts: source.facts().clone(), audio })
        }
        (FinalizerSource::Whole(_), None, None) => None,
        _ => return Err(Fault::Identity),
    };
    Ok(VerifiedCandidate {
        source,
        scope,
        candidate,
        origin: common_origin,
        comparisons,
        duration,
        selected,
        _attempt: directories,
    })
}

#[cfg(test)]
mod tests;
