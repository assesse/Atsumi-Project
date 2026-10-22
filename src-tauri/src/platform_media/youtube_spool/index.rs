//! Dormant, read-only container facts. These records are NOT packet plans or
//! recording authority. A caller must inspect the terminal outcome after every
//! provisional sink record; neither an init hash nor a complete unit is media QA.
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::atomic::{AtomicBool, Ordering},
};

mod mp4;
#[cfg(test)]
mod tests;
mod webm;

#[cfg(test)]
pub(super) fn authored_pair() -> [Vec<u8>; 2] {
    [tests::authored_video(), mp4::authored_audio()]
}

#[cfg(test)]
pub(super) fn authored_audio_durations(tick: u64, durations: &[u32]) -> Vec<u8> {
    mp4::authored_audio_durations(tick, durations)
}

pub(super) const SCRATCH: usize = 131_072;
pub(super) const METADATA: usize = 65_536;
pub(super) const MAX_ITEM: u64 = 33_554_432;
const MAX_RAW: u64 = 137_438_953_472;
const MAX_UNITS: u64 = 100_000;
const MAX_ITEMS: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Vp9Webm,
    AacMp4,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Malformed,
    Unsafe,
    Unsupported,
    Clock,
    Limit,
    Io,
    Cancelled,
    Sink,
    Tail,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Error {
    pub kind: Failure,
    pub offset: u64,
}
pub(super) type Result<T> = std::result::Result<T, Error>;
pub(super) fn error(kind: Failure, offset: u64) -> Error {
    Error { kind, offset }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Boundary {
    NextInitialization,
    FiniteEnd,
    CommittedEof,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InitRelation {
    First,
    IdenticalValidatedInit,
    ChangedParsedConfiguration,
    UnprovedInitDifference,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClockRelation {
    First,
    Contiguous,
    Gap,
    OverlapOrReset,
}

/// Video clock uses exact integer nanoseconds; audio uses exact mdhd ticks.
/// This is container timing, not a VP9 decode-order/presentation proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Clock {
    pub first: i128,
    pub last: i128,
    pub end: i128,
    pub ticks_per_second: u64,
}
/// Relocation observations only. They do not authorize reconstruction, prove
/// codec equivalence, or make a provisionally emitted unit publishable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReconstructionFacts {
    Webm(WebmReconstruction),
    Aac(AacReconstruction),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WebmReconstruction {
    pub unknown_segment: bool,
    pub segment_crc: bool,
    pub info_duration: bool,
    pub cluster_position: bool,
    pub cluster_prev_size: bool,
    pub unknown_cluster_crc: bool,
    pub finite_cluster_crc: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeaderDuration {
    Zero,
    UnknownSentinel,
    Finite(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AacReconstruction {
    /// Resolved duration of the first actual trun sample, not a codec default.
    pub first_sample_duration: u32,
    pub all_fragments_default_base_is_moof: bool,
    pub first_sequence: u32,
    pub last_sequence: u32,
    pub movie_duration: HeaderDuration,
    pub track_duration: HeaderDuration,
    pub media_duration: HeaderDuration,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Configuration {
    pub format: Format,
    pub track_id: u64,
    /// WebM TimestampScaleNs or MP4 mdhd timescale, not a shared A/V ordinal.
    pub timebase: u64,
    pub width: u32,
    pub height: u32,
    pub default_duration_ns: Option<u64>,
    pub sample_rate: u32,
    pub channels: u8,
    /// Closed parsed decoder/display/default fields, canonically encoded by
    /// the source-local reader. Raw init hash remains a separate stricter fact.
    pub semantic_sha256: [u8; 32],
}
impl Configuration {
    fn fingerprint(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"atsumi-youtube-structural-config-v1\0zero-offset\0segments\0default-window\0");
        h.update([match self.format {
            Format::Vp9Webm => 0,
            Format::AacMp4 => 1,
        }]);
        h.update(self.track_id.to_be_bytes());
        h.update(self.timebase.to_be_bytes());
        h.update(self.width.to_be_bytes());
        h.update(self.height.to_be_bytes());
        h.update([u8::from(self.default_duration_ns.is_some())]);
        h.update(self.default_duration_ns.unwrap_or(0).to_be_bytes());
        h.update(self.sample_rate.to_be_bytes());
        h.update([self.channels]);
        h.update(self.semantic_sha256);
        h.finalize().into()
    }
}

/// Bounded native metadata only. No source/recording authority, frame inspection,
/// random-access proof or media-ready certificate is created by this value.
#[derive(Debug)]
pub(crate) struct RawInitialization {
    configuration: Configuration,
    length: usize,
    init_sha256: [u8; 32],
    configuration_sha256: [u8; 32],
}
impl RawInitialization {
    pub(crate) fn track(&self) -> super::Track {
        match self.configuration.format {
            Format::Vp9Webm => super::Track::Video,
            Format::AacMp4 => super::Track::Audio,
        }
    }
    /// Video: TimestampScale nanoseconds. Audio: mdhd ticks per second.
    pub(crate) fn timebase(&self) -> u64 {
        self.configuration.timebase
    }
    pub(crate) fn len(&self) -> usize {
        self.length
    }
    pub(crate) fn init_sha256(&self) -> [u8; 32] {
        self.init_sha256
    }
    pub(crate) fn configuration_sha256(&self) -> [u8; 32] {
        self.configuration_sha256
    }
}

fn inspect_initialization(
    bytes: &[u8],
    format: Format,
    cancel: &AtomicBool,
) -> std::result::Result<RawInitialization, super::Error> {
    if cancel.load(Ordering::Acquire) {
        return Err(super::Error::Cancelled);
    }
    if bytes.is_empty() {
        return Err(super::Error::Proof);
    }
    if bytes.len() > METADATA {
        return Err(super::Error::Bounds);
    }
    let mut reader = std::io::Cursor::new(bytes);
    let mut input = Input { reader: &mut reader, len: bytes.len() as u64, cancel };
    let configuration = match format {
        Format::Vp9Webm => webm::initialization(&mut input),
        Format::AacMp4 => mp4::initialization(&mut input),
    }.map_err(|error| match error.kind {
        Failure::Cancelled => super::Error::Cancelled,
        Failure::Limit => super::Error::Bounds,
        _ => super::Error::Proof,
    })?;
    let result = RawInitialization {
        configuration_sha256: configuration.fingerprint(),
        configuration,
        length: bytes.len(),
        init_sha256: Sha256::digest(bytes).into(),
    };
    if cancel.load(Ordering::Acquire) {
        return Err(super::Error::Cancelled);
    }
    Ok(result)
}

pub(crate) fn inspect_vp9_initialization(
    bytes: &[u8],
    cancel: &AtomicBool,
) -> std::result::Result<RawInitialization, super::Error> {
    inspect_initialization(bytes, Format::Vp9Webm, cancel)
}

pub(crate) fn inspect_aac_initialization(
    bytes: &[u8],
    cancel: &AtomicBool,
) -> std::result::Result<RawInitialization, super::Error> {
    inspect_initialization(bytes, Format::AacMp4, cancel)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UnitDraft {
    pub start: u64,
    pub init_end: u64,
    pub end: u64,
    pub boundary: Boundary,
    pub init_sha256: [u8; 32],
    pub configuration: Configuration,
    pub clock: Clock,
    pub items: u64,
    pub fragments: u64,
    pub rap_candidate: bool,
    pub references: u64,
    pub reconstruction: ReconstructionFacts,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Unit {
    pub ordinal: u64,
    pub facts: UnitDraft,
    pub configuration_sha256: [u8; 32],
    pub init_relation: InitRelation,
    pub clock_relation: ClockRelation,
    pub codec_packet_validation_pending: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Terminal {
    Complete,
    PartialTail { offset: u64 },
    Failed(Error),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Outcome {
    pub complete_units: u64,
    pub terminal: Terminal,
}

pub(super) struct Input<'a, R> {
    reader: &'a mut R,
    pub len: u64,
    cancel: &'a AtomicBool,
}
impl<R: Read + Seek> Input<'_, R> {
    fn check_backing_length(&mut self) -> Result<()> {
        self.checkpoint(0)?;
        let original = self
            .reader
            .stream_position()
            .map_err(|_| error(Failure::Io, 0))?;
        let backing = self
            .reader
            .seek(SeekFrom::End(0))
            .map_err(|_| error(Failure::Io, 0))?;
        self.reader
            .seek(SeekFrom::Start(original))
            .map_err(|_| error(Failure::Io, 0))?;
        self.checkpoint(0)?;
        if backing < self.len {
            Err(error(Failure::Io, backing))
        } else {
            Ok(())
        }
    }
    pub fn checkpoint(&self, at: u64) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            Err(error(Failure::Cancelled, at))
        } else {
            Ok(())
        }
    }
    pub fn bytes(&mut self, at: u64, count: usize) -> Result<Vec<u8>> {
        self.checkpoint(at)?;
        if count > SCRATCH {
            return Err(error(Failure::Limit, at));
        }
        let end = at
            .checked_add(count as u64)
            .ok_or(error(Failure::Malformed, at))?;
        if at > self.len || end > self.len {
            return Err(error(Failure::Tail, at));
        }
        self.reader
            .seek(SeekFrom::Start(at))
            .map_err(|_| error(Failure::Io, at))?;
        let mut bytes = vec![0; count];
        let mut read = 0;
        while read < count {
            self.checkpoint(at + read as u64)?;
            let n = self
                .reader
                .read(&mut bytes[read..])
                .map_err(|_| error(Failure::Io, at + read as u64))?;
            // The declared committed prefix promised these bytes. A missing
            // underlying byte is I/O/identity loss, not a legitimate raw tail.
            if n == 0 {
                return Err(error(Failure::Io, at + read as u64));
            }
            read += n;
        }
        Ok(bytes)
    }
    pub fn hash_range(&mut self, start: u64, end: u64) -> Result<[u8; 32]> {
        if start > end || end > self.len {
            return Err(error(Failure::Malformed, start));
        }
        let mut hash = Sha256::new();
        let mut at = start;
        while at < end {
            let count = (end - at).min(SCRATCH as u64) as usize;
            hash.update(self.bytes(at, count)?);
            at += count as u64;
        }
        Ok(hash.finalize().into())
    }
}

pub(super) struct Emit<'a> {
    sink: &'a mut dyn FnMut(Unit) -> std::result::Result<(), ()>,
    pub count: u64,
    items: u64,
    previous: Option<([u8; 32], [u8; 32], Clock, u64)>,
}
impl Emit<'_> {
    pub fn unit(&mut self, facts: UnitDraft) -> Result<()> {
        if facts.start >= facts.init_end
            || facts.init_end >= facts.end
            || facts.items == 0
            || facts.clock.ticks_per_second == 0
            || facts.clock.first > facts.clock.last
            || facts.clock.last >= facts.clock.end
        {
            return Err(error(Failure::Malformed, facts.start));
        }
        let total = self
            .items
            .checked_add(facts.items)
            .ok_or(error(Failure::Limit, facts.start))?;
        if self.count >= MAX_UNITS || total > MAX_ITEMS {
            return Err(error(Failure::Limit, facts.start));
        }
        let fingerprint = facts.configuration.fingerprint();
        let (init_relation, clock_relation) = match self.previous {
            None => (InitRelation::First, ClockRelation::First),
            Some((init, config, clock, end)) => {
                if end != facts.start {
                    return Err(error(Failure::Malformed, facts.start));
                }
                let relation = if init == facts.init_sha256 {
                    InitRelation::IdenticalValidatedInit
                } else if config != fingerprint {
                    InitRelation::ChangedParsedConfiguration
                } else {
                    InitRelation::UnprovedInitDifference
                };
                let lhs = clock
                    .end
                    .checked_mul(facts.clock.ticks_per_second as i128)
                    .ok_or(error(Failure::Clock, facts.start))?;
                let rhs = facts
                    .clock
                    .first
                    .checked_mul(clock.ticks_per_second as i128)
                    .ok_or(error(Failure::Clock, facts.start))?;
                let timing = if lhs == rhs {
                    ClockRelation::Contiguous
                } else if lhs < rhs {
                    ClockRelation::Gap
                } else {
                    ClockRelation::OverlapOrReset
                };
                (relation, timing)
            }
        };
        let next = (facts.init_sha256, fingerprint, facts.clock, facts.end);
        let at = facts.start;
        (self.sink)(Unit {
            ordinal: self.count,
            facts,
            configuration_sha256: fingerprint,
            init_relation,
            clock_relation,
            codec_packet_validation_pending: true,
        })
        .map_err(|_| error(Failure::Sink, at))?;
        self.count += 1;
        self.items = total;
        self.previous = Some(next);
        Ok(())
    }
}

/// Private disabled parser seam only. expected_timebase comes from a future
/// guarded phase1 descriptor, not an inferred arrival time or browser claim.
pub(super) fn scan<R: Read + Seek>(
    format: Format,
    reader: &mut R,
    committed_len: u64,
    expected_timebase: u64,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Unit) -> std::result::Result<(), ()>,
) -> Outcome {
    if committed_len > MAX_RAW || expected_timebase == 0 || expected_timebase > 1_000_000_000 {
        return Outcome {
            complete_units: 0,
            terminal: Terminal::Failed(error(Failure::Limit, 0)),
        };
    }
    let mut input = Input {
        reader,
        len: committed_len,
        cancel,
    };
    let mut emit = Emit {
        sink,
        count: 0,
        items: 0,
        previous: None,
    };
    let result = input
        .checkpoint(0)
        .and_then(|_| input.check_backing_length())
        .and_then(|_| {
            if committed_len == 0 {
                return Err(error(Failure::Tail, 0));
            }
            match format {
                Format::Vp9Webm => webm::run(&mut input, &mut emit, expected_timebase),
                Format::AacMp4 => mp4::run(&mut input, &mut emit, expected_timebase),
            }
        })
        .and_then(|_| input.checkpoint(committed_len));
    let terminal = match result {
        Ok(()) => Terminal::Complete,
        Err(Error {
            kind: Failure::Tail,
            offset,
        }) => Terminal::PartialTail { offset },
        Err(e) => Terminal::Failed(e),
    };
    Outcome {
        complete_units: emit.count,
        terminal,
    }
}

/// Inspect one already native-bound unit using the same structural walker.
/// This seam is not a source constructor. Its caller retains the UnitSpan and
/// must check the enclosing source visit before accepting any header evidence.
pub(super) fn inspect_vp9_unit<R: Read + Seek>(
    reader: &mut R,
    unit: &Unit,
    cancel: &AtomicBool,
    frames: &mut super::vp9_admission::Frames,
) -> Result<()> {
    let expected = &unit.facts;
    if expected.configuration.format != Format::Vp9Webm
        || expected.start >= expected.init_end || expected.init_end >= expected.end
        || expected.end > MAX_RAW || !unit.codec_packet_validation_pending
    {
        return Err(error(Failure::Malformed, expected.start));
    }
    let mut input = Input { reader, len: expected.end, cancel };
    input.check_backing_length()?;
    let mut matched = false;
    let mut sink = |mut found: Unit| {
        // The held span stops at a previously proved next-init boundary. The
        // one-unit rescan cannot read that successor, so only this boundary
        // representation can differ; every extent/config/clock stays exact.
        if expected.boundary == Boundary::NextInitialization
            && found.facts.boundary == Boundary::CommittedEof
        {
            found.facts.boundary = expected.boundary;
        }
        if matched || &found.facts != expected
            || found.configuration_sha256 != unit.configuration_sha256
        { return Err(()); }
        matched = true;
        Ok(())
    };
    let mut emit = Emit { sink: &mut sink, count: 0, items: 0, previous: None };
    webm::run_observed(&mut input, &mut emit, expected.configuration.timebase,
        expected.start, &mut Some(frames))?;
    input.checkpoint(expected.end)?;
    if emit.count != 1 { return Err(error(Failure::Malformed, expected.start)); }
    Ok(())
}
