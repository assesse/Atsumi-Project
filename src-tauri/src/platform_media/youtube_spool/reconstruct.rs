//! Derived bytes only: no codec certificate, publication, file opening or tool
//! execution. Each push and finish remains provisional until its enclosing
//! SequentialSource visit, destination sync and caller verification succeed.
use super::{
    index::{self, Clock, Configuration, Format, HeaderDuration, ReconstructionFacts, Unit},
    recovery::{selection::SelectionBinding, PrefixState, SpanFacts, UnitSpan},
    ClockDescriptor, Error, Result, SealCause, SpoolIdentity, Track, TrackExtent,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    sync::atomic::{AtomicBool, Ordering},
};

const SCRATCH: usize = 131_072;
const METADATA: usize = 65_536;
const MAX_BYTES: u64 = 137_438_953_472;
const MAX_UNITS: u64 = 100_000;
const MAX_BOXES: u64 = 100_000_000;
const MAX_CHILDREN: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
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
impl Scope {
    fn from_facts(facts: &SpanFacts<'_>) -> Self {
        Self {
            identity: facts.identity.clone(),
            clock: facts.clock.clone(),
            tracks: (*facts.tracks).clone(),
            receipts: facts.receipts,
            state: facts.state,
            seal_cause: facts.seal_cause,
            incomplete_append: facts.incomplete_append,
            torn_journal: facts.torn_journal,
            uncommitted_bytes: facts.uncommitted_bytes,
        }
    }
}

/// Not a durable-file, ready-media or source-authority capability.
#[derive(Debug)]
pub(super) struct Summary {
    pub track: Track,
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub units: u64,
    pub fragments: u64,
    pub clock: Clock,
    pub state: PrefixState,
    pub seal_cause: Option<SealCause>,
    pub incomplete_append: bool,
    pub torn_journal: bool,
    pub uncommitted_bytes: [u64; 2],
}

pub(super) struct TrackBuilder<'output, W: Write> {
    track: Track,
    output: &'output mut W,
    scope: Option<Scope>,
    selection: Option<SelectionBinding>,
    config: Option<Configuration>,
    init_comparison: Vec<u8>,
    source_end: u64,
    bytes: u64,
    hash: Sha256,
    units: u64,
    fragments: u64,
    boxes: u64,
    clock: Option<Clock>,
    failed: Option<Error>,
    sealed: bool,
}
impl<'output, W: Write> TrackBuilder<'output, W> {
    pub(super) fn new(track: Track, output: &'output mut W) -> Self {
        Self {
            track,
            output,
            scope: None,
            selection: None,
            config: None,
            init_comparison: Vec::new(),
            source_end: 0,
            bytes: 0,
            hash: Sha256::new(),
            units: 0,
            fragments: 0,
            boxes: 0,
            clock: None,
            failed: None,
            sealed: false,
        }
    }

    /// Uses native original coordinates. The binding does not itself own media;
    /// every push still requires a guarded span and the outer full source visit.
    pub(super) fn for_selection(
        track: Track,
        output: &'output mut W,
        binding: &SelectionBinding,
    ) -> Self {
        let mut builder = Self::new(track, output);
        builder.source_end = binding.range(track).start;
        builder.selection = Some(binding.clone());
        builder
    }

    /// Only the native held-source lease is accepted. No caller-supplied Unit,
    /// range, path, or serialized prefix can enter this interface.
    pub(super) fn push(&mut self, mut span: UnitSpan<'_>, cancel: &AtomicBool) -> Result<()> {
        self.available()?;
        // An unwind or partial write must never leave a reusable builder.
        self.failed = Some(Error::Order);
        let result = (|| {
            span.check()?;
            if let Some(selection) = &self.selection {
                selection.check_span(&span)?;
            }
            let facts = span.facts();
            let track = facts.track;
            let unit = facts.unit.clone();
            let scope = Scope::from_facts(&facts);
            let mut reader = span.take_reader()?;
            self.push_inner(track, &scope, &unit, &mut reader, &|| {
                checkpoint(cancel)?;
                span.check()
            })
        })();
        self.failed = result.as_ref().err().copied();
        result
    }

    fn available(&self) -> Result<()> {
        if let Some(error) = self.failed {
            Err(error)
        } else if self.sealed {
            Err(Error::Sealed)
        } else {
            Ok(())
        }
    }

    fn validate(&self, track: Track, scope: &Scope, unit: &Unit) -> Result<()> {
        let facts = &unit.facts;
        scope.identity.validate()?;
        scope.clock.validate()?;
        if track != self.track
            || matches!(scope.state, PrefixState::Cancelled | PrefixState::Corrupt)
            || scope.seal_cause == Some(SealCause::Cancelled)
            || self.scope.as_ref().is_some_and(|known| known != scope)
        {
            return Err(Error::Identity);
        }
        let first_ordinal = self.selection.as_ref()
            .map_or(0, |selection| selection.range(track).first_ordinal);
        if self.units >= MAX_UNITS
            || Some(unit.ordinal) != first_ordinal.checked_add(self.units)
            || facts.start != self.source_end
            || facts.start >= facts.init_end
            || facts.init_end >= facts.end
            || facts.end > scope.tracks[track.index()].bytes
            || facts.end > MAX_BYTES
            || facts.init_end - facts.start > METADATA as u64
            || facts.items == 0
            || facts.fragments == 0
            || !unit.codec_packet_validation_pending
        {
            return Err(Error::Bounds);
        }
        if self.selection.as_ref().is_some_and(|selection| {
            let range = selection.range(track);
            unit.ordinal > range.last_ordinal || facts.end > range.end
        }) {
            return Err(Error::Bounds);
        }
        let (format, timebase, clock_scale) = match track {
            Track::Video => (
                Format::Vp9Webm,
                scope.clock.video_timestamp_scale_ns,
                1_000_000_000,
            ),
            Track::Audio => (
                Format::AacMp4,
                scope.clock.audio_timescale,
                scope.clock.audio_timescale,
            ),
        };
        if facts.configuration.format != format
            || facts.configuration.timebase != timebase
            || facts.clock.ticks_per_second != clock_scale
            || facts.clock.first > facts.clock.last
            || facts.clock.last >= facts.clock.end
            || self
                .config
                .as_ref()
                .is_some_and(|known| known != &facts.configuration)
            || self
                .clock
                .is_some_and(|previous| previous.end != facts.clock.first)
        {
            return Err(Error::Proof);
        }
        match (track, facts.reconstruction) {
            (Track::Video, ReconstructionFacts::Webm(r))
                if r.unknown_segment
                    && !r.segment_crc
                    && !r.info_duration
                    && !r.cluster_position
                    && !r.cluster_prev_size
                    && !r.unknown_cluster_crc
                    && !r.finite_cluster_crc => {}
            (Track::Audio, ReconstructionFacts::Aac(r))
                if r.all_fragments_default_base_is_moof
                    && r.movie_duration == HeaderDuration::Zero
                    && r.track_duration == HeaderDuration::Zero
                    && r.media_duration == HeaderDuration::Zero
                    && u64::from(r.last_sequence)
                        .checked_sub(u64::from(r.first_sequence))
                        .and_then(|n| n.checked_add(1))
                        == Some(facts.fragments) => {}
            _ => return Err(Error::Proof),
        }
        self.fragments
            .checked_add(facts.fragments)
            .ok_or(Error::Bounds)?;
        let added = (facts.end - facts.init_end)
            .checked_add(if self.units == 0 {
                facts.init_end - facts.start
            } else {
                0
            })
            .ok_or(Error::Bounds)?;
        if self.bytes.checked_add(added).ok_or(Error::Bounds)? > MAX_BYTES {
            return Err(Error::Bounds);
        }
        Ok(())
    }

    fn push_inner<R: Read + Seek>(
        &mut self,
        track: Track,
        scope: &Scope,
        unit: &Unit,
        reader: &mut R,
        guard: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        guard()?;
        self.validate(track, scope, unit)?;
        with_restored_cursor(reader, |reader| {
            guard()?;
            reader
                .seek(SeekFrom::Start(unit.facts.start))
                .map_err(|_| Error::Io)?;
            let mut init = vec![0; (unit.facts.init_end - unit.facts.start) as usize];
            read_exact(reader, &mut init, guard)?;
            let digest: [u8; 32] = Sha256::digest(&init).into();
            if digest != unit.facts.init_sha256 {
                return Err(Error::Corrupt);
            }
            let comparison = match track {
                Track::Video => webm_comparison(&init)?,
                Track::Audio => init.clone(),
            };
            if self.units != 0 && self.init_comparison != comparison {
                return Err(Error::Proof);
            }
            if self.units == 0 {
                self.write_bytes(&init, guard)?;
                self.init_comparison = comparison;
            }
            match track {
                Track::Video => self.copy(reader, unit.facts.end - unit.facts.init_end, guard)?,
                Track::Audio => self.audio_media(reader, unit, guard)?,
            }
            guard()?;
            if reader.stream_position().map_err(|_| Error::Io)? != unit.facts.end {
                return Err(Error::Order);
            }
            Ok(())
        })?;
        guard()?;
        if self.units == 0 {
            self.scope = Some(scope.clone());
            self.config = Some(unit.facts.configuration.clone());
            self.clock = Some(unit.facts.clock);
        } else if let Some(clock) = &mut self.clock {
            clock.last = unit.facts.clock.last;
            clock.end = unit.facts.clock.end;
        }
        self.source_end = unit.facts.end;
        self.units += 1;
        self.fragments += unit.facts.fragments;
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8], guard: &impl Fn() -> Result<()>) -> Result<()> {
        if self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Bounds)?
            > MAX_BYTES
        {
            return Err(Error::Bounds);
        }
        let mut at = 0;
        while at < bytes.len() {
            guard()?;
            match self.output.write(&bytes[at..]) {
                Ok(0) => return Err(Error::Io),
                Ok(n) if n <= bytes.len() - at => {
                    self.hash.update(&bytes[at..at + n]);
                    self.bytes += n as u64;
                    at += n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                _ => return Err(Error::Io),
            }
            guard()?;
        }
        Ok(())
    }

    fn copy<R: Read>(
        &mut self,
        reader: &mut R,
        mut count: u64,
        guard: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        let mut scratch = vec![0; SCRATCH];
        while count != 0 {
            let n = count.min(SCRATCH as u64) as usize;
            read_exact(reader, &mut scratch[..n], guard)?;
            self.write_bytes(&scratch[..n], guard)?;
            count -= n as u64;
        }
        Ok(())
    }

    fn audio_media<R: Read>(
        &mut self,
        reader: &mut R,
        unit: &Unit,
        guard: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        let ReconstructionFacts::Aac(reconstruction) = unit.facts.reconstruction else {
            return Err(Error::Proof);
        };
        let mut remaining = unit.facts.end - unit.facts.init_end;
        let mut fragments = 0u64;
        let mut expect_data = false;
        let mut padding = 0u64;
        let mut padding_boxes = 0usize;
        while remaining != 0 {
            guard()?;
            self.boxes = self.boxes.checked_add(1).ok_or(Error::Bounds)?;
            if self.boxes > MAX_BOXES || remaining < 8 {
                return Err(Error::Bounds);
            }
            let mut header = [0; 16];
            read_exact(reader, &mut header[..8], guard)?;
            let header_len = if u32::from_be_bytes(header[..4].try_into().unwrap()) == 1 {
                16
            } else {
                8
            };
            if header_len == 16 {
                if remaining < 16 {
                    return Err(Error::Proof);
                }
                read_exact(reader, &mut header[8..], guard)?;
            }
            let atom = atom_header(&header[..header_len], remaining)?;
            remaining -= atom.length;
            if expect_data && atom.kind != *b"mdat" {
                return Err(Error::Proof);
            }
            match &atom.kind {
                b"free" | b"skip" if !expect_data => {
                    padding = padding.checked_add(atom.length).ok_or(Error::Bounds)?;
                    padding_boxes += 1;
                    if padding > METADATA as u64 || padding_boxes > MAX_CHILDREN {
                        return Err(Error::Bounds);
                    }
                    self.write_bytes(&header[..header_len], guard)?;
                    self.copy(reader, atom.length - header_len as u64, guard)?;
                }
                b"moof" if !expect_data => {
                    if atom.length > METADATA as u64 {
                        return Err(Error::Bounds);
                    }
                    let mut moof = vec![0; atom.length as usize];
                    moof[..header_len].copy_from_slice(&header[..header_len]);
                    read_exact(reader, &mut moof[header_len..], guard)?;
                    let original = u64::from(reconstruction.first_sequence)
                        .checked_add(fragments)
                        .ok_or(Error::Bounds)?;
                    let next = self
                        .fragments
                        .checked_add(fragments)
                        .and_then(|n| n.checked_add(1))
                        .ok_or(Error::Bounds)?;
                    rewrite_sequence(
                        &mut moof,
                        u32::try_from(original).map_err(|_| Error::Bounds)?,
                        u32::try_from(next).map_err(|_| Error::Bounds)?,
                    )?;
                    self.write_bytes(&moof, guard)?;
                    fragments += 1;
                    if fragments > unit.facts.fragments {
                        return Err(Error::Proof);
                    }
                    expect_data = true;
                }
                b"mdat" if expect_data => {
                    self.write_bytes(&header[..header_len], guard)?;
                    self.copy(reader, atom.length - header_len as u64, guard)?;
                    expect_data = false;
                }
                _ => return Err(Error::Proof),
            }
        }
        if expect_data || fragments != unit.facts.fragments {
            return Err(Error::Proof);
        }
        Ok(())
    }

    pub(super) fn finish(&mut self, cancel: &AtomicBool) -> Result<Summary> {
        self.available()?;
        self.failed = Some(Error::Order);
        let result = (|| {
            checkpoint(cancel)?;
            let scope = self.scope.as_ref().ok_or(Error::Proof)?;
            let selected = self.selection.as_ref().map(|selection| selection.range(self.track));
            let expected_end = selected.map_or(scope.tracks[self.track.index()].bytes, |range| range.end);
            if self.units == 0
                || self.fragments == 0
                || self.bytes == 0
                || self.source_end != expected_end
            {
                return Err(Error::Proof);
            }
            if let Some(range) = selected {
                let count = range.last_ordinal.checked_sub(range.first_ordinal)
                    .and_then(|n| n.checked_add(1)).ok_or(Error::Bounds)?;
                let clock = self.clock.ok_or(Error::Proof)?;
                if self.units != count || clock.first != range.first_tick
                    || clock.end != range.end_tick || clock.ticks_per_second != range.ticks_per_second
                { return Err(Error::Proof); }
            }
            self.output.flush().map_err(|_| Error::Io)?;
            checkpoint(cancel)?;
            Ok(Summary {
                track: self.track,
                bytes: self.bytes,
                sha256: self.hash.clone().finalize().into(),
                units: self.units,
                fragments: self.fragments,
                clock: self.clock.ok_or(Error::Proof)?,
                state: scope.state,
                seal_cause: scope.seal_cause,
                incomplete_append: scope.incomplete_append,
                torn_journal: scope.torn_journal,
                uncommitted_bytes: scope.uncommitted_bytes,
            })
        })();
        self.failed = result.as_ref().err().copied();
        self.sealed = result.is_ok();
        result
    }
}

fn checkpoint(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn read_exact<R: Read>(
    reader: &mut R,
    bytes: &mut [u8],
    guard: &impl Fn() -> Result<()>,
) -> Result<()> {
    let mut at = 0;
    while at < bytes.len() {
        guard()?;
        match reader.read(&mut bytes[at..]) {
            Ok(0) => return Err(Error::Io),
            Ok(n) if n <= bytes.len() - at => at += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            _ => return Err(Error::Io),
        }
        guard()?;
    }
    Ok(())
}

struct Restore<'a, R: Seek> {
    reader: &'a mut R,
    position: u64,
    restored: bool,
}
impl<R: Seek> Drop for Restore<'_, R> {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.reader.seek(SeekFrom::Start(self.position));
        }
    }
}
fn with_restored_cursor<R: Seek, T>(
    reader: &mut R,
    work: impl FnOnce(&mut R) -> Result<T>,
) -> Result<T> {
    let position = reader.stream_position().map_err(|_| Error::Io)?;
    let mut restore = Restore {
        reader,
        position,
        restored: false,
    };
    let result = work(restore.reader);
    restore
        .reader
        .seek(SeekFrom::Start(position))
        .map_err(|_| Error::Io)?;
    restore.restored = true;
    result
}

struct Atom {
    kind: [u8; 4],
    header: usize,
    length: u64,
}
fn atom_header(bytes: &[u8], available: u64) -> Result<Atom> {
    if bytes.len() < 8 {
        return Err(Error::Proof);
    }
    let short = u32::from_be_bytes(bytes[..4].try_into().unwrap());
    let (length, header) = match short {
        0 => return Err(Error::Proof),
        1 if bytes.len() >= 16 => (u64::from_be_bytes(bytes[8..16].try_into().unwrap()), 16),
        1 => return Err(Error::Proof),
        n => (u64::from(n), 8),
    };
    if length < header as u64 || length > available {
        return Err(Error::Proof);
    }
    Ok(Atom {
        kind: bytes[4..8].try_into().unwrap(),
        header,
        length,
    })
}

/// Scan only structural immediate moof children. Never search sample payload
/// for mfhd magic, or change box lengths, tfdt, trun or data offsets.
fn rewrite_sequence(moof: &mut [u8], expected: u32, next: u32) -> Result<()> {
    if moof.len() > METADATA || next == 0 {
        return Err(Error::Bounds);
    }
    let outer = atom_header(moof, moof.len() as u64)?;
    if outer.kind != *b"moof" || outer.length != moof.len() as u64 {
        return Err(Error::Proof);
    }
    let mut at = outer.header;
    let mut sequence_at = None;
    let mut traf = false;
    let mut children = 0;
    while at < moof.len() {
        children += 1;
        if children > MAX_CHILDREN {
            return Err(Error::Bounds);
        }
        let child = atom_header(&moof[at..], (moof.len() - at) as u64)?;
        let end = at.checked_add(child.length as usize).ok_or(Error::Bounds)?;
        let body = at + child.header;
        match &child.kind {
            b"mfhd" => {
                if sequence_at.is_some()
                    || end - body != 8
                    || moof[body..body + 4] != [0; 4]
                    || u32::from_be_bytes(moof[body + 4..end].try_into().unwrap()) != expected
                {
                    return Err(Error::Proof);
                }
                sequence_at = Some(body + 4);
            }
            b"traf" if !traf => traf = true,
            b"free" | b"skip" => {}
            _ => return Err(Error::Proof),
        }
        at = end;
    }
    if !traf {
        return Err(Error::Proof);
    }
    let at = sequence_at.ok_or(Error::Proof)?;
    moof[at..at + 4].copy_from_slice(&next.to_be_bytes());
    Ok(())
}

struct Element {
    id: u64,
    body: usize,
    end: Option<usize>,
}
fn element(bytes: &[u8], start: usize) -> Result<Element> {
    let first = *bytes.get(start).ok_or(Error::Proof)?;
    if first == 0 {
        return Err(Error::Proof);
    }
    let id_width = first.leading_zeros() as usize + 1;
    if id_width > 4 || start.checked_add(id_width).ok_or(Error::Bounds)? >= bytes.len() {
        return Err(Error::Proof);
    }
    let id = bytes[start..start + id_width]
        .iter()
        .fold(0u64, |n, b| n * 256 + u64::from(*b));
    let size_at = start + id_width;
    let first = bytes[size_at];
    if first == 0 {
        return Err(Error::Proof);
    }
    let width = first.leading_zeros() as usize + 1;
    let body = size_at.checked_add(width).ok_or(Error::Bounds)?;
    if width > 8 || body > bytes.len() {
        return Err(Error::Proof);
    }
    let mut size = u64::from(first & ((0xffu16 >> width) as u8));
    for b in &bytes[size_at + 1..body] {
        size = size * 256 + u64::from(*b);
    }
    let end = if size == (1u64 << (width * 7)) - 1 {
        None
    } else {
        let end = body
            .checked_add(usize::try_from(size).map_err(|_| Error::Bounds)?)
            .ok_or(Error::Bounds)?;
        if end > bytes.len() {
            return Err(Error::Proof);
        }
        Some(end)
    };
    Ok(Element { id, body, end })
}

/// Only direct Segment Void elements are omitted for compatibility comparison.
/// Nested bytes, EBML/Segment headers and every other metadata byte stay exact.
fn webm_comparison(init: &[u8]) -> Result<Vec<u8>> {
    if init.is_empty() || init.len() > METADATA {
        return Err(Error::Bounds);
    }
    let ebml = element(init, 0)?;
    if ebml.id != 0x1a45dfa3 {
        return Err(Error::Proof);
    }
    let segment_at = ebml.end.ok_or(Error::Proof)?;
    let segment = element(init, segment_at)?;
    if segment.id != 0x18538067 || segment.end.is_some() {
        return Err(Error::Proof);
    }
    let mut comparison = init[..segment.body].to_vec();
    let mut at = segment.body;
    let mut nodes = 0;
    let mut info = false;
    let mut tracks = false;
    while at < init.len() {
        nodes += 1;
        if nodes > MAX_CHILDREN {
            return Err(Error::Bounds);
        }
        let child = element(init, at)?;
        let end = child.end.ok_or(Error::Proof)?;
        match child.id {
            0xec => {}
            0x1549a966 if !info => {
                info = true;
                comparison.extend_from_slice(&init[at..end]);
            }
            0x1654ae6b if !tracks => {
                tracks = true;
                comparison.extend_from_slice(&init[at..end]);
            }
            _ => return Err(Error::Proof),
        }
        at = end;
    }
    if !info || !tracks {
        return Err(Error::Proof);
    }
    Ok(comparison)
}

#[cfg(test)]
mod tests;
