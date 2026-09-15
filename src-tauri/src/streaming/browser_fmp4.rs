//! Narrow, clear AVC/AAC fMP4 remuxer for bytes ALREADY appended by the page.
//! No URLs, downloads, decryption, codecs, subprocesses, or wall-clock timing.
//! Unsupported boxes/layout changes fail closed. Source clocks drive samples.
use crate::streaming::model::StreamError;
use std::collections::VecDeque;

const MAX_INIT: usize = 64 * 1024;
const MAX_BOX: usize = 16 * 1024 * 1024;
const MAX_PENDING: usize = 32 * 1024 * 1024;
const MAX_SAMPLES: usize = 120_000;
const TARGET_SECONDS: f64 = 12.0;

fn bad(reason: &'static str) -> StreamError {
    StreamError::new("ENCODED_UNSUPPORTED", reason, false)
}
fn bound() -> StreamError {
    bad("압축 영상 저장 버퍼 한도를 초과했습니다.")
}
type Result<T> = std::result::Result<T, StreamError>;

#[derive(Clone, Debug)]
pub struct EncodedTrackInput {
    pub track_index: u32,
    pub mime_type: String,
    pub init: Vec<u8>,
}
#[derive(Debug)]
pub struct EncodedSegment {
    pub bytes: Vec<u8>,
    pub duration_seconds: f64,
    pub source_start_seconds: f64,
    pub source_end_seconds: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Video,
    Audio,
}
#[derive(Clone)]
struct Track {
    input: u32,
    original_id: u32,
    id: u32,
    kind: Kind,
    scale: u32,
    trak: Vec<u8>,
    trex: Vec<u8>,
    default_duration: u32,
    default_size: u32,
    default_flags: u32,
    last_end: Option<u64>,
    recent: VecDeque<(u64, u64, [u8; 32])>,
}
struct Input {
    id: u32,
    pending: Vec<u8>,
    moof: Option<Vec<u8>>,
    init: Vec<u8>,
}
#[derive(Clone)]
struct Sample {
    dts: u64,
    duration: u32,
    flags: u32,
    cts: i32,
    data: Vec<u8>,
}
impl Sample {
    fn end(&self) -> u64 {
        self.dts + u64::from(self.duration)
    }
    fn key(&self) -> bool {
        self.flags & 0x0001_0000 == 0 && (self.flags >> 24) & 3 == 2
    }
}
pub struct EncodedMuxer {
    inputs: Vec<Input>,
    tracks: Vec<Track>,
    init: Vec<u8>,
    video: VecDeque<Sample>,
    audio: VecDeque<Sample>,
    started: bool,
    sequence: u32,
    failed: bool,
}

#[derive(Clone, Copy)]
struct Atom<'a> {
    kind: [u8; 4],
    bytes: &'a [u8],
}
impl<'a> Atom<'a> {
    fn data(self) -> &'a [u8] {
        &self.bytes[8..]
    }
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(|| bad("잘린 MP4 필드입니다."))?
            .try_into()
            .unwrap(),
    ))
}
fn u64_at(bytes: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(
        bytes
            .get(at..at + 8)
            .ok_or_else(|| bad("잘린 MP4 시각입니다."))?
            .try_into()
            .unwrap(),
    ))
}
fn put32(bytes: &mut [u8], at: usize, n: u32) -> Result<()> {
    bytes
        .get_mut(at..at + 4)
        .ok_or_else(|| bad("잘못된 MP4 필드 위치입니다."))?
        .copy_from_slice(&n.to_be_bytes());
    Ok(())
}
fn atoms(bytes: &[u8]) -> Result<Vec<Atom<'_>>> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if result.len() >= 1024 {
            return Err(bound());
        }
        let size = u32_at(bytes, at)? as usize;
        if !(8..=MAX_BOX).contains(&size)
            || at.checked_add(size).is_none_or(|end| end > bytes.len())
        {
            return Err(bad("MP4 박스 크기를 지원하지 않습니다."));
        }
        let kind = bytes[at + 4..at + 8].try_into().unwrap();
        result.push(Atom {
            kind,
            bytes: &bytes[at..at + size],
        });
        at += size;
    }
    Ok(result)
}
fn one<'a>(items: &[Atom<'a>], kind: &[u8; 4]) -> Result<Atom<'a>> {
    let mut found = items.iter().filter(|item| &item.kind == kind);
    let result = *found
        .next()
        .ok_or_else(|| bad("필수 MP4 박스가 없습니다."))?;
    if found.next().is_some() {
        return Err(bad("중복 MP4 박스입니다."));
    }
    Ok(result)
}
fn allowed(items: &[Atom<'_>], kinds: &[&[u8; 4]]) -> Result<()> {
    if items.iter().any(|item| !kinds.contains(&&item.kind)) {
        return Err(bad("지원하지 않는 MP4 박스 또는 암호화 정보가 있습니다."));
    }
    Ok(())
}
fn boxed(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(data.len() + 8);
    bytes.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    bytes
}
fn container(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
    boxed(kind, &children.concat())
}
fn full_flags(atom: Atom<'_>) -> Result<u32> {
    Ok(u32_at(atom.data(), 0)? & 0x00ff_ffff)
}

fn validate_avcc(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 7 || bytes[0] != 1 || bytes[4] & 3 != 3 {
        return Err(bad("4바이트 AVC NAL 크기와 avcC 초기화가 필요합니다."));
    }
    let mut at = 6;
    let sps = bytes[5] & 31;
    if sps == 0 || sps > 8 {
        return Err(bad("AVC SPS 수가 올바르지 않습니다."));
    }
    for _ in 0..sps {
        let size = u16::from_be_bytes(
            bytes
                .get(at..at + 2)
                .ok_or_else(|| bad("잘린 AVC SPS입니다."))?
                .try_into()
                .unwrap(),
        ) as usize;
        at += 2;
        if size == 0 || size > 65535 || bytes.get(at..at + size).is_none() {
            return Err(bad("AVC SPS 크기가 올바르지 않습니다."));
        }
        at += size;
    }
    let pps = *bytes.get(at).ok_or_else(|| bad("AVC PPS가 없습니다."))?;
    at += 1;
    if pps == 0 || pps > 16 {
        return Err(bad("AVC PPS 수가 올바르지 않습니다."));
    }
    for _ in 0..pps {
        let size = u16::from_be_bytes(
            bytes
                .get(at..at + 2)
                .ok_or_else(|| bad("잘린 AVC PPS입니다."))?
                .try_into()
                .unwrap(),
        ) as usize;
        at += 2;
        if size == 0 || bytes.get(at..at + size).is_none() {
            return Err(bad("AVC PPS 크기가 올바르지 않습니다."));
        }
        at += size;
    }
    // High-profile extension fields are allowed; the elementary stream remains
    // AVC and neither these fields nor SPS/PPS are treated as executable data.
    if bytes.len() - at > 65536 {
        return Err(bound());
    }
    Ok(())
}
fn descriptor(bytes: &[u8], at: &mut usize) -> Result<(u8, Vec<u8>)> {
    let tag = *bytes
        .get(*at)
        .ok_or_else(|| bad("잘린 AAC descriptor입니다."))?;
    *at += 1;
    let mut length = 0usize;
    let mut done = false;
    for _ in 0..4 {
        let b = *bytes
            .get(*at)
            .ok_or_else(|| bad("잘린 AAC descriptor 길이입니다."))?;
        *at += 1;
        length = (length << 7) | usize::from(b & 127);
        if b & 128 == 0 {
            done = true;
            break;
        }
    }
    if !done || length > 4096 {
        return Err(bound());
    }
    let value = bytes
        .get(*at..*at + length)
        .ok_or_else(|| bad("잘린 AAC descriptor 내용입니다."))?
        .to_vec();
    *at += length;
    Ok((tag, value))
}
fn validate_esds(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 5 || u32_at(bytes, 0)? != 0 {
        return Err(bad("지원하지 않는 esds 버전입니다."));
    }
    let (tag, es) = descriptor(bytes, &mut 4)?;
    if tag != 3 || es.len() < 3 || es[2] & 0xe0 != 0 {
        return Err(bad("외부 AAC descriptor는 지원하지 않습니다."));
    }
    let (tag, decoder) = descriptor(&es, &mut 3)?;
    if tag != 4 || decoder.len() < 13 || decoder[0] != 0x40 || (decoder[1] >> 2) & 0x3f != 5 {
        return Err(bad("MPEG-4 AAC 오디오가 아닙니다."));
    }
    let (tag, asc) = descriptor(&decoder, &mut 13)?;
    if tag != 5 || asc.len() < 2 || asc.len() > 8 || asc[0] >> 3 != 2 {
        return Err(bad("초기 버전은 AAC-LC만 지원합니다."));
    }
    let frequency = ((asc[0] & 7) << 1) | (asc[1] >> 7);
    let channels = (asc[1] >> 3) & 15;
    if frequency > 12 || !(1..=2).contains(&channels) {
        return Err(bad("지원하지 않는 AAC 샘플레이트 또는 채널입니다."));
    }
    Ok(())
}

fn parse_track(input: u32, trak: Atom<'_>, trex: Atom<'_>) -> Result<Track> {
    let children = atoms(trak.data())?;
    allowed(&children, &[b"tkhd", b"mdia"])?;
    let tkhd = one(&children, b"tkhd")?;
    let version = *tkhd
        .data()
        .first()
        .ok_or_else(|| bad("tkhd 버전이 없습니다."))?;
    if version > 1 {
        return Err(bad("지원하지 않는 tkhd 버전입니다."));
    }
    let original_id = u32_at(tkhd.data(), if version == 0 { 12 } else { 20 })?;
    if original_id == 0
        || u32_at(trex.data(), 4)? != original_id
        || trex.data().len() != 24
        || u32_at(trex.data(), 0)? != 0
        || u32_at(trex.data(), 8)? != 1
    {
        return Err(bad(
            "MP4 트랙 ID 또는 sample description이 올바르지 않습니다.",
        ));
    }
    let mdia = atoms(one(&children, b"mdia")?.data())?;
    allowed(&mdia, &[b"mdhd", b"hdlr", b"minf"])?;
    let mdhd = one(&mdia, b"mdhd")?;
    let mdversion = *mdhd
        .data()
        .first()
        .ok_or_else(|| bad("mdhd 버전이 없습니다."))?;
    if mdversion > 1 {
        return Err(bad("지원하지 않는 mdhd 버전입니다."));
    }
    let scale = u32_at(mdhd.data(), if mdversion == 0 { 12 } else { 20 })?;
    if scale == 0 || scale > 1_000_000_000 {
        return Err(bad("MP4 트랙 시간 단위가 올바르지 않습니다."));
    }
    let hdlr = one(&mdia, b"hdlr")?;
    let kind = match hdlr.data().get(8..12) {
        Some(b"vide") => Kind::Video,
        Some(b"soun") => Kind::Audio,
        _ => return Err(bad("영상·음성 외 트랙은 지원하지 않습니다.")),
    };
    let minf = atoms(one(&mdia, b"minf")?.data())?;
    allowed(&minf, &[b"vmhd", b"smhd", b"dinf", b"stbl"])?;
    let dinf = atoms(one(&minf, b"dinf")?.data())?;
    allowed(&dinf, &[b"dref"])?;
    let dref = one(&dinf, b"dref")?;
    if u32_at(dref.data(), 0)? != 0 || u32_at(dref.data(), 4)? != 1 {
        return Err(bad("외부 MP4 데이터 참조를 지원하지 않습니다."));
    }
    let refs = atoms(
        dref.data()
            .get(8..)
            .ok_or_else(|| bad("잘린 dref입니다."))?,
    )?;
    if refs.len() != 1 || refs[0].kind != *b"url " || refs[0].data() != [0, 0, 0, 1] {
        return Err(bad("외부 MP4 데이터 참조를 지원하지 않습니다."));
    }
    let stbl = atoms(one(&minf, b"stbl")?.data())?;
    allowed(
        &stbl,
        &[b"stsd", b"stts", b"stsc", b"stsz", b"stco", b"stss"],
    )?;
    // CHZZK's muxed AVC/AAC init includes an empty sync-sample table.
    // Fragment RAP flags remain authoritative; a populated/duplicate table
    // belongs to a different layout and must not be silently ignored.
    if stbl.iter().any(|a| a.kind == *b"stss") {
        let sync = one(&stbl, b"stss")?;
        if sync.data().len() != 8 || sync.data().iter().any(|b| *b != 0) {
            return Err(bad(
                "비어 있지 않은 MP4 sync sample 표는 지원하지 않습니다.",
            ));
        }
    }
    for name in [b"stts", b"stsc", b"stco"] {
        let table = one(&stbl, name)?;
        if table.data().len() != 8 || u32_at(table.data(), 0)? != 0 || u32_at(table.data(), 4)? != 0
        {
            return Err(bad("비어 있지 않은 일반 MP4 표는 지원하지 않습니다."));
        }
    }
    let stsz = one(&stbl, b"stsz")?;
    if stsz.data().len() != 12 || stsz.data().iter().any(|b| *b != 0) {
        return Err(bad("fragmented MP4 sample 표가 아닙니다."));
    }
    let stsd = one(&stbl, b"stsd")?;
    if u32_at(stsd.data(), 0)? != 0 || u32_at(stsd.data(), 4)? != 1 {
        return Err(bad("트랙별 하나의 codec description이 필요합니다."));
    }
    let descriptions = atoms(
        stsd.data()
            .get(8..)
            .ok_or_else(|| bad("잘린 stsd입니다."))?,
    )?;
    if descriptions.len() != 1 {
        return Err(bad("하나의 codec description이 필요합니다."));
    }
    let sample = descriptions[0];
    let offset = match kind {
        Kind::Video if sample.kind == *b"avc1" || sample.kind == *b"avc3" => 78,
        Kind::Audio if sample.kind == *b"mp4a" => 28,
        _ => return Err(bad("clear H.264/AAC 이외의 codec 또는 암호화 트랙입니다.")),
    };
    if sample.data().get(6..8) != Some(&[0, 1]) {
        return Err(bad("잘못된 sample 데이터 참조입니다."));
    }
    let extensions = atoms(
        sample
            .data()
            .get(offset..)
            .ok_or_else(|| bad("잘린 sample description입니다."))?,
    )?;
    match kind {
        Kind::Video => {
            allowed(&extensions, &[b"avcC", b"btrt", b"pasp", b"colr"])?;
            validate_avcc(one(&extensions, b"avcC")?.data())?;
        }
        Kind::Audio => {
            if sample
                .data()
                .get(8..16)
                .is_none_or(|v| v.iter().any(|b| *b != 0))
            {
                return Err(bad("QuickTime 오디오 확장은 지원하지 않습니다."));
            }
            allowed(&extensions, &[b"esds", b"btrt"])?;
            validate_esds(one(&extensions, b"esds")?.data())?;
        }
    }
    Ok(Track {
        input,
        original_id,
        id: if kind == Kind::Video { 1 } else { 2 },
        kind,
        scale,
        trak: trak.bytes.to_vec(),
        trex: trex.bytes.to_vec(),
        default_duration: u32_at(trex.data(), 12)?,
        default_size: u32_at(trex.data(), 16)?,
        default_flags: u32_at(trex.data(), 20)?,
        last_end: None,
        recent: VecDeque::new(),
    })
}

fn rewrite_trak(track: &Track) -> Result<Vec<u8>> {
    let original = atoms(&track.trak)?;
    let children = atoms(original[0].data())?;
    let mut output = Vec::new();
    for child in children {
        let mut bytes = child.bytes.to_vec();
        if child.kind == *b"tkhd" {
            let v = child.data()[0];
            put32(&mut bytes, 8 + if v == 0 { 12 } else { 20 }, track.id)?;
            let duration = 8 + if v == 0 { 20 } else { 28 };
            let size = if v == 0 { 4 } else { 8 };
            bytes
                .get_mut(duration..duration + size)
                .ok_or_else(|| bad("잘린 tkhd duration입니다."))?
                .fill(0);
        } else if child.kind == *b"mdia" {
            let mut mdia = Vec::new();
            for item in atoms(child.data())? {
                let mut value = item.bytes.to_vec();
                if item.kind == *b"mdhd" {
                    let v = item.data()[0];
                    let at = 8 + if v == 0 { 16 } else { 24 };
                    let n = if v == 0 { 4 } else { 8 };
                    value
                        .get_mut(at..at + n)
                        .ok_or_else(|| bad("잘린 mdhd duration입니다."))?
                        .fill(0);
                }
                mdia.push(value);
            }
            bytes = container(b"mdia", &mdia);
        }
        output.push(bytes);
    }
    Ok(container(b"trak", &output))
}

impl EncodedMuxer {
    pub fn new(inputs: Vec<EncodedTrackInput>) -> Result<Self> {
        if inputs.is_empty() || inputs.len() > 2 {
            return Err(bad("최대 두 개의 영상·음성 입력만 지원합니다."));
        }
        let mut tracks = Vec::new();
        let mut readers = Vec::new();
        let mut movie_header = None;
        for input in inputs {
            if input.init.len() > MAX_INIT
                || readers.iter().any(|p: &Input| p.id == input.track_index)
                || input.track_index > 1
            {
                return Err(bound());
            }
            let mime = input.mime_type.to_ascii_lowercase().replace([' ', '"'], "");
            if !(mime.starts_with("video/mp4;codecs=") || mime.starts_with("audio/mp4;codecs="))
                || mime.len() > 120
            {
                return Err(bad("fMP4 MIME과 명시적인 codec이 필요합니다."));
            }
            let top = atoms(&input.init)?;
            allowed(&top, &[b"ftyp", b"moov"])?;
            if top.len() != 2 || top[0].kind != *b"ftyp" || top[1].kind != *b"moov" {
                return Err(bad("완전한 ftyp+moov 초기화가 필요합니다."));
            }
            let ftyp = one(&top, b"ftyp")?;
            if ftyp.data().len() < 8 || ftyp.data().len() % 4 != 0 {
                return Err(bad("잘못된 ftyp입니다."));
            }
            let children = atoms(one(&top, b"moov")?.data())?;
            allowed(&children, &[b"mvhd", b"trak", b"mvex"])?;
            let mvhd = one(&children, b"mvhd")?;
            if mvhd.data().len() < 100 || mvhd.data()[0] > 1 {
                return Err(bad("지원하지 않는 mvhd입니다."));
            }
            if movie_header.is_none() {
                movie_header = Some(mvhd.bytes.to_vec());
            }
            let mvex = atoms(one(&children, b"mvex")?.data())?;
            allowed(&mvex, &[b"trex"])?;
            for trak in children.iter().filter(|p| p.kind == *b"trak") {
                let subs = atoms(trak.data())?;
                let tkhd = one(&subs, b"tkhd")?;
                let id = u32_at(
                    tkhd.data(),
                    if tkhd.data().first() == Some(&0) {
                        12
                    } else {
                        20
                    },
                )?;
                let matching: Vec<_> = mvex
                    .iter()
                    .filter(|trex| u32_at(trex.data(), 4).ok() == Some(id))
                    .collect();
                if matching.len() != 1 {
                    return Err(bad("트랙과 trex가 일치하지 않습니다."));
                }
                let track = parse_track(input.track_index, *trak, *matching[0])?;
                if tracks.iter().any(|t: &Track| {
                    t.kind == track.kind
                        || (t.input == track.input && t.original_id == track.original_id)
                }) {
                    return Err(bad("영상과 음성은 각각 한 트랙이어야 합니다."));
                }
                tracks.push(track);
            }
            if tracks.len() > 2 {
                return Err(bound());
            }
            readers.push(Input {
                id: input.track_index,
                pending: Vec::new(),
                moof: None,
                init: input.init,
            });
        }
        if tracks.len() != 2
            || !tracks.iter().any(|t| t.kind == Kind::Video)
            || !tracks.iter().any(|t| t.kind == Kind::Audio)
        {
            return Err(bad("확인된 H.264 영상과 AAC 음성이 모두 필요합니다."));
        }
        tracks.sort_by_key(|t| t.id);
        let mut mvhd = movie_header.ok_or_else(|| bad("mvhd가 없습니다."))?;
        let v = mvhd[8];
        let at = 8 + if v == 0 { 16 } else { 24 };
        let n = if v == 0 { 4 } else { 8 };
        mvhd.get_mut(at..at + n)
            .ok_or_else(|| bad("잘린 mvhd입니다."))?
            .fill(0);
        let end = mvhd.len();
        put32(&mut mvhd, end - 4, 3)?;
        let mut children = vec![mvhd];
        let mut defaults = Vec::new();
        for track in &tracks {
            children.push(rewrite_trak(track)?);
            let mut trex = track.trex.clone();
            put32(&mut trex, 12, track.id)?;
            defaults.push(trex);
        }
        children.push(container(b"mvex", &defaults));
        let mut init = boxed(b"ftyp", b"isom\0\0\x02\0isomiso6mp41avc1");
        init.extend(container(b"moov", &children));
        Ok(Self {
            inputs: readers,
            tracks,
            init,
            video: VecDeque::new(),
            audio: VecDeque::new(),
            started: false,
            sequence: 0,
            failed: false,
        })
    }

    pub fn push(&mut self, track_index: u32, bytes: &[u8]) -> Result<Vec<EncodedSegment>> {
        if self.failed {
            return Err(bad("이미 중단된 압축 녹화입니다."));
        }
        let result = self.push_inner(track_index, bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn push_inner(&mut self, track_index: u32, bytes: &[u8]) -> Result<Vec<EncodedSegment>> {
        if bytes.is_empty() || bytes.len() > MAX_BOX {
            return Err(bound());
        }
        if self.pending_bytes().saturating_add(bytes.len()) > MAX_PENDING {
            return Err(bound());
        }
        let index = self
            .inputs
            .iter()
            .position(|p| p.id == track_index)
            .ok_or_else(|| bad("다른 입력 트랙입니다."))?;
        self.inputs[index].pending.extend_from_slice(bytes);
        let mut segments = Vec::new();
        loop {
            let pending = &self.inputs[index].pending;
            if pending.len() < 8 {
                break;
            }
            let size = u32_at(pending, 0)? as usize;
            if !(8..=MAX_BOX).contains(&size) {
                return Err(bad("지원하지 않는 fragment 크기입니다."));
            }
            if pending.len() < size {
                break;
            }
            let atom: Vec<_> = self.inputs[index].pending.drain(..size).collect();
            let kind = &atom[4..8];
            match kind {
                b"moof" => {
                    if self.inputs[index].moof.is_some() {
                        return Err(bad("mdat가 없는 moof입니다."));
                    }
                    self.inputs[index].moof = Some(atom);
                }
                b"mdat" => {
                    let moof = self.inputs[index]
                        .moof
                        .take()
                        .ok_or_else(|| bad("moof가 없는 mdat입니다."))?;
                    self.accept_fragment(track_index, &moof, &atom)?;
                    segments.extend(self.flush(false)?);
                }
                b"styp" | b"sidx" | b"emsg" | b"prft" | b"free" => {
                    if self.inputs[index].moof.is_some() {
                        return Err(bad("moof와 mdat 사이의 박스는 지원하지 않습니다."));
                    }
                }
                // Repeated initialization is not silently accepted: a codec or
                // profile switch needs a new recording epoch and authorization.
                _ => return Err(bad("초기화 변경 또는 지원하지 않는 fragment 박스입니다.")),
            }
            if self.pending_bytes() > MAX_PENDING
                || self.video.len() + self.audio.len() > MAX_SAMPLES
            {
                return Err(bound());
            }
        }
        Ok(segments)
    }
    fn pending_bytes(&self) -> usize {
        self.inputs
            .iter()
            .map(|p| p.pending.len() + p.moof.as_ref().map_or(0, Vec::len) + p.init.len())
            .sum::<usize>()
            + self
                .video
                .iter()
                .chain(self.audio.iter())
                .map(|s| s.data.len() + 40)
                .sum::<usize>()
    }
    fn accept_fragment(&mut self, input: u32, moof: &[u8], mdat: &[u8]) -> Result<()> {
        let outer = atoms(moof)?;
        let children = atoms(outer[0].data())?;
        allowed(&children, &[b"mfhd", b"traf"])?;
        let mfhd = one(&children, b"mfhd")?;
        if mfhd.data().len() != 8 || u32_at(mfhd.data(), 0)? != 0 {
            return Err(bad("지원하지 않는 mfhd입니다."));
        }
        let trafs: Vec<_> = children.iter().filter(|p| p.kind == *b"traf").collect();
        if trafs.is_empty() || trafs.len() > 2 {
            return Err(bad("지원하지 않는 fragment 트랙 수입니다."));
        }
        let mut used = Vec::new();
        let mut occupied = Vec::new();
        for traf in &trafs {
            let fields = atoms(traf.data())?;
            allowed(&fields, &[b"tfhd", b"tfdt", b"trun", b"sdtp"])?;
            let tfhd = one(&fields, b"tfhd")?;
            let flags = full_flags(tfhd)?;
            if tfhd.data()[0] != 0
                || flags & !0x020038 != 0
                || (trafs.len() > 1 && flags & 0x020000 == 0)
            {
                return Err(bad("절대 offset 또는 지원하지 않는 tfhd입니다."));
            }
            let id = u32_at(tfhd.data(), 4)?;
            let ti = self
                .tracks
                .iter()
                .position(|t| t.input == input && t.original_id == id)
                .ok_or_else(|| bad("알 수 없는 fragment 트랙입니다."))?;
            if used.contains(&ti) {
                return Err(bad("fragment 트랙이 중복되었습니다."));
            }
            used.push(ti);
            let track = &self.tracks[ti];
            let mut at = 8;
            let duration = if flags & 8 != 0 {
                let v = u32_at(tfhd.data(), at)?;
                at += 4;
                v
            } else {
                track.default_duration
            };
            let size = if flags & 16 != 0 {
                let v = u32_at(tfhd.data(), at)?;
                at += 4;
                v
            } else {
                track.default_size
            };
            let sample_flags = if flags & 32 != 0 {
                let v = u32_at(tfhd.data(), at)?;
                at += 4;
                v
            } else {
                track.default_flags
            };
            if at != tfhd.data().len() {
                return Err(bad("잘못된 tfhd 크기입니다."));
            }
            let tfdt = one(&fields, b"tfdt")?;
            if full_flags(tfdt)? != 0 {
                return Err(bad("지원하지 않는 tfdt flags입니다."));
            }
            let start = match tfdt.data().first() {
                Some(0) if tfdt.data().len() == 8 => u64::from(u32_at(tfdt.data(), 4)?),
                Some(1) if tfdt.data().len() == 12 => u64_at(tfdt.data(), 4)?,
                _ => return Err(bad("지원하지 않는 tfdt입니다.")),
            };
            if start > 9_007_199_254_740_991 {
                return Err(bound());
            }
            let run = one(&fields, b"trun")?;
            let rf = full_flags(run)?;
            if run.data()[0] > 1
                || rf & !0x000f05 != 0
                || rf & 1 == 0
                || (rf & 4 != 0 && rf & 0x400 != 0)
            {
                return Err(bad("지원하지 않는 trun flags입니다."));
            }
            let count = u32_at(run.data(), 4)? as usize;
            if count == 0 || count > MAX_SAMPLES {
                return Err(bound());
            }
            let data_at = i32::from_be_bytes(u32_at(run.data(), 8)?.to_be_bytes());
            if data_at < 0 || (data_at as usize) < moof.len() + 8 {
                return Err(bad("fragment data offset이 잘못되었습니다."));
            }
            let mut payload_at = data_at as usize - moof.len();
            let payload_start = payload_at;
            let mut at = 12;
            let first_flags = if rf & 4 != 0 {
                let v = u32_at(run.data(), at)?;
                at += 4;
                Some(v)
            } else {
                None
            };
            let mut dts = start;
            let mut samples = Vec::with_capacity(count);
            for i in 0..count {
                let d = if rf & 0x100 != 0 {
                    let v = u32_at(run.data(), at)?;
                    at += 4;
                    v
                } else {
                    duration
                };
                let n = if rf & 0x200 != 0 {
                    let v = u32_at(run.data(), at)?;
                    at += 4;
                    v
                } else {
                    size
                };
                let f = if rf & 0x400 != 0 {
                    let v = u32_at(run.data(), at)?;
                    at += 4;
                    v
                } else {
                    if i == 0 {
                        first_flags.unwrap_or(sample_flags)
                    } else {
                        sample_flags
                    }
                };
                let cts = if rf & 0x800 != 0 {
                    let v = u32_at(run.data(), at)?;
                    at += 4;
                    if run.data()[0] == 0 && v > i32::MAX as u32 {
                        return Err(bound());
                    }
                    v as i32
                } else {
                    0
                };
                if d == 0
                    || n == 0
                    || n as usize > MAX_BOX
                    || u64::from(d) > u64::from(track.scale) * 10
                    || cts.unsigned_abs() as u64 > u64::from(track.scale) * 10
                {
                    return Err(bad("잘못된 sample 길이 또는 시각입니다."));
                }
                let data = mdat
                    .get(payload_at..payload_at + n as usize)
                    .ok_or_else(|| bad("mdat에 sample이 모두 들어 있지 않습니다."))?
                    .to_vec();
                payload_at += n as usize;
                if track.kind == Kind::Video {
                    validate_nals(&data, f)?;
                } else if cts != 0 {
                    return Err(bad("AAC composition offset은 지원하지 않습니다."));
                }
                samples.push(Sample {
                    dts,
                    duration: d,
                    flags: f,
                    cts,
                    data,
                });
                dts = dts.checked_add(u64::from(d)).ok_or_else(bound)?;
            }
            if at != run.data().len()
                || occupied
                    .iter()
                    .any(|(a, b)| payload_start < *b && payload_at > *a)
            {
                return Err(bad("겹치거나 잘못된 sample 범위입니다."));
            }
            occupied.push((payload_start, payload_at));
            let hash: [u8; 32] = {
                use sha2::{Digest, Sha256};
                let mut hash = Sha256::new();
                hash.update(traf.bytes);
                hash.update(&mdat[payload_start..payload_at]);
                hash.finalize().into()
            };
            let track = &mut self.tracks[ti];
            if let Some(last) = track.last_end {
                // Timestamp quantization differs between the standard (48 kHz
                // AAC +/-1 tick) and Grid (6 kHz video +/-4 ticks) encoders.
                // Bound correction by BOTH 1 ms and 1/16 of one sample; even
                // an unusually short actual frame must never be discarded.
                let rounding =
                    u64::from(track.scale / 1000).min(u64::from(samples[0].duration / 16));
                if start < last {
                    if track
                        .recent
                        .iter()
                        .any(|(a, b, h)| *a == start && *b == dts && *h == hash)
                    {
                        continue;
                    }
                    if last - start > rounding {
                        return Err(StreamError::new("ENCODED_UNSUPPORTED", &format!("과거·중복 sample을 안전하게 구분하지 못했습니다. (트랙 {}, 시작 {}, 이전 끝 {}, 시간 단위 {})", track.id, start, last, track.scale), false));
                    }
                }
                if start != last {
                    if start.abs_diff(last) > rounding {
                        return Err(StreamError::new("ENCODED_UNSUPPORTED", &format!("수신 영상 시간축에 누락 또는 불연속이 있습니다. (트랙 {}, 시작 {}, 이전 끝 {}, 시간 단위 {})", track.id, start, last, track.scale), false));
                    }
                    // Adjust only the first container
                    // duration/start, preserving its end, all following source
                    // timestamps and every encoded byte. Never accumulate a
                    // clock shift or accept an actual missing/duplicate frame.
                    let first = samples.first_mut().ok_or_else(bound)?;
                    let duration = i64::from(first.duration) + start as i64 - last as i64;
                    first.duration = u32::try_from(duration)
                        .ok()
                        .filter(|d| *d > 0)
                        .ok_or_else(bound)?;
                    first.dts = last;
                }
            }
            track.last_end = Some(dts);
            track.recent.push_back((start, dts, hash));
            if track.recent.len() > 128 {
                track.recent.pop_front();
            }
            if track.kind == Kind::Video {
                self.video.extend(samples);
            } else {
                self.audio.extend(samples);
            }
        }
        occupied.sort();
        if occupied.first().is_none_or(|r| r.0 != 8)
            || occupied.last().is_none_or(|r| r.1 != mdat.len())
            || occupied.windows(2).any(|p| p[0].1 != p[1].0)
        {
            return Err(bad("mdat에 알 수 없는 데이터가 있습니다."));
        }
        Ok(())
    }

    fn flush(&mut self, finishing: bool) -> Result<Vec<EncodedSegment>> {
        let vs = self.tracks[0].scale;
        let aus = self.tracks[1].scale;
        while !self.started {
            while self.video.front().is_some_and(|s| !s.key()) {
                self.video.pop_front();
            }
            if self.video.is_empty() {
                return Ok(Vec::new());
            }
            let start = self.video[0].dts;
            // Audio must already cover the first selected video RAP. Otherwise
            // wait, or skip this RAP when the stream starts much later.
            while self
                .audio
                .front()
                .is_some_and(|s| before(s.dts, aus, start, vs))
            {
                self.audio.pop_front();
            }
            let Some(audio) = self.audio.front() else {
                return Ok(Vec::new());
            };
            if (audio.dts as f64 / aus as f64 - start as f64 / vs as f64) > 0.1 {
                self.video.pop_front();
                continue;
            }
            self.started = true;
        }
        let mut output = Vec::new();
        while let Some(first) = self.video.front() {
            let start = first.dts;
            let boundary = self
                .video
                .iter()
                .skip(1)
                .find(|s| s.key() && (s.dts - start) as f64 / vs as f64 >= TARGET_SECONDS)
                .map(|s| s.dts);
            let end = if let Some(boundary) = boundary {
                boundary
            } else if finishing {
                self.video.back().map(Sample::end).unwrap_or(start)
            } else {
                break;
            };
            if end <= start {
                break;
            }
            if !self
                .audio
                .back()
                .is_some_and(|s| !before(s.end(), aus, end, vs))
            {
                if finishing {
                    // A/V packet boundaries need not end at the same instant.
                    // At the final cut ONLY, retain complete received packets
                    // when the audio tail is less than one packet/frame short
                    // (and under 50 ms). Never synthesize audio or discard video.
                    let gap = self
                        .audio
                        .back()
                        .map(|s| end as f64 / vs as f64 - s.end() as f64 / aus as f64);
                    let cadence = self.audio.back().zip(self.video.back()).map(|(a, v)| {
                        (a.duration as f64 / aus as f64)
                            .max(v.duration as f64 / vs as f64)
                            .min(0.05)
                    });
                    let aligned_tail = boundary.is_none()
                        && gap
                            .zip(cadence)
                            .is_some_and(|(gap, cadence)| gap > 0.0 && gap < cadence);
                    if !aligned_tail {
                        return Err(StreamError::new("ENCODED_UNSUPPORTED", &format!("마지막 영상 구간의 음성이 완전하지 않습니다. (음성 끝 차이 {:?}초)", gap), false));
                    }
                } else {
                    break;
                }
            }
            let mut video = Vec::new();
            while self.video.front().is_some_and(|s| s.dts < end) {
                video.push(self.video.pop_front().unwrap());
            }
            let mut audio = Vec::new();
            while self
                .audio
                .front()
                .is_some_and(|s| before(s.dts, aus, end, vs))
            {
                let sample = self.audio.pop_front().unwrap();
                if !before(sample.dts, aus, start, vs) {
                    audio.push(sample);
                }
            }
            if video.is_empty() || audio.is_empty() || !video[0].key() {
                return Err(bad("독립 재생 가능한 영상·음성 시작점을 찾지 못했습니다."));
            }
            self.sequence = self.sequence.checked_add(1).ok_or_else(bound)?;
            let mut bytes = self.init.clone();
            let sequence = self.sequence.checked_mul(2).ok_or_else(bound)?;
            bytes.extend(make_fragment(1, &video, start, sequence - 1)?);
            let audio_base = ((u128::from(start) * u128::from(aus)) / u128::from(vs)) as u64;
            bytes.extend(make_fragment(2, &audio, audio_base, sequence)?);
            if bytes.len() > MAX_PENDING {
                return Err(bound());
            }
            let actual_end = (video.last().unwrap().end() as f64 / vs as f64)
                .max(audio.last().unwrap().end() as f64 / aus as f64);
            output.push(EncodedSegment {
                bytes,
                duration_seconds: actual_end - start as f64 / vs as f64,
                source_start_seconds: start as f64 / vs as f64,
                source_end_seconds: actual_end,
            });
            if output.len() > 16 {
                return Err(bound());
            }
        }
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<Vec<EncodedSegment>> {
        if self.failed {
            return Err(bad("중단된 압축 녹화의 미완료 구간입니다."));
        }
        if self
            .inputs
            .iter()
            .any(|p| !p.pending.is_empty() || p.moof.is_some())
        {
            return Err(bad("마지막 MP4 fragment가 불완전합니다."));
        }
        let result = self.flush(true);
        self.failed = true;
        if result.as_ref().is_ok_and(|segments| segments.is_empty()) && !self.started {
            return Err(bad("완전한 영상·음성 시작 구간을 받지 못했습니다."));
        }
        result
    }
}

fn before(a: u64, a_scale: u32, b: u64, b_scale: u32) -> bool {
    u128::from(a) * u128::from(b_scale) < u128::from(b) * u128::from(a_scale)
}
fn validate_nals(bytes: &[u8], flags: u32) -> Result<()> {
    let mut at = 0;
    let mut idr = false;
    let mut count = 0;
    while at < bytes.len() {
        let n = u32_at(bytes, at)? as usize;
        at += 4;
        count += 1;
        if count > 4096 || n == 0 || at.checked_add(n).is_none_or(|end| end > bytes.len()) {
            return Err(bad("잘못된 AVC NAL 경계입니다."));
        }
        let kind = bytes[at] & 31;
        if kind == 5 {
            idr = true;
        }
        if kind == 0 || kind >= 24 {
            return Err(bad("지원하지 않는 AVC NAL입니다."));
        }
        at += n;
    }
    if flags & 0x0001_0000 == 0 && (flags >> 24) & 3 == 2 && !idr {
        return Err(bad("키프레임 표시와 AVC IDR이 일치하지 않습니다."));
    }
    Ok(())
}
fn make_fragment(id: u32, samples: &[Sample], base: u64, sequence: u32) -> Result<Vec<u8>> {
    let mut header = vec![0, 0, 0, 0];
    header.extend(sequence.to_be_bytes());
    let mfhd = boxed(b"mfhd", &header);
    let mut th = vec![0, 2, 0, 0];
    th.extend(id.to_be_bytes());
    let tfhd = boxed(b"tfhd", &th);
    let mut td = vec![1, 0, 0, 0];
    td.extend(
        samples[0]
            .dts
            .checked_sub(base)
            .ok_or_else(|| bad("음성·영상 원점이 잘못되었습니다."))?
            .to_be_bytes(),
    );
    let tfdt = boxed(b"tfdt", &td);
    let mut tr = vec![1, 0, 15, 1];
    tr.extend((samples.len() as u32).to_be_bytes());
    tr.extend(0u32.to_be_bytes());
    let mut payload = Vec::new();
    for sample in samples {
        if i128::from(sample.dts) + i128::from(sample.cts) < i128::from(base) {
            return Err(bad("키프레임 앞의 표시 시각은 지원하지 않습니다."));
        }
        tr.extend(sample.duration.to_be_bytes());
        tr.extend((sample.data.len() as u32).to_be_bytes());
        tr.extend(sample.flags.to_be_bytes());
        tr.extend(sample.cts.to_be_bytes());
        payload.extend_from_slice(&sample.data);
    }
    let trun = boxed(b"trun", &tr);
    let traf = container(b"traf", &[tfhd.clone(), tfdt.clone(), trun]);
    let moof_size = 8 + mfhd.len() + traf.len();
    put32(&mut tr, 8, (moof_size + 8) as u32)?;
    let mut moof = container(
        b"moof",
        &[mfhd, container(b"traf", &[tfhd, tfdt, boxed(b"trun", &tr)])],
    );
    moof.extend(boxed(b"mdat", &payload));
    Ok(moof)
}

/// Structurally valid synthetic fixtures, not a claim of decodable H.264/AAC.
/// Runtime decode probes must additionally feed actual synthetic encoder output.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    pub const VIDEO_SCALE: u32 = 90_000;
    pub const AUDIO_SCALE: u32 = 48_000;
    fn init(index: u32) -> EncodedTrackInput {
        let video = index == 0;
        let id = 7;
        let mut mvhd = vec![0; 100];
        put32(&mut mvhd, 12, 1000).unwrap();
        put32(&mut mvhd, 96, 8).unwrap();
        let mut tkhd = vec![0; 84];
        tkhd[3] = 7;
        put32(&mut tkhd, 12, id).unwrap();
        let mut mdhd = vec![0; 24];
        put32(&mut mdhd, 12, if video { VIDEO_SCALE } else { AUDIO_SCALE }).unwrap();
        let mut hdlr = vec![0; 24];
        hdlr[8..12].copy_from_slice(if video { b"vide" } else { b"soun" });
        let mut sample = vec![0; if video { 78 } else { 28 }];
        sample[7] = 1;
        let sample = if video {
            sample.extend(boxed(
                b"avcC",
                &[
                    1, 66, 0, 30, 255, 225, 0, 4, 103, 66, 0, 30, 1, 0, 2, 104, 0,
                ],
            ));
            boxed(b"avc1", &sample)
        } else {
            let mut decoder = vec![0x40, 0x15];
            decoder.extend([0; 11]);
            decoder.extend([5, 2, 0x11, 0x90]);
            let mut es = vec![0, 7, 0, 4, decoder.len() as u8];
            es.extend(decoder);
            let mut esds = vec![0, 0, 0, 0, 3, es.len() as u8];
            esds.extend(es);
            sample.extend(boxed(b"esds", &esds));
            boxed(b"mp4a", &sample)
        };
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(sample);
        let stbl = container(
            b"stbl",
            &[
                boxed(b"stsd", &stsd),
                boxed(b"stts", &[0; 8]),
                boxed(b"stsc", &[0; 8]),
                boxed(b"stsz", &[0; 12]),
                boxed(b"stco", &[0; 8]),
            ],
        );
        let mut dref = vec![0, 0, 0, 0, 0, 0, 0, 1];
        dref.extend(boxed(b"url ", &[0, 0, 0, 1]));
        let minf = container(
            b"minf",
            &[
                boxed(if video { b"vmhd" } else { b"smhd" }, &[0; 8]),
                container(b"dinf", &[boxed(b"dref", &dref)]),
                stbl,
            ],
        );
        let trak = container(
            b"trak",
            &[
                boxed(b"tkhd", &tkhd),
                container(
                    b"mdia",
                    &[boxed(b"mdhd", &mdhd), boxed(b"hdlr", &hdlr), minf],
                ),
            ],
        );
        let mut trex = vec![0; 24];
        put32(&mut trex, 4, id).unwrap();
        put32(&mut trex, 8, 1).unwrap();
        let mut init = boxed(b"ftyp", b"isom\0\0\0\0isomiso6");
        init.extend(container(
            b"moov",
            &[
                boxed(b"mvhd", &mvhd),
                trak,
                container(b"mvex", &[boxed(b"trex", &trex)]),
            ],
        ));
        EncodedTrackInput {
            track_index: index,
            mime_type: if video {
                "video/mp4;codecs=avc1.42001e"
            } else {
                "audio/mp4;codecs=mp4a.40.2"
            }
            .into(),
            init,
        }
    }
    pub fn inputs() -> Vec<EncodedTrackInput> {
        vec![init(0), init(1)]
    }
    /// Each synthetic sample is 0.5 source seconds. Video sample zero can be RAP.
    pub fn fragment(track_index: u32, start: u64, count: u32, key_first: bool) -> Vec<u8> {
        let video = track_index == 0;
        let duration = if video {
            VIDEO_SCALE / 2
        } else {
            AUDIO_SCALE / 2
        };
        let samples: Vec<_> = (0..count)
            .map(|i| Sample {
                dts: start + u64::from(i * duration),
                duration,
                flags: if !video || (key_first && i == 0) {
                    0x02000000
                } else {
                    0x01010000
                },
                cts: 0,
                data: if video {
                    vec![
                        0,
                        0,
                        0,
                        2,
                        if key_first && i == 0 { 0x65 } else { 0x41 },
                        i as u8,
                    ]
                } else {
                    vec![0x21, i as u8]
                },
            })
            .collect();
        make_fragment(7, &samples, 0, 1).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_chzzk_muxed_init_accepts_empty_sync_table_only() {
        use base64::Engine;
        // Codec/container headers only; no content samples, URLs or account data.
        let init = base64::engine::general_purpose::STANDARD
            .decode(include_str!("browser_fmp4_chzzk_init.b64").trim())
            .unwrap();
        let input = EncodedTrackInput {
            track_index: 0,
            mime_type: "video/mp4;codecs=mp4a.40.2,avc1.4D001F".into(),
            init,
        };
        let muxer = EncodedMuxer::new(vec![input.clone()]).unwrap();
        assert_eq!(muxer.tracks.len(), 2);
        let mut invalid = input;
        let table = invalid.init.windows(4).position(|b| b == b"stss").unwrap();
        invalid.init[table + 11] = 1;
        assert!(EncodedMuxer::new(vec![invalid]).is_err());
    }
    #[test]
    fn actual_chzzk_grid_high_profile_header_is_accepted() {
        use base64::Engine;
        let init = base64::engine::general_purpose::STANDARD
            .decode(include_str!("browser_fmp4_chzzk_grid_init.b64").trim())
            .unwrap();
        let muxer = EncodedMuxer::new(vec![EncodedTrackInput {
            track_index: 0,
            mime_type: "video/mp4;codecs=mp4a.40.2,avc1.64002A".into(),
            init,
        }])
        .unwrap();
        assert_eq!(
            muxer.tracks.iter().map(|t| t.scale).collect::<Vec<_>>(),
            vec![6000, 48000]
        );
    }
    fn mux() -> EncodedMuxer {
        EncodedMuxer::new(fixtures::inputs()).unwrap()
    }
    #[test]
    fn tolerates_bounded_boundary_rounding_without_changing_payload_or_source_end() {
        for delta in [-1i64, 1] {
            let mut m = mux();
            m.push(0, &fixtures::fragment(0, 0, 2, true)).unwrap();
            m.push(1, &fixtures::fragment(1, 0, 2, true)).unwrap();
            let start = (48_000i64 + delta) as u64;
            m.push(1, &fixtures::fragment(1, start, 2, true)).unwrap();
            let samples: Vec<_> = m.audio.iter().collect();
            assert_eq!(samples[2].dts, 48_000);
            assert_eq!(samples[2].duration, (24_000i64 + delta) as u32);
            assert_eq!(samples[2].end(), start + 24_000);
            assert_eq!(samples[2].data, vec![0x21, 0]);
            assert_eq!(samples[3].dts, start + 24_000);
            assert_eq!(m.tracks[1].last_end, Some(start + 48_000));
        }
        for delta in [-49i64, 49] {
            let mut m = mux();
            m.push(1, &fixtures::fragment(1, 0, 2, true)).unwrap();
            assert!(m
                .push(
                    1,
                    &fixtures::fragment(1, (48_000i64 + delta) as u64, 2, true)
                )
                .is_err());
        }
    }
    #[test]
    fn grid_video_submillisecond_rounding_does_not_accumulate_or_hide_a_frame() {
        let mut inputs = fixtures::inputs();
        let scale = inputs[0]
            .init
            .windows(4)
            .position(|b| b == b"mdhd")
            .unwrap()
            + 16;
        put32(&mut inputs[0].init, scale, 6000).unwrap();
        let mut m = EncodedMuxer::new(inputs).unwrap();
        let frame = |dts| Sample {
            dts,
            duration: 100,
            flags: 0x02000000,
            cts: 0,
            data: vec![0, 0, 0, 2, 0x65, 1],
        };
        for start in [0, 96, 200, 299, 400] {
            m.push(0, &make_fragment(7, &[frame(start)], 0, 1).unwrap())
                .unwrap();
        }
        assert_eq!(m.tracks[0].last_end, Some(500));
        assert_eq!(m.video.iter().map(|s| s.duration).sum::<u32>(), 500);
        assert!(m.video.iter().all(|s| s.data == frame(0).data));
        assert!(m
            .push(0, &make_fragment(7, &[frame(600)], 0, 1).unwrap())
            .is_err());
    }
    #[test]
    fn final_av_packet_alignment_keeps_both_tracks_but_rejects_real_missing_audio() {
        for (short, accepted) in [(480u32, true), (4800, false)] {
            let mut m = mux();
            m.push(0, &fixtures::fragment(0, 0, 2, true)).unwrap();
            let mut audio = fixtures::fragment(1, 0, 2, true);
            let run = audio.windows(4).position(|b| b == b"trun").unwrap() + 4;
            put32(&mut audio, run + 12 + 16, 24000 - short).unwrap();
            m.push(1, &audio).unwrap();
            let result = m.finish();
            assert_eq!(result.is_ok(), accepted);
            if let Ok(segments) = result {
                assert_eq!(segments.len(), 1);
                assert_eq!(segments[0].duration_seconds, 1.0);
                assert!(segments[0].bytes.windows(2).any(|b| b == [0x21, 1]));
            }
        }
    }
    #[test]
    fn merges_two_track_inits_with_unique_ids() {
        let m = mux();
        assert_eq!(
            m.tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(m.tracks[0].scale, 90_000);
        assert_eq!(m.tracks[1].scale, 48_000);
        let root = atoms(&m.init).unwrap();
        let moov = atoms(one(&root, b"moov").unwrap().data()).unwrap();
        assert_eq!(moov.iter().filter(|a| a.kind == *b"trak").count(), 2);
    }
    #[test]
    fn reassembles_arbitrary_append_boundaries_and_finishes_actual_muxed_bytes() {
        let mut m = mux();
        for index in [0, 1] {
            let fragment = fixtures::fragment(index, 0, 2, true);
            for bytes in fragment.chunks(7) {
                assert!(m.push(index, bytes).unwrap().is_empty());
            }
        }
        let segments = m.finish().unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].duration_seconds, 1.0);
        let top = atoms(&segments[0].bytes).unwrap();
        assert_eq!(top.iter().filter(|a| a.kind == *b"moof").count(), 2);
        assert_eq!(top.iter().filter(|a| a.kind == *b"mdat").count(), 2);
    }
    #[test]
    fn source_clock_not_playback_speed_defines_archive_duration() {
        let mut m = mux();
        let origin = 4000;
        m.push(0, &fixtures::fragment(0, origin * 90000, 4, true))
            .unwrap();
        m.push(1, &fixtures::fragment(1, origin * 48000, 4, true))
            .unwrap();
        let segment = m.finish().unwrap().remove(0);
        assert_eq!(segment.source_start_seconds, 4000.0);
        assert_eq!(segment.source_end_seconds, 4002.0);
        assert_eq!(segment.duration_seconds, 2.0);
        let top = atoms(&segment.bytes).unwrap();
        for moof in top.iter().filter(|a| a.kind == *b"moof") {
            let c = atoms(moof.data()).unwrap();
            let traf = atoms(one(&c, b"traf").unwrap().data()).unwrap();
            assert_eq!(u64_at(one(&traf, b"tfdt").unwrap().data(), 4).unwrap(), 0);
        }
    }
    #[test]
    fn presentation_clock_mapping_preserves_positive_and_negative_b_frame_offsets() {
        let mut m = mux();
        let origin = 4000 * 90_000;
        let mut input = fixtures::fragment(0, origin, 4, true);
        let trun = input.windows(4).position(|bytes| bytes == b"trun").unwrap() + 4;
        let offsets = [1500_i32, -1500, 3000, -3000];
        for (index, offset) in offsets.iter().enumerate() {
            // Full-box/version+flags, count, data offset, then 16-byte entries.
            let at = trun + 12 + index * 16 + 12;
            input[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        }
        m.push(0, &input).unwrap();
        m.push(1, &fixtures::fragment(1, 4000 * 48_000, 4, true))
            .unwrap();
        let segment = m.finish().unwrap().remove(0);
        let top = atoms(&segment.bytes).unwrap();
        let moof = atoms(
            top.iter()
                .find(|atom| atom.kind == *b"moof")
                .unwrap()
                .data(),
        )
        .unwrap();
        let traf = atoms(one(&moof, b"traf").unwrap().data()).unwrap();
        let base = u64_at(one(&traf, b"tfdt").unwrap().data(), 4).unwrap();
        let output = one(&traf, b"trun").unwrap().data();
        for (index, source_cts) in offsets.iter().enumerate() {
            let output_cts = u32_at(output, 12 + index * 16 + 12).unwrap() as i32;
            let source_pts =
                (origin as f64 + index as f64 * 45_000.0 + f64::from(*source_cts)) / 90_000.0;
            let output_pts =
                (base as f64 + index as f64 * 45_000.0 + f64::from(output_cts)) / 90_000.0;
            assert_eq!(output_cts, *source_cts);
            assert!((source_pts - segment.source_start_seconds - output_pts).abs() < 1e-9);
        }
    }

    #[test]
    fn repeated_exact_fragment_is_deduplicated_but_changed_payload_is_rejected() {
        let mut m = mux();
        let fragment = fixtures::fragment(0, 0, 2, true);
        m.push(0, &fragment).unwrap();
        m.push(0, &fragment).unwrap();
        assert_eq!(m.video.len(), 2);
        let mut changed = fragment;
        *changed.last_mut().unwrap() ^= 1;
        assert!(m.push(0, &changed).is_err());
    }
    #[test]
    fn rejects_timestamp_gaps_and_incomplete_last_fragment() {
        let mut m = mux();
        m.push(0, &fixtures::fragment(0, 0, 2, true)).unwrap();
        assert!(m.push(0, &fixtures::fragment(0, 180000, 2, true)).is_err());
        let mut m = mux();
        m.push(0, &[0, 0, 0, 20, b'm', b'o', b'o', b'f']).unwrap();
        assert!(m.finish().is_err());
    }
    #[test]
    fn rejects_encryption_unsupported_codec_missing_audio_and_huge_boxes() {
        let mut inputs = fixtures::inputs();
        let at = inputs[0]
            .init
            .windows(4)
            .position(|s| s == b"avc1")
            .unwrap();
        inputs[0].init[at..at + 4].copy_from_slice(b"encv");
        assert!(EncodedMuxer::new(inputs).is_err());
        assert!(EncodedMuxer::new(vec![fixtures::inputs().remove(0)]).is_err());
        let mut m = mux();
        assert!(m
            .push(0, &[0x7f, 0xff, 0xff, 0xff, b'm', b'd', b'a', b't'])
            .is_err());
    }
    #[test]
    fn cuts_at_a_later_keyframe_and_keeps_every_audio_sample_once() {
        let mut m = mux();
        let mut segments = Vec::new();
        for second in 0..14 {
            segments.extend(
                m.push(0, &fixtures::fragment(0, second * 90000, 2, true))
                    .unwrap(),
            );
            segments.extend(
                m.push(1, &fixtures::fragment(1, second * 48000, 2, true))
                    .unwrap(),
            );
        }
        segments.extend(m.finish().unwrap());
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments.iter().map(|s| s.duration_seconds).sum::<f64>(),
            14.0
        );
        assert_eq!(
            segments[0].source_end_seconds,
            segments[1].source_start_seconds
        );
    }
    #[test]
    fn rejects_absolute_offsets_and_sample_ranges_outside_mdat() {
        let mut m = mux();
        let mut f = fixtures::fragment(0, 0, 2, true);
        let pos = f.windows(4).position(|s| s == b"tfhd").unwrap();
        f[pos + 7] |= 1;
        assert!(m.push(0, &f).is_err());
        let mut m = mux();
        let mut f = fixtures::fragment(0, 0, 2, true);
        let pos = f.windows(4).position(|s| s == b"trun").unwrap();
        f[pos + 12..pos + 16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(m.push(0, &f).is_err());
    }
    #[test]
    fn supports_hlsjs_single_traf_implicit_moof_relative_addressing() {
        let mut m = mux();
        for index in [0, 1] {
            let mut f = fixtures::fragment(index, 0, 2, true);
            let at = f.windows(4).position(|s| s == b"tfhd").unwrap();
            f[at + 5] = 0;
            m.push(index, &f).unwrap();
        }
        assert_eq!(m.finish().unwrap().len(), 1);
    }
}
