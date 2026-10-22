//! Dormant fixed packet/stream scalar projection. Not codec-equivalence proof.
//! Stream side data is deliberately outside this projection, not known absent.
use super::{
    packet_facts::{Decoder, Fault, FoldResult, PacketFact, Streams, TimeBase, TrackFold},
    Track,
};

type Result<T> = std::result::Result<T, Fault>;
const CHUNK: usize = 128 * 1024;
const LINE: usize = 2048;
const TEXT: u64 = 4 * 1024 * 1024 * 1024;
const DIMENSION: u32 = 16_384;

/// Non-applicable video/audio fields are exactly zero, never inferred values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StreamFacts {
    pub track: Track,
    pub time_base: TimeBase,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
    pub channels: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProbeFacts {
    pub packets: FoldResult,
    pub stream: StreamFacts,
}

/// One bounded line, one provisional integer fold and one trailing stream row.
/// The runner still owns physical EOF, process success and all source guards.
pub(super) struct ProjectionDecoder {
    track: Track,
    stream_index: u8,
    packets: Decoder,
    fold: TrackFold,
    line: Vec<u8>,
    bytes: u64,
    seen_packet: bool,
    stream: Option<StreamFacts>,
    terminal: bool,
}

impl ProjectionDecoder {
    pub(super) fn new(track: Track) -> Result<Self> {
        Self::with_stream_index(track, 0)
    }

    /// Native projection mapping only. Selecting a stream does not establish
    /// that a candidate has no additional tracks; a finalizer must prove that.
    pub(super) fn with_stream_index(track: Track, stream_index: u8) -> Result<Self> {
        Ok(Self {
            track,
            stream_index,
            packets: Decoder::new(Streams::new(&[(stream_index, track)])?),
            fold: TrackFold::new(track),
            line: Vec::with_capacity(LINE),
            bytes: 0,
            seen_packet: false,
            stream: None,
            terminal: false,
        })
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.feed_with(bytes, |_| Ok(()))
    }

    /// Consumer output remains provisional until the trailing stream row and
    /// the caller's physical EOF/process/source/tool checks have all succeeded.
    pub(super) fn feed_with(
        &mut self,
        bytes: &[u8],
        mut consume: impl FnMut(PacketFact) -> Result<()>,
    ) -> Result<()> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        let result = self.feed_inner(bytes, &mut consume);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }

    fn feed_inner(
        &mut self,
        bytes: &[u8],
        consume: &mut impl FnMut(PacketFact) -> Result<()>,
    ) -> Result<()> {
        if bytes.len() > CHUNK {
            return Err(Fault::Bounds);
        }
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= TEXT)
            .ok_or(Fault::Bounds)?;
        for &byte in bytes {
            if self.line.len() == LINE {
                return Err(Fault::Bounds);
            }
            self.line.push(byte);
            if byte != b'\n' {
                continue;
            }
            if self.line.starts_with(b"packet|") {
                if self.stream.is_some() {
                    return Err(Fault::InvalidRow);
                }
                let fold = &mut self.fold;
                self.packets.feed(&self.line, |packet| {
                    fold.push(packet)?;
                    consume(packet)
                })?;
                self.seen_packet = true;
            } else if self.line.starts_with(b"stream|") {
                if !self.seen_packet || self.stream.is_some() {
                    return Err(Fault::InvalidRow);
                }
                self.stream = Some(parse_stream(&self.line, self.track, self.stream_index)?);
            } else {
                return Err(Fault::InvalidRow);
            }
            self.line.clear();
        }
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<ProbeFacts> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        self.terminal = true;
        if !self.line.is_empty() {
            return Err(Fault::Truncated);
        }
        if !self.seen_packet {
            return Err(Fault::Empty);
        }
        let stream = self.stream.take().ok_or(Fault::InvalidRow)?;
        self.packets.finish()?;
        let packets = self.fold.finish()?;
        Ok(ProbeFacts { packets, stream })
    }
}

const KEYS: [&str; 8] = [
    "index",
    "codec_name",
    "codec_type",
    "time_base",
    "width",
    "height",
    "sample_rate",
    "channels",
];

fn unsigned(value: &str) -> Result<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Fault::InvalidValue);
    }
    value.parse().map_err(|_| Fault::InvalidValue)
}

fn positive(value: &str) -> Result<u32> {
    let n = unsigned(value)?;
    if n == 0 {
        Err(Fault::InvalidValue)
    } else {
        Ok(n)
    }
}

fn time_base(value: &str) -> Result<TimeBase> {
    let (numerator, denominator) = value.split_once('/').ok_or(Fault::InvalidValue)?;
    TimeBase::new(positive(numerator)?, positive(denominator)?)
}

fn parse_stream(line: &[u8], expected: Track, expected_index: u8) -> Result<StreamFacts> {
    let body = line.strip_suffix(b"\n").ok_or(Fault::Truncated)?;
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    if !body.iter().all(|&b| (32..=126).contains(&b) && b != b'\\') {
        return Err(Fault::InvalidRow);
    }
    let body = std::str::from_utf8(body).map_err(|_| Fault::InvalidRow)?;
    let mut parts = body.split('|');
    if parts.next() != Some("stream") {
        return Err(Fault::InvalidRow);
    }
    let mut fields = [None; 8];
    for part in parts {
        let (key, value) = part.split_once('=').ok_or(Fault::InvalidRow)?;
        let slot = KEYS.iter().position(|candidate| *candidate == key).ok_or(
            if key.starts_with("side_") {
                Fault::UnsupportedSideData
            } else {
                Fault::InvalidRow
            },
        )?;
        if fields[slot].replace(value).is_some() {
            return Err(Fault::InvalidRow);
        }
    }
    let get = |i: usize| fields[i].ok_or(Fault::InvalidRow);
    if unsigned(get(0)?)? != expected_index as u32 {
        return Err(Fault::UnknownStream);
    }
    let track = match (get(1)?, get(2)?) {
        ("vp9", "video") => Track::Video,
        ("aac", "audio") => Track::Audio,
        _ => return Err(Fault::InvalidValue),
    };
    if track != expected {
        return Err(Fault::UnknownStream);
    }
    let time_base = time_base(get(3)?)?;
    let mut out = StreamFacts {
        track,
        time_base,
        width: 0,
        height: 0,
        sample_rate: 0,
        channels: 0,
    };
    match track {
        Track::Video => {
            if fields[6].is_some() || fields[7].is_some() {
                return Err(Fault::InvalidRow);
            }
            out.width = positive(get(4)?)?;
            out.height = positive(get(5)?)?;
            if out.width > DIMENSION || out.height > DIMENSION {
                return Err(Fault::Bounds);
            }
        }
        Track::Audio => {
            if fields[4].is_some() || fields[5].is_some() {
                return Err(Fault::InvalidRow);
            }
            out.sample_rate = positive(get(6)?)?;
            out.channels = u8::try_from(positive(get(7)?)?).map_err(|_| Fault::InvalidValue)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: &str = "packet|stream_index=0|pts=46800001|dts=46800001|duration=33|size=3|flags=K__|data_hash=SHA256:1111111111111111111111111111111111111111111111111111111111111111\n";
    const VIDEO: &str =
        "stream|index=0|codec_name=vp9|codec_type=video|time_base=1/1000|width=854|height=480\n";
    const AUDIO: &str = "stream|index=0|codec_name=aac|codec_type=audio|time_base=1/44100|sample_rate=44100|channels=2\n";

    fn decode(bytes: &[u8], track: Track) -> Result<ProbeFacts> {
        let mut decoder = ProjectionDecoder::new(track)?;
        decoder.feed(bytes)?;
        decoder.finish()
    }
    fn transcript(stream: &str) -> Vec<u8> {
        format!("{PACKET}{stream}").into_bytes()
    }

    #[test]
    fn both_shapes_every_split_and_windows_newlines() {
        for (track, stream) in [(Track::Video, VIDEO), (Track::Audio, AUDIO)] {
            let bytes = transcript(stream);
            let expected = decode(&bytes, track).unwrap();
            assert_eq!(expected.stream.track, track);
            assert_eq!(expected.packets.track, track);
            assert_eq!(expected.packets.count, 1);
            for split in 0..=bytes.len() {
                let mut decoder = ProjectionDecoder::new(track).unwrap();
                decoder.feed(&bytes[..split]).unwrap();
                decoder.feed(&bytes[split..]).unwrap();
                assert_eq!(decoder.finish().unwrap(), expected);
            }
            let crlf = String::from_utf8(bytes).unwrap().replace('\n', "\r\n");
            assert_eq!(decode(crlf.as_bytes(), track).unwrap(), expected);
        }
        let v = decode(&transcript(VIDEO), Track::Video).unwrap().stream;
        assert_eq!(
            (v.width, v.height, v.sample_rate, v.channels),
            (854, 480, 0, 0)
        );
        let a = decode(&transcript(AUDIO), Track::Audio).unwrap().stream;
        assert_eq!(
            (a.width, a.height, a.sample_rate, a.channels),
            (0, 0, 44100, 2)
        );
    }

    #[test]
    fn key_order_and_reduced_timebase_are_not_assumed() {
        let reordered = "stream|height=480|time_base=1000/1000000|codec_type=video|width=854|index=0|codec_name=vp9\n";
        assert_eq!(
            decode(&transcript(reordered), Track::Video),
            decode(&transcript(VIDEO), Track::Video)
        );
        let max = VIDEO.replace("1/1000", "1000000000/1000000000");
        assert_eq!(
            decode(&transcript(&max), Track::Video)
                .unwrap()
                .stream
                .time_base,
            TimeBase::new(1, 1).unwrap()
        );
    }

    #[test]
    fn stream_must_follow_packets_exactly_once_and_nothing_after_it() {
        assert_eq!(decode(b"", Track::Video), Err(Fault::Empty));
        for bytes in [
            VIDEO.as_bytes().to_vec(),
            PACKET.as_bytes().to_vec(),
            format!("{PACKET}{VIDEO}{VIDEO}").into_bytes(),
            format!("{PACKET}{VIDEO}{PACKET}").into_bytes(),
            format!("{PACKET}{VIDEO}\n").into_bytes(),
            format!("{PACKET}format|duration=5\n").into_bytes(),
        ] {
            assert_eq!(decode(&bytes, Track::Video), Err(Fault::InvalidRow));
        }
        let short = format!("{PACKET}{}", VIDEO.trim_end());
        assert_eq!(
            decode(short.as_bytes(), Track::Video),
            Err(Fault::Truncated)
        );
    }

    #[test]
    fn missing_duplicate_unknown_and_foreign_fields_fail() {
        for changed in [
            VIDEO.replace("|index=0", ""),
            VIDEO.replace("|codec_name=vp9", ""),
            VIDEO.replace("|codec_type=video", ""),
            VIDEO.replace("|time_base=1/1000", ""),
            VIDEO.replace("|width=854", ""),
            VIDEO.replace("|height=480", ""),
            VIDEO.replace('\n', "|index=0\n"),
            VIDEO.replace('\n', "|height=480\n"),
            VIDEO.replace('\n', "|sample_rate=0\n"),
            VIDEO.replace('\n', "|channels=0\n"),
            VIDEO.replace('\n', "|tag:title=test\n"),
            VIDEO.replace('\n', "|codec_long_name=VP9\n"),
        ] {
            assert_eq!(
                decode(&transcript(&changed), Track::Video),
                Err(Fault::InvalidRow)
            );
        }
        assert_eq!(
            decode(
                &transcript(&VIDEO.replace("index=0", "index=1")),
                Track::Video
            ),
            Err(Fault::UnknownStream)
        );
        assert_eq!(
            decode(&transcript(AUDIO), Track::Video),
            Err(Fault::UnknownStream)
        );
        for key in ["width=0", "height=0"] {
            let changed = AUDIO.replace('\n', &format!("|{key}\n"));
            assert_eq!(
                decode(&transcript(&changed), Track::Audio),
                Err(Fault::InvalidRow)
            );
        }
        let side = VIDEO.replace('\n', "|side_data_type=unknown\n");
        assert_eq!(
            decode(&transcript(&side), Track::Video),
            Err(Fault::UnsupportedSideData)
        );
    }

    #[test]
    fn timebase_rejects_ambiguous_zero_and_overflow_values() {
        for value in [
            "",
            "N/A",
            "0/1",
            "1/0",
            "-1/1",
            "1/-1",
            "+1/1",
            "1/+1",
            "1.0/1",
            "1e3/1",
            "1/1/1",
            "1",
            " 1/1",
            "1/1 ",
            "1000000001/1",
            "1/1000000001",
            "4294967296/1",
        ] {
            let changed = VIDEO.replace("1/1000", value);
            assert_eq!(
                decode(&transcript(&changed), Track::Video),
                Err(Fault::InvalidValue),
                "{value}"
            );
        }
    }

    #[test]
    fn scalar_values_are_typed_bounded_and_codec_specific() {
        for changed in [
            VIDEO.replace("width=854", "width=0"),
            VIDEO.replace("height=480", "height=-1"),
            VIDEO.replace("codec_name=vp9", "codec_name=av1"),
            VIDEO.replace("codec_type=video", "codec_type=audio"),
            VIDEO.replace("index=0", "index=N/A"),
            VIDEO.replace("width=854", "width=4294967296"),
        ] {
            assert_eq!(
                decode(&transcript(&changed), Track::Video),
                Err(Fault::InvalidValue)
            );
        }
        assert_eq!(
            decode(
                &transcript(&VIDEO.replace("width=854", "width=16385")),
                Track::Video
            ),
            Err(Fault::Bounds)
        );
        assert!(decode(
            &transcript(&VIDEO.replace("width=854", "width=16384")),
            Track::Video
        )
        .is_ok());
        for changed in [
            AUDIO.replace("sample_rate=44100", "sample_rate=0"),
            AUDIO.replace("sample_rate=44100", "sample_rate=4294967296"),
            AUDIO.replace("channels=2", "channels=0"),
            AUDIO.replace("channels=2", "channels=256"),
        ] {
            assert_eq!(
                decode(&transcript(&changed), Track::Audio),
                Err(Fault::InvalidValue)
            );
        }
        // Expression bounds are not support claims; native Unit equality is later.
        assert!(decode(
            &transcript(
                &AUDIO
                    .replace("sample_rate=44100", "sample_rate=4294967295")
                    .replace("channels=2", "channels=255")
            ),
            Track::Audio
        )
        .is_ok());
    }

    #[test]
    fn non_ascii_escape_control_and_extra_delimiters_fail() {
        for changed in [
            VIDEO.replace("vp9", "vp\\9"),
            VIDEO.replace("vp9", "vp\0"),
            VIDEO.replace("vp9", "vp\t9"),
            VIDEO.replace("vp9", "한글"),
            VIDEO.replace('\n', "\r\r\n"),
            VIDEO.replace('\n', "|\n"),
        ] {
            assert_eq!(
                decode(&transcript(&changed), Track::Video),
                Err(Fault::InvalidRow)
            );
        }
        let mut invalid = transcript(VIDEO);
        let at = invalid.len() - 2;
        invalid[at] = 0xff;
        assert_eq!(decode(&invalid, Track::Video), Err(Fault::InvalidRow));
    }

    #[test]
    fn packet_rules_stay_strict_and_known_side_data_is_preserved() {
        let side = "|side_datum/skip_samples:side_data_type=Skip Samples|side_datum/skip_samples:skip_samples=651|side_datum/skip_samples:discard_padding=0|side_datum/skip_samples:skip_reason=0|side_datum/skip_samples:discard_reason=0";
        let changed = format!("{}{side}\n{AUDIO}", PACKET.trim_end());
        let with_side = decode(changed.as_bytes(), Track::Audio).unwrap();
        let plain = decode(&transcript(AUDIO), Track::Audio).unwrap();
        assert_ne!(with_side.packets.digest, plain.packets.digest);
        for (from, to, error) in [
            ("stream_index=0", "stream_index=1", Fault::UnknownStream),
            ("duration=33", "duration=0", Fault::InvalidValue),
            ("flags=K__", "flags=KD_", Fault::UnsupportedFlags),
        ] {
            let changed = format!("{}{VIDEO}", PACKET.replace(from, to));
            assert_eq!(decode(changed.as_bytes(), Track::Video), Err(error));
        }
    }

    #[test]
    fn chunk_line_text_limits_and_terminal_state_are_bounded() {
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        assert_eq!(decoder.feed(&vec![b'x'; CHUNK + 1]), Err(Fault::Bounds));
        assert_eq!(decoder.feed(b""), Err(Fault::Terminal));
        assert_eq!(decoder.finish(), Err(Fault::Terminal));
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        assert_eq!(decoder.feed(&vec![b'x'; LINE]), Ok(()));
        assert_eq!(decoder.feed(b"\n"), Err(Fault::Bounds));
        let bytes = transcript(VIDEO);
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        decoder.bytes = TEXT - bytes.len() as u64;
        decoder.feed(&bytes).unwrap();
        decoder.finish().unwrap();
        assert_eq!(decoder.feed(b""), Err(Fault::Terminal));
        assert_eq!(decoder.finish(), Err(Fault::Terminal));
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        decoder.bytes = TEXT;
        assert_eq!(decoder.feed(b"x"), Err(Fault::Bounds));
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        decoder.feed(PACKET.as_bytes()).unwrap();
        assert_eq!(decoder.finish(), Err(Fault::InvalidRow));
        assert_eq!(decoder.feed(VIDEO.as_bytes()), Err(Fault::Terminal));
    }

    #[test]
    fn many_packets_keep_only_fold_and_one_stream_record() {
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        for _ in 0..10_000 {
            decoder.feed(PACKET.as_bytes()).unwrap();
            assert!(decoder.line.is_empty());
            assert!(decoder.line.capacity() <= LINE);
            assert!(decoder.stream.is_none());
        }
        decoder.feed(VIDEO.as_bytes()).unwrap();
        let result = decoder.finish().unwrap();
        assert_eq!(result.packets.count, 10_000);
        assert_eq!(result.packets.bytes, 30_000);
    }

    #[test]
    fn callback_receives_each_valid_packet_once_for_every_split() {
        let input = format!("{PACKET}{PACKET}{VIDEO}");
        let expected = decode(input.as_bytes(), Track::Video).unwrap();
        for split in 0..=input.len() {
            let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
            let mut received = TrackFold::new(Track::Video);
            decoder
                .feed_with(&input.as_bytes()[..split], |p| received.push(p))
                .unwrap();
            decoder
                .feed_with(&input.as_bytes()[split..], |p| received.push(p))
                .unwrap();
            assert_eq!(decoder.finish().unwrap(), expected);
            assert_eq!(received.finish().unwrap(), expected.packets);
        }
    }

    #[test]
    fn callback_failure_and_late_stream_failure_cannot_finish() {
        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        let mut calls = 0;
        assert_eq!(
            decoder.feed_with(&transcript(VIDEO), |_| {
                calls += 1;
                Err(Fault::Sink)
            }),
            Err(Fault::Sink)
        );
        assert_eq!(calls, 1);
        assert_eq!(decoder.feed(b""), Err(Fault::Terminal));
        assert_eq!(decoder.finish(), Err(Fault::Terminal));

        let mut decoder = ProjectionDecoder::new(Track::Video).unwrap();
        let wrong = transcript(&VIDEO.replace("index=0", "index=1"));
        assert_eq!(
            decoder.feed_with(&wrong, |_| {
                calls += 1;
                Ok(())
            }),
            Err(Fault::UnknownStream)
        );
        assert_eq!(calls, 2); // A provisional packet is not a successful probe.
        assert_eq!(decoder.finish(), Err(Fault::Terminal));
    }

    #[test]
    fn explicit_projection_index_must_match_both_packets_and_stream() {
        assert!(matches!(
            ProjectionDecoder::with_stream_index(Track::Audio, 32),
            Err(Fault::InvalidValue)
        ));
        for index in [1, 31] {
            let input = transcript(AUDIO);
            let mapped = String::from_utf8(input)
                .unwrap()
                .replace("index=0", &format!("index={index}"));
            let mut decoder = ProjectionDecoder::with_stream_index(Track::Audio, index).unwrap();
            decoder.feed(mapped.as_bytes()).unwrap();
            assert_eq!(
                decoder.finish().unwrap(),
                decode(&transcript(AUDIO), Track::Audio).unwrap()
            );
            let mut decoder = ProjectionDecoder::with_stream_index(Track::Audio, index).unwrap();
            assert_eq!(decoder.feed(PACKET.as_bytes()), Err(Fault::UnknownStream));
            let mut decoder = ProjectionDecoder::new(Track::Audio).unwrap();
            assert_eq!(decoder.feed(mapped.as_bytes()), Err(Fault::UnknownStream));
        }
    }
}
