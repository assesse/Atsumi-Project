//! Exact AAC packet selection evidence only. No process, file opening, media
//! publication, or permission to discard the original source/tape.
use super::{check_report, checkpoint, rational_cmp, seconds, Fault, Result};
use crate::platform_media::youtube_spool::{
    packet_facts::{FoldResult, PacketFact, PairComparison, PairReport, Rational, TimeBase, Times, TrackFold},
    packet_tape::{Reader, Summary},
    recovery::selection::{Range, SelectionBinding},
    stream_facts::{ProbeFacts, StreamFacts},
    Track,
};
use std::{io::Read, sync::atomic::AtomicBool, time::Instant};

const BEFORE: usize = 0;
const RETAINED: usize = 1;
const AFTER: usize = 2;
const MAX_PACKETS: u64 = 20_000_000;
const AUDIO_SCALE: u64 = 44_100;

fn audio_base() -> Result<TimeBase> {
    TimeBase::new(1, AUDIO_SCALE as u32).map_err(Fault::Packet)
}
fn audio_stream(stream: &StreamFacts, timebase: TimeBase) -> Result<()> {
    if stream.track != Track::Audio || stream.time_base != timebase
        || stream.width != 0 || stream.height != 0
        || stream.sample_rate != AUDIO_SCALE as u32 || stream.channels != 2
    { return Err(Fault::Coverage); }
    Ok(())
}
fn interval(range: Range) -> Result<(Rational, Rational)> {
    if range.first_tick < 0 || range.first_tick >= range.end_tick
        || range.ticks_per_second == 0 || range.first_ordinal > range.last_ordinal
        || range.start >= range.end
    { return Err(Fault::Coverage); }
    Ok((seconds(range.first_tick, range.ticks_per_second)?, seconds(range.end_tick, range.ticks_per_second)?))
}
fn packet_time(packet: PacketFact) -> Result<(Times, i64)> {
    let record = packet.encode_record().map_err(Fault::Packet)?;
    let time = Times::from(packet);
    // encode_record checks representation, not skip/discard support.
    if record[0] != Track::Audio.index() as u8 || time.pts < 0
        || time.pts != time.dts || time.duration <= 0
        || record[68..78].iter().any(|byte| *byte != 0)
    { return Err(Fault::Coverage); }
    let end = time.pts.checked_add(time.duration).ok_or(Fault::Bounds)?;
    Ok((time, end))
}

/// Native-only, non-Clone evidence bound to one full tape and selection.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::platform_media::youtube_spool) struct AacRetention {
    binding: SelectionBinding,
    tape: Summary,
    leading: Option<FoldResult>,
    retained: FoldResult,
    trailing: Option<FoldResult>,
    first: u64,
    end: u64,
}
impl AacRetention {
    pub(in crate::platform_media::youtube_spool) fn ordinals(&self) -> (u64, u64) { (self.first, self.end) }
    pub(in crate::platform_media::youtube_spool) fn first_retained(&self) -> Times { self.retained.first }
    pub(in crate::platform_media::youtube_spool) fn retained(&self) -> &FoldResult { &self.retained }
    pub(in crate::platform_media::youtube_spool) fn leading(&self) -> Option<&FoldResult> { self.leading.as_ref() }
    pub(in crate::platform_media::youtube_spool) fn trailing(&self) -> Option<&FoldResult> { self.trailing.as_ref() }
    pub(in crate::platform_media::youtube_spool) fn input(&self) -> &Summary { &self.tape }
    pub(in crate::platform_media::youtube_spool) fn check_binding(&self, binding: &SelectionBinding) -> Result<()> {
        if &self.binding != binding { return Err(Fault::Identity); }
        Ok(())
    }
}

pub(in crate::platform_media::youtube_spool) struct Partition<'a> {
    binding: SelectionBinding,
    timebase: TimeBase,
    video: (Rational, Rational),
    full: TrackFold,
    folds: [TrackFold; 3],
    counts: [u64; 3],
    count: u64,
    phase: usize,
    previous_end: Option<i64>,
    cancel: &'a AtomicBool,
    deadline: Instant,
    failure: Option<Fault>,
    finished: bool,
}
impl<'a> Partition<'a> {
    pub(in crate::platform_media::youtube_spool) fn new(
        binding: &SelectionBinding, timebase: TimeBase, cancel: &'a AtomicBool, deadline: Instant,
    ) -> Result<Self> {
        checkpoint(cancel, deadline)?;
        let video = interval(binding.range(Track::Video))?;
        let audio_range = binding.range(Track::Audio);
        interval(audio_range)?;
        if timebase != audio_base()? || audio_range.ticks_per_second != AUDIO_SCALE
            || !binding.audio_boundary().allows(binding.range(Track::Video), audio_range).map_err(super::selection_error)?
        { return Err(Fault::Coverage); }
        Ok(Self { binding: binding.clone(), timebase, video, full: TrackFold::new(Track::Audio),
            folds: std::array::from_fn(|_| TrackFold::new(Track::Audio)), counts: [0; 3], count: 0,
            phase: BEFORE, previous_end: None, cancel, deadline, failure: None, finished: false })
    }
    fn available(&self) -> Result<()> {
        if let Some(error) = self.failure { return Err(error); }
        if self.finished { return Err(Fault::Terminal); }
        Ok(())
    }
    pub(in crate::platform_media::youtube_spool) fn push(&mut self, packet: PacketFact) -> Result<()> {
        self.push_region(packet).map(|_| ())
    }
    fn push_region(&mut self, packet: PacketFact) -> Result<usize> {
        self.available()?;
        self.failure = Some(Fault::Terminal);
        let result = (|| {
            checkpoint(self.cancel, self.deadline)?;
            let (time, end) = packet_time(packet)?;
            if self.previous_end.is_some_and(|previous| previous != time.pts) {
                return Err(Fault::Coverage);
            }
            let region = if rational_cmp(seconds(i128::from(end), AUDIO_SCALE)?, self.video.0)?.is_le() {
                BEFORE
            } else if rational_cmp(seconds(i128::from(time.pts), AUDIO_SCALE)?, self.video.1)?.is_ge() {
                AFTER
            } else { RETAINED };
            if region < self.phase { return Err(Fault::Coverage); }
            let count = self.count.checked_add(1).filter(|n| *n <= MAX_PACKETS).ok_or(Fault::Bounds)?;
            self.full.push(packet).map_err(Fault::Packet)?;
            self.folds[region].push(packet).map_err(Fault::Packet)?;
            self.counts[region] = self.counts[region].checked_add(1).ok_or(Fault::Bounds)?;
            self.count = count;
            self.phase = region;
            self.previous_end = Some(end);
            checkpoint(self.cancel, self.deadline)?;
            Ok(region)
        })();
        self.failure = result.as_ref().err().copied();
        result
    }
    pub(in crate::platform_media::youtube_spool) fn finish(&mut self, tape: &Summary) -> Result<AacRetention> {
        self.available()?;
        self.failure = Some(Fault::Terminal);
        self.finished = true;
        let result = (|| {
            checkpoint(self.cancel, self.deadline)?;
            let facts = tape.facts();
            audio_stream(&facts.stream, self.timebase)?;
            let full = self.full.finish().map_err(Fault::Packet)?;
            if full != facts.packets || self.count != full.count || self.counts[RETAINED] == 0 {
                return Err(Fault::Changed);
            }
            let audio = self.binding.range(Track::Audio);
            if i128::from(full.first.pts) != audio.first_tick
                || i128::from(self.previous_end.ok_or(Fault::Coverage)?) != audio.end_tick
                || full.first.duration != i64::from(self.binding.audio_boundary().first_ticks())
                || full.last.duration != i64::from(self.binding.audio_boundary().last_ticks())
            { return Err(Fault::Coverage); }
            let mut parts: [Option<FoldResult>; 3] = [None, None, None];
            for (at, slot) in parts.iter_mut().enumerate() {
                if self.counts[at] != 0 { *slot = Some(self.folds[at].finish().map_err(Fault::Packet)?); }
            }
            let [leading, retained, trailing] = parts;
            let retained = retained.ok_or(Fault::Coverage)?;
            let first = self.counts[BEFORE];
            let end = first.checked_add(retained.count).ok_or(Fault::Bounds)?;
            let mut count = retained.count;
            let mut bytes = retained.bytes;
            for part in [&leading, &trailing].into_iter().flatten() {
                count = count.checked_add(part.count).ok_or(Fault::Bounds)?;
                bytes = bytes.checked_add(part.bytes).ok_or(Fault::Bounds)?;
            }
            let start = seconds(i128::from(retained.first.pts), AUDIO_SCALE)?;
            let finish = seconds(i128::from(retained.last.pts.checked_add(retained.last.duration).ok_or(Fault::Bounds)?), AUDIO_SCALE)?;
            // Existing intersection rule proves overhangs. A native outer-edge
            // deficit cannot be borrowed by an interior retained packet.
            let first_end = seconds(i128::from(retained.first.pts.checked_add(retained.first.duration).ok_or(Fault::Bounds)?), AUDIO_SCALE)?;
            let last_start = seconds(i128::from(retained.last.pts), AUDIO_SCALE)?;
            let retained_range = Range { first_tick: i128::from(retained.first.pts),
                end_tick: i128::from(retained.last.pts.checked_add(retained.last.duration).ok_or(Fault::Bounds)?),
                ..audio };
            if count != full.count || bytes != full.bytes
                || end.checked_add(self.counts[AFTER]) != Some(full.count)
                || (rational_cmp(start, self.video.0)?.is_gt() && leading.is_some())
                || (rational_cmp(finish, self.video.1)?.is_lt() && trailing.is_some())
                || !self.binding.audio_boundary().allows(self.binding.range(Track::Video), retained_range).map_err(super::selection_error)?
                || rational_cmp(first_end, self.video.0)?.is_le() || rational_cmp(last_start, self.video.1)?.is_ge()
            { return Err(Fault::Coverage); }
            checkpoint(self.cancel, self.deadline)?;
            Ok(AacRetention { binding: self.binding.clone(), tape: tape.clone(), leading, retained, trailing, first, end })
        })();
        self.failure = result.as_ref().err().copied();
        result
    }
}

pub(in crate::platform_media::youtube_spool) struct RetainedComparison<'a, R: Read + ?Sized> {
    reader: Reader<'a, R>,
    plan: &'a AacRetention,
    partition: Partition<'a>,
    comparison: PairComparison,
    output: TrackFold,
    output_timebase: TimeBase,
    count: u64,
    cancel: &'a AtomicBool,
    deadline: Instant,
    failure: Option<Fault>,
    finished: bool,
}
impl<'a, R: Read + ?Sized> RetainedComparison<'a, R> {
    pub(in crate::platform_media::youtube_spool) fn new(
        input: &'a mut R, summary: &Summary, plan: &'a AacRetention, binding: &SelectionBinding,
        output_timebase: TimeBase, origin: Rational, cancel: &'a AtomicBool, deadline: Instant,
    ) -> Result<Self> {
        checkpoint(cancel, deadline)?;
        plan.check_binding(binding)?;
        if summary != &plan.tape { return Err(Fault::Changed); }
        let partition = Partition::new(binding, summary.facts().stream.time_base, cancel, deadline)?;
        let comparison = PairComparison::new(Track::Audio, summary.facts().stream.time_base, output_timebase, origin).map_err(Fault::Packet)?;
        let reader = Reader::new(input, &plan.tape).map_err(Fault::Tape)?;
        checkpoint(cancel, deadline)?;
        Ok(Self { reader, plan, partition, comparison, output: TrackFold::new(Track::Audio),
            output_timebase, count: 0, cancel, deadline, failure: None, finished: false })
    }
    fn available(&self) -> Result<()> {
        if let Some(error) = self.failure { return Err(error); }
        if self.finished { return Err(Fault::Terminal); }
        Ok(())
    }
    fn next_source(&mut self) -> Result<Option<(PacketFact, usize)>> {
        checkpoint(self.cancel, self.deadline)?;
        let packet = self.reader.next_fact().map_err(Fault::Tape)?;
        checkpoint(self.cancel, self.deadline)?;
        packet.map(|packet| self.partition.push_region(packet).map(|region| (packet, region))).transpose()
    }
    pub(in crate::platform_media::youtube_spool) fn push(&mut self, packet: PacketFact) -> Result<()> {
        self.available()?;
        self.failure = Some(Fault::Terminal);
        let result = (|| {
            checkpoint(self.cancel, self.deadline)?;
            if self.count >= self.plan.retained.count { return Err(Fault::Changed); }
            let source = loop {
                match self.next_source()? {
                    Some((packet, RETAINED)) => break packet,
                    Some((_, BEFORE)) => (),
                    _ => return Err(Fault::Changed),
                }
            };
            self.comparison.push(source, packet).map_err(Fault::Packet)?;
            self.output.push(packet).map_err(Fault::Packet)?;
            self.count = self.count.checked_add(1).ok_or(Fault::Bounds)?;
            checkpoint(self.cancel, self.deadline)
        })();
        self.failure = result.as_ref().err().copied();
        result
    }
    pub(in crate::platform_media::youtube_spool) fn finish(&mut self, facts: &ProbeFacts) -> Result<PairReport> {
        self.available()?;
        self.failure = Some(Fault::Terminal);
        self.finished = true;
        let result = (|| {
            checkpoint(self.cancel, self.deadline)?;
            audio_stream(&facts.stream, self.output_timebase)?;
            let output = self.output.finish().map_err(Fault::Packet)?;
            if output != facts.packets || self.count != self.plan.retained.count
                || output.count != self.plan.retained.count || output.bytes != self.plan.retained.bytes
                || output.digest != self.plan.retained.digest
            { return Err(Fault::Changed); }
            while let Some((_, region)) = self.next_source()? {
                if region != AFTER { return Err(Fault::Changed); }
            }
            // Reader None has checked record count, full byte hash, fold and
            // physical EOF, including every excluded leading/trailing record.
            let recomputed = self.partition.finish(&self.plan.tape)?;
            if &recomputed != self.plan { return Err(Fault::Changed); }
            let report = self.comparison.finish().map_err(Fault::Packet)?;
            check_report(&report, false)?;
            checkpoint(self.cancel, self.deadline)?;
            Ok(report)
        })();
        self.failure = result.as_ref().err().copied();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_is_not_an_inferred_boundary() {
        assert_eq!(rational_cmp(Rational::new(i128::MAX, 1).unwrap(), Rational::new(1, 2).unwrap()), Err(Fault::Bounds));
        let mut record = [0; 80];
        record[0] = 1;
        record[4..12].copy_from_slice(&i64::MAX.to_be_bytes());
        record[12..20].copy_from_slice(&i64::MAX.to_be_bytes());
        record[20..28].copy_from_slice(&1i64.to_be_bytes());
        record[28..36].copy_from_slice(&1u64.to_be_bytes());
        assert_eq!(packet_time(PacketFact::decode_record(&record).unwrap()), Err(Fault::Bounds));
    }
}
