use super::{
    files::{Directories, GuardedFile},
    index,
    schema::{hash, Proof, Row, JOURNAL, PROOF, RAW},
    ClockDescriptor, Error, Limits, Progress, Result, SealCause, SpoolIdentity, Track, TrackExtent,
    LIMITS,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[path = "selection.rs"]
pub(super) mod selection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrefixState {
    Sealed,
    Interrupted,
    Corrupt,
    Cancelled,
}

/// Read-only durability facts. This is deliberately not a playable-media capability.
pub(crate) struct CommittedPrefix {
    pub identity: SpoolIdentity,
    pub tracks: [TrackExtent; 2],
    pub receipts: u64,
    pub state: PrefixState,
    pub seal_cause: Option<SealCause>,
    pub incomplete_append: bool,
    /// Native journal-verified start, not an externally supplied trim boundary.
    incomplete_append_start: Option<(Track, u64)>,
    pub torn_journal: bool,
    pub uncommitted_bytes: [u64; 2],
    clock: ClockDescriptor,
    proof_length: u64,
    journal_length: u64,
    physical: [u64; 2],
    pub(super) raw: [GuardedFile; 2],
    pub(super) proof: GuardedFile,
    pub(super) journal: GuardedFile,
    pub(super) directories: Directories,
    // Optional timing evidence is held with the source, but never grants media
    // admission and cannot turn invalid video into a playable prefix.
    raw_anchor: Option<super::raw_anchor::PendingAnchor>,
}

impl CommittedPrefix {
    fn index_guards(&self, cancel: &AtomicBool) -> Result<()> {
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        self.directories.check()?;
        if self.proof.check()? != self.proof_length
            || self.journal.check()? != self.journal_length
            || self.raw[0].check()? != self.physical[0]
            || self.raw[1].check()? != self.physical[1]
        {
            return Err(Error::Corrupt);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

/// Exactly one native-indexed structural unit, not a packet/media certificate.
/// No constructor accepts a caller-owned Unit, mutable prefix, path or range.
pub(super) struct SelectedUnit {
    prefix: CommittedPrefix,
    track: Track,
    unit: index::Unit,
}

/// Facts may be copied as evidence; they cannot construct a new source lease.
pub(super) struct SpanFacts<'a> {
    pub identity: &'a SpoolIdentity,
    pub clock: &'a ClockDescriptor,
    pub tracks: &'a [TrackExtent; 2],
    pub receipts: u64,
    pub state: PrefixState,
    pub seal_cause: Option<SealCause>,
    pub incomplete_append: bool,
    pub torn_journal: bool,
    pub uncommitted_bytes: [u64; 2],
    pub track: Track,
    pub unit: &'a index::Unit,
}

/// Borrowed for one synchronous operation. A scoped producer may use the single
/// shared File cursor while its watchdog performs metadata-only checks.
pub(super) struct UnitSpan<'a> {
    prefix: &'a CommittedPrefix,
    track: Track,
    unit: &'a index::Unit,
    cancel: &'a AtomicBool,
    reader_taken: bool,
}
impl UnitSpan<'_> {
    pub(super) fn facts(&self) -> SpanFacts<'_> {
        let prefix = self.prefix;
        SpanFacts {
            identity: &prefix.identity,
            clock: &prefix.clock,
            tracks: &prefix.tracks,
            receipts: prefix.receipts,
            state: prefix.state,
            seal_cause: prefix.seal_cause,
            incomplete_append: prefix.incomplete_append,
            torn_journal: prefix.torn_journal,
            uncommitted_bytes: prefix.uncommitted_bytes,
            track: self.track,
            unit: self.unit,
        }
    }
    pub(super) fn check(&self) -> Result<()> {
        self.prefix.index_guards(self.cancel)
    }
}
impl<'a> UnitSpan<'a> {
    /// Never reopen/clone this file or run two cursor users concurrently. Hashing
    /// and transfer must run sequentially or after all borrowed workers joined.
    pub(super) fn take_reader(&mut self) -> Result<&'a File> {
        self.check()?;
        if self.reader_taken {
            return Err(Error::Order);
        }
        self.reader_taken = true;
        Ok(&self.prefix.raw[self.track.index()].file)
    }
}
impl SelectedUnit {
    /// Higher-ranked closure prevents a borrowed span/file escaping in T.
    /// T may itself be a Result: the final source revocation still takes precedence.
    pub(super) fn with_span<T>(
        &mut self,
        cancel: &AtomicBool,
        work: impl for<'span> FnOnce(UnitSpan<'span>) -> T,
    ) -> Result<T> {
        self.prefix.index_guards(cancel)?;
        let result = work(UnitSpan {
            prefix: &self.prefix,
            track: self.track,
            unit: &self.unit,
            cancel,
            reader_taken: false,
        });
        self.prefix.index_guards(cancel)?;
        Ok(result)
    }
}

/// One held durability inspection and two complete structural scans. Neither
/// preparation nor a successful visit is a codec/relocation/publication gate.
pub(super) struct SequentialSource {
    prefix: CommittedPrefix,
    outcomes: [index::Outcome; 2],
    observations: [[u8; 32]; 2],
    failure: Option<Error>,
}
impl SequentialSource {
    pub(super) fn prepare(
        root: &Path,
        identity: &SpoolIdentity,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::prepare_inner(root, identity, cancel, LIMITS)
    }

    fn prepare_inner(
        root: &Path,
        identity: &SpoolIdentity,
        cancel: &AtomicBool,
        limits: Limits,
    ) -> Result<Self> {
        let mut observations = [
            ObservationHash::new(Track::Video),
            ObservationHash::new(Track::Audio),
        ];
        let (prefix, outcomes) =
            inspect_and_index_inner(root, identity, cancel, limits, &mut |track, unit| {
                observations[track.index()].push(&unit).map_err(|_| ())
            })?;
        let outcomes = [
            outcomes[0].ok_or(Error::Proof)?,
            outcomes[1].ok_or(Error::Proof)?,
        ];
        for at in 0..2 {
            if outcomes[at].terminal != index::Terminal::Complete
                || outcomes[at].complete_units == 0
                || outcomes[at].complete_units != observations[at].count
            {
                return Err(Error::Proof);
            }
        }
        prefix.index_guards(cancel)?;
        Ok(Self {
            prefix,
            outcomes,
            observations: observations.map(ObservationHash::finish),
            failure: None,
        })
    }

    /// Keep a prepared source lease valid across owned derived-file work. This
    /// does not reclassify an index or clear a failed sequential visit.
    pub(super) fn check(&self, cancel: &AtomicBool) -> Result<()> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        self.prefix.index_guards(cancel)
    }

    /// Each callback is provisional until this entire call succeeds. The
    /// callback must join any scoped producer before returning the single
    /// cursor to the scanner; it must not clone/reopen the source file.
    pub(super) fn visit(
        &mut self,
        track: Track,
        cancel: &AtomicBool,
        mut work: impl for<'span> FnMut(UnitSpan<'span>) -> Result<()>,
    ) -> Result<()> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        // Also poison on unwind. Only a fully checked terminal visit clears it.
        self.failure = Some(Error::Order);
        let result = self.visit_inner(track, cancel, &mut work);
        self.failure = result.as_ref().err().copied();
        result
    }

    fn visit_inner(
        &self,
        track: Track,
        cancel: &AtomicBool,
        work: &mut impl for<'span> FnMut(UnitSpan<'span>) -> Result<()>,
    ) -> Result<()> {
        let prefix = &self.prefix;
        prefix.index_guards(cancel)?;
        let at = track.index();
        let (format, timebase) = match track {
            Track::Video => (
                index::Format::Vp9Webm,
                prefix.clock.video_timestamp_scale_ns,
            ),
            Track::Audio => (index::Format::AacMp4, prefix.clock.audio_timescale),
        };
        let file = &prefix.raw[at].file;
        let mut reader = file;
        let mut observation = ObservationHash::new(track);
        let mut callback_error = None;
        let outcome = index::scan(
            format,
            &mut reader,
            prefix.tracks[at].bytes,
            timebase,
            cancel,
            &mut |unit| {
                let result = (|| {
                    prefix.index_guards(cancel)?;
                    if unit.facts.configuration.format != format
                        || unit.facts.configuration.timebase != timebase
                        || unit.facts.start >= unit.facts.init_end
                        || unit.facts.init_end >= unit.facts.end
                        || unit.facts.end > prefix.tracks[at].bytes
                    {
                        return Err(Error::Proof);
                    }
                    observation.push(&unit)?;
                    let mut cursor = CursorPosition::save(file)?;
                    let worked = work(UnitSpan {
                        prefix,
                        track,
                        unit: &unit,
                        cancel,
                        reader_taken: false,
                    });
                    // Restore even when the callback fails. The scanner is
                    // paused for the entire synchronous callback interval.
                    let restored = cursor.restore();
                    prefix.index_guards(cancel)?;
                    restored?;
                    worked
                })();
                if let Err(error) = result {
                    callback_error = Some(error);
                    Err(())
                } else {
                    Ok(())
                }
            },
        );
        prefix.index_guards(cancel)?;
        if let Some(error) = callback_error {
            return Err(error);
        }
        if outcome.terminal != index::Terminal::Complete
            || outcome != self.outcomes[at]
            || observation.count != outcome.complete_units
            || observation.finish() != self.observations[at]
        {
            return Err(Error::Proof);
        }
        prefix.index_guards(cancel)
    }

    #[cfg(test)]
    pub(super) fn prepare_fixture(
        root: &Path,
        identity: &SpoolIdentity,
        cancel: &AtomicBool,
        limits: Limits,
    ) -> Result<Self> {
        Self::prepare_inner(root, identity, cancel, limits)
    }

    #[cfg(test)]
    pub(super) fn cursor_fixture(&self, track: Track) -> Result<u64> {
        let mut file = &self.prefix.raw[track.index()].file;
        file.stream_position().map_err(|_| Error::Io)
    }
}

struct CursorPosition<'a> {
    file: &'a File,
    position: u64,
    restored: bool,
}
impl<'a> CursorPosition<'a> {
    fn save(mut file: &'a File) -> Result<Self> {
        let position = file.stream_position().map_err(|_| Error::Io)?;
        Ok(Self {
            file,
            position,
            restored: false,
        })
    }
    fn restore(&mut self) -> Result<()> {
        let mut file = self.file;
        file.seek(SeekFrom::Start(self.position))
            .map_err(|_| Error::Io)?;
        self.restored = true;
        Ok(())
    }
}
impl Drop for CursorPosition<'_> {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore();
        }
    }
}

/// Fixed, versioned native observations only. No serde/Debug/string framing,
/// arbitrary metadata, retained units or media bytes enter this accumulator.
struct ObservationHash {
    digest: Sha256,
    count: u64,
}
impl ObservationHash {
    fn new(track: Track) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"atsumi-youtube-sequential-observations-v1\0");
        digest.update([match track {
            Track::Video => 0,
            Track::Audio => 1,
        }]);
        Self { digest, count: 0 }
    }
    fn push(&mut self, unit: &index::Unit) -> Result<()> {
        if unit.ordinal != self.count || self.count >= 100_000 {
            return Err(Error::Proof);
        }
        let facts = &unit.facts;
        let config = &facts.configuration;
        let h = &mut self.digest;
        for value in [unit.ordinal, facts.start, facts.init_end, facts.end] {
            h.update(value.to_be_bytes());
        }
        h.update([match facts.boundary {
            index::Boundary::NextInitialization => 0,
            index::Boundary::FiniteEnd => 1,
            index::Boundary::CommittedEof => 2,
        }]);
        h.update(facts.init_sha256);
        h.update(unit.configuration_sha256);
        h.update([match config.format {
            index::Format::Vp9Webm => 0,
            index::Format::AacMp4 => 1,
        }]);
        h.update(config.track_id.to_be_bytes());
        h.update(config.timebase.to_be_bytes());
        h.update(config.width.to_be_bytes());
        h.update(config.height.to_be_bytes());
        h.update([u8::from(config.default_duration_ns.is_some())]);
        h.update(config.default_duration_ns.unwrap_or(0).to_be_bytes());
        h.update(config.sample_rate.to_be_bytes());
        h.update([config.channels]);
        h.update(config.semantic_sha256);
        for value in [facts.clock.first, facts.clock.last, facts.clock.end] {
            h.update(value.to_be_bytes());
        }
        for value in [
            facts.clock.ticks_per_second,
            facts.items,
            facts.fragments,
            facts.references,
        ] {
            h.update(value.to_be_bytes());
        }
        h.update([u8::from(facts.rap_candidate)]);
        h.update([match unit.init_relation {
            index::InitRelation::First => 0,
            index::InitRelation::IdenticalValidatedInit => 1,
            index::InitRelation::ChangedParsedConfiguration => 2,
            index::InitRelation::UnprovedInitDifference => 3,
        }]);
        h.update([match unit.clock_relation {
            index::ClockRelation::First => 0,
            index::ClockRelation::Contiguous => 1,
            index::ClockRelation::Gap => 2,
            index::ClockRelation::OverlapOrReset => 3,
        }]);
        match facts.reconstruction {
            index::ReconstructionFacts::Webm(value) => {
                h.update([0]);
                h.update(
                    [
                        value.unknown_segment,
                        value.segment_crc,
                        value.info_duration,
                        value.cluster_position,
                        value.cluster_prev_size,
                        value.unknown_cluster_crc,
                        value.finite_cluster_crc,
                    ]
                    .map(u8::from),
                );
            }
            index::ReconstructionFacts::Aac(value) => {
                h.update([1, u8::from(value.all_fragments_default_base_is_moof)]);
                h.update(value.first_sample_duration.to_be_bytes());
                h.update(value.first_sequence.to_be_bytes());
                h.update(value.last_sequence.to_be_bytes());
                for duration in [
                    value.movie_duration,
                    value.track_duration,
                    value.media_duration,
                ] {
                    let (tag, ticks) = match duration {
                        index::HeaderDuration::Zero => (0, 0),
                        index::HeaderDuration::UnknownSentinel => (1, 0),
                        index::HeaderDuration::Finite(value) => (2, value),
                    };
                    h.update([tag]);
                    h.update(ticks.to_be_bytes());
                }
            }
        }
        h.update([u8::from(unit.codec_packet_validation_pending)]);
        self.count += 1;
        Ok(())
    }
    fn finish(mut self) -> [u8; 32] {
        self.digest.update(self.count.to_be_bytes());
        self.digest.finalize().into()
    }
}

#[cfg(test)]
pub(super) fn observation_fixture(track: Track, units: &[index::Unit]) -> Result<[u8; 32]> {
    let mut observation = ObservationHash::new(track);
    for unit in units {
        observation.push(unit)?;
    }
    Ok(observation.finish())
}

pub(super) fn select_unit(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    track: Track,
    ordinal: u64,
) -> Result<SelectedUnit> {
    select_unit_inner(root, identity, cancel, track, ordinal, LIMITS)
}
fn select_unit_inner(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    track: Track,
    ordinal: u64,
    limits: Limits,
) -> Result<SelectedUnit> {
    // Same fixed per-track ceiling as the structural index, not a duration hint.
    if ordinal >= 100_000 {
        return Err(Error::Bounds);
    }
    let mut selected = None;
    let (prefix, outcomes) =
        inspect_and_index_inner(root, identity, cancel, limits, &mut |kind, unit| {
            if kind == track && unit.ordinal == ordinal {
                if selected.is_some() {
                    return Err(());
                }
                selected = Some(unit);
            }
            Ok(())
        })?;
    if !outcomes.iter().all(|o| {
        matches!(
            o,
            Some(index::Outcome {
                terminal: index::Terminal::Complete,
                ..
            })
        )
    }) {
        return Err(Error::Proof);
    }
    let unit = selected.ok_or(Error::Bounds)?;
    let complete_count = outcomes[track.index()].ok_or(Error::Proof)?.complete_units;
    let (format, timebase) = match track {
        Track::Video => (
            index::Format::Vp9Webm,
            prefix.clock.video_timestamp_scale_ns,
        ),
        Track::Audio => (index::Format::AacMp4, prefix.clock.audio_timescale),
    };
    if unit.ordinal != ordinal
        || ordinal >= complete_count
        || unit.facts.configuration.format != format
        || unit.facts.configuration.timebase != timebase
        || unit.facts.start >= unit.facts.init_end
        || unit.facts.init_end >= unit.facts.end
        || unit.facts.end > prefix.tracks[track.index()].bytes
        || prefix.tracks[track.index()].bytes > prefix.physical[track.index()]
    {
        return Err(Error::Proof);
    }
    prefix.index_guards(cancel)?;
    Ok(SelectedUnit {
        prefix,
        track,
        unit,
    })
}

#[cfg(test)]
pub(super) fn select_unit_fixture(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    track: Track,
    ordinal: u64,
    limits: Limits,
) -> Result<SelectedUnit> {
    select_unit_inner(root, identity, cancel, track, ordinal, limits)
}

/// Dormant structural inspection only. All sink rows stay provisional until
/// both outcomes and the inherited durability facts are checked by a later stage.
pub(super) fn inspect_and_index(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Track, index::Unit) -> std::result::Result<(), ()>,
) -> Result<(CommittedPrefix, [Option<index::Outcome>; 2])> {
    inspect_and_index_inner(root, identity, cancel, LIMITS, sink)
}

fn inspect_and_index_inner(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    limits: Limits,
    sink: &mut dyn FnMut(Track, index::Unit) -> std::result::Result<(), ()>,
) -> Result<(CommittedPrefix, [Option<index::Outcome>; 2])> {
    // Do not accept an externally assembled/mutated prefix or reopen its files.
    let mut prefix = inspect(root, identity, cancel, limits)?;
    match prefix.state {
        PrefixState::Corrupt => return Err(Error::Corrupt),
        PrefixState::Cancelled => return Err(Error::Cancelled),
        PrefixState::Sealed | PrefixState::Interrupted => (),
    }
    let mut outcomes = [None; 2];
    for (track, format, timebase) in [
        (
            Track::Video,
            index::Format::Vp9Webm,
            prefix.clock.video_timestamp_scale_ns,
        ),
        (
            Track::Audio,
            index::Format::AacMp4,
            prefix.clock.audio_timescale,
        ),
    ] {
        prefix.index_guards(cancel)?;
        let at = track.index();
        let outcome = index::scan(
            format,
            &mut prefix.raw[at].file,
            prefix.tracks[at].bytes,
            timebase,
            cancel,
            &mut |unit| sink(track, unit),
        );
        outcomes[at] = Some(outcome);
        prefix.index_guards(cancel)?;
        if matches!(outcome.terminal, index::Terminal::Failed(_)) {
            break;
        }
    }
    prefix.index_guards(cancel)?;
    Ok((prefix, outcomes))
}

#[cfg(test)]
pub(super) fn inspect_and_index_fixture(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    limits: Limits,
    sink: &mut dyn FnMut(Track, index::Unit) -> std::result::Result<(), ()>,
) -> Result<(CommittedPrefix, [Option<index::Outcome>; 2])> {
    inspect_and_index_inner(root, identity, cancel, limits, sink)
}

enum ReadLine {
    End,
    Complete(Vec<u8>),
    Torn,
    Cancelled,
}
fn bounded_line(reader: &mut impl BufRead, cancel: &AtomicBool) -> Result<ReadLine> {
    let mut line = Vec::with_capacity(1024);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(ReadLine::Cancelled);
        }
        let available = reader.fill_buf().map_err(|_| Error::Io)?;
        if available.is_empty() {
            return Ok(if line.is_empty() {
                ReadLine::End
            } else {
                ReadLine::Torn
            });
        }
        let count = available
            .iter()
            .position(|v| *v == b'\n')
            .map_or(available.len(), |i| i + 1);
        if line.len().checked_add(count).is_none_or(|n| n > 1024) {
            return Err(Error::Corrupt);
        }
        let complete = available[count - 1] == b'\n';
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if complete {
            return Ok(ReadLine::Complete(line));
        }
    }
}

pub(crate) fn inspect_committed_prefix(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
) -> Result<CommittedPrefix> {
    inspect(root, identity, cancel, LIMITS)
}
pub(super) fn inspect(
    root: &Path,
    identity: &SpoolIdentity,
    cancel: &AtomicBool,
    limits: Limits,
) -> Result<CommittedPrefix> {
    identity.validate()?;
    let directory = super::super::storage::job_dir(root, "youtube", &identity.job_id)
        .map_err(|_| Error::Path)?;
    let directories = Directories::hold(&directory)?;
    let mut proof_file = GuardedFile::open(&directory.join(PROOF), false)?;
    let proof_length = proof_file.check()?;
    if proof_length == 0 || proof_length > 8192 {
        return Err(Error::Proof);
    }
    let mut proof_bytes = vec![0; proof_length as usize];
    proof_file
        .file
        .read_exact(&mut proof_bytes)
        .map_err(|_| Error::Io)?;
    let proof: Proof = serde_json::from_slice(&proof_bytes).map_err(|_| Error::Proof)?;
    if proof.version != 1 || &proof.identity != identity || proof.limits != limits {
        return Err(Error::Proof);
    }
    proof.identity.validate()?;
    proof.clock.validate()?;
    let mut raw = [
        GuardedFile::open(&directory.join(RAW[0]), false)?,
        GuardedFile::open(&directory.join(RAW[1]), false)?,
    ];
    let mut journal = GuardedFile::open(&directory.join(JOURNAL), false)?;
    let journal_length = journal.check()?;
    let physical = [raw[0].check()?, raw[1].check()?];
    let mut raw_anchor = super::raw_anchor::PendingAnchor::open(&directory, identity).ok();
    // The writer reserves capacity before writing, including an uncommitted tail.
    if journal_length > limits.journal
        || physical[0]
            .checked_add(physical[1])
            .is_none_or(|n| n > limits.raw)
    {
        return Err(Error::Bounds);
    }
    let mut reader = BufReader::with_capacity(1024, &mut journal.file);
    let mut progress = Progress::default();
    let mut state = PrefixState::Interrupted;
    let mut seal_cause = None;
    let mut torn = false;
    let mut chain = String::new();
    let mut bytes_read = 0u64;
    let mut header = false;
    let mut sealed = false;
    let mut scratch = vec![0; limits.chunk as usize];
    loop {
        let line = match bounded_line(&mut reader, cancel) {
            Ok(ReadLine::Complete(line)) => line,
            Ok(ReadLine::End) => break,
            Ok(ReadLine::Torn) => {
                torn = true;
                state = if sealed || !header {
                    PrefixState::Corrupt
                } else {
                    PrefixState::Interrupted
                };
                break;
            }
            Ok(ReadLine::Cancelled) => {
                state = PrefixState::Cancelled;
                break;
            }
            Err(_) => {
                state = PrefixState::Corrupt;
                break;
            }
        };
        bytes_read = match bytes_read.checked_add(line.len() as u64) {
            Some(n) if n <= limits.journal => n,
            _ => {
                state = PrefixState::Corrupt;
                break;
            }
        };
        if sealed {
            state = PrefixState::Corrupt;
            break;
        }
        let row: Row = match serde_json::from_slice(&line) {
            Ok(row) => row,
            Err(_) => {
                state = PrefixState::Corrupt;
                break;
            }
        };
        if !header {
            if !matches!(&row, Row::Header { version: 1, proof_sha256 } if proof_sha256 == &hash(&proof_bytes))
            {
                state = PrefixState::Corrupt;
                break;
            }
            header = true;
            chain = hash(&line);
            continue;
        }
        match row {
            Row::Chunk(row) => {
                if row.previous_record_sha256 != chain || progress.validate(&row, limits).is_err() {
                    state = PrefixState::Corrupt;
                    break;
                }
                let track = row.track.index();
                let length = row.length as usize;
                if row
                    .offset
                    .checked_add(row.length as u64)
                    .is_none_or(|end| end > physical[track])
                {
                    state = PrefixState::Corrupt;
                    break;
                }
                if cancel.load(Ordering::Acquire) {
                    state = PrefixState::Cancelled;
                    break;
                }
                if raw[track].file.read_exact(&mut scratch[..length]).is_err()
                    || hash(&scratch[..length]) != row.payload_sha256
                {
                    state = PrefixState::Corrupt;
                    break;
                }
                progress.commit(&row, &scratch[..length]);
                if let Some(anchor) = &mut raw_anchor { anchor.observe_receipt(&row); }
                chain = hash(&line);
            }
            Row::Seal(seal) => {
                if seal.previous_record_sha256 != chain
                    || seal.receipts != progress.receipts
                    || seal.tracks != progress.extents()
                    || seal.partial != progress.partial
                    || physical != progress.lengths
                {
                    state = PrefixState::Corrupt;
                    break;
                }
                sealed = true;
                seal_cause = Some(seal.cause);
                state = match seal.cause {
                    SealCause::Cancelled => PrefixState::Cancelled,
                    SealCause::Interrupted | SealCause::Limit => PrefixState::Interrupted,
                    _ if seal.partial.is_some() => PrefixState::Interrupted,
                    _ => PrefixState::Sealed,
                };
            }
            Row::Header { .. } => {
                state = PrefixState::Corrupt;
                break;
            }
        }
    }
    drop(reader);
    if !header && state != PrefixState::Cancelled {
        state = PrefixState::Corrupt;
    }
    if cancel.load(Ordering::Acquire) {
        state = PrefixState::Cancelled;
    }
    directories.check()?;
    if proof_file.check()? != proof_length
        || journal.check()? != journal_length
        || raw[0].check()? != physical[0]
        || raw[1].check()? != physical[1]
    {
        return Err(Error::Corrupt);
    }
    let incomplete_append_start = progress.partial.as_ref().map(|partial| {
        progress.lengths[partial.track.index()]
            .checked_sub(partial.bytes)
            .map(|start| (partial.track, start))
            .ok_or(Error::Corrupt)
    }).transpose()?;
    Ok(CommittedPrefix {
        identity: identity.clone(),
        tracks: progress.extents(),
        receipts: progress.receipts,
        state,
        seal_cause,
        incomplete_append: progress.partial.is_some(),
        incomplete_append_start,
        torn_journal: torn,
        uncommitted_bytes: std::array::from_fn(|i| physical[i].saturating_sub(progress.lengths[i])),
        clock: proof.clock,
        proof_length,
        journal_length,
        physical,
        raw,
        proof: proof_file,
        journal,
        directories,
        raw_anchor,
    })
}
