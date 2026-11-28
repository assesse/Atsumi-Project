//! Sealed source samples -> bounded, self-initializing AVC/AAC transport segments.
//! No I/O, process, account, capture, or transport completion authority.
use super::*;
use uuid::Uuid;

const SEGMENT_BYTES: usize = 16 * 1024 * 1024;
const TARGET: u64 = 180_000;
const MAX_DURATION: u64 = 450_000;
const VIDEO_PID: u16 = 256;
const AUDIO_PID: u16 = 257;
const PMT_PID: u16 = 4096;

pub(crate) struct TsSegment {
    bytes: Arc<[u8]>,
    ordinal: u64,
    stream_id: Uuid,
    duration_ms: u64,
    duration_ticks: u64,
    // Exact source ticks are retained independently of 90kHz wire rounding.
    video: SampleRange,
    audio: SampleRange,
    origin: SourceOrigin,
    timing: TsTiming,
}
/// Only the validated muxer can construct these facts. Source ticks remain
/// rational; wire ticks describe the emitted PES clock, not an archive clock.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TsTiming {
    origin_ticks: u64,
    origin_timescale: u32,
    segment_start_90k: u64,
    segment_end_90k: u64,
    video: TrackTiming,
    audio: TrackTiming,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackTiming {
    timescale: u32,
    first_dts: u64,
    last_dts_end: u64,
    min_pts: u64,
    max_pts_end: u64,
    samples: u64,
    first_dts_90k: u64,
    last_dts_end_90k: u64,
    min_pts_90k: u64,
    max_pts_end_90k: u64,
}
fn track_timing(samples: &VecDeque<Sample>, count: usize, scale: u32, origin: SourceOrigin) -> Result<TrackTiming> {
    let first = samples.front().ok_or_else(bound)?;
    let last = samples.get(count.checked_sub(1).ok_or_else(bound)?).ok_or_else(bound)?;
    let mut low = u64::MAX;
    let mut high = 0;
    for s in samples.iter().take(count) {
        let p = u64::try_from(pts(s)?).map_err(|_| bound())?;
        low = low.min(p);
        high = high.max(p.checked_add(u64::from(s.duration)).ok_or_else(bound)?);
    }
    Ok(TrackTiming { timescale: scale, first_dts:first.dts, last_dts_end:last.end(),
        min_pts:low, max_pts_end:high, samples:count as u64,
        first_dts_90k:clock(i128::from(first.dts),scale,origin)?,
        last_dts_end_90k:clock(i128::from(last.end()),scale,origin)?,
        min_pts_90k:clock(i128::from(low),scale,origin)?, max_pts_end_90k:clock(i128::from(high),scale,origin)? })
}
impl TsSegment {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn bytes_arc(&self) -> Arc<[u8]> {
        self.bytes.clone()
    }
    pub(crate) fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    /// Exact difference on the single90kHz wire clock; playlists must not sum
    /// the rounded millisecond presentation value above.
    pub(crate) fn duration_ticks(&self) -> u64 {
        self.duration_ticks
    }
    pub(crate) fn ordinal(&self) -> u64 {
        self.ordinal
    }
    pub(crate) fn stream_id(&self) -> Uuid {
        self.stream_id
    }
    pub(crate) fn timing(&self) -> TsTiming { self.timing }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TsEnd {
    stream_id: Uuid,
    segments: u64,
    origin: SourceOrigin,
    preroll: PrerollCounts,
}
impl TsEnd {
    pub(crate) fn segments(&self) -> u64 {
        self.segments
    }
    pub(crate) fn stream_id(&self) -> Uuid {
        self.stream_id
    }
    pub(crate) fn origin(&self) -> SourceOrigin { self.origin }
    pub(crate) fn preroll(&self) -> PrerollCounts { self.preroll }
}
struct Codec {
    parameters: Vec<Vec<u8>>,
    frequency: u8,
    channels: u8,
}

/// Fixed metadata from the actual first failed TS validation, never sample bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TsRejectReason {
    AacFrame,
    VideoPresentationBoundary,
    AvcAccessUnit,
    InputOrder,
    GopBoundary,
    TransportClock,
    Bounds,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RejectedAacSample {
    pending_index: usize,
    normalized_dts: u64,
    normalized_duration: u32,
    cts: i32,
    payload_bytes: usize,
    nonzero_cts: bool,
    duration_not_1024: bool,
    duration_not_proven_aac1024: bool,
    empty_payload: bool,
    payload_too_large: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TsRejection {
    reason: TsRejectReason,
    aac_sample: Option<RejectedAacSample>,
}
fn proven_aac1024(s: &Sample, timescale: u32) -> bool {
    let Some(rounding) = &s.aac_rounding else { return s.duration == 1024; };
    let delta = rounding.original_dts.abs_diff(s.dts);
    let bound = u64::from(timescale / 1000).min(u64::from(rounding.original_duration / 16));
    rounding.original_duration == 1024 && rounding.timescale == timescale && timescale > 0
        && delta > 0 && delta <= bound && s.duration > 0
        && rounding.original_dts.checked_add(u64::from(rounding.original_duration))
            .zip(s.dts.checked_add(u64::from(s.duration))).is_some_and(|(a,b)| a == b)
}
fn rejected_aac(s: &Sample, pending_index: usize, timescale: u32) -> Option<RejectedAacSample> {
    let sample = RejectedAacSample {
        pending_index, normalized_dts: s.dts, normalized_duration: s.duration,
        cts: s.cts, payload_bytes: s.data.len(), nonzero_cts: s.cts != 0,
        duration_not_1024: s.duration != 1024, empty_payload: s.data.is_empty(),
        duration_not_proven_aac1024: !proven_aac1024(s, timescale),
        payload_too_large: s.data.len() > 8191 - 7,
    };
    (sample.nonzero_cts || sample.duration_not_proven_aac1024 || sample.empty_payload || sample.payload_too_large)
        .then_some(sample)
}
fn rejection_reason(error: &StreamError) -> TsRejectReason {
    if error.code == "ENCODED_TS_CLOCK_UNSUPPORTED" { return TsRejectReason::TransportClock; }
    match error.message.as_str() {
        "ADTS AAC-LC frame 범위가 올바르지 않습니다." => TsRejectReason::AacFrame,
        "영상 표시 시각이 IDR 조각 경계를 벗어납니다." => TsRejectReason::VideoPresentationBoundary,
        "AVC forbidden bit입니다." | "IDR access unit과 샘플 표시가 다릅니다."
        | "AVC parameter set이 바뀌었습니다." | "TS에서 검증하지 않은 AVC NAL 종류입니다."
        | "AVC access unit에 영상 slice가 없습니다." => TsRejectReason::AvcAccessUnit,
        "다른 입력 또는 순서가 변경된 TS 샘플입니다." | "TS 입력 종료의 소유자가 다릅니다." => TsRejectReason::InputOrder,
        "TS 조각은 IDR로 시작해야 합니다." | "TS IDR 간격이 5초를 초과했습니다."
        | "5초 안에 다음 IDR이 없습니다." | "TS 조각의 오디오가 없습니다."
        | "IDR 경계까지 오디오가 도착하지 않았습니다." | "TS 마지막 오디오가 없습니다."
        | "TS 마지막 트랙 범위가 완결되지 않았습니다." => TsRejectReason::GopBoundary,
        "압축 영상 저장 버퍼 한도를 초과했습니다." => TsRejectReason::Bounds,
        _ => TsRejectReason::Other,
    }
}

pub(crate) struct TsMuxer {
    // The existing exact startup selection and preroll accounting have one owner.
    base: ContinuousMp4Muxer,
    codec: Codec,
    stream_id: Uuid,
    segments: u64,
    continuity: [u8; 4],
    segment_start: u64,
    last_pcr: Option<u64>,
    rejection: Option<TsRejection>,
}

fn configuration(d: &Description) -> Result<Codec> {
    let roots = atoms(&d.init)?;
    let moov = atoms(one(&roots, b"moov")?.data())?;
    let mut video = None;
    let mut audio = None;
    for trak in moov.iter().filter(|a| a.kind == *b"trak") {
        let t = atoms(trak.data())?;
        let mdia = atoms(one(&t, b"mdia")?.data())?;
        let h = one(&mdia, b"hdlr")?;
        let is_video = h.data().get(8..12) == Some(b"vide");
        let minf = atoms(one(&mdia, b"minf")?.data())?;
        let stbl = atoms(one(&minf, b"stbl")?.data())?;
        let stsd = one(&stbl, b"stsd")?;
        let entry = atoms(stsd.data().get(8..).ok_or_else(bound)?)?;
        if entry.len() != 1 {
            return Err(bound());
        }
        let ext = atoms(
            entry[0]
                .data()
                .get(if is_video { 78 } else { 28 }..)
                .ok_or_else(bound)?,
        )?;
        if is_video {
            if video.is_some() {
                return Err(bound());
            }
            let avcc = one(&ext, b"avcC")?.data();
            validate_avcc(avcc)?;
            let mut at = 6;
            let mut params = Vec::new();
            for (kind, count) in [(7, avcc[5] & 31), (8, 0)] {
                let count = if kind == 8 {
                    let n = *avcc.get(at).ok_or_else(bound)?;
                    at += 1;
                    n
                } else {
                    count
                };
                for _ in 0..count {
                    let n = u16::from_be_bytes(
                        avcc.get(at..at + 2).ok_or_else(bound)?.try_into().unwrap(),
                    ) as usize;
                    at += 2;
                    let nal = avcc.get(at..at + n).ok_or_else(bound)?;
                    at += n;
                    if nal.is_empty() || nal[0] & 0x80 != 0 || nal[0] & 31 != kind {
                        return Err(bad("AVC parameter set 형식이 올바르지 않습니다."));
                    }
                    params.push(nal.to_vec());
                }
            }
            video = Some(params);
        } else {
            if audio.is_some() {
                return Err(bound());
            }
            let esds = one(&ext, b"esds")?.data();
            validate_esds(esds)?;
            let (_, es) = descriptor(esds, &mut 4)?;
            let (_, decoder) = descriptor(&es, &mut 3)?;
            let (_, asc) = descriptor(&decoder, &mut 13)?;
            // ADTS does not carry GASpecificConfig dependencies or 960-frame mode.
            if asc[1] & 7 != 0 || asc[2..].iter().any(|b| *b != 0) {
                return Err(bad("ADTS로 보존할 수 없는 AAC 확장입니다."));
            }
            let frequency = ((asc[0] & 7) << 1) | (asc[1] >> 7);
            let channels = (asc[1] >> 3) & 15;
            let rates = [
                96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
                7350,
            ];
            if rates.get(frequency as usize).copied() != Some(d.audio_scale) {
                return Err(bad("AAC 시계와 codec 샘플레이트가 다릅니다."));
            }
            audio = Some((frequency, channels));
        }
    }
    let (frequency, channels) = audio.ok_or_else(bound)?;
    Ok(Codec {
        parameters: video.ok_or_else(bound)?,
        frequency,
        channels,
    })
}

// Each timestamp is rescaled from its original rational clock. Never accumulate
// rounded durations or independently reset tracks at a segment boundary.
fn clock(ticks: i128, scale: u32, origin: SourceOrigin) -> Result<u64> {
    let value = ticks
        .checked_mul(i128::from(origin.timescale))
        .and_then(|v| v.checked_sub(i128::from(origin.ticks) * i128::from(scale)))
        .ok_or_else(bound)?;
    if value < 0 || scale == 0 || origin.timescale == 0 {
        return Err(bad("공통 시작점보다 이른 시각입니다."));
    }
    let denominator = u128::from(scale) * u128::from(origin.timescale);
    let n = (value as u128)
        .checked_mul(90000)
        .and_then(|v| v.checked_add(denominator / 2))
        .ok_or_else(bound)?;
    u64::try_from(n / denominator).map_err(|_| bound())
}
fn pts(s: &Sample) -> Result<i128> {
    i128::from(s.dts)
        .checked_add(i128::from(s.cts))
        .ok_or_else(bound)
}
fn unsupported_clock() -> StreamError {
    StreamError::new(
        "ENCODED_TS_CLOCK_UNSUPPORTED",
        "이 영상의 표시·디코드 시각은 별도 TS 전송 시계 보정이 필요합니다.",
        false,
    )
}
fn transport_clock(dts: u64, presentation: u64, pcr: u64) -> Result<()> {
    if presentation < dts || pcr > dts {
        return Err(unsupported_clock());
    }
    Ok(())
}
fn append(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if out
        .len()
        .checked_add(bytes.len())
        .is_none_or(|n| n > SEGMENT_BYTES)
    {
        return Err(bound());
    }
    out.extend_from_slice(bytes);
    Ok(())
}
fn crc(bytes: &[u8]) -> u32 {
    let mut value = u32::MAX;
    for b in bytes {
        value ^= u32::from(*b) << 24;
        for _ in 0..8 {
            value = if value & 0x80000000 != 0 {
                (value << 1) ^ 0x04c11db7
            } else {
                value << 1
            };
        }
    }
    value
}
fn stamp(value: u64, prefix: u8) -> [u8; 5] {
    let v = value & 0x1ffffffff;
    [
        prefix | (((v >> 30) as u8 & 7) << 1) | 1,
        (v >> 22) as u8,
        (((v >> 15) as u8 & 127) << 1) | 1,
        (v >> 7) as u8,
        ((v as u8 & 127) << 1) | 1,
    ]
}

impl TsMuxer {
    pub(crate) fn rejection(&self) -> Option<TsRejection> { self.rejection }
    pub(crate) fn sample_state(&self) -> SampleState {
        self.base.sample_state()
    }
    pub(crate) fn new(descriptor: SampleDescriptor) -> Result<Self> {
        let codec = configuration(&descriptor.0)?;
        Ok(Self {
            base: ContinuousMp4Muxer::new(descriptor),
            codec,
            stream_id: Uuid::new_v4(),
            segments: 0,
            continuity: [0; 4],
            segment_start: 0,
            last_pcr: None,
            rejection: None,
        })
    }
    fn poison(&mut self) {
        self.base.closed = true;
        self.base.video.clear();
        self.base.audio.clear();
    }
    pub(crate) fn push(&mut self, batch: ValidatedSampleBatch) -> Result<Vec<TsSegment>> {
        if self.base.closed {
            return Err(bad("중단된 TS 출력입니다."));
        }
        let result = self.push_inner(batch);
        if let Err(error) = &result {
            self.rejection.get_or_insert(TsRejection { reason: rejection_reason(error), aac_sample: None });
            self.poison();
        }
        result
    }
    fn push_inner(&mut self, mut batch: ValidatedSampleBatch) -> Result<Vec<TsSegment>> {
        if !Arc::ptr_eq(&self.base.descriptor.0, &batch.descriptor.0)
            || batch.ordinal != self.base.ordinal
        {
            return Err(bad("다른 입력 또는 순서가 변경된 TS 샘플입니다."));
        }
        let count = self
            .base
            .video
            .len()
            .checked_add(self.base.audio.len())
            .and_then(|n| n.checked_add(batch.video.len()))
            .and_then(|n| n.checked_add(batch.audio.len()))
            .ok_or_else(bound)?;
        let bytes = accounted(
            self.base
                .video
                .iter()
                .chain(&self.base.audio)
                .chain(&batch.video)
                .chain(&batch.audio)
                .map(|s| s.data.len()),
        )?;
        if count > MAX_SAMPLES || bytes > MAX_PENDING {
            return Err(bound());
        }
        self.base.ordinal = self.base.ordinal.checked_add(1).ok_or_else(bound)?;
        self.base.video.append(&mut batch.video);
        self.base.audio.append(&mut batch.audio);
        if self.base.origin.is_none() && !self.base.start()? {
            return Ok(Vec::new());
        }
        self.ready(false)
    }
    fn ready(&mut self, finishing: bool) -> Result<Vec<TsSegment>> {
        let d = &self.base.descriptor.0;
        let vs = d.video_scale;
        let ascale = d.audio_scale;
        let origin = self.base.origin.ok_or_else(bound)?;
        let mut result = Vec::new();
        let mut total = 0usize;
        loop {
            let Some(first) = self.base.video.front() else {
                break;
            };
            if !first.key() {
                return Err(bad("TS 조각은 IDR로 시작해야 합니다."));
            }
            let start_dts = clock(i128::from(first.dts), vs, origin)?;
            let cut = self
                .base
                .video
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, s)| {
                    s.key()
                        && u128::from(s.dts - first.dts) * 90000
                            >= u128::from(TARGET) * u128::from(vs)
                })
                .map(|(i, s)| (i, s.dts, pts(s)));
            let (nv, na, end_clock, next_start) = if let Some((nv, cut_dts, cut_pts)) = cut {
                let end_clock = clock(cut_pts?, vs, origin)?;
                if end_clock
                    .checked_sub(self.segment_start)
                    .is_none_or(|n| n > MAX_DURATION)
                {
                    return Err(bad("TS IDR 간격이 5초를 초과했습니다."));
                }
                let na = self
                    .base
                    .audio
                    .iter()
                    .take_while(|s| before(s.dts, ascale, cut_dts, vs))
                    .count();
                if na == 0 {
                    if finishing {
                        return Err(bad("TS 조각의 오디오가 없습니다."));
                    }
                    break;
                }
                let last = &self.base.audio[na - 1];
                if before(last.end(), ascale, cut_dts, vs) {
                    if finishing {
                        return Err(bad("IDR 경계까지 오디오가 도착하지 않았습니다."));
                    }
                    break;
                }
                (nv, na, end_clock, end_clock)
            } else {
                let last = self.base.video.back().ok_or_else(bound)?;
                let end = clock(i128::from(last.end()), vs, origin)?;
                if end.checked_sub(start_dts).is_none_or(|n| n > MAX_DURATION) {
                    return Err(bad("5초 안에 다음 IDR이 없습니다."));
                }
                if !finishing {
                    break;
                }
                let audio_last = self
                    .base
                    .audio
                    .back()
                    .ok_or_else(|| bad("TS 마지막 오디오가 없습니다."))?;
                let ae = clock(i128::from(audio_last.end()), ascale, origin)?;
                let tolerance = (u64::from(last.duration) * 90000 / u64::from(vs))
                    .max(u64::from(audio_last.duration) * 90000 / u64::from(ascale))
                    + 1;
                if end.abs_diff(ae) > tolerance {
                    return Err(bad("TS 마지막 트랙 범위가 완결되지 않았습니다."));
                }
                let presentation_end = self.base.video.iter().try_fold(0u64, |max, s| {
                    clock(pts(s)? + i128::from(s.duration), vs, origin).map(|p| max.max(p))
                })?;
                let physical = presentation_end.max(ae);
                (
                    self.base.video.len(),
                    self.base.audio.len(),
                    physical,
                    physical,
                )
            };
            let segment = self.emit(nv, na, end_clock)?;
            total = total.checked_add(segment.bytes.len()).ok_or_else(bound)?;
            if total > MAX_PENDING {
                return Err(bound());
            }
            result.push(segment);
            self.segment_start = next_start;
        }
        Ok(result)
    }
    pub(crate) fn finish(&mut self, end: ReaderEnd) -> Result<(Vec<TsSegment>, TsEnd)> {
        if self.base.closed {
            return Err(bad("중단된 TS 출력입니다."));
        }
        let result = (|| {
            if !Arc::ptr_eq(&self.base.descriptor.0, &end.descriptor.0)
                || end.ordinal != self.base.ordinal
            {
                return Err(bad("TS 입력 종료의 소유자가 다릅니다."));
            }
            let segments = self.ready(true)?;
            if !self.base.video.is_empty()
                || !self.base.audio.is_empty()
                || self.base.video_end != end.video_end
                || self.base.audio_end != end.audio_end
                || self.segments == 0
            {
                return Err(bad("TS 입력 끝을 확인하지 못했습니다."));
            }
            Ok((
                segments,
                TsEnd {
                    stream_id: self.stream_id,
                    segments: self.segments,
                    origin: self.base.origin.ok_or_else(bound)?,
                    preroll: self.base.preroll,
                },
            ))
        })();
        if let Err(error) = &result {
            self.rejection.get_or_insert(TsRejection { reason: rejection_reason(error), aac_sample: None });
        }
        self.poison();
        result
    }
    fn emit(&mut self, nv: usize, na: usize, end_clock: u64) -> Result<TsSegment> {
        let origin = self.base.origin.ok_or_else(bound)?;
        let vs = self.base.descriptor.0.video_scale;
        let ascale = self.base.descriptor.0.audio_scale;
        let duration = end_clock
            .checked_sub(self.segment_start)
            .filter(|v| *v > 0 && *v <= MAX_DURATION)
            .ok_or_else(bound)?;
        let first = self.base.video.front().ok_or_else(bound)?;
        let idr_pts = pts(first)?;
        for s in self.base.video.iter().take(nv) {
            let p = pts(s)?;
            // MP4 signed composition offsets are preserved by the reader. This
            // TS path has not established a decoder-clock rebase for PTS<DTS.
            // Reject before rounding can conceal a sub90kHz-tick difference.
            if p < i128::from(s.dts) {
                return Err(unsupported_clock());
            }
            let p90 = clock(p, vs, origin)?;
            if p < idr_pts
                || p90 >= end_clock
                || clock(p + i128::from(s.duration), vs, origin)? > end_clock + 1
            {
                return Err(bad("영상 표시 시각이 IDR 조각 경계를 벗어납니다."));
            }
            self.annex_b(s)?;
        }
        for (index, s) in self.base.audio.iter().take(na).enumerate() {
            if let Some(sample) = rejected_aac(s, index, ascale) {
                self.rejection.get_or_insert(TsRejection { reason: TsRejectReason::AacFrame, aac_sample: Some(sample) });
                return Err(bad("ADTS AAC-LC frame 범위가 올바르지 않습니다."));
            }
            self.adts(s)?;
        }
        let video = SampleRange {
            timescale: vs,
            first_dts: first.dts,
            last_end: self.base.video[nv - 1].end(),
            samples: nv as u64,
        };
        let audio = SampleRange {
            timescale: ascale,
            first_dts: self.base.audio[0].dts,
            last_end: self.base.audio[na - 1].end(),
            samples: na as u64,
        };
        let timing = TsTiming { origin_ticks:origin.ticks, origin_timescale:origin.timescale,
            segment_start_90k:self.segment_start, segment_end_90k:end_clock,
            video:track_timing(&self.base.video,nv,vs,origin)?,
            audio:track_timing(&self.base.audio,na,ascale,origin)? };
        let mut out = Vec::new();
        self.tables(&mut out)?;
        let (mut vi, mut ai) = (0, 0);
        while vi < nv || ai < na {
            let choose_video = ai == na
                || (vi < nv
                    && before(self.base.video[vi].dts, vs, self.base.audio[ai].dts, ascale));
            let (payload, dts, presentation, key, pid, index) = if choose_video {
                let s = &self.base.video[vi];
                vi += 1;
                (
                    self.annex_b(s)?,
                    clock(i128::from(s.dts), vs, origin)?,
                    clock(pts(s)?, vs, origin)?,
                    s.key(),
                    VIDEO_PID,
                    2,
                )
            } else {
                let s = &self.base.audio[ai];
                ai += 1;
                (
                    self.adts(s)?,
                    clock(i128::from(s.dts), ascale, origin)?,
                    clock(pts(s)?, ascale, origin)?,
                    false,
                    AUDIO_PID,
                    3,
                )
            };
            self.pcr_until(&mut out, dts)?;
            transport_clock(dts, presentation, self.last_pcr.ok_or_else(bound)?)?;
            self.pes(&mut out, &payload, pid, index, dts, presentation, key)?;
        }
        self.base.video.drain(..nv);
        self.base.audio.drain(..na);
        self.base.video_end = Some(video.last_end);
        self.base.audio_end = Some(audio.last_end);
        let ordinal = self.segments;
        self.segments = self.segments.checked_add(1).ok_or_else(bound)?;
        Ok(TsSegment {
            bytes: out.into(),
            ordinal,
            stream_id: self.stream_id,
            duration_ms: (duration + 89) / 90,
            duration_ticks: duration,
            video,
            audio,
            origin,
            timing,
        })
    }
    fn annex_b(&self, s: &Sample) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        append(&mut out, &[0, 0, 0, 1, 9, 0xf0])?;
        if s.key() {
            for p in &self.codec.parameters {
                append(&mut out, &[0, 0, 0, 1])?;
                append(&mut out, p)?;
            }
        }
        let mut at = 0usize;
        let mut vcl = 0;
        while at < s.data.len() {
            let n = u32_at(&s.data, at)? as usize;
            at = at.checked_add(4).ok_or_else(bound)?;
            let nal = s
                .data
                .get(at..at.checked_add(n).ok_or_else(bound)?)
                .ok_or_else(bound)?;
            at += n;
            let header = *nal.first().ok_or_else(bound)?;
            let kind = header & 31;
            if header & 0x80 != 0 {
                return Err(bad("AVC forbidden bit입니다."));
            }
            match kind {
                1 | 5 => {
                    vcl += 1;
                    if s.key() != (kind == 5) {
                        return Err(bad("IDR access unit과 샘플 표시가 다릅니다."));
                    }
                }
                7 | 8 => {
                    if !self.codec.parameters.iter().any(|p| p.as_slice() == nal) {
                        return Err(bad("AVC parameter set이 바뀌었습니다."));
                    }
                }
                6 | 9 | 12 => {}
                _ => return Err(bad("TS에서 검증하지 않은 AVC NAL 종류입니다.")),
            }
            if kind != 9 {
                append(&mut out, &[0, 0, 0, 1])?;
                append(&mut out, nal)?;
            }
        }
        if vcl == 0 {
            return Err(bad("AVC access unit에 영상 slice가 없습니다."));
        }
        Ok(out)
    }
    fn adts(&self, s: &Sample) -> Result<Vec<u8>> {
        if rejected_aac(s, 0, self.base.descriptor.0.audio_scale).is_some() {
            return Err(bad("ADTS AAC-LC frame 범위가 올바르지 않습니다."));
        }
        let n = s.data.len() + 7;
        let c = self.codec.channels;
        let mut out = vec![
            0xff,
            0xf1,
            0x40 | (self.codec.frequency << 2) | (c >> 2),
            (c << 6) | ((n >> 11) as u8),
            (n >> 3) as u8,
            ((n as u8 & 7) << 5) | 31,
            0xfc,
        ];
        out.extend_from_slice(&s.data);
        Ok(out)
    }
    fn tables(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let mut pat = vec![
            0,
            0xb0,
            13,
            0,
            1,
            0xc1,
            0,
            0,
            0,
            1,
            0xe0 | ((PMT_PID >> 8) as u8),
            PMT_PID as u8,
        ];
        pat.extend(crc(&pat).to_be_bytes());
        let mut pmt = vec![
            2, 0xb0, 23, 0, 1, 0xc1, 0, 0, 0xe1, 0, 0xf0, 0, 0x1b, 0xe1, 0, 0xf0, 0, 0x0f, 0xe1, 1,
            0xf0, 0,
        ];
        pmt.extend(crc(&pmt).to_be_bytes());
        for (i, pid, section) in [(0, 0, pat), (1, PMT_PID, pmt)] {
            let mut payload = vec![0];
            payload.extend(section);
            let mut packet = [0xff; 188];
            packet[..4].copy_from_slice(&[
                0x47,
                0x40 | ((pid >> 8) as u8),
                pid as u8,
                0x10 | self.continuity[i],
            ]);
            packet[4..4 + payload.len()].copy_from_slice(&payload);
            self.continuity[i] = (self.continuity[i] + 1) & 15;
            append(out, &packet)?;
        }
        Ok(())
    }
    fn pcr_until(&mut self, out: &mut Vec<u8>, dts: u64) -> Result<()> {
        if self.last_pcr.is_none() {
            self.pcr_packet(out, dts)?;
        }
        while dts
            .checked_sub(self.last_pcr.ok_or_else(bound)?)
            .ok_or_else(bound)?
            > 3600
        {
            self.pcr_packet(
                out,
                self.last_pcr.unwrap().checked_add(3600).ok_or_else(bound)?,
            )?;
        }
        Ok(())
    }
    fn pcr_packet(&mut self, out: &mut Vec<u8>, dts: u64) -> Result<()> {
        let v = dts & 0x1ffffffff;
        let mut p = [0xff; 188];
        let cc = if self.last_pcr.is_none() {
            self.continuity[2]
        } else {
            self.continuity[2].wrapping_sub(1) & 15
        };
        p[..6].copy_from_slice(&[
            0x47,
            (VIDEO_PID >> 8) as u8,
            VIDEO_PID as u8,
            0x20 | cc,
            183,
            0x10,
        ]);
        p[6..12].copy_from_slice(&[
            (v >> 25) as u8,
            (v >> 17) as u8,
            (v >> 9) as u8,
            (v >> 1) as u8,
            ((v as u8 & 1) << 7) | 0x7e,
            0,
        ]);
        self.last_pcr = Some(dts);
        append(out, &p)
    }
    fn pes(
        &mut self,
        out: &mut Vec<u8>,
        payload: &[u8],
        pid: u16,
        index: usize,
        dts: u64,
        pts: u64,
        key: bool,
    ) -> Result<()> {
        let both = dts != pts;
        let h = if both { 10 } else { 5 };
        let n = payload.len().checked_add(3 + h).ok_or_else(bound)?;
        let length = if pid == VIDEO_PID {
            0
        } else {
            u16::try_from(n).map_err(|_| bound())?
        };
        let mut pes = vec![
            0,
            0,
            1,
            if pid == VIDEO_PID { 0xe0 } else { 0xc0 },
            (length >> 8) as u8,
            length as u8,
            0x84,
            if both { 0xc0 } else { 0x80 },
            h as u8,
        ];
        pes.extend(stamp(pts, if both { 0x30 } else { 0x20 }));
        if both {
            pes.extend(stamp(dts, 0x10));
        }
        pes.extend_from_slice(payload);
        let mut at = 0;
        let mut first = true;
        while at < pes.len() {
            let mandatory = if first && key { 2 } else { 0 };
            let n = (pes.len() - at).min(184 - mandatory);
            let adaptation = 184 - n;
            let mut p = [0xff; 188];
            p[..4].copy_from_slice(&[
                0x47,
                ((pid >> 8) as u8) | if first { 0x40 } else { 0 },
                pid as u8,
                if adaptation > 0 { 0x30 } else { 0x10 } | self.continuity[index],
            ]);
            if adaptation > 0 {
                p[4] = (adaptation - 1) as u8;
                if adaptation > 1 {
                    p[5] = if first && key { 0x40 } else { 0 };
                }
            }
            p[4 + adaptation..].copy_from_slice(&pes[at..at + n]);
            append(out, &p)?;
            self.continuity[index] = (self.continuity[index] + 1) & 15;
            at += n;
            first = false;
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
#[path = "browser_fmp4_relay_ts_actual.rs"]
mod actual;
#[cfg(test)]
#[path = "browser_fmp4_relay_ts_tests.rs"]
mod tests;
