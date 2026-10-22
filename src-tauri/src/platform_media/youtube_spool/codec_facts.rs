//! Dormant fixed FFprobe metadata facts, not codec/packet validation or media authority.
//! Source construction crosschecks the native structural index. Candidate metadata
//! still cannot prove random access, in-band configuration, packet coverage or EOF.
use super::{
    index::{Configuration, Format},
    packet_facts::TimeBase,
    Track,
};
use serde::{
    de::{self, IgnoredAny, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_JSON: usize = 65_536;
const MAX_TEXT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    Bounds,
    Json,
    Unsupported,
    NativeMismatch,
    Mismatch,
}
type Result<T> = std::result::Result<T, Fault>;

/// `Missing` is different from an explicitly reported value. A JSON null never
/// deserializes through T and cannot silently become absence as Option<T> would.
#[derive(Debug, PartialEq, Eq)]
enum Field<T> {
    Missing,
    Present(T),
}
impl<T> Default for Field<T> {
    fn default() -> Self {
        Self::Missing
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        T::deserialize(d).map(Self::Present)
    }
}
impl<T> Field<T> {
    fn missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
    fn get(&self) -> Result<&T> {
        match self {
            Self::Present(value) => Ok(value),
            Self::Missing => Err(Fault::Unsupported),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Text(String);
impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct TextVisitor;
        impl<'de> Visitor<'de> for TextVisitor {
            type Value = Text;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bounded ASCII metadata")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Text, E> {
                if value.is_empty()
                    || value.len() > MAX_TEXT
                    || !value.bytes().all(|b| (32..=126).contains(&b))
                {
                    return Err(E::custom("invalid metadata string"));
                }
                Ok(Text(value.to_owned()))
            }
        }
        d.deserialize_str(TextVisitor)
    }
}
impl Text {
    fn is(&self, value: &str) -> bool {
        self.0 == value
    }
}

#[derive(Debug, PartialEq, Eq)]
struct EmptySideData;
impl<'de> Deserialize<'de> for EmptySideData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct EmptyVisitor;
        impl<'de> Visitor<'de> for EmptyVisitor {
            type Value = EmptySideData;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("empty stream side-data list")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("unsupported stream side data"));
                }
                Ok(EmptySideData)
            }
        }
        d.deserialize_seq(EmptyVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStream {
    index: u8,
    codec_name: Text,
    profile: Text,
    codec_type: Text,
    codec_tag_string: Text,
    time_base: Text,
    #[serde(default)]
    width: Field<u32>,
    #[serde(default)]
    height: Field<u32>,
    #[serde(default)]
    sample_aspect_ratio: Field<Text>,
    #[serde(default)]
    pix_fmt: Field<Text>,
    #[serde(default)]
    color_range: Field<Text>,
    #[serde(default)]
    color_space: Field<Text>,
    #[serde(default)]
    color_transfer: Field<Text>,
    #[serde(default)]
    color_primaries: Field<Text>,
    #[serde(default)]
    chroma_location: Field<Text>,
    #[serde(default)]
    field_order: Field<Text>,
    #[serde(default)]
    sample_rate: Field<Text>,
    #[serde(default)]
    channels: Field<u8>,
    #[serde(default)]
    channel_layout: Field<Text>,
    #[serde(default)]
    initial_padding: Field<u32>,
    #[serde(default)]
    extradata_size: Field<u32>,
    #[serde(default)]
    extradata_hash: Field<Text>,
    #[serde(default)]
    side_data_list: Field<EmptySideData>,
}

struct Streams(Vec<RawStream>);
impl<'de> Deserialize<'de> for Streams {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct StreamsVisitor;
        impl<'de> Visitor<'de> for StreamsVisitor {
            type Value = Streams;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("one or two streams")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::with_capacity(2);
                while values.len() < 2 {
                    match seq.next_element::<RawStream>()? {
                        Some(value) => values.push(value),
                        None => return Ok(Streams(values)),
                    }
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("too many streams"));
                }
                Ok(Streams(values))
            }
        }
        d.deserialize_seq(StreamsVisitor)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    streams: Streams,
    // The pinned FFprobe projection also emits these empty wrapper sections.
    // They may be absent but never contain entries, null or arbitrary data.
    #[serde(default)]
    programs: Field<EmptySideData>,
    #[serde(default)]
    stream_groups: Field<EmptySideData>,
}

fn parse(bytes: &[u8]) -> Result<Vec<RawStream>> {
    if bytes.is_empty() || bytes.len() > MAX_JSON {
        return Err(Fault::Bounds);
    }
    serde_json::from_slice::<Projection>(bytes)
        .map(|p| {
            let _ = (p.programs, p.stream_groups);
            p.streams.0
        })
        .map_err(|_| Fault::Json)
}
fn time_base(value: &Text) -> Result<TimeBase> {
    let (a, b) = value.0.split_once('/').ok_or(Fault::Unsupported)?;
    let number = |s: &str| -> Result<u32> {
        if s.is_empty()
            || s.len() > 10
            || !s.bytes().all(|b| b.is_ascii_digit())
            || s.starts_with('0')
        {
            return Err(Fault::Unsupported);
        }
        s.parse().map_err(|_| Fault::Unsupported)
    };
    TimeBase::new(number(a)?, number(b)?).map_err(|_| Fault::Unsupported)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AscForm {
    TwoBytes,
    SixteenZeroPaddedBytes,
}
fn asc(size: u32, hash: &Text) -> Result<AscForm> {
    let (bytes, form): (&[u8], _) = match size {
        2 => (&[0x12, 0x10], AscForm::TwoBytes),
        16 => (
            &[0x12, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            AscForm::SixteenZeroPaddedBytes,
        ),
        _ => return Err(Fault::Unsupported),
    };
    if hash.0 != format!("SHA256:{:x}", Sha256::digest(bytes)) {
        return Err(Fault::Unsupported);
    }
    Ok(form)
}

#[derive(Debug, PartialEq, Eq)]
struct Video {
    width: u32,
    height: u32,
    sar: Field<Text>,
    pixel: Field<Text>,
    range: Field<Text>,
    space: Field<Text>,
    transfer: Field<Text>,
    primaries: Field<Text>,
    chroma: Field<Text>,
    field: Field<Text>,
}
#[derive(Debug, PartialEq, Eq)]
enum Codec {
    Video(Video),
    Audio(AscForm),
}

/// No public/deserializable constructor. This is only crosschecked metadata,
/// not a source-file capability or a complete codec-equivalence certificate.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SourceFacts {
    codec: Codec,
    time_base: TimeBase,
}
impl SourceFacts {
    pub(super) fn track(&self) -> Track {
        match self.codec {
            Codec::Video(_) => Track::Video,
            Codec::Audio(_) => Track::Audio,
        }
    }
    pub(super) fn time_base(&self) -> TimeBase {
        self.time_base
    }
    pub(super) fn asc_form(&self) -> Option<AscForm> {
        match self.codec {
            Codec::Audio(form) => Some(form),
            Codec::Video(_) => None,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CandidateFacts {
    video_time_base: TimeBase,
    audio_time_base: TimeBase,
    audio_asc: AscForm,
}
impl CandidateFacts {
    pub(super) fn video_time_base(&self) -> TimeBase {
        self.video_time_base
    }
    pub(super) fn audio_time_base(&self) -> TimeBase {
        self.audio_time_base
    }
    pub(super) fn audio_asc_form(&self) -> AscForm {
        self.audio_asc
    }
}

fn video(raw: RawStream, index: u8, tag: &str) -> Result<(Video, TimeBase)> {
    if raw.index != index
        || !raw.codec_name.is("vp9")
        || !raw.codec_type.is("video")
        || !raw.profile.is("Profile 0")
        || !raw.codec_tag_string.is(tag)
        || !raw.sample_rate.missing()
        || !raw.channels.missing()
        || !raw.channel_layout.missing()
        || !raw.initial_padding.missing()
        || !raw.extradata_size.missing()
        || !raw.extradata_hash.missing()
    {
        return Err(Fault::Unsupported);
    }
    let width = *raw.width.get()?;
    let height = *raw.height.get()?;
    if !(1..=16384).contains(&width)
        || !(1..=16384).contains(&height)
        || !raw.sample_aspect_ratio.get()?.is("1:1")
        || !raw.pix_fmt.get()?.is("yuv420p")
        || !raw.color_range.get()?.is("tv")
        || !raw.color_space.get()?.is("bt709")
    {
        return Err(Fault::Unsupported);
    }
    // These explicit values are a narrow comparison subset, not claims that
    // an actual platform stream reporting them has been exercised. Missing
    // stays missing and cannot equal even a plausible explicit default.
    for (field, supported) in [
        (&raw.color_transfer, "bt709"),
        (&raw.color_primaries, "bt709"),
        (&raw.chroma_location, "left"),
        (&raw.field_order, "progressive"),
    ] {
        if let Field::Present(value) = field {
            if !value.is(supported) {
                return Err(Fault::Unsupported);
            }
        }
    }
    let time_base = time_base(&raw.time_base)?;
    let _ = raw.side_data_list; // Deserializer allows only absent or empty.
    Ok((
        Video {
            width,
            height,
            sar: raw.sample_aspect_ratio,
            pixel: raw.pix_fmt,
            range: raw.color_range,
            space: raw.color_space,
            transfer: raw.color_transfer,
            primaries: raw.color_primaries,
            chroma: raw.chroma_location,
            field: raw.field_order,
        },
        time_base,
    ))
}
fn audio(raw: RawStream, index: u8) -> Result<(AscForm, TimeBase)> {
    if raw.index != index
        || !raw.codec_name.is("aac")
        || !raw.codec_type.is("audio")
        || !raw.profile.is("LC")
        || !raw.codec_tag_string.is("mp4a")
        || !raw.width.missing()
        || !raw.height.missing()
        || !raw.sample_aspect_ratio.missing()
        || !raw.pix_fmt.missing()
        || !raw.color_range.missing()
        || !raw.color_space.missing()
        || !raw.color_transfer.missing()
        || !raw.color_primaries.missing()
        || !raw.chroma_location.missing()
        || !raw.field_order.missing()
        || !raw.sample_rate.get()?.is("44100")
        || *raw.channels.get()? != 2
        || !raw.channel_layout.get()?.is("stereo")
        || *raw.initial_padding.get()? != 0
    {
        return Err(Fault::Unsupported);
    }
    let form = asc(*raw.extradata_size.get()?, raw.extradata_hash.get()?)?;
    let _ = raw.side_data_list;
    Ok((form, time_base(&raw.time_base)?))
}

pub(super) fn source(bytes: &[u8], configuration: &Configuration) -> Result<SourceFacts> {
    let mut streams = parse(bytes)?;
    if streams.len() != 1 {
        return Err(Fault::Unsupported);
    }
    let raw = streams.pop().ok_or(Fault::Unsupported)?;
    let scale = u32::try_from(configuration.timebase).map_err(|_| Fault::NativeMismatch)?;
    match configuration.format {
        Format::Vp9Webm => {
            let (video, time_base) = video(raw, 0, "[0][0][0][0]")?;
            let expected =
                TimeBase::new(scale, 1_000_000_000).map_err(|_| Fault::NativeMismatch)?;
            if video.width != configuration.width
                || video.height != configuration.height
                || configuration.sample_rate != 0
                || configuration.channels != 0
                || time_base != expected
            {
                return Err(Fault::NativeMismatch);
            }
            Ok(SourceFacts {
                codec: Codec::Video(video),
                time_base,
            })
        }
        Format::AacMp4 => {
            let (form, time_base) = audio(raw, 0)?;
            let expected = TimeBase::new(1, scale).map_err(|_| Fault::NativeMismatch)?;
            if configuration.sample_rate != 44100
                || configuration.channels != 2
                || configuration.width != 0
                || configuration.height != 0
                || time_base != expected
            {
                return Err(Fault::NativeMismatch);
            }
            Ok(SourceFacts {
                codec: Codec::Audio(form),
                time_base,
            })
        }
    }
}

pub(super) fn candidate(
    bytes: &[u8],
    video_source: &SourceFacts,
    audio_source: &SourceFacts,
) -> Result<CandidateFacts> {
    let Codec::Video(expected_video) = &video_source.codec else {
        return Err(Fault::Mismatch);
    };
    let Codec::Audio(_) = &audio_source.codec else {
        return Err(Fault::Mismatch);
    };
    let streams = parse(bytes)?;
    if streams.len() != 2 {
        return Err(Fault::Unsupported);
    }
    let mut streams = streams.into_iter();
    let (actual_video, video_time_base) =
        video(streams.next().ok_or(Fault::Unsupported)?, 0, "vp09")?;
    let (audio_asc, audio_time_base) = audio(streams.next().ok_or(Fault::Unsupported)?, 1)?;
    if actual_video != *expected_video {
        return Err(Fault::Mismatch);
    }
    // Both ASC forms are already proven to represent only the same exact LC
    // 44.1kHz stereo ASC with zero padding. All other audio fields are fixed
    // by audio(), not discarded or inferred from the filename.
    Ok(CandidateFacts {
        video_time_base,
        audio_time_base,
        audio_asc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn raw_video() -> Value {
        json!({"index":0,"codec_name":"vp9","profile":"Profile 0","codec_type":"video",
        "codec_tag_string":"[0][0][0][0]","width":854,"height":480,"sample_aspect_ratio":"1:1",
        "pix_fmt":"yuv420p","color_range":"tv","color_space":"bt709","time_base":"1/1000"})
    }
    fn raw_audio() -> Value {
        json!({"index":0,"codec_name":"aac","profile":"LC","codec_type":"audio",
        "codec_tag_string":"mp4a","sample_rate":"44100","channels":2,"channel_layout":"stereo",
        "initial_padding":0,"time_base":"1/44100","extradata_size":16,
        "extradata_hash":"SHA256:cabb50233fa5538a610abfe04caaa74502e99d0bbee694f437026906bf1f1568"})
    }
    fn config(format: Format) -> Configuration {
        let video = format == Format::Vp9Webm;
        Configuration {
            format,
            track_id: 1,
            timebase: if video { 1_000_000 } else { 44100 },
            width: if video { 854 } else { 0 },
            height: if video { 480 } else { 0 },
            default_duration_ns: if video { Some(33333333) } else { None },
            sample_rate: if video { 0 } else { 44100 },
            channels: if video { 0 } else { 2 },
            semantic_sha256: [0; 32],
        }
    }
    fn projection(streams: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"streams":streams})).unwrap()
    }
    fn sources() -> (SourceFacts, SourceFacts) {
        (
            source(&projection(vec![raw_video()]), &config(Format::Vp9Webm)).unwrap(),
            source(&projection(vec![raw_audio()]), &config(Format::AacMp4)).unwrap(),
        )
    }
    fn output() -> Vec<Value> {
        let mut video = raw_video();
        video["codec_tag_string"] = json!("vp09");
        video["time_base"] = json!("1/16000");
        let mut audio = raw_audio();
        audio["index"] = json!(1);
        vec![video, audio]
    }
    fn check_output(values: Vec<Value>) -> Result<CandidateFacts> {
        let (video, audio) = sources();
        candidate(&projection(values), &video, &audio)
    }

    #[test]
    fn observed_sources_and_output_preserve_distinct_timebases() {
        let (video, audio) = sources();
        assert_eq!(video.track(), Track::Video);
        assert_eq!(audio.track(), Track::Audio);
        assert_eq!(video.time_base(), TimeBase::new(1, 1000).unwrap());
        assert_eq!(audio.time_base(), TimeBase::new(1, 44100).unwrap());
        assert_eq!(video.asc_form(), None);
        assert_eq!(audio.asc_form(), Some(AscForm::SixteenZeroPaddedBytes));
        let facts = candidate(&projection(output()), &video, &audio).unwrap();
        assert_eq!(facts.video_time_base(), TimeBase::new(1, 16000).unwrap());
        assert_eq!(facts.audio_time_base(), TimeBase::new(1, 44100).unwrap());
        assert_eq!(facts.audio_asc_form(), AscForm::SixteenZeroPaddedBytes);
    }

    #[test]
    fn only_exact_two_and_sixteen_byte_asc_forms_are_equivalent() {
        let hash = format!("SHA256:{:x}", Sha256::digest([0x12, 0x10]));
        let mut canonical = raw_audio();
        canonical["extradata_size"] = json!(2);
        canonical["extradata_hash"] = json!(hash);
        let audio = source(
            &projection(vec![canonical.clone()]),
            &config(Format::AacMp4),
        )
        .unwrap();
        assert_eq!(audio.asc_form(), Some(AscForm::TwoBytes));
        let (video, padded) = sources();
        assert_eq!(
            candidate(&projection(output()), &video, &audio)
                .unwrap()
                .audio_asc_form(),
            AscForm::SixteenZeroPaddedBytes
        );
        let mut values = output();
        canonical["index"] = json!(1);
        values[1] = canonical.clone();
        assert_eq!(
            candidate(&projection(values), &video, &padded)
                .unwrap()
                .audio_asc_form(),
            AscForm::TwoBytes
        );
        canonical["index"] = json!(0);
        for size in [0, 1, 3, 15, 17, 65536] {
            canonical["extradata_size"] = json!(size);
            assert!(source(
                &projection(vec![canonical.clone()]),
                &config(Format::AacMp4)
            )
            .is_err());
        }
        let mut bad = raw_audio();
        bad["extradata_size"] = json!(2);
        assert!(source(&projection(vec![bad]), &config(Format::AacMp4)).is_err());
        let mut bad = raw_audio();
        bad["extradata_hash"] = json!(format!(
            "SHA256:{:x}",
            Sha256::digest([0x12, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        ));
        assert!(source(&projection(vec![bad]), &config(Format::AacMp4)).is_err());
    }

    #[test]
    fn native_dimensions_scalars_and_timescale_must_match() {
        let mut video = config(Format::Vp9Webm);
        for (width, height, scale) in [
            (853, 480, 1_000_000),
            (854, 481, 1_000_000),
            (854, 480, 1),
            (854, 480, 0),
            (854, 480, u64::MAX),
        ] {
            video.width = width;
            video.height = height;
            video.timebase = scale;
            assert_eq!(
                source(&projection(vec![raw_video()]), &video),
                Err(Fault::NativeMismatch)
            );
        }
        for (rate, channels, scale) in [(48000, 2, 44100), (44100, 1, 44100), (44100, 2, 48000)] {
            let mut audio = config(Format::AacMp4);
            audio.sample_rate = rate;
            audio.channels = channels;
            audio.timebase = scale;
            assert_eq!(
                source(&projection(vec![raw_audio()]), &audio),
                Err(Fault::NativeMismatch)
            );
        }
        assert!(source(&projection(vec![raw_video()]), &config(Format::AacMp4)).is_err());
    }

    #[test]
    fn missing_extra_duplicate_and_reordered_streams_fail() {
        let values = output();
        for streams in [
            vec![],
            vec![values[0].clone()],
            vec![values[1].clone(), values[0].clone()],
            vec![values[0].clone(), values[0].clone()],
            vec![values[0].clone(), values[1].clone(), raw_video()],
        ] {
            assert!(check_output(streams).is_err());
        }
        let mut duplicate = output();
        duplicate[1]["index"] = json!(0);
        assert!(check_output(duplicate).is_err());
        assert!(source(&projection(output()), &config(Format::Vp9Webm)).is_err());
        let (video, audio) = sources();
        assert_eq!(
            candidate(&projection(output()), &audio, &video),
            Err(Fault::Mismatch)
        );
    }

    #[test]
    fn every_required_field_rejects_missing_null_and_wrong_types() {
        for (value, format) in [
            (raw_video(), Format::Vp9Webm),
            (raw_audio(), Format::AacMp4),
        ] {
            for key in value.as_object().unwrap().keys() {
                let mut missing = value.clone();
                missing.as_object_mut().unwrap().remove(key);
                assert!(
                    source(&projection(vec![missing]), &config(format)).is_err(),
                    "missing {key}"
                );
                for invalid in [Value::Null, json!(false), json!([]), json!({})] {
                    let mut changed = value.clone();
                    changed[key] = invalid;
                    assert!(
                        source(&projection(vec![changed]), &config(format)).is_err(),
                        "invalid {key}"
                    );
                }
            }
        }
        let mut audio = raw_audio();
        audio["sample_rate"] = json!(44100);
        assert!(source(&projection(vec![audio]), &config(Format::AacMp4)).is_err());
        let mut video = raw_video();
        video["width"] = json!(854.0);
        assert!(source(&projection(vec![video]), &config(Format::Vp9Webm)).is_err());
    }

    #[test]
    fn duplicate_keys_unknown_keys_and_non_json_are_rejected() {
        let valid = String::from_utf8(projection(vec![raw_video()])).unwrap();
        let duplicate = valid.replacen("\"index\":0", "\"index\":0,\"index\":0", 1);
        assert_eq!(
            source(duplicate.as_bytes(), &config(Format::Vp9Webm)),
            Err(Fault::Json)
        );
        let repeated = valid.replacen("\"width\":854", "\"width\":854,\"width\":854", 1);
        assert_eq!(
            source(repeated.as_bytes(), &config(Format::Vp9Webm)),
            Err(Fault::Json)
        );
        let streams = serde_json::to_string(&vec![raw_video()]).unwrap();
        let repeated_root = format!("{{\"streams\":{streams},\"streams\":{streams}}}");
        assert_eq!(
            source(repeated_root.as_bytes(), &config(Format::Vp9Webm)),
            Err(Fault::Json)
        );
        for bytes in [
            b"{}".as_slice(),
            b"[]",
            b"null",
            b"{",
            b"{\"streams\":null}",
            b"{\"streams\":[],\"format\":{}}",
        ] {
            assert!(source(bytes, &config(Format::Vp9Webm)).is_err());
        }
        let mut unknown = raw_video();
        unknown["tags"] = json!({});
        assert_eq!(
            source(&projection(vec![unknown]), &config(Format::Vp9Webm)),
            Err(Fault::Json)
        );
        let mut trailing = valid.into_bytes();
        trailing.extend_from_slice(b" {}");
        assert_eq!(
            source(&trailing, &config(Format::Vp9Webm)),
            Err(Fault::Json)
        );
    }

    #[test]
    fn json_strings_numbers_and_timebases_are_bounded() {
        assert_eq!(
            source(&vec![b' '; MAX_JSON + 1], &config(Format::Vp9Webm)),
            Err(Fault::Bounds)
        );
        assert_eq!(source(&[], &config(Format::Vp9Webm)), Err(Fault::Bounds));
        let mut long = raw_video();
        long["profile"] = json!("x".repeat(MAX_TEXT + 1));
        assert_eq!(
            source(&projection(vec![long]), &config(Format::Vp9Webm)),
            Err(Fault::Json)
        );
        for invalid in [
            "0/1",
            "1/0",
            "-1/1000",
            "+1/1000",
            "01/1000",
            "1/01000",
            "1/1000000001",
            "4294967296/1",
            "1/1/1",
            "1.0/1000",
        ] {
            let mut out = output();
            out[0]["time_base"] = json!(invalid);
            assert!(check_output(out).is_err(), "{invalid}");
        }
        for invalid in [json!(-1), json!(u64::MAX), json!(0), json!(16385)] {
            let mut out = output();
            out[0]["width"] = invalid;
            assert!(check_output(out).is_err());
        }
        let mut out = output();
        out[0]["time_base"] = json!("1/1000000000");
        assert_eq!(
            check_output(out).unwrap().video_time_base(),
            TimeBase::new(1, 1_000_000_000).unwrap()
        );
    }

    #[test]
    fn narrow_video_and_audio_subset_rejects_different_configuration() {
        for (field, value) in [
            ("codec_name", json!("h264")),
            ("profile", json!("Profile 2")),
            ("pix_fmt", json!("yuv420p10le")),
            ("sample_aspect_ratio", json!("4:3")),
            ("color_range", json!("pc")),
            ("color_space", json!("bt2020nc")),
            ("color_transfer", json!("unknown")),
            ("field_order", json!("tt")),
            ("width", json!(1280)),
            ("height", json!(720)),
            ("extradata_size", json!(0)),
            ("codec_tag_string", json!("avc1")),
        ] {
            let mut values = output();
            values[0][field] = value;
            assert!(check_output(values).is_err(), "{field}");
        }
        for (field, value) in [
            ("codec_name", json!("opus")),
            ("profile", json!("HE-AAC")),
            ("sample_rate", json!("48000")),
            ("channels", json!(1)),
            ("channel_layout", json!("mono")),
            ("initial_padding", json!(651)),
            ("extradata_hash", json!("SHA256:unknown")),
            ("pix_fmt", json!("yuv420p")),
            ("codec_tag_string", json!("aac ")),
        ] {
            let mut values = output();
            values[1][field] = value;
            assert!(check_output(values).is_err(), "{field}");
        }
    }

    #[test]
    fn optional_metadata_is_never_defaulted_and_explicit_values_must_match() {
        for (key, value) in [
            ("color_transfer", "bt709"),
            ("color_primaries", "bt709"),
            ("chroma_location", "left"),
            ("field_order", "progressive"),
        ] {
            let mut raw = raw_video();
            raw[key] = json!(value);
            let source_video = source(&projection(vec![raw]), &config(Format::Vp9Webm)).unwrap();
            let (missing, audio) = sources();
            assert_eq!(
                candidate(&projection(output()), &source_video, &audio),
                Err(Fault::Mismatch)
            );
            let mut out = output();
            out[0][key] = json!(value);
            assert_eq!(
                candidate(&projection(out.clone()), &missing, &audio),
                Err(Fault::Mismatch)
            );
            assert!(candidate(&projection(out), &source_video, &audio).is_ok());
            let mut raw = raw_video();
            raw[key] = Value::Null;
            assert_eq!(
                source(&projection(vec![raw]), &config(Format::Vp9Webm)),
                Err(Fault::Json)
            );
            let mut raw = raw_video();
            raw[key] = json!("unspecified");
            assert!(source(&projection(vec![raw]), &config(Format::Vp9Webm)).is_err());
        }
    }

    #[test]
    fn absent_and_empty_side_data_allowed_but_null_or_any_entry_fails() {
        let mut out = output();
        out[0]["side_data_list"] = json!([]);
        out[1]["side_data_list"] = json!([]);
        assert!(check_output(out).is_ok());
        for invalid in [
            Value::Null,
            json!({}),
            json!([{}]),
            json!([{"side_data_type":"Display Matrix"}]),
            json!([{"side_data_type":"Skip Samples"}]),
        ] {
            for index in 0..2 {
                let mut out = output();
                out[index]["side_data_list"] = invalid.clone();
                assert_eq!(check_output(out), Err(Fault::Json));
            }
        }
    }

    #[test]
    fn pinned_empty_root_sections_do_not_change_stream_facts() {
        let expected = source(&projection(vec![raw_video()]), &config(Format::Vp9Webm)).unwrap();
        for keys in [
            vec!["programs"],
            vec!["stream_groups"],
            vec!["programs", "stream_groups"],
        ] {
            let mut value = json!({"streams":[raw_video()]});
            for key in keys {
                value[key] = json!([]);
            }
            assert_eq!(
                source(
                    &serde_json::to_vec(&value).unwrap(),
                    &config(Format::Vp9Webm)
                )
                .unwrap(),
                expected
            );
        }
        let (video, audio) = sources();
        let value = json!({"programs":[], "stream_groups":[], "streams":output()});
        assert_eq!(
            candidate(&serde_json::to_vec(&value).unwrap(), &video, &audio).unwrap(),
            check_output(output()).unwrap()
        );
    }

    #[test]
    fn root_wrappers_reject_null_entries_wrong_types_duplicates_and_unknowns() {
        for key in ["programs", "stream_groups"] {
            for invalid in [
                Value::Null,
                json!({}),
                json!(false),
                json!(0),
                json!("[]"),
                json!([{}]),
                json!([[]]),
            ] {
                let mut value = json!({"streams":[raw_video()]});
                value[key] = invalid;
                assert_eq!(
                    source(
                        &serde_json::to_vec(&value).unwrap(),
                        &config(Format::Vp9Webm)
                    ),
                    Err(Fault::Json)
                );
            }
            let streams = serde_json::to_string(&vec![raw_video()]).unwrap();
            let duplicate = format!("{{\"streams\":{streams},\"{key}\":[],\"{key}\":[]}}");
            assert_eq!(
                source(duplicate.as_bytes(), &config(Format::Vp9Webm)),
                Err(Fault::Json)
            );
        }
        for key in ["chapters", "format", "program", "stream_group"] {
            let mut value = json!({"streams":[raw_video()], "programs":[], "stream_groups":[]});
            value[key] = json!([]);
            assert_eq!(
                source(
                    &serde_json::to_vec(&value).unwrap(),
                    &config(Format::Vp9Webm)
                ),
                Err(Fault::Json)
            );
        }
        let value = json!({"streams":[], "programs":[], "stream_groups":[]});
        assert_eq!(
            source(
                &serde_json::to_vec(&value).unwrap(),
                &config(Format::Vp9Webm)
            ),
            Err(Fault::Unsupported)
        );
    }
}
