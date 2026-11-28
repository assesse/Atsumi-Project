//! Measured remote-video -> acknowledged-TS alignment. This module performs no
//! capture, IPC, I/O or playback. Its inputs have no Deserialize implementation:
//! the native owner must first verify the source manifest/timing lease and bind
//! each pixel/clock observation to the selected official-player document.
use super::live_fingerprint::{FrameFeature, ReferenceSet, ReferenceWindow};
use super::relay_timing_read::RelayTimingSummary;
use uuid::Uuid;

// All coordinates are integer rational seconds: ticks / 90_000. Rounded remote
// rVFC mediaTime must include its rounding/capture uncertainty, never wall time.
const HZ: u64 = 90_000;
const MAX_TIME: u64 = 7 * 24 * 60 * 60 * HZ;
const MAX_STEP: u64 = HZ / 5;
const MIN_SPAN: u64 = HZ * 3 / 5;
const MIN_SEPARATION: u64 = HZ * 3;
const MAX_REMOTE_UNCERTAINTY: u32 = 22_500;
const MAX_OFFSET_WIDTH: i64 = 22_500;
const MAX_FRAME_DISTANCE: u32 = 18;
const MAX_MEAN_DISTANCE_MILLI: u32 = 10_000;
const NEAR_BEST_MILLI: u32 = 1_500;
const UNIQUE_WINDOW_MARGIN_MILLI: u32 = 4_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AlignmentBinding {
    pub run_id: Uuid,
    pub source_id: Uuid,
    pub stream_id: Uuid,
    pub video_id: String,
    pub remote_source_id: Uuid,
    pub document_epoch: u64,
    pub source_generation: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct RemoteFrame {
    pub media_pts90k: u64,
    pub uncertainty_ticks: u32,
    pub feature: FrameFeature,
}
/// Created only after native capture checks before/during/after the window.
/// One window cannot cross a source/seek/ad/buffering transition. Deliberately
/// seeking between separately requested stable windows is allowed.
#[derive(Clone, Debug)]
pub(crate) struct RemoteWindow {
    pub capture_id: Uuid,
    pub video_id: String,
    pub document_epoch: u64,
    pub source_id: Uuid,
    pub source_generation: u64,
    pub seek_generation: u64,
    pub ad_observed: bool,
    pub seeking_observed: bool,
    pub buffering_observed: bool,
    pub frames: Vec<RemoteFrame>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlignmentError {
    SourceIdentity,
    SourceShape,
    IncompleteSource,
    RemoteIdentity,
    Boundary,
    Advertisement,
    Clock,
    InsufficientWindows,
    Static,
    NoMatch,
    Ambiguous,
    ReusedWindow,
    OffsetDisagreement,
    Uncertainty,
}
type Result<T> = std::result::Result<T, AlignmentError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OffsetInterval90k {
    /// transport = remote + offset. A negative offset is valid when observed.
    pub lower: i64,
    pub upper: i64,
}
#[derive(Clone, Debug)]
pub(crate) struct WindowMatch {
    pub source_ordinal: u64,
    pub segment_sha256: String,
    pub capture_id: Uuid,
    pub seek_generation: u64,
    pub matched_frames: u16,
    pub mean_hamming_milli: u32,
    pub source_first90k: u64,
    pub source_last90k: u64,
    pub remote_first90k: u64,
    pub remote_last90k: u64,
    pub offset: OffsetInterval90k,
}
/// A bounded visual measurement, not cryptographic content identity or proof
/// of every intervening frame/chat timestamp. The caller retains the native
/// source seal, capture authority and this binding before using the interval.
#[derive(Clone, Debug)]
pub(crate) struct MeasuredAlignment {
    pub binding: AlignmentBinding,
    pub offset: OffsetInterval90k,
    pub windows: Vec<WindowMatch>,
    pub two_window_estimate: bool,
    pub history_incomplete: bool,
}

/// No timestamp, successful HTTP response or renderer JSON can independently
/// call this a verified archive. Only the native selected replay owner calls
/// this with its admitted source reference and frame-capture observations.
pub(crate) fn align(
    source: &ReferenceSet,
    timing: &RelayTimingSummary,
    binding: &AlignmentBinding,
    remote: &[RemoteWindow],
) -> Result<MeasuredAlignment> {
    validate_source(source, timing, binding)?;
    validate_remote(remote, binding)?;
    let mut used = Vec::new();
    let mut windows = Vec::new();
    for observed in remote {
        let mut choices = Vec::new();
        for (index, reference) in source.windows.iter().enumerate() {
            if let Some(fit) = fit_window(reference, observed)? {
                choices.push((index, fit));
            }
        }
        choices.sort_by_key(|(_, fit)| fit.mean_hamming_milli);
        let Some((index, best)) = choices.first() else {
            return Err(AlignmentError::NoMatch);
        };
        if choices.get(1).is_some_and(|(_, second)| {
            second.mean_hamming_milli < best.mean_hamming_milli + UNIQUE_WINDOW_MARGIN_MILLI
        }) {
            return Err(AlignmentError::Ambiguous);
        }
        if used.contains(index) {
            return Err(AlignmentError::ReusedWindow);
        }
        if used.last().is_some_and(|previous| previous >= index) {
            return Err(AlignmentError::Boundary);
        }
        used.push(*index);
        windows.push(best.clone());
    }
    // Intersect only to test whether one constant offset is possible. Report
    // the hull, not the artificially narrow intersection: visual sampling is
    // correlated, so more matches must not fabricate sub-frame precision.
    let intersection_low = windows.iter().map(|w| w.offset.lower).max().unwrap();
    let intersection_high = windows.iter().map(|w| w.offset.upper).min().unwrap();
    if intersection_low > intersection_high {
        return Err(AlignmentError::OffsetDisagreement);
    }
    let offset = OffsetInterval90k {
        lower: windows.iter().map(|w| w.offset.lower).min().unwrap(),
        upper: windows.iter().map(|w| w.offset.upper).max().unwrap(),
    };
    if offset.upper - offset.lower > MAX_OFFSET_WIDTH {
        return Err(AlignmentError::Uncertainty);
    }
    Ok(MeasuredAlignment {
        binding: binding.clone(),
        offset,
        two_window_estimate: windows.len() == 2,
        windows,
        history_incomplete: timing.sidecar_incomplete,
    })
}

fn validate_source(s: &ReferenceSet, t: &RelayTimingSummary, b: &AlignmentBinding) -> Result<()> {
    if b.run_id.is_nil()
        || b.source_id.is_nil()
        || b.stream_id.is_nil()
        || s.run_id != b.run_id
        || s.source_id != b.source_id
        || s.stream_id != Some(b.stream_id)
        || t.stream_id != Some(b.stream_id)
    {
        return Err(AlignmentError::SourceIdentity);
    }
    if !(2..=3).contains(&s.windows.len()) {
        return Err(AlignmentError::InsufficientWindows);
    }
    if s.cancelled
        || s.queue_dropped != 0
        || s.decode_failures != 0
        || (s.incomplete && s.windows.len() == 3)
        || !t.transport_complete
        || !t.end_seen
        || t.timing_incomplete
    {
        return Err(AlignmentError::IncompleteSource);
    }
    let (Some(origin), Some(video), Some(end)) = (t.origin, t.video, t.transport_end_90k) else {
        return Err(AlignmentError::SourceShape);
    };
    if s.version != 1
        || s.remote_anchor != "unknown"
        || end == 0
        || end > MAX_TIME
        || origin.timescale == 0
        || video.timescale == 0
        || s.acknowledged_segments == 0
        || s.acknowledged_segments != t.acknowledged_segments
        || t.prefix_segments != t.acknowledged_segments
        || s.sampled_segments < s.windows.len() as u64
        || s.sampled_segments > s.acknowledged_segments
    {
        return Err(AlignmentError::SourceShape);
    }
    for (i, w) in s.windows.iter().enumerate() {
        if w.ordinal >= s.acknowledged_segments
            || !hex(&w.segment_sha256)
            || w.source_origin_ticks != origin.ticks
            || w.source_origin_timescale != origin.timescale
            || w.source_video_timescale != video.timescale
            || !(8..=72).contains(&w.frames.len())
        {
            return Err(AlignmentError::SourceShape);
        }
        validate_clock(w.frames.iter().map(|f| f.pts90k))?;
        if w.frames.last().unwrap().pts90k >= end {
            return Err(AlignmentError::Clock);
        }
        if !dynamic(w.frames.iter().map(|f| &f.feature)) {
            return Err(AlignmentError::Static);
        }
        if i > 0 {
            let old = &s.windows[i - 1];
            if old.ordinal >= w.ordinal
                || old.frames.last().unwrap().pts90k >= w.frames[0].pts90k
                || center(w) - center(old) < MIN_SEPARATION
                || s.windows[..i]
                    .iter()
                    .any(|v| v.segment_sha256 == w.segment_sha256)
            {
                return Err(AlignmentError::SourceShape);
            }
        }
    }
    Ok(())
}
fn validate_remote(windows: &[RemoteWindow], b: &AlignmentBinding) -> Result<()> {
    if !(2..=3).contains(&windows.len()) {
        return Err(AlignmentError::InsufficientWindows);
    }
    if !video_id(&b.video_id) || b.remote_source_id.is_nil() {
        return Err(AlignmentError::RemoteIdentity);
    }
    for (i, w) in windows.iter().enumerate() {
        if w.capture_id.is_nil()
            || w.video_id != b.video_id
            || w.document_epoch != b.document_epoch
            || w.source_id != b.remote_source_id
            || w.source_generation != b.source_generation
        {
            return Err(AlignmentError::RemoteIdentity);
        }
        if w.ad_observed {
            return Err(AlignmentError::Advertisement);
        }
        if w.seeking_observed || w.buffering_observed {
            return Err(AlignmentError::Boundary);
        }
        if !(8..=48).contains(&w.frames.len()) {
            return Err(AlignmentError::Clock);
        }
        validate_clock(w.frames.iter().map(|f| f.media_pts90k))?;
        if w.frames
            .iter()
            .any(|f| f.uncertainty_ticks == 0 || f.uncertainty_ticks > MAX_REMOTE_UNCERTAINTY)
        {
            return Err(AlignmentError::Uncertainty);
        }
        if !dynamic(w.frames.iter().map(|f| &f.feature)) {
            return Err(AlignmentError::Static);
        }
        if i > 0 {
            let old = &windows[i - 1];
            if old.seek_generation > w.seek_generation
                || old.frames.last().unwrap().media_pts90k >= w.frames[0].media_pts90k
                || windows[..i].iter().any(|v| v.capture_id == w.capture_id)
            {
                return Err(AlignmentError::Boundary);
            }
        }
    }
    Ok(())
}
fn video_id(s: &str) -> bool {
    s.len() == 11
        && s.bytes()
            .all(|v| v.is_ascii_alphanumeric() || v == b'-' || v == b'_')
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn center(w: &ReferenceWindow) -> u64 {
    let first = w.frames[0].pts90k;
    first + (w.frames.last().unwrap().pts90k - first) / 2
}
fn validate_clock(values: impl Iterator<Item = u64>) -> Result<()> {
    let values: Vec<_> = values.collect();
    if values.iter().any(|v| *v > MAX_TIME)
        || values
            .windows(2)
            .any(|p| p[1] <= p[0] || p[1] - p[0] > MAX_STEP)
        || values.last().unwrap() - values[0] < MIN_SPAN
    {
        return Err(AlignmentError::Clock);
    }
    Ok(())
}
fn hamming(a: &FrameFeature, b: &FrameFeature) -> u32 {
    a.gradient
        .iter()
        .zip(&b.gradient)
        .map(|(a, b)| (a ^ b).count_ones())
        .sum()
}
fn distance(a: &FrameFeature, b: &FrameFeature) -> u32 {
    if a.mean.abs_diff(b.mean) > 48 || a.spread.abs_diff(b.spread) > 48 {
        129
    } else {
        hamming(a, b)
    }
}
fn dynamic<'a>(features: impl Iterator<Item = &'a FrameFeature>) -> bool {
    let features: Vec<_> = features.collect();
    let mut distinct = Vec::new();
    for feature in &features {
        if !distinct.contains(&feature.gradient) {
            distinct.push(feature.gradient);
        }
    }
    distinct.len() >= 6
        && features.iter().filter(|f| f.spread >= 8).count() * 4 >= features.len() * 3
        && features
            .windows(2)
            .map(|p| hamming(p[0], p[1]))
            .sum::<u32>()
            >= 32
}

#[derive(Clone)]
struct Candidate {
    offset: i64,
    allowance: i64,
    matched: usize,
    score: u32,
    source_first: u64,
    source_last: u64,
    remote_first: u64,
    remote_last: u64,
}
fn evaluate(
    s: &ReferenceWindow,
    r: &RemoteWindow,
    offset: i64,
    allowance: i64,
) -> Option<Candidate> {
    let mut matched = 0;
    let mut sum = 0;
    let mut previous = None;
    let mut first = None;
    let mut last = None;
    for frame in &r.frames {
        let predicted = frame.media_pts90k as i64 + offset;
        let upper = s.frames.partition_point(|f| (f.pts90k as i64) < predicted);
        let index = [
            upper.checked_sub(1),
            (upper < s.frames.len()).then_some(upper),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|i| (s.frames[*i].pts90k as i64 - predicted).unsigned_abs())?;
        if previous.is_some_and(|old| index <= old) {
            continue;
        }
        let source = &s.frames[index];
        if (source.pts90k as i64 - predicted).abs() > allowance {
            continue;
        }
        let d = distance(&source.feature, &frame.feature);
        if d > MAX_FRAME_DISTANCE {
            continue;
        }
        previous = Some(index);
        matched += 1;
        sum += d;
        let pair = (source.pts90k, frame.media_pts90k);
        first.get_or_insert(pair);
        last = Some(pair);
    }
    if matched < 8 || matched * 5 < r.frames.len() * 4 {
        return None;
    }
    let (first, last) = (first?, last?);
    let remote_span = r.frames.last()?.media_pts90k - r.frames[0].media_pts90k;
    if last.0 - first.0 < MIN_SPAN || (last.1 - first.1) * 4 < remote_span * 3 {
        return None;
    }
    let score = sum * 1_000 / matched as u32;
    (score <= MAX_MEAN_DISTANCE_MILLI).then_some(Candidate {
        offset,
        allowance,
        matched,
        score,
        source_first: first.0,
        source_last: last.0,
        remote_first: first.1,
        remote_last: last.1,
    })
}
fn fit_window(s: &ReferenceWindow, r: &RemoteWindow) -> Result<Option<WindowMatch>> {
    let step = s
        .frames
        .windows(2)
        .map(|p| p[1].pts90k - p[0].pts90k)
        .max()
        .unwrap();
    let uncertainty = r.frames.iter().map(|f| f.uncertainty_ticks).max().unwrap();
    // Sample phase and explicit native clock precision, not a guessed latency.
    let allowance = (step.div_ceil(2) + u64::from(uncertainty) + 1) as i64;
    if allowance * 2 > MAX_OFFSET_WIDTH {
        return Err(AlignmentError::Uncertainty);
    }
    let mut offsets = Vec::new();
    for index in 0..8 {
        let sample = &s.frames[index * (s.frames.len() - 1) / 7];
        for frame in &r.frames {
            if distance(&sample.feature, &frame.feature) <= MAX_FRAME_DISTANCE {
                offsets.push(sample.pts90k as i64 - frame.media_pts90k as i64);
            }
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    let mut candidates: Vec<_> = offsets
        .into_iter()
        .filter_map(|offset| evaluate(s, r, offset, allowance))
        .collect();
    candidates.sort_by_key(|c| (c.score, std::cmp::Reverse(c.matched)));
    let Some(best) = candidates.first() else {
        return Ok(None);
    };
    let plausible: Vec<_> = candidates
        .iter()
        .filter(|c| {
            c.score <= best.score + NEAR_BEST_MILLI
                && c.matched + r.frames.len() / 10 + 1 >= best.matched
        })
        .collect();
    let interval = OffsetInterval90k {
        lower: plausible
            .iter()
            .map(|c| c.offset - c.allowance)
            .min()
            .unwrap(),
        upper: plausible
            .iter()
            .map(|c| c.offset + c.allowance)
            .max()
            .unwrap(),
    };
    // Several separated equally good phases indicate repeated content. Do not
    // select the first match or use a metadata/wall-clock guess to break ties.
    if interval.upper - interval.lower > MAX_OFFSET_WIDTH {
        return Err(AlignmentError::Ambiguous);
    }
    Ok(Some(WindowMatch {
        source_ordinal: s.ordinal,
        segment_sha256: s.segment_sha256.clone(),
        capture_id: r.capture_id,
        seek_generation: r.seek_generation,
        matched_frames: best.matched as u16,
        mean_hamming_milli: best.score,
        source_first90k: best.source_first,
        source_last90k: best.source_last,
        remote_first90k: best.remote_first,
        remote_last90k: best.remote_last,
        offset: interval,
    }))
}

#[cfg(test)]
#[path = "live_alignment_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "native_live_alignment_qa.rs"]
mod native_alignment_qa;
#[cfg(all(test, windows))]
pub(crate) use native_alignment_qa::verify_selected_live_alignment;
