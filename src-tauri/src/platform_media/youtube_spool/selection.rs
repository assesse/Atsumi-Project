//! Dormant structural selection only. It neither changes Whole-Complete source
//! admission nor certifies a retained packet set, codec, final media or cleanup.
use super::{
    index, inspect, ClockDescriptor, CommittedPrefix, CursorPosition, Error, Limits,
    ObservationHash, PrefixState, SealCause, SpoolIdentity, Track, UnitSpan, LIMITS,
};
use std::{cmp::Ordering, path::Path, sync::atomic::AtomicBool};

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) enum Fault {
    Source(Error),
    Index(index::Error),
    NoCompleteUnits,
    NoCoveredInterval,
    Discontinuity,
    Overflow,
    Changed,
    Poisoned,
}
type Result<T> = std::result::Result<T, Fault>;
impl From<Error> for Fault {
    fn from(value: Error) -> Self {
        Self::Source(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct Range {
    pub first_ordinal: u64,
    pub last_ordinal: u64,
    pub start: u64,
    pub end: u64,
    pub first_tick: i128,
    pub end_tick: i128,
    pub ticks_per_second: u64,
}
impl Range {
    fn first(unit: &index::Unit) -> Self {
        Self {
            first_ordinal: unit.ordinal,
            last_ordinal: unit.ordinal,
            start: unit.facts.start,
            end: unit.facts.end,
            first_tick: unit.facts.clock.first,
            end_tick: unit.facts.clock.end,
            ticks_per_second: unit.facts.clock.ticks_per_second,
        }
    }
    fn extend(&mut self, unit: &index::Unit) -> Result<()> {
        if self.last_ordinal.checked_add(1) != Some(unit.ordinal)
            || self.end != unit.facts.start
            || self.ticks_per_second != unit.facts.clock.ticks_per_second
            || self.end_tick != unit.facts.clock.first
        {
            return Err(Fault::Discontinuity);
        }
        self.last_ordinal = unit.ordinal;
        self.end = unit.facts.end;
        self.end_tick = unit.facts.clock.end;
        Ok(())
    }
}

/// Actual native packet durations at the selected AAC outer edges. These are
/// temporal observations, deliberately excluded from codec configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct AudioBoundary {
    first: u32,
    last: u32,
}
impl AudioBoundary {
    fn unit(unit: &index::Unit) -> Result<Self> {
        let index::ReconstructionFacts::Aac(native) = unit.facts.reconstruction else {
            return Err(Fault::Changed);
        };
        let clock = unit.facts.clock;
        let last = clock.end.checked_sub(clock.last)
            .and_then(|ticks| u32::try_from(ticks).ok()).filter(|ticks| *ticks > 0)
            .ok_or(Fault::Changed)?;
        if native.first_sample_duration == 0 || clock.first < 0 || clock.first > clock.last
            || clock.ticks_per_second == 0
            || clock.end.checked_sub(clock.first).is_none_or(|ticks| ticks < i128::from(native.first_sample_duration))
        { return Err(Fault::Changed); }
        Ok(Self { first: native.first_sample_duration, last })
    }
    pub(in crate::platform_media::youtube_spool) fn first_ticks(self) -> u32 { self.first }
    pub(in crate::platform_media::youtube_spool) fn last_ticks(self) -> u32 { self.last }

    /// Compare temporal intervals only. The caller separately binds byte/unit
    /// coordinates. No rounded clocks, inferred packet durations or epsilon.
    pub(in crate::platform_media::youtube_spool) fn allows(self, video: Range, audio: Range) -> Result<bool> {
        if self.first == 0 || self.last == 0 || video.first_tick < 0 || audio.first_tick < 0
            || video.first_tick >= video.end_tick || audio.first_tick >= audio.end_tick
            || video.ticks_per_second == 0 || audio.ticks_per_second == 0
        { return Err(Fault::Changed); }
        let product = |ticks: i128, scale: u64| ticks.checked_mul(i128::from(scale)).ok_or(Fault::Overflow);
        let v0 = product(video.first_tick, audio.ticks_per_second)?;
        let v1 = product(video.end_tick, audio.ticks_per_second)?;
        let a0 = product(audio.first_tick, video.ticks_per_second)?;
        let a1 = product(audio.end_tick, video.ticks_per_second)?;
        if a0 >= v1 || v0 >= a1 { return Ok(false); }
        let leading = a0.checked_sub(v0).ok_or(Fault::Overflow)?;
        let trailing = v1.checked_sub(a1).ok_or(Fault::Overflow)?;
        Ok((leading <= 0 || leading < product(i128::from(self.first), video.ticks_per_second)?)
            && (trailing <= 0 || trailing < product(i128::from(self.last), video.ticks_per_second)?))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct ExclusionReasons {
    pub startup_coverage: bool,
    pub audio_coverage_tail: bool,
    pub outside_selected_video: bool,
    pub incomplete_structure: bool,
    pub incomplete_append: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct Excluded {
    pub start: u64,
    pub end: u64,
    pub complete_units: u64,
    pub reasons: ExclusionReasons,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct TrackPlan {
    pub selected: Range,
    pub leading: Option<Excluded>,
    pub trailing: Option<Excluded>,
    pub committed_bytes: u64,
    pub physical_bytes: u64,
    pub original_outcome: index::Outcome,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct SelectionFacts {
    pub identity: SpoolIdentity,
    pub clock: ClockDescriptor,
    pub state: PrefixState,
    pub seal_cause: Option<SealCause>,
    pub torn_journal: bool,
    pub incomplete_append_start: Option<(Track, u64)>,
    pub uncommitted_bytes: [u64; 2],
    pub tracks: [TrackPlan; 2],
    pub audio_boundary: AudioBoundary,
    pub packet_selection_pending: bool,
    pub codec_validation_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScanFacts {
    outcome: index::Outcome,
    digest: [u8; 32],
}

/// Evidence from one native selection, not a path/reader capability. Only this
/// module can construct it; copied public facts cannot approve another range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct SelectionBinding {
    facts: SelectionFacts,
    tracks: [super::TrackExtent; 2],
    receipts: u64,
    incomplete_append: bool,
    proof_length: u64,
    journal_length: u64,
    scans: [ScanFacts; 2],
}
impl SelectionBinding {
    pub(in crate::platform_media::youtube_spool) fn range(&self, track: Track) -> Range {
        self.facts.tracks[track.index()].selected
    }
    pub(in crate::platform_media::youtube_spool) fn audio_boundary(&self) -> AudioBoundary {
        self.facts.audio_boundary
    }
    pub(in crate::platform_media::youtube_spool) fn check_coverage(&self, video: index::Clock, audio: index::Clock) -> Result<()> {
        for (track, clock) in [(Track::Video, video), (Track::Audio, audio)] {
            let range = self.range(track);
            if clock.first > clock.last || clock.last >= clock.end
                || (clock.first, clock.end, clock.ticks_per_second)
                    != (range.first_tick, range.end_tick, range.ticks_per_second)
            { return Err(Fault::Changed); }
        }
        if !self.audio_boundary().allows(self.range(Track::Video), self.range(Track::Audio))? {
            return Err(Fault::NoCoveredInterval);
        }
        Ok(())
    }

    pub(in crate::platform_media::youtube_spool) fn check_span(
        &self,
        span: &UnitSpan<'_>,
    ) -> super::Result<()> {
        span.check()?;
        let prefix = span.prefix;
        if prefix.identity != self.facts.identity
            || prefix.clock != self.facts.clock
            || prefix.tracks != self.tracks
            || prefix.receipts != self.receipts
            || prefix.state != self.facts.state
            || prefix.seal_cause != self.facts.seal_cause
            || prefix.incomplete_append != self.incomplete_append
            || prefix.incomplete_append_start != self.facts.incomplete_append_start
            || prefix.torn_journal != self.facts.torn_journal
            || prefix.uncommitted_bytes != self.facts.uncommitted_bytes
            || prefix.proof_length != self.proof_length
            || prefix.journal_length != self.journal_length
            || prefix.physical != self.facts.tracks.map(|track| track.physical_bytes)
        {
            return Err(Error::Identity);
        }
        let range = self.range(span.track);
        let unit = span.unit;
        let facts = &unit.facts;
        if !(range.first_ordinal..=range.last_ordinal).contains(&unit.ordinal)
            || facts.start < range.start || facts.end > range.end
            || facts.start >= facts.init_end || facts.init_end >= facts.end
            || facts.clock.ticks_per_second != range.ticks_per_second
            || facts.clock.first < range.first_tick || facts.clock.end > range.end_tick
            || (unit.ordinal == range.first_ordinal
                && (facts.start != range.start || facts.clock.first != range.first_tick))
            || (unit.ordinal == range.last_ordinal
                && (facts.end != range.end || facts.clock.end != range.end_tick))
            || !unit.codec_packet_validation_pending
        {
            return Err(Error::Proof);
        }
        if span.track == Track::Audio {
            let observed = AudioBoundary::unit(unit).map_err(|_| Error::Proof)?;
            if (unit.ordinal == range.first_ordinal && observed.first != self.facts.audio_boundary.first)
                || (unit.ordinal == range.last_ordinal && observed.last != self.facts.audio_boundary.last)
            { return Err(Error::Proof); }
        }
        span.check()
    }
}
#[derive(Default)]
struct Summary {
    first: Option<index::Unit>,
    previous: Option<index::Unit>,
    last: Option<index::Unit>,
}
struct Eligible {
    range: Range,
    withheld_eof: bool,
    audio_boundary: Option<AudioBoundary>,
}
impl Summary {
    fn push(&mut self, unit: &index::Unit) -> Result<()> {
        if let Some(first) = &self.first {
            if unit.configuration_sha256 != first.configuration_sha256
                || unit.clock_relation != index::ClockRelation::Contiguous
            {
                return Err(Fault::Discontinuity);
            }
        } else {
            if unit.ordinal != 0
                || unit.facts.start != 0
                || unit.clock_relation != index::ClockRelation::First
            {
                return Err(Fault::Discontinuity);
            }
            self.first = Some(unit.clone());
        }
        self.previous = self.last.replace(unit.clone());
        Ok(())
    }
    fn finish(
        self,
        prefix: &CommittedPrefix,
        track: Track,
        scanned: ScanFacts,
    ) -> Result<Eligible> {
        let first = self.first.ok_or(Fault::NoCompleteUnits)?;
        let last = self.last.ok_or(Fault::NoCompleteUnits)?;
        let bytes = prefix.tracks[track.index()].bytes;
        if scanned.outcome.complete_units != last.ordinal.checked_add(1).ok_or(Fault::Overflow)? {
            return Err(Fault::Changed);
        }
        match scanned.outcome.terminal {
            index::Terminal::Complete if last.facts.end == bytes => (),
            index::Terminal::PartialTail { offset }
                if last.facts.end <= offset && offset <= bytes =>
            {
                ()
            }
            _ => return Err(Fault::Changed),
        }
        let withheld_eof = last.facts.boundary == index::Boundary::CommittedEof
            && prefix
                .incomplete_append_start
                .is_some_and(|(kind, start)| kind == track && last.facts.end > start);
        let last = if withheld_eof {
            self.previous.ok_or(Fault::NoCompleteUnits)?
        } else {
            last
        };
        let range = Range {
            first_ordinal: first.ordinal,
            last_ordinal: last.ordinal,
            start: first.facts.start,
            end: last.facts.end,
            first_tick: first.facts.clock.first,
            end_tick: last.facts.clock.end,
            ticks_per_second: first.facts.clock.ticks_per_second,
        };
        Ok(Eligible {
            range,
            withheld_eof,
            audio_boundary: if track == Track::Audio {
                Some(AudioBoundary { first: AudioBoundary::unit(&first)?.first, last: AudioBoundary::unit(&last)?.last })
            } else { None },
        })
    }
}

fn compare(a: i128, a_scale: u64, b: i128, b_scale: u64) -> Result<Ordering> {
    if a_scale == 0 || b_scale == 0 {
        return Err(Fault::Overflow);
    }
    let left = a.checked_mul(b_scale as i128).ok_or(Fault::Overflow)?;
    let right = b.checked_mul(a_scale as i128).ok_or(Fault::Overflow)?;
    Ok(left.cmp(&right))
}
fn intersects(outer: Range, clock: index::Clock) -> Result<bool> {
    Ok(compare(
        clock.end,
        clock.ticks_per_second,
        outer.first_tick,
        outer.ticks_per_second,
    )? == Ordering::Greater
        && compare(
            clock.first,
            clock.ticks_per_second,
            outer.end_tick,
            outer.ticks_per_second,
        )? == Ordering::Less)
}
fn append_range(range: &mut Option<Range>, unit: &index::Unit) -> Result<()> {
    match range {
        Some(range) => range.extend(unit),
        None => {
            *range = Some(Range::first(unit));
            Ok(())
        }
    }
}

/// The whole committed prefix is scanned, including excluded data. A callback
/// cannot hide a later malformed/tampered tail by asking to shorten the reader.
fn scan(
    prefix: &CommittedPrefix,
    track: Track,
    cancel: &AtomicBool,
    work: &mut impl FnMut(&index::Unit) -> Result<()>,
) -> Result<ScanFacts> {
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
    let mut cursor = CursorPosition::save(file)?;
    let mut reader = file;
    let mut observations = ObservationHash::new(track);
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
                    || unit.facts.end > prefix.tracks[at].bytes
                {
                    return Err(Fault::Changed);
                }
                observations.push(&unit)?;
                let mut saved = CursorPosition::save(file)?;
                let worked = work(&unit);
                let restored = saved.restore();
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
    let restored = cursor.restore();
    prefix.index_guards(cancel)?;
    restored?;
    if let Some(error) = callback_error {
        return Err(error);
    }
    if let index::Terminal::Failed(error) = outcome.terminal {
        return Err(Fault::Index(error));
    }
    if observations.count != outcome.complete_units {
        return Err(Fault::Changed);
    }
    Ok(ScanFacts {
        outcome,
        digest: observations.finish(),
    })
}

fn plan(
    prefix: &CommittedPrefix,
    track: Track,
    eligible: &Eligible,
    scanned: ScanFacts,
    selected: Range,
) -> Result<TrackPlan> {
    let at = track.index();
    if selected.first_ordinal > selected.last_ordinal
        || selected.last_ordinal > eligible.range.last_ordinal
        || selected.start >= selected.end
        || selected.end > eligible.range.end
    {
        return Err(Fault::Changed);
    }
    let leading = (selected.start > 0).then_some(Excluded {
        start: 0,
        end: selected.start,
        complete_units: selected.first_ordinal,
        reasons: ExclusionReasons {
            startup_coverage: track == Track::Video,
            outside_selected_video: track == Track::Audio,
            ..Default::default()
        },
    });
    let trailing_units = scanned
        .outcome
        .complete_units
        .checked_sub(selected.last_ordinal + 1)
        .ok_or(Fault::Overflow)?;
    let trailing = (selected.end < prefix.tracks[at].bytes).then_some(Excluded {
        start: selected.end,
        end: prefix.tracks[at].bytes,
        complete_units: trailing_units,
        reasons: ExclusionReasons {
            audio_coverage_tail: track == Track::Video
                && selected.last_ordinal < eligible.range.last_ordinal,
            outside_selected_video: track == Track::Audio
                && selected.last_ordinal < eligible.range.last_ordinal,
            incomplete_structure: matches!(
                scanned.outcome.terminal,
                index::Terminal::PartialTail { .. }
            ),
            incomplete_append: eligible.withheld_eof,
            ..Default::default()
        },
    });
    Ok(TrackPlan {
        selected,
        leading,
        trailing,
        committed_bytes: prefix.tracks[at].bytes,
        physical_bytes: prefix.physical[at],
        original_outcome: scanned.outcome,
    })
}

pub(in crate::platform_media::youtube_spool) struct CompletedSelection {
    prefix: CommittedPrefix,
    scans: [ScanFacts; 2],
    facts: SelectionFacts,
    poisoned: bool,
}
impl CompletedSelection {
    pub(in crate::platform_media::youtube_spool) fn prepare(
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
        let mut prefix = inspect(root, identity, cancel, limits)?;
        match prefix.state {
            PrefixState::Corrupt => return Err(Error::Corrupt.into()),
            PrefixState::Cancelled => return Err(Error::Cancelled.into()),
            PrefixState::Sealed | PrefixState::Interrupted => (),
        }
        let mut video_summary = Summary::default();
        let mut first_video_unit = None;
        let video_scan = scan(&prefix, Track::Video, cancel, &mut |unit| {
            if unit.ordinal == 0 { first_video_unit = Some(unit.clone()); }
            video_summary.push(unit)
        })?;
        if let (Some(anchor), Some(unit)) = (&mut prefix.raw_anchor, first_video_unit.as_ref()) {
            anchor.observe_first_unit(unit);
        }
        let video = video_summary.finish(&prefix, Track::Video, video_scan)?;
        let mut audio_summary = Summary::default();
        let audio_scan = scan(&prefix, Track::Audio, cancel, &mut |unit| {
            audio_summary.push(unit)
        })?;
        let audio = audio_summary.finish(&prefix, Track::Audio, audio_scan)?;
        let audio_boundary = audio.audio_boundary.ok_or(Fault::Changed)?;
        let mut selected_video = None;
        let repeated = scan(&prefix, Track::Video, cancel, &mut |unit| {
            if unit.ordinal <= video.range.last_ordinal && audio_boundary.allows(Range::first(unit), audio.range)?
            {
                append_range(&mut selected_video, unit)?;
            }
            Ok(())
        })?;
        if repeated != video_scan {
            return Err(Fault::Changed);
        }
        let selected_video = selected_video.ok_or(Fault::NoCoveredInterval)?;
        let mut selected_audio = None;
        let mut selected_boundary: Option<AudioBoundary> = None;
        let repeated = scan(&prefix, Track::Audio, cancel, &mut |unit| {
            if unit.ordinal <= audio.range.last_ordinal
                && intersects(selected_video, unit.facts.clock)?
            {
                append_range(&mut selected_audio, unit)?;
                let observed = AudioBoundary::unit(unit)?;
                selected_boundary.get_or_insert(observed).last = observed.last;
            }
            Ok(())
        })?;
        if repeated != audio_scan {
            return Err(Fault::Changed);
        }
        let selected_audio = selected_audio.ok_or(Fault::NoCoveredInterval)?;
        let selected_boundary = selected_boundary.ok_or(Fault::Changed)?;
        if !selected_boundary.allows(selected_video, selected_audio)? {
            return Err(Fault::NoCoveredInterval);
        }
        let tracks = [
            plan(&prefix, Track::Video, &video, video_scan, selected_video)?,
            plan(&prefix, Track::Audio, &audio, audio_scan, selected_audio)?,
        ];
        let facts = SelectionFacts {
            identity: prefix.identity.clone(),
            clock: prefix.clock.clone(),
            state: prefix.state,
            seal_cause: prefix.seal_cause,
            torn_journal: prefix.torn_journal,
            incomplete_append_start: prefix.incomplete_append_start,
            uncommitted_bytes: prefix.uncommitted_bytes,
            tracks,
            audio_boundary: selected_boundary,
            packet_selection_pending: true,
            codec_validation_pending: true,
        };
        prefix.index_guards(cancel)?;
        Ok(Self {
            prefix,
            scans: [video_scan, audio_scan],
            facts,
            poisoned: false,
        })
    }
    pub(in crate::platform_media::youtube_spool) fn facts(&self) -> &SelectionFacts {
        &self.facts
    }
    pub(in crate::platform_media::youtube_spool) fn raw_timeline(
        &self, origin: super::super::packet_facts::Rational, cancel: &AtomicBool,
    ) -> Option<super::super::raw_anchor::RawTimelineFacts> {
        self.check(cancel).ok()?;
        self.prefix.raw_anchor.as_ref()?.timeline(origin, cancel).ok()
    }
    pub(in crate::platform_media::youtube_spool) fn check(&self, cancel: &AtomicBool) -> Result<()> {
        if self.poisoned { return Err(Fault::Poisoned); }
        self.prefix.index_guards(cancel)?;
        Ok(())
    }
    pub(in crate::platform_media::youtube_spool) fn binding(
        &self,
        cancel: &AtomicBool,
    ) -> Result<SelectionBinding> {
        self.check(cancel)?;
        Ok(SelectionBinding {
            facts: self.facts.clone(),
            tracks: self.prefix.tracks.clone(),
            receipts: self.prefix.receipts,
            incomplete_append: self.prefix.incomplete_append,
            proof_length: self.prefix.proof_length,
            journal_length: self.prefix.journal_length,
            scans: self.scans,
        })
    }
    pub(in crate::platform_media::youtube_spool) fn visit(
        &mut self,
        track: Track,
        cancel: &AtomicBool,
        mut work: impl for<'span> FnMut(UnitSpan<'span>) -> super::Result<()>,
    ) -> Result<()> {
        if self.poisoned {
            return Err(Fault::Poisoned);
        }
        self.poisoned = true;
        let range = self.facts.tracks[track.index()].selected;
        let mut observed_range = None;
        let prefix = &self.prefix;
        let result = scan(prefix, track, cancel, &mut |unit| {
            if (range.first_ordinal..=range.last_ordinal).contains(&unit.ordinal) {
                append_range(&mut observed_range, unit)?;
                work(UnitSpan {
                    prefix,
                    track,
                    unit,
                    cancel,
                    reader_taken: false,
                })?;
            }
            Ok(())
        })?;
        if result != self.scans[track.index()] || observed_range != Some(range) {
            return Err(Fault::Changed);
        }
        prefix.index_guards(cancel)?;
        self.poisoned = false;
        Ok(())
    }
}
