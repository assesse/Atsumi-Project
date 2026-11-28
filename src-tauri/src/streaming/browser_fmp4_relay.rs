//! Pure, bounded sample admission and continuous-clock fMP4 output.
//! This does not start a relay, own a viewer, touch an archive, or perform I/O.
use super::*;
#[path = "browser_fmp4_relay_ts.rs"]
pub(crate) mod ts;
use std::sync::Arc;

#[cfg(all(test, windows))]
#[path = "browser_fmp4_relay_actual.rs"]
mod actual;

struct Description {
    init: Vec<u8>,
    video_id: u32,
    audio_id: u32,
    video_scale: u32,
    audio_scale: u32,
}

/// Only a reader can construct this codec/epoch capability. Batches from a
/// different reader cannot be submitted even when their codec bytes match.
#[derive(Clone)]
pub(crate) struct SampleDescriptor(Arc<Description>);

pub(crate) struct ValidatedSampleBatch {
    descriptor: SampleDescriptor,
    ordinal: u64,
    video: VecDeque<Sample>,
    audio: VecDeque<Sample>,
}

pub(crate) struct ReaderEnd {
    descriptor: SampleDescriptor,
    ordinal: u64,
    video_end: Option<u64>,
    audio_end: Option<u64>,
}

/// Fixed-size numeric inspection only. No sample contents or source identity.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleState {
    pub video_timescale: u32,
    pub audio_timescale: u32,
    pub video_first_dts: Option<u64>,
    pub audio_first_dts: Option<u64>,
    pub video_end: Option<u64>,
    pub audio_end: Option<u64>,
    pub video_pending_samples: usize,
    pub audio_pending_samples: usize,
    pub container_pending_bytes: usize,
    pub pending_moofs: usize,
    pub ordinal: u64,
}

pub(crate) struct EncodedSampleReader {
    parser: EncodedMuxer,
    descriptor: SampleDescriptor,
    ordinal: u64,
    closed: bool,
}

impl EncodedSampleReader {
    pub(crate) fn sample_state(&self) -> SampleState {
        SampleState {
            video_timescale: self.descriptor.0.video_scale,
            audio_timescale: self.descriptor.0.audio_scale,
            video_first_dts: self.parser.video.front().map(|s| s.dts),
            audio_first_dts: self.parser.audio.front().map(|s| s.dts),
            video_end: self
                .parser
                .tracks
                .iter()
                .find(|t| t.kind == Kind::Video)
                .and_then(|t| t.last_end),
            audio_end: self
                .parser
                .tracks
                .iter()
                .find(|t| t.kind == Kind::Audio)
                .and_then(|t| t.last_end),
            video_pending_samples: self.parser.video.len(),
            audio_pending_samples: self.parser.audio.len(),
            container_pending_bytes: self.parser.inputs.iter().map(|p| p.pending.len()).sum(),
            pending_moofs: self
                .parser
                .inputs
                .iter()
                .filter(|p| p.moof.is_some())
                .count(),
            ordinal: self.ordinal,
        }
    }
    pub(crate) fn new(inputs: Vec<EncodedTrackInput>) -> Result<Self> {
        let parser = EncodedMuxer::new(inputs)?;
        let video = parser
            .tracks
            .iter()
            .find(|t| t.kind == Kind::Video)
            .ok_or_else(bound)?;
        let audio = parser
            .tracks
            .iter()
            .find(|t| t.kind == Kind::Audio)
            .ok_or_else(bound)?;
        let descriptor = SampleDescriptor(Arc::new(Description {
            init: parser.init.clone(),
            video_id: video.id,
            audio_id: audio.id,
            video_scale: video.scale,
            audio_scale: audio.scale,
        }));
        Ok(Self {
            parser,
            descriptor,
            ordinal: 0,
            closed: false,
        })
    }

    pub(crate) fn descriptor(&self) -> SampleDescriptor {
        self.descriptor.clone()
    }

    pub(crate) fn push(&mut self, input: u32, bytes: &[u8]) -> Result<ValidatedSampleBatch> {
        if self.closed || self.parser.failed {
            return Err(bad("중단된 연속 영상 입력입니다."));
        }
        // No samples leave the owner until *all* atoms/tracks/ranges in this
        // append have passed. Failed private state is permanently unreachable.
        let result = self.parser.push_inner(input, bytes, ParseMode::Samples);
        if let Err(error) = result {
            self.parser.failed = true;
            self.closed = true;
            return Err(error);
        }
        let ordinal = self.ordinal;
        self.ordinal = match self.ordinal.checked_add(1) {
            Some(value) => value,
            None => {
                self.closed = true;
                return Err(bound());
            }
        };
        Ok(ValidatedSampleBatch {
            descriptor: self.descriptor.clone(),
            ordinal,
            video: std::mem::take(&mut self.parser.video),
            audio: std::mem::take(&mut self.parser.audio),
        })
    }

    /// Clean transport/container closure, not a claim of equal A/V ends or a
    /// completed YouTube archive. No archive flush or duration adjustment.
    pub(crate) fn finish(&mut self) -> Result<ReaderEnd> {
        if self.closed || self.parser.failed {
            return Err(bad("중단된 연속 영상 입력입니다."));
        }
        self.closed = true;
        if self
            .parser
            .inputs
            .iter()
            .any(|p| !p.pending.is_empty() || p.moof.is_some())
        {
            return Err(bad("완성되지 않은 연속 영상 입력입니다."));
        }
        Ok(ReaderEnd {
            descriptor: self.descriptor.clone(),
            ordinal: self.ordinal,
            video_end: self
                .parser
                .tracks
                .iter()
                .find(|t| t.kind == Kind::Video)
                .and_then(|t| t.last_end),
            audio_end: self
                .parser
                .tracks
                .iter()
                .find(|t| t.kind == Kind::Audio)
                .and_then(|t| t.last_end),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceOrigin {
    pub ticks: u64,
    pub timescale: u32,
    pub video_base: u64,
    pub audio_base: u64,
    // Fractional tick numerator; denominator is `timescale`. Each remainder
    // is strictly smaller than that denominator, and is never accumulated.
    pub video_remainder: u32,
    pub audio_remainder: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PrerollCounts {
    pub video_samples: u64,
    pub audio_samples: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SampleRange {
    pub timescale: u32,
    pub first_dts: u64,
    pub last_end: u64,
    pub samples: u64,
}

pub(crate) struct ContinuousChunk {
    pub bytes: Vec<u8>,
    pub includes_init: bool,
    pub origin: SourceOrigin,
    pub video: Option<SampleRange>,
    pub audio: Option<SampleRange>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContinuousEnd {
    pub origin: SourceOrigin,
    pub preroll: PrerollCounts,
    pub video_end: u64,
    pub video_timescale: u32,
    pub audio_end: u64,
    pub audio_timescale: u32,
}

pub(crate) struct ContinuousMp4Muxer {
    descriptor: SampleDescriptor,
    ordinal: u64,
    sequence: u32,
    video: VecDeque<Sample>,
    audio: VecDeque<Sample>,
    origin: Option<SourceOrigin>,
    preroll: PrerollCounts,
    init_sent: bool,
    video_end: Option<u64>,
    audio_end: Option<u64>,
    closed: bool,
}

fn rescale_origin(ticks: u64, from: u32, to: u32) -> Result<(u64, u32)> {
    if from == 0 || to == 0 {
        return Err(bound());
    }
    let product = u128::from(ticks)
        .checked_mul(u128::from(to))
        .ok_or_else(bound)?;
    let denominator = u128::from(from);
    Ok((
        u64::try_from(product / denominator).map_err(|_| bound())?,
        u32::try_from(product % denominator).map_err(|_| bound())?,
    ))
}

fn accounted(mut samples: impl Iterator<Item = usize>) -> Result<usize> {
    samples.try_fold(0usize, |n, size| {
        n.checked_add(size)
            .and_then(|v| v.checked_add(40))
            .ok_or_else(bound)
    })
}

impl ContinuousMp4Muxer {
    pub(crate) fn sample_state(&self) -> SampleState {
        SampleState {
            video_timescale: self.descriptor.0.video_scale,
            audio_timescale: self.descriptor.0.audio_scale,
            video_first_dts: self.video.front().map(|s| s.dts),
            audio_first_dts: self.audio.front().map(|s| s.dts),
            video_end: self.video.back().map(Sample::end).or(self.video_end),
            audio_end: self.audio.back().map(Sample::end).or(self.audio_end),
            video_pending_samples: self.video.len(),
            audio_pending_samples: self.audio.len(),
            container_pending_bytes: 0,
            pending_moofs: 0,
            ordinal: self.ordinal,
        }
    }
    pub(crate) fn new(descriptor: SampleDescriptor) -> Self {
        Self {
            descriptor,
            ordinal: 0,
            sequence: 0,
            video: VecDeque::new(),
            audio: VecDeque::new(),
            origin: None,
            preroll: PrerollCounts::default(),
            init_sent: false,
            video_end: None,
            audio_end: None,
            closed: false,
        }
    }

    pub(crate) fn push(&mut self, batch: ValidatedSampleBatch) -> Result<Option<ContinuousChunk>> {
        if self.closed {
            return Err(bad("중단된 연속 영상 출력입니다."));
        }
        let result = self.push_inner(batch);
        if result.is_err() {
            self.closed = true;
            self.video.clear();
            self.audio.clear();
        }
        result
    }

    fn push_inner(&mut self, mut batch: ValidatedSampleBatch) -> Result<Option<ContinuousChunk>> {
        if !Arc::ptr_eq(&self.descriptor.0, &batch.descriptor.0) || batch.ordinal != self.ordinal {
            return Err(bad("다른 입력 또는 순서가 변경된 연속 영상입니다."));
        }
        let count = self
            .video
            .len()
            .checked_add(self.audio.len())
            .and_then(|n| n.checked_add(batch.video.len()))
            .and_then(|n| n.checked_add(batch.audio.len()))
            .ok_or_else(bound)?;
        let bytes = accounted(
            self.video
                .iter()
                .chain(&self.audio)
                .chain(&batch.video)
                .chain(&batch.audio)
                .map(|s| s.data.len()),
        )?;
        if count > MAX_SAMPLES || bytes > MAX_PENDING {
            return Err(bound());
        }
        self.ordinal = self.ordinal.checked_add(1).ok_or_else(bound)?;
        self.video.append(&mut batch.video);
        self.audio.append(&mut batch.audio);
        if self.origin.is_none() && !self.start()? {
            return Ok(None);
        }
        if self.video.is_empty() && self.audio.is_empty() {
            return Ok(None);
        }
        self.emit().map(Some)
    }

    fn start(&mut self) -> Result<bool> {
        let d = &self.descriptor.0;
        let mut audio_index = 0usize;
        let mut selected = None;
        for (video_index, video) in self.video.iter().enumerate().filter(|(_, s)| s.key()) {
            let video_clock = u128::from(video.dts)
                .checked_mul(u128::from(d.audio_scale))
                .ok_or_else(bound)?;
            while let Some(audio) = self.audio.get(audio_index) {
                let audio_end = u128::from(audio.end())
                    .checked_mul(u128::from(d.video_scale))
                    .ok_or_else(bound)?;
                if audio_end > video_clock {
                    break;
                }
                audio_index += 1;
            }
            let Some(audio) = self.audio.get(audio_index) else {
                break;
            };
            let audio_start = u128::from(audio.dts)
                .checked_mul(u128::from(d.video_scale))
                .ok_or_else(bound)?;
            if audio_start <= video_clock {
                selected = Some((video_index, audio_index, audio.dts));
                break;
            }
        }
        let Some((video_index, audio_index, ticks)) = selected else {
            return Ok(false);
        };
        // The overlapping packet starts no later than the RAP, therefore its
        // exact rational DTS is min(video RAP, audio packet), the common origin.
        let (video_base, video_remainder) = rescale_origin(ticks, d.audio_scale, d.video_scale)?;
        let (audio_base, audio_remainder) = rescale_origin(ticks, d.audio_scale, d.audio_scale)?;
        self.origin = Some(SourceOrigin {
            ticks,
            timescale: d.audio_scale,
            video_base,
            audio_base,
            video_remainder,
            audio_remainder,
        });
        for _ in 0..video_index {
            let sample = self.video.pop_front().ok_or_else(bound)?;
            self.preroll.video_samples += 1;
            self.preroll.bytes = self
                .preroll
                .bytes
                .checked_add(sample.data.len() as u64)
                .ok_or_else(bound)?;
        }
        for _ in 0..audio_index {
            let sample = self.audio.pop_front().ok_or_else(bound)?;
            self.preroll.audio_samples += 1;
            self.preroll.bytes = self
                .preroll
                .bytes
                .checked_add(sample.data.len() as u64)
                .ok_or_else(bound)?;
        }
        Ok(true)
    }

    fn emit(&mut self) -> Result<ContinuousChunk> {
        let d = &self.descriptor.0;
        let origin = self.origin.ok_or_else(bound)?;
        let includes_init = !self.init_sent;
        let mut size = if includes_init { d.init.len() } else { 0 };
        // Include conservative box overhead before creating serialized copies.
        for queue in [&self.video, &self.audio] {
            if !queue.is_empty() {
                size = size.checked_add(1024).ok_or_else(bound)?;
            }
            for s in queue {
                size = size
                    .checked_add(s.data.len())
                    .and_then(|n| n.checked_add(16))
                    .ok_or_else(bound)?;
            }
        }
        if size > MAX_PENDING {
            return Err(bound());
        }
        let mut bytes = Vec::with_capacity(size);
        if includes_init {
            bytes.extend_from_slice(&d.init);
        }
        let mut sequence = self.sequence;
        let mut ranges = [None, None];
        for (index, (queue, scale, id, base)) in [
            (
                &mut self.video,
                d.video_scale,
                d.video_id,
                origin.video_base,
            ),
            (
                &mut self.audio,
                d.audio_scale,
                d.audio_id,
                origin.audio_base,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if queue.is_empty() {
                continue;
            }
            for sample in queue.iter() {
                let pts = i128::from(sample.dts)
                    .checked_add(i128::from(sample.cts))
                    .ok_or_else(bound)?;
                let scaled = pts
                    .checked_mul(i128::from(origin.timescale))
                    .ok_or_else(bound)?;
                let origin_scaled = i128::from(origin.ticks)
                    .checked_mul(i128::from(scale))
                    .ok_or_else(bound)?;
                if scaled < origin_scaled {
                    return Err(bad("시작점보다 이른 표시 시각은 지원하지 않습니다."));
                }
            }
            sequence = sequence.checked_add(1).ok_or_else(bound)?;
            bytes.extend(make_fragment(id, queue.make_contiguous(), base, sequence)?);
            ranges[index] = Some(SampleRange {
                timescale: scale,
                first_dts: queue.front().ok_or_else(bound)?.dts,
                last_end: queue.back().ok_or_else(bound)?.end(),
                samples: queue.len() as u64,
            });
        }
        if bytes.len() > MAX_PENDING {
            return Err(bound());
        }
        self.sequence = sequence;
        self.init_sent = true;
        if let Some(range) = ranges[0] {
            self.video_end = Some(range.last_end);
        }
        if let Some(range) = ranges[1] {
            self.audio_end = Some(range.last_end);
        }
        self.video.clear();
        self.audio.clear();
        Ok(ContinuousChunk {
            bytes,
            includes_init,
            origin,
            video: ranges[0],
            audio: ranges[1],
        })
    }

    pub(crate) fn finish(&mut self, end: ReaderEnd) -> Result<ContinuousEnd> {
        if self.closed {
            return Err(bad("중단된 연속 영상 출력입니다."));
        }
        self.closed = true;
        if !Arc::ptr_eq(&self.descriptor.0, &end.descriptor.0)
            || end.ordinal != self.ordinal
            || self.video_end != end.video_end
            || self.audio_end != end.audio_end
            || !self.video.is_empty()
            || !self.audio.is_empty()
        {
            return Err(bad("연속 영상 입력의 끝을 확인하지 못했습니다."));
        }
        Ok(ContinuousEnd {
            origin: self.origin.ok_or_else(bound)?,
            preroll: self.preroll,
            video_end: self.video_end.ok_or_else(bound)?,
            video_timescale: self.descriptor.0.video_scale,
            audio_end: self.audio_end.ok_or_else(bound)?,
            audio_timescale: self.descriptor.0.audio_scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn reader() -> EncodedSampleReader {
        EncodedSampleReader::new(fixtures::inputs()).unwrap()
    }
    fn pair() -> (EncodedSampleReader, ContinuousMp4Muxer) {
        let r = reader();
        let m = ContinuousMp4Muxer::new(r.descriptor());
        (r, m)
    }
    fn push(
        r: &mut EncodedSampleReader,
        m: &mut ContinuousMp4Muxer,
        track: u32,
        start: u64,
        count: u32,
    ) -> Option<ContinuousChunk> {
        m.push(
            r.push(track, &fixtures::fragment(track, start, count, true))
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn archive_output_matches_captured_pre_edit_bytes_and_clocks() {
        let hashes = [
            "9e319badce3967b7b1f25b07e2f211fda8ea418c608245a12c492f5ae2fa5c70",
            "d572ef8d793eedc9821fae1ea197249e4990474c4d40bd782220cf867f2cabb4",
            "ee31bc609989a72b262e1b115af503b363e6c61a0dcdff34729ce9be20cd9187",
        ];
        for seconds in [12u32, 26] {
            let mut mux = EncodedMuxer::new(fixtures::inputs()).unwrap();
            let mut output = Vec::new();
            for second in (0..seconds).step_by(2) {
                for track in [0, 1] {
                    let scale = if track == 0 { 90000 } else { 48000 };
                    output.extend(
                        mux.push(
                            track,
                            &fixtures::fragment(track, u64::from(second) * scale, 4, true),
                        )
                        .unwrap(),
                    );
                }
            }
            output.extend(mux.finish().unwrap());
            assert_eq!(output.len(), if seconds == 12 { 1 } else { 3 });
            for (i, segment) in output.iter().enumerate() {
                assert_eq!(format!("{:x}", Sha256::digest(&segment.bytes)), hashes[i]);
                assert_eq!(segment.bytes.len(), if i == 2 { 1403 } else { 2203 });
                assert_eq!(segment.source_start_seconds, (i * 12) as f64);
                assert_eq!(
                    segment.source_end_seconds,
                    if i == 2 { 26.0 } else { ((i + 1) * 12) as f64 }
                );
                assert_eq!(segment.duration_seconds, if i == 2 { 2.0 } else { 12.0 });
            }
        }
    }

    #[test]
    fn whole_append_failure_never_returns_preceding_valid_samples() {
        for mut suffix in [
            boxed(b"nope", &[]),
            vec![0, 0, 0, 1, b'm', b'd', b'a', b't'],
        ] {
            let mut r = reader();
            let mut input = fixtures::fragment(0, 0, 4, true);
            input.append(&mut suffix);
            assert!(r.push(0, &input).is_err());
            assert!(r.push(0, &fixtures::fragment(0, 0, 4, true)).is_err());
            assert!(r.finish().is_err());
        }
    }

    #[test]
    fn occupied_payload_failure_closes_reader_even_after_track_mutation() {
        let mut r = reader();
        let mut input = fixtures::fragment(0, 0, 4, true);
        let mdat = u32_at(&input, 0).unwrap() as usize;
        let size = u32_at(&input, mdat).unwrap();
        put32(&mut input, mdat, size + 1).unwrap();
        input.push(0);
        assert!(r.push(0, &input).is_err());
        assert!(r.finish().is_err());
    }

    #[test]
    fn muxed_append_invalid_second_track_never_emits_first_track() {
        let init = reader().descriptor.0.init.clone();
        let mut r = EncodedSampleReader::new(vec![EncodedTrackInput {
            track_index: 0,
            mime_type: "video/mp4;codecs=avc1.42001e,mp4a.40.2".into(),
            init,
        }])
        .unwrap();
        let samples = [
            Sample {
                dts: 0,
                duration: 45000,
                flags: 0x02000000,
                cts: 0,
                data: vec![0, 0, 0, 2, 0x65, 1],
                aac_rounding: None,
            },
            Sample {
                dts: 0,
                duration: 0,
                flags: 0x02000000,
                cts: 0,
                data: vec![0x21, 1],
                aac_rounding: None,
            },
        ];
        let fragments: Vec<_> = samples
            .iter()
            .enumerate()
            .map(|(i, s)| make_fragment(i as u32 + 1, std::slice::from_ref(s), 0, 1).unwrap())
            .collect();
        let mut trafs = Vec::new();
        let mut header = Vec::new();
        let mut payload = Vec::new();
        for fragment in &fragments {
            let top = atoms(fragment).unwrap();
            let moof = atoms(top[0].data()).unwrap();
            header = moof
                .iter()
                .find(|a| a.kind == *b"mfhd")
                .unwrap()
                .bytes
                .to_vec();
            trafs.push(
                moof.iter()
                    .find(|a| a.kind == *b"traf")
                    .unwrap()
                    .bytes
                    .to_vec(),
            );
            payload.extend_from_slice(top[1].data());
        }
        let total_moof = 8 + header.len() + trafs.iter().map(Vec::len).sum::<usize>();
        let mut offset = total_moof + 8;
        for (traf, sample) in trafs.iter_mut().zip(&samples) {
            let trun = traf.windows(4).position(|p| p == b"trun").unwrap();
            put32(traf, trun + 12, offset as u32).unwrap();
            offset += sample.data.len();
        }
        let mut input = container(b"moof", &[header, trafs.remove(0), trafs.remove(0)]);
        input.extend(boxed(b"mdat", &payload));
        assert!(r.push(0, &input).is_err());
        assert!(r.closed && r.parser.failed);
        assert!(r.finish().is_err());
    }

    #[test]
    fn emit_failure_on_second_audio_track_does_not_return_video_or_advance_sequence() {
        let (mut r, mut m) = pair();
        assert!(push(&mut r, &mut m, 0, 90000, 4).is_none());
        let audio = fixtures::fragment(1, 48000, 4, true);
        let mut batch = r.push(1, &audio).unwrap();
        // Fault injection after valid admission: real AAC CTS != 0 is rejected
        // by the reader. This tests the emitter's independent atomic boundary.
        batch.audio[0].cts = -1;
        assert!(m.push(batch).is_err());
        assert_eq!(m.sequence, 0);
        assert!(!m.init_sent);
        assert!(m.closed);
        assert!(m.video.is_empty() && m.audio.is_empty());
    }

    #[test]
    fn split_input_and_retransmission_are_safe_after_move_drain() {
        let mut r = reader();
        let input = fixtures::fragment(0, 0, 4, true);
        let cut = input.len() - 3;
        assert!(r.push(0, &input[..cut]).unwrap().video.is_empty());
        assert_eq!(r.push(0, &input[cut..]).unwrap().video.len(), 4);
        assert!(r.parser.video.is_empty());
        assert!(r.push(0, &input).unwrap().video.is_empty());
        let mut changed = input;
        *changed.last_mut().unwrap() ^= 1;
        assert!(r.push(0, &changed).is_err());
        assert!(r.finish().is_err());
    }

    #[test]
    fn clean_samples_before_pending_bytes_do_not_make_clean_end() {
        let mut r = reader();
        let mut input = fixtures::fragment(0, 0, 4, true);
        input.extend([0, 0, 0]);
        assert_eq!(r.push(0, &input).unwrap().video.len(), 4);
        assert!(r.finish().is_err());
        assert!(r.push(1, &fixtures::fragment(1, 0, 4, true)).is_err());
    }

    #[test]
    fn descriptor_epoch_and_batch_order_cannot_be_forged_or_skipped() {
        let (mut r, mut m) = pair();
        let mut other = reader();
        assert!(m
            .push(other.push(0, &fixtures::fragment(0, 0, 4, true)).unwrap())
            .is_err());
        assert!(m
            .push(r.push(0, &fixtures::fragment(0, 0, 4, true)).unwrap())
            .is_err());
        let (mut r, mut m) = pair();
        let first = r.push(0, &fixtures::fragment(0, 0, 4, true)).unwrap();
        let second = r.push(1, &fixtures::fragment(1, 0, 4, true)).unwrap();
        assert!(m.push(second).is_err());
        assert!(m.push(first).is_err());
    }

    #[test]
    fn end_capability_checks_epoch_and_every_accepted_batch() {
        let (mut r, mut m) = pair();
        push(&mut r, &mut m, 0, 0, 4);
        push(&mut r, &mut m, 1, 0, 4).unwrap();
        let mut other = reader();
        assert!(m.finish(other.finish().unwrap()).is_err());
        let (mut r, mut m) = pair();
        push(&mut r, &mut m, 0, 0, 4);
        push(&mut r, &mut m, 1, 0, 4).unwrap();
        let _not_delivered = r.push(0, &fixtures::fragment(0, 180000, 4, true)).unwrap();
        assert!(m.finish(r.finish().unwrap()).is_err());
    }

    #[test]
    fn exact_audio_overlap_uses_one_rational_origin_and_retains_spanning_packet() {
        let (mut r, mut m) = pair();
        assert!(push(&mut r, &mut m, 0, 90000, 4).is_none());
        let output = push(&mut r, &mut m, 1, 47999, 4).unwrap();
        assert_eq!(output.origin.ticks, 47999);
        assert_eq!(output.origin.timescale, 48000);
        assert_eq!(output.origin.video_base, 89998);
        assert_eq!(output.origin.video_remainder, 6000);
        assert_eq!(output.origin.audio_base, 47999);
        assert_eq!(output.video.unwrap().first_dts, 90000);
        assert_eq!(output.audio.unwrap().samples, 4);
        assert_eq!(
            m.finish(r.finish().unwrap()).unwrap().preroll,
            PrerollCounts::default()
        );
    }

    #[test]
    fn aac_ending_at_rap_is_not_overlap_but_next_packet_starting_there_is() {
        let (mut r, mut m) = pair();
        assert!(push(&mut r, &mut m, 0, 90000, 2).is_none());
        assert!(push(&mut r, &mut m, 1, 0, 2).is_none());
        let output = push(&mut r, &mut m, 1, 48000, 2).unwrap();
        assert_eq!(output.origin.ticks, 48000);
        assert_eq!(m.preroll.audio_samples, 2);
        assert_eq!(m.preroll.video_samples, 0);
    }

    #[test]
    fn late_audio_chooses_later_covered_rap_without_gap_tolerance() {
        let (mut r, mut m) = pair();
        assert!(push(&mut r, &mut m, 0, 0, 4).is_none());
        assert!(push(&mut r, &mut m, 1, 2400, 8).is_none()); // 50ms late is not admitted.
        let output = push(&mut r, &mut m, 0, 180000, 4).unwrap();
        assert_eq!(output.video.unwrap().first_dts, 180000);
        assert_eq!(output.origin.ticks, 74400); // packet 1.55..2.05 spans RAP2s.
        assert_eq!(m.preroll.video_samples, 4);
        assert_eq!(m.preroll.audio_samples, 3);
    }

    #[test]
    fn unmatched_startup_is_not_a_completed_stream() {
        let (mut r, mut m) = pair();
        push(&mut r, &mut m, 0, 0, 4);
        push(&mut r, &mut m, 1, 2400, 4);
        assert!(m.finish(r.finish().unwrap()).is_err());
    }

    #[test]
    fn preorigin_cts_is_rejected_without_partial_output() {
        let (mut r, mut m) = pair();
        let mut video = fixtures::fragment(0, 0, 4, true);
        let trun = video.windows(4).position(|p| p == b"trun").unwrap();
        put32(&mut video, trun + 28, u32::MAX).unwrap();
        assert!(m.push(r.push(0, &video).unwrap()).unwrap().is_none());
        assert!(m
            .push(r.push(1, &fixtures::fragment(1, 0, 4, true)).unwrap())
            .is_err());
        assert!(m.finish(r.finish().unwrap()).is_err());
    }

    #[test]
    fn continuous_thirty_seconds_keeps_one_init_all_payloads_and_nonreset_clocks() {
        let (mut r, mut m) = pair();
        let mut outputs = Vec::new();
        for seconds in (0u64..30).step_by(2) {
            for track in [0, 1] {
                let scale = if track == 0 { 90000 } else { 48000 };
                if let Some(out) = push(&mut r, &mut m, track, seconds * scale, 4) {
                    outputs.push(out);
                }
            }
        }
        let end = m.finish(r.finish().unwrap()).unwrap();
        assert_eq!((end.video_end, end.audio_end), (30 * 90000, 30 * 48000));
        assert_eq!((end.video_timescale, end.audio_timescale), (90000, 48000));
        assert_eq!(end.origin.ticks, 0);
        assert_eq!(outputs.iter().filter(|o| o.includes_init).count(), 1);
        let init = r.descriptor.0.init.clone();
        let mut check = EncodedSampleReader::new(vec![EncodedTrackInput {
            track_index: 0,
            mime_type: "video/mp4;codecs=avc1.42001e,mp4a.40.2".into(),
            init: init.clone(),
        }])
        .unwrap();
        let mut video = Vec::new();
        let mut audio = Vec::new();
        let mut sequence = 0;
        for output in outputs {
            let payload = if output.includes_init {
                &output.bytes[init.len()..]
            } else {
                &output.bytes[..]
            };
            for atom in atoms(payload)
                .unwrap()
                .into_iter()
                .filter(|a| a.kind == *b"moof")
            {
                let children = atoms(atom.data()).unwrap();
                let header = children.iter().find(|a| a.kind == *b"mfhd").unwrap();
                sequence += 1;
                assert_eq!(u32_at(header.data(), 4).unwrap(), sequence);
            }
            let batch = check.push(0, payload).unwrap();
            video.extend(batch.video);
            audio.extend(batch.audio);
        }
        assert_eq!((video.len(), audio.len(), sequence), (60, 60, 30));
        for (i, s) in video.iter().enumerate() {
            assert_eq!((s.dts, s.duration, s.cts), (i as u64 * 45000, 45000, 0));
            assert_eq!(s.data[5], (i % 4) as u8);
        }
        for (i, s) in audio.iter().enumerate() {
            assert_eq!((s.dts, s.duration, s.cts), (i as u64 * 24000, 24000, 0));
            assert_eq!(s.data, [0x21, (i % 4) as u8]);
        }
        check.finish().unwrap();
    }

    #[test]
    fn startup_queue_and_integer_rescale_fail_closed_at_bounds() {
        let (mut r, mut m) = pair();
        let mut batch = r.push(0, &fixtures::fragment(0, 0, 1, true)).unwrap();
        let sample = batch.video[0].clone();
        batch.video = std::iter::repeat(sample).take(MAX_SAMPLES + 1).collect();
        assert!(m.push(batch).is_err());
        assert!(m.finish(r.finish().unwrap()).is_err());
        assert!(accounted([usize::MAX].into_iter()).is_err());
        assert!(rescale_origin(u64::MAX, 1, u32::MAX).is_err());
        assert!(rescale_origin(1, 0, 1).is_err());
        assert_eq!(rescale_origin(47999, 48000, 90000).unwrap(), (89998, 6000));
    }
}
