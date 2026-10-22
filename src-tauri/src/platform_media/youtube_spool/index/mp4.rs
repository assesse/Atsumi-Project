//! Read-only, deliberately narrow AAC-LC fragmented MP4 container facts.
//! No payload rewrite, decoding certificate, packet plan, or production caller.
use super::{
    error, AacReconstruction, Boundary, Clock, Configuration, Emit, Failure, Format,
    HeaderDuration, Input, ReconstructionFacts, Result, UnitDraft, MAX_ITEM, METADATA,
};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek};

const MAX_NODES: usize = 512;
const MAX_SAMPLES: u64 = 100_000_000;
const MAX_RAW: u64 = 137_438_953_472;
const MAX_PADDING_BYTES: u64 = 65_536;
const MAX_PADDING_BOXES: usize = 512;

#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    start: usize,
    body: usize,
    end: usize,
}
#[derive(Clone, Copy)]
struct Top {
    kind: [u8; 4],
    start: u64,
    body: u64,
    end: u64,
}
#[derive(Clone, Copy)]
struct Defaults {
    duration: u32,
    size: u32,
    flags: u32,
}
struct Init {
    end: u64,
    hash: [u8; 32],
    config: Configuration,
    defaults: Defaults,
    movie_duration: HeaderDuration,
    track_duration: HeaderDuration,
    media_duration: HeaderDuration,
}
struct Fragment {
    first_sample_duration: u32,
    sequence: u32,
    first: u64,
    last: u64,
    end: u64,
    samples: u64,
    default_base_is_moof: bool,
}

fn add(a: u64, b: u64, at: u64) -> Result<u64> {
    a.checked_add(b).ok_or(error(Failure::Malformed, at))
}
fn unexpected(kind: [u8; 4], at: u64) -> super::Error {
    let failure = match &kind {
        b"encv" | b"enca" | b"sinf" | b"schm" | b"schi" | b"tenc" | b"senc" | b"saiz" | b"saio"
        | b"pssh" | b"urn " => Failure::Unsafe,
        _ => Failure::Unsupported,
    };
    error(failure, at)
}

/// A complete header is distinct from a complete body. In particular, an
/// unknown semantic box is never converted to a recoverable EOF shortage.
fn top<R: Read + Seek>(input: &mut Input<'_, R>, at: u64) -> Result<Top> {
    let head = input.bytes(at, 8)?;
    let short = u32::from_be_bytes(head[..4].try_into().unwrap());
    let kind = head[4..8].try_into().unwrap();
    let (length, header) = match short {
        0 => return Err(error(Failure::Unsupported, at)),
        1 => {
            let extended = input.bytes(add(at, 8, at)?, 8)?;
            (u64::from_be_bytes(extended.try_into().unwrap()), 16)
        }
        n => (n as u64, 8),
    };
    if length < header || length > MAX_RAW {
        return Err(error(Failure::Malformed, at));
    }
    Ok(Top {
        kind,
        start: at,
        body: add(at, header, at)?,
        end: add(at, length, at)?,
    })
}
fn complete<R: Read + Seek>(input: &Input<'_, R>, atom: Top) -> Result<()> {
    input.checkpoint(atom.start)?;
    if atom.end > input.len {
        Err(error(Failure::Tail, atom.start))
    } else {
        Ok(())
    }
}
fn initialization_marker<R: Read + Seek>(input: &Input<'_, R>, atom: Top) -> Result<()> {
    if atom.kind != *b"ftyp" {
        return Err(unexpected(atom.kind, atom.start));
    }
    if atom.end - atom.start > METADATA as u64 {
        return Err(error(Failure::Limit, atom.start));
    }
    let body_len = atom.end - atom.body;
    if body_len < 8 || body_len % 4 != 0 {
        return Err(error(Failure::Malformed, atom.body));
    }
    complete(input, atom)
}

struct Meta<'a> {
    bytes: &'a [u8],
    base: u64,
    nodes: usize,
    checkpoint: &'a dyn Fn(u64) -> Result<()>,
}
impl Meta<'_> {
    fn at(&self, offset: usize) -> u64 {
        self.base + offset as u64
    }
    fn body(&self, atom: Atom) -> &[u8] {
        &self.bytes[atom.body..atom.end]
    }
    fn children(&mut self, start: usize, end: usize) -> Result<Vec<Atom>> {
        if start > end || end > self.bytes.len() {
            return Err(error(Failure::Malformed, self.at(start)));
        }
        let mut result = Vec::new();
        let mut at = start;
        while at < end {
            (self.checkpoint)(self.at(at))?;
            self.nodes += 1;
            if self.nodes > MAX_NODES {
                return Err(error(Failure::Limit, self.at(at)));
            }
            if end - at < 8 {
                return Err(error(Failure::Malformed, self.at(at)));
            }
            let n = u32::from_be_bytes(self.bytes[at..at + 4].try_into().unwrap());
            let kind = self.bytes[at + 4..at + 8].try_into().unwrap();
            let (size, header) = match n {
                0 => return Err(error(Failure::Unsupported, self.at(at))),
                1 => {
                    if end - at < 16 {
                        return Err(error(Failure::Malformed, self.at(at)));
                    }
                    (
                        u64::from_be_bytes(self.bytes[at + 8..at + 16].try_into().unwrap()),
                        16usize,
                    )
                }
                _ => (n as u64, 8),
            };
            if size < header as u64 || size > (end - at) as u64 {
                return Err(error(Failure::Malformed, self.at(at)));
            }
            let next = at + size as usize;
            result.push(Atom {
                kind,
                start: at,
                body: at + header,
                end: next,
            });
            at = next;
        }
        Ok(result)
    }
    fn full(&self, a: Atom, allowed_flags: u32, versions: &[u8]) -> Result<(u8, u32)> {
        let b = self.body(a);
        if b.len() < 4 {
            return Err(error(Failure::Malformed, self.at(a.body)));
        }
        let flags = u32::from_be_bytes([0, b[1], b[2], b[3]]);
        if !versions.contains(&b[0]) || flags & !allowed_flags != 0 {
            return Err(error(Failure::Unsupported, self.at(a.body)));
        }
        Ok((b[0], flags))
    }
    fn exact(&self, a: Atom, length: usize) -> Result<()> {
        if a.end - a.body != length {
            Err(error(Failure::Malformed, self.at(a.body)))
        } else {
            Ok(())
        }
    }
    fn u32(&self, a: Atom, offset: usize) -> Result<u32> {
        let b = self
            .body(a)
            .get(offset..offset + 4)
            .ok_or(error(Failure::Malformed, self.at(a.body + offset)))?;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    }
    fn u64(&self, a: Atom, offset: usize) -> Result<u64> {
        let b = self
            .body(a)
            .get(offset..offset + 8)
            .ok_or(error(Failure::Malformed, self.at(a.body + offset)))?;
        Ok(u64::from_be_bytes(b.try_into().unwrap()))
    }
}
fn unique(slot: &mut Option<Atom>, atom: Atom, base: u64) -> Result<()> {
    if slot.replace(atom).is_some() {
        Err(error(Failure::Malformed, base + atom.start as u64))
    } else {
        Ok(())
    }
}
fn required(slot: Option<Atom>, at: u64) -> Result<Atom> {
    slot.ok_or(error(Failure::Malformed, at))
}

fn descriptor<'a>(bytes: &'a [u8], cursor: &mut usize, base: u64) -> Result<(u8, &'a [u8])> {
    let begin = *cursor;
    let tag = *bytes
        .get(*cursor)
        .ok_or(error(Failure::Malformed, base + begin as u64))?;
    *cursor += 1;
    let mut length = 0usize;
    for _ in 0..4 {
        let byte = *bytes
            .get(*cursor)
            .ok_or(error(Failure::Malformed, base + *cursor as u64))?;
        *cursor += 1;
        length = length
            .checked_mul(128)
            .and_then(|n| n.checked_add((byte & 127) as usize))
            .ok_or(error(Failure::Malformed, base + begin as u64))?;
        if length > METADATA {
            return Err(error(Failure::Limit, base + begin as u64));
        }
        if byte & 128 == 0 {
            let end = cursor
                .checked_add(length)
                .ok_or(error(Failure::Malformed, base + begin as u64))?;
            let payload = bytes
                .get(*cursor..end)
                .ok_or(error(Failure::Malformed, base + begin as u64))?;
            *cursor = end;
            return Ok((tag, payload));
        }
    }
    Err(error(Failure::Malformed, base + begin as u64))
}

/// Preserve source bytes. The padded ASC is parsed, not normalized. Other
/// rates/layouts require explicit fixtures before this intentionally small
/// supported subset expands.
fn asc(meta: &Meta<'_>, a: Atom) -> Result<[u8; 2]> {
    meta.full(a, 0, &[0])?;
    let b = meta.body(a);
    let at = meta.at(a.body);
    let mut cursor = 4;
    let (tag, es) = descriptor(b, &mut cursor, at)?;
    if tag != 3 || cursor != b.len() || es.len() < 3 {
        return Err(error(Failure::Malformed, at));
    }
    // ES URL/dependency/OCR flags must not provide another input reference.
    if es[2] & 0xe0 != 0 {
        return Err(error(Failure::Unsafe, at));
    }
    if es[2] != 0 {
        return Err(error(Failure::Unsupported, at));
    }
    let mut cursor = 3;
    let (tag, decoder) = descriptor(es, &mut cursor, at)?;
    if tag != 4 || decoder.len() < 13 {
        return Err(error(Failure::Malformed, at));
    }
    if decoder[..2] != [0x40, 0x15] {
        return Err(error(Failure::Unsupported, at));
    }
    if cursor != es.len() {
        let (tag, sl) = descriptor(es, &mut cursor, at)?;
        if tag != 6 || sl != [2] || cursor != es.len() {
            return Err(error(Failure::Unsupported, at));
        }
    }
    let mut cursor = 13;
    let (tag, config) = descriptor(decoder, &mut cursor, at)?;
    if tag != 5 || cursor != decoder.len() {
        return Err(error(Failure::Malformed, at));
    }
    if !matches!(config.len(), 2 | 16) || config.len() == 16 && config[2..].iter().any(|v| *v != 0)
    {
        return Err(error(Failure::Unsupported, at));
    }
    if config.get(..2) != Some(&[0x12, 0x10][..]) {
        return Err(error(Failure::Unsupported, at));
    }
    Ok([config[0], config[1]])
}

fn parse_dinf(meta: &mut Meta<'_>, a: Atom) -> Result<()> {
    let children = meta.children(a.body, a.end)?;
    if children.len() != 1 || children[0].kind != *b"dref" {
        return Err(error(Failure::Unsafe, meta.at(a.start)));
    }
    let dref = children[0];
    meta.full(dref, 0, &[0])?;
    if meta.u32(dref, 4)? != 1 {
        return Err(error(Failure::Unsafe, meta.at(dref.body + 4)));
    }
    let entries = meta.children(dref.body + 8, dref.end)?;
    if entries.len() != 1 || entries[0].kind != *b"url " {
        return Err(error(Failure::Unsafe, meta.at(dref.start)));
    }
    let url = entries[0];
    if meta.body(url) != [0, 0, 0, 1] {
        return Err(error(Failure::Unsafe, meta.at(url.body)));
    }
    Ok(())
}

fn parse_stbl(meta: &mut Meta<'_>, a: Atom) -> Result<[u8; 2]> {
    let children = meta.children(a.body, a.end)?;
    let (mut stsd, mut stts, mut stsc, mut stsz, mut stco, mut stss) =
        (None, None, None, None, None, None);
    for child in children {
        match &child.kind {
            b"stsd" => unique(&mut stsd, child, meta.base)?,
            b"stts" => unique(&mut stts, child, meta.base)?,
            b"stsc" => unique(&mut stsc, child, meta.base)?,
            b"stsz" => unique(&mut stsz, child, meta.base)?,
            b"stco" => unique(&mut stco, child, meta.base)?,
            b"stss" => unique(&mut stss, child, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(child.kind, meta.at(child.start))),
        }
    }
    for table in [stts, stsc, stco] {
        let table = required(table, meta.at(a.start))?;
        if meta.body(table) != [0; 8] {
            return Err(error(Failure::Unsupported, meta.at(table.body)));
        }
    }
    if let Some(table) = stss {
        if meta.body(table) != [0; 8] {
            return Err(error(Failure::Unsupported, meta.at(table.body)));
        }
    }
    let table = required(stsz, meta.at(a.start))?;
    if meta.body(table) != [0; 12] {
        return Err(error(Failure::Unsupported, meta.at(table.body)));
    }
    let stsd = required(stsd, meta.at(a.start))?;
    meta.full(stsd, 0, &[0])?;
    if meta.u32(stsd, 4)? != 1 {
        return Err(error(Failure::Unsupported, meta.at(stsd.body + 4)));
    }
    let entries = meta.children(stsd.body + 8, stsd.end)?;
    if entries.len() != 1 {
        return Err(error(Failure::Malformed, meta.at(stsd.start)));
    }
    let entry = entries[0];
    if entry.kind != *b"mp4a" {
        return Err(unexpected(entry.kind, meta.at(entry.start)));
    }
    let data = meta.body(entry);
    if data.len() < 28 {
        return Err(error(Failure::Malformed, meta.at(entry.body)));
    }
    if data[..6] != [0; 6] || data[6..8] != [0, 1] {
        return Err(error(Failure::Unsafe, meta.at(entry.body)));
    }
    if data[8..16] != [0; 8]
        || data[16..18] != [0, 2]
        || data[18..20] != [0, 16]
        || data[20..24] != [0; 4]
        || data[24..28] != (44_100u32 << 16).to_be_bytes()
    {
        return Err(error(Failure::Unsupported, meta.at(entry.body + 8)));
    }
    let fields = meta.children(entry.body + 28, entry.end)?;
    let (mut esds, mut btrt) = (None, None);
    for child in fields {
        match &child.kind {
            b"esds" => unique(&mut esds, child, meta.base)?,
            b"btrt" => {
                unique(&mut btrt, child, meta.base)?;
                meta.exact(child, 12)?;
            }
            b"free" | b"skip" => {}
            _ => return Err(unexpected(child.kind, meta.at(child.start))),
        }
    }
    asc(meta, required(esds, meta.at(entry.start))?)
}

fn header_duration(
    meta: &Meta<'_>,
    atom: Atom,
    version: u8,
    v0: usize,
    v1: usize,
) -> Result<HeaderDuration> {
    let (value, sentinel) = if version == 0 {
        (meta.u32(atom, v0)? as u64, u32::MAX as u64)
    } else {
        (meta.u64(atom, v1)?, u64::MAX)
    };
    Ok(match value {
        0 => HeaderDuration::Zero,
        n if n == sentinel => HeaderDuration::UnknownSentinel,
        n => HeaderDuration::Finite(n),
    })
}

fn parse_trak(
    meta: &mut Meta<'_>,
    trak: Atom,
) -> Result<(u32, u32, [u8; 2], HeaderDuration, HeaderDuration)> {
    let (mut tkhd, mut mdia) = (None, None);
    for a in meta.children(trak.body, trak.end)? {
        match &a.kind {
            b"tkhd" => unique(&mut tkhd, a, meta.base)?,
            b"mdia" => unique(&mut mdia, a, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    let tkhd = required(tkhd, meta.at(trak.start))?;
    let (version, flags) = meta.full(tkhd, 7, &[0, 1])?;
    meta.exact(tkhd, if version == 0 { 84 } else { 96 })?;
    if flags & 1 == 0 {
        return Err(error(Failure::Unsupported, meta.at(tkhd.body)));
    }
    let track = meta.u32(tkhd, if version == 0 { 12 } else { 20 })?;
    if track == 0 {
        return Err(error(Failure::Malformed, meta.at(tkhd.body)));
    }
    let track_duration = header_duration(meta, tkhd, version, 20, 28)?;
    let mdia = required(mdia, meta.at(trak.start))?;
    let (mut mdhd, mut hdlr, mut minf) = (None, None, None);
    for a in meta.children(mdia.body, mdia.end)? {
        match &a.kind {
            b"mdhd" => unique(&mut mdhd, a, meta.base)?,
            b"hdlr" => unique(&mut hdlr, a, meta.base)?,
            b"minf" => unique(&mut minf, a, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    let mdhd = required(mdhd, meta.at(mdia.start))?;
    let (version, _) = meta.full(mdhd, 0, &[0, 1])?;
    meta.exact(mdhd, if version == 0 { 24 } else { 36 })?;
    let scale = meta.u32(mdhd, if version == 0 { 12 } else { 20 })?;
    if scale != 44_100 {
        return Err(error(Failure::Unsupported, meta.at(mdhd.body)));
    }
    let media_duration = header_duration(meta, mdhd, version, 16, 24)?;
    let hdlr = required(hdlr, meta.at(mdia.start))?;
    meta.full(hdlr, 0, &[0])?;
    let b = meta.body(hdlr);
    if b.len() < 24 {
        return Err(error(Failure::Malformed, meta.at(hdlr.body)));
    }
    if b[4..8] != [0; 4] || b[8..12] != *b"soun" || b[12..24] != [0; 12] {
        return Err(error(Failure::Unsupported, meta.at(hdlr.body + 4)));
    }
    let minf = required(minf, meta.at(mdia.start))?;
    let (mut smhd, mut dinf, mut stbl) = (None, None, None);
    for a in meta.children(minf.body, minf.end)? {
        match &a.kind {
            b"smhd" => unique(&mut smhd, a, meta.base)?,
            b"dinf" => unique(&mut dinf, a, meta.base)?,
            b"stbl" => unique(&mut stbl, a, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    let smhd = required(smhd, meta.at(minf.start))?;
    if meta.body(smhd) != [0; 8] {
        return Err(error(Failure::Unsupported, meta.at(smhd.body)));
    }
    parse_dinf(meta, required(dinf, meta.at(minf.start))?)?;
    let config = parse_stbl(meta, required(stbl, meta.at(minf.start))?)?;
    Ok((track, scale, config, track_duration, media_duration))
}

fn init<R: Read + Seek>(input: &mut Input<'_, R>, start: u64, expected: u64) -> Result<Init> {
    let first = top(input, start)?;
    initialization_marker(input, first)?;
    let mut at = first.end;
    let moov = loop {
        let atom = top(input, at)?;
        if atom.end - start > METADATA as u64 {
            return Err(error(Failure::Limit, at));
        }
        match &atom.kind {
            b"moov" => {
                complete(input, atom)?;
                break atom;
            }
            b"free" | b"skip" => {
                complete(input, atom)?;
                at = atom.end;
            }
            _ => return Err(unexpected(atom.kind, atom.start)),
        }
    };
    let bytes = input.bytes(start, (moov.end - start) as usize)?;
    let checkpoint = |at| input.checkpoint(at);
    let mut meta = Meta {
        bytes: &bytes,
        base: start,
        nodes: 0,
        checkpoint: &checkpoint,
    };
    let roots = meta.children(0, bytes.len())?;
    let ftyp = roots[0];
    if ftyp.end - ftyp.body < 8 || (ftyp.end - ftyp.body) % 4 != 0 {
        return Err(error(Failure::Malformed, meta.at(ftyp.body)));
    }
    let moov_atom = *roots.last().unwrap();
    let (mut mvhd, mut trak, mut mvex) = (None, None, None);
    for a in meta.children(moov_atom.body, moov_atom.end)? {
        input.checkpoint(meta.at(a.start))?;
        match &a.kind {
            b"mvhd" => unique(&mut mvhd, a, meta.base)?,
            b"trak" => unique(&mut trak, a, meta.base)?,
            b"mvex" => unique(&mut mvex, a, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    let mvhd = required(mvhd, moov.start)?;
    let (version, _) = meta.full(mvhd, 0, &[0, 1])?;
    meta.exact(mvhd, if version == 0 { 100 } else { 112 })?;
    if meta.u32(mvhd, if version == 0 { 12 } else { 20 })? == 0 {
        return Err(error(Failure::Malformed, meta.at(mvhd.body)));
    }
    let movie_duration = header_duration(&meta, mvhd, version, 16, 24)?;
    let (track, scale, asc, track_duration, media_duration) =
        parse_trak(&mut meta, required(trak, moov.start)?)?;
    if scale as u64 != expected {
        return Err(error(Failure::Clock, start));
    }
    let mvex = required(mvex, moov.start)?;
    let mut trex = None;
    for a in meta.children(mvex.body, mvex.end)? {
        match &a.kind {
            b"trex" => unique(&mut trex, a, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    let trex = required(trex, meta.at(mvex.start))?;
    meta.full(trex, 0, &[0])?;
    meta.exact(trex, 24)?;
    if meta.u32(trex, 4)? != track || meta.u32(trex, 8)? != 1 {
        return Err(error(Failure::Malformed, meta.at(trex.body + 4)));
    }
    let defaults = Defaults {
        duration: meta.u32(trex, 12)?,
        size: meta.u32(trex, 16)?,
        flags: meta.u32(trex, 20)?,
    };
    let mut semantic = Sha256::new();
    semantic.update(b"atsumi-youtube-aac-container-v1\0AAC-LC\0");
    semantic.update(track.to_be_bytes());
    semantic.update(scale.to_be_bytes());
    semantic.update([2]);
    semantic.update(asc);
    semantic.update(1u32.to_be_bytes());
    semantic.update(defaults.duration.to_be_bytes());
    semantic.update(defaults.size.to_be_bytes());
    semantic.update(defaults.flags.to_be_bytes());
    input.checkpoint(start)?;
    Ok(Init {
        end: moov.end,
        hash: Sha256::digest(&bytes).into(),
        defaults,
        movie_duration,
        track_duration,
        media_duration,
        config: Configuration {
            format: Format::AacMp4,
            track_id: track as u64,
            timebase: scale as u64,
            width: 0,
            height: 0,
            default_duration_ns: None,
            sample_rate: 44_100,
            channels: 2,
            semantic_sha256: semantic.finalize().into(),
        },
    })
}

/// Exact ftyp-through-moov prefix only; no media or post-init padding is part
/// of this metadata request. The raw archive will preserve those later bytes.
pub(super) fn initialization<R: Read + Seek>(input: &mut Input<'_, R>) -> Result<Configuration> {
    input.checkpoint(0)?;
    let parsed = init(input, 0, 44_100)?;
    if parsed.end != input.len
        || parsed.movie_duration != HeaderDuration::Zero
        || parsed.track_duration != HeaderDuration::Zero
        || parsed.media_duration != HeaderDuration::Zero
    {
        return Err(error(Failure::Unsupported, parsed.end));
    }
    input.checkpoint(input.len)?;
    Ok(parsed.config)
}

fn sample_flags(flags: u32, at: u64) -> Result<()> {
    // Unknown/dependent/non-sync samples cannot be silently described as an
    // independently decodable AAC packet by this narrow structural subset.
    if matches!(flags, 0 | 0x0200_0000) {
        Ok(())
    } else {
        Err(error(Failure::Unsupported, at))
    }
}
fn fragment<R: Read + Seek>(
    input: &mut Input<'_, R>,
    moof: Top,
    mdat: Top,
    init: &Init,
    remaining_samples: u64,
) -> Result<Fragment> {
    if moof.end - moof.start > METADATA as u64 {
        return Err(error(Failure::Limit, moof.start));
    }
    let bytes = input.bytes(moof.start, (moof.end - moof.start) as usize)?;
    let checkpoint = |at| input.checkpoint(at);
    let mut meta = Meta {
        bytes: &bytes,
        base: moof.start,
        nodes: 0,
        checkpoint: &checkpoint,
    };
    let root = meta.children(0, bytes.len())?[0];
    let (mut mfhd, mut traf) = (None, None);
    for a in meta.children(root.body, root.end)? {
        match &a.kind {
            b"mfhd" => unique(&mut mfhd, a, meta.base)?,
            b"traf" => unique(&mut traf, a, meta.base)?,
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    let mfhd = required(mfhd, moof.start)?;
    meta.full(mfhd, 0, &[0])?;
    meta.exact(mfhd, 8)?;
    let sequence = meta.u32(mfhd, 4)?;
    let traf = required(traf, moof.start)?;
    let (mut tfhd, mut tfdt) = (None, None);
    let mut runs = Vec::new();
    for a in meta.children(traf.body, traf.end)? {
        match &a.kind {
            b"tfhd" => unique(&mut tfhd, a, meta.base)?,
            b"tfdt" => unique(&mut tfdt, a, meta.base)?,
            b"trun" => runs.push(a),
            b"free" | b"skip" => {}
            _ => return Err(unexpected(a.kind, meta.at(a.start))),
        }
    }
    if runs.is_empty() {
        return Err(error(Failure::Malformed, meta.at(traf.start)));
    }
    let tfhd = required(tfhd, moof.start)?;
    let (_, flags) = meta.full(tfhd, 0x02003a, &[0])?;
    let default_base_is_moof = flags & 0x020000 != 0;
    if meta.u32(tfhd, 4)? as u64 != init.config.track_id {
        return Err(error(Failure::Malformed, meta.at(tfhd.body + 4)));
    }
    let mut cursor = 8;
    if flags & 2 != 0 {
        if meta.u32(tfhd, cursor)? != 1 {
            return Err(error(Failure::Unsupported, meta.at(tfhd.body + cursor)));
        }
        cursor += 4;
    }
    let mut defaults = init.defaults;
    if flags & 8 != 0 {
        defaults.duration = meta.u32(tfhd, cursor)?;
        cursor += 4;
    }
    if flags & 16 != 0 {
        defaults.size = meta.u32(tfhd, cursor)?;
        cursor += 4;
    }
    if flags & 32 != 0 {
        defaults.flags = meta.u32(tfhd, cursor)?;
        cursor += 4;
    }
    meta.exact(tfhd, cursor)?;
    let tfdt = required(tfdt, moof.start)?;
    let (version, _) = meta.full(tfdt, 0, &[0, 1])?;
    meta.exact(tfdt, if version == 0 { 8 } else { 12 })?;
    let first = if version == 0 {
        meta.u32(tfdt, 4)? as u64
    } else {
        meta.u64(tfdt, 4)?
    };
    let (mut tick, mut last, mut samples, mut extent) = (first, first, 0u64, mdat.body);
    let mut first_sample_duration = 0;
    for run in runs {
        input.checkpoint(meta.at(run.start))?;
        let (_, flags) = meta.full(run, 0x000f05, &[0, 1])?;
        if flags & 1 == 0 || flags & 4 != 0 && flags & 0x400 != 0 {
            return Err(error(Failure::Unsupported, meta.at(run.body)));
        }
        let count = meta.u32(run, 4)? as u64;
        if count == 0 {
            return Err(error(Failure::Malformed, meta.at(run.body + 4)));
        }
        if count > remaining_samples
            || samples
                .checked_add(count)
                .is_none_or(|v| v > remaining_samples)
        {
            return Err(error(Failure::Limit, meta.at(run.body + 4)));
        }
        let offset = meta.u32(run, 8)? as i32;
        if offset < 0 {
            return Err(error(Failure::Malformed, meta.at(run.body + 8)));
        }
        let run_start = add(moof.start, offset as u64, meta.at(run.body + 8))?;
        if run_start != extent {
            return Err(error(Failure::Malformed, meta.at(run.body + 8)));
        }
        let mut cursor = 12;
        let first_flags = if flags & 4 != 0 {
            let value = meta.u32(run, cursor)?;
            cursor += 4;
            Some(value)
        } else {
            None
        };
        let fields = ((flags & 0x100 != 0) as u64
            + (flags & 0x200 != 0) as u64
            + (flags & 0x400 != 0) as u64
            + (flags & 0x800 != 0) as u64)
            * 4;
        let needed = count
            .checked_mul(fields)
            .and_then(|n| n.checked_add(cursor as u64))
            .ok_or(error(Failure::Malformed, meta.at(run.body)))?;
        if needed != (run.end - run.body) as u64 {
            return Err(error(Failure::Malformed, meta.at(run.body)));
        }
        for index in 0..count {
            input.checkpoint(meta.at(run.start))?;
            let duration = if flags & 0x100 != 0 {
                let n = meta.u32(run, cursor)?;
                cursor += 4;
                n
            } else {
                defaults.duration
            };
            let size = if flags & 0x200 != 0 {
                let n = meta.u32(run, cursor)?;
                cursor += 4;
                n
            } else {
                defaults.size
            };
            let sample_flag = if flags & 0x400 != 0 {
                let n = meta.u32(run, cursor)?;
                cursor += 4;
                n
            } else if index == 0 {
                first_flags.unwrap_or(defaults.flags)
            } else {
                defaults.flags
            };
            sample_flags(sample_flag, meta.at(run.body + cursor))?;
            if flags & 0x800 != 0 {
                if meta.u32(run, cursor)? != 0 {
                    return Err(error(Failure::Clock, meta.at(run.body + cursor)));
                }
                cursor += 4;
            }
            if duration == 0 || size == 0 {
                return Err(error(Failure::Malformed, meta.at(run.body + cursor)));
            }
            if samples == 0 && index == 0 {
                first_sample_duration = duration;
            }
            if size as u64 > MAX_ITEM {
                return Err(error(Failure::Limit, meta.at(run.body + cursor)));
            }
            extent = add(extent, size as u64, meta.at(run.body + cursor))?;
            if extent > mdat.end {
                return Err(error(Failure::Malformed, meta.at(run.body + cursor)));
            }
            last = tick;
            tick = tick
                .checked_add(duration as u64)
                .ok_or(error(Failure::Clock, meta.at(run.body + cursor)))?;
        }
        samples += count;
    }
    if extent != mdat.end {
        return Err(error(Failure::Malformed, extent));
    }
    Ok(Fragment {
        first_sample_duration,
        sequence,
        first,
        last,
        end: tick,
        samples,
        default_base_is_moof,
    })
}

pub(super) fn run<R: Read + Seek>(
    input: &mut Input<'_, R>,
    emit: &mut Emit<'_>,
    expected_timebase: u64,
) -> Result<()> {
    let mut start = 0;
    let mut total_samples = 0u64;
    loop {
        input.checkpoint(start)?;
        let initialization = init(input, start, expected_timebase)?;
        let mut at = initialization.end;
        let (mut count, mut fragments) = (0u64, 0u64);
        let (mut first, mut last, mut end) = (0u64, 0u64, 0u64);
        let mut previous_sequence = None::<u32>;
        let mut reconstruction = AacReconstruction {
            first_sample_duration: 0,
            all_fragments_default_base_is_moof: true,
            first_sequence: 0,
            last_sequence: 0,
            movie_duration: initialization.movie_duration,
            track_duration: initialization.track_duration,
            media_duration: initialization.media_duration,
        };
        let (mut padding_bytes, mut padding_boxes) = (0u64, 0usize);
        let mut pending_padding = false;
        loop {
            input.checkpoint(at)?;
            if at == input.len {
                if fragments == 0 || pending_padding {
                    return Err(error(Failure::Tail, at));
                }
                emit.unit(UnitDraft {
                    start,
                    init_end: initialization.end,
                    end: at,
                    boundary: Boundary::CommittedEof,
                    init_sha256: initialization.hash,
                    configuration: initialization.config,
                    clock: Clock {
                        first: first as i128,
                        last: last as i128,
                        end: end as i128,
                        ticks_per_second: expected_timebase,
                    },
                    items: count,
                    fragments,
                    rap_candidate: true,
                    references: 0,
                    reconstruction: ReconstructionFacts::Aac(reconstruction),
                })?;
                return input.checkpoint(at);
            }
            let atom = top(input, at)?;
            if matches!(&atom.kind, b"free" | b"skip") {
                let length = atom.end - atom.start;
                padding_bytes = padding_bytes
                    .checked_add(length)
                    .ok_or(error(Failure::Limit, at))?;
                padding_boxes = padding_boxes
                    .checked_add(1)
                    .ok_or(error(Failure::Limit, at))?;
                if length > MAX_PADDING_BYTES
                    || padding_bytes > MAX_PADDING_BYTES
                    || padding_boxes > MAX_PADDING_BOXES
                {
                    return Err(error(Failure::Limit, at));
                }
                complete(input, atom)?;
                // Opaque padding is never scanned for a successor header and
                // does not prove a complete unit at EOF by itself.
                pending_padding = true;
                at = atom.end;
                continue;
            }
            if atom.kind == *b"ftyp" && fragments > 0 {
                // The completed initialization-start marker closes the prior
                // unit. A partial/invalid successor moov still makes the final
                // scan Tail/Failed; this provisional fact is not publication.
                initialization_marker(input, atom)?;
                input.checkpoint(at)?;
                emit.unit(UnitDraft {
                    start,
                    init_end: initialization.end,
                    end: at,
                    boundary: Boundary::NextInitialization,
                    init_sha256: initialization.hash,
                    configuration: initialization.config,
                    clock: Clock {
                        first: first as i128,
                        last: last as i128,
                        end: end as i128,
                        ticks_per_second: expected_timebase,
                    },
                    items: count,
                    fragments,
                    rap_candidate: true,
                    references: 0,
                    reconstruction: ReconstructionFacts::Aac(reconstruction),
                })?;
                start = at;
                break;
            }
            if atom.kind != *b"moof" {
                return Err(unexpected(atom.kind, atom.start));
            }
            if atom.end - atom.start > METADATA as u64 {
                return Err(error(Failure::Limit, atom.start));
            }
            complete(input, atom)?;
            let mdat = top(input, atom.end)?;
            if mdat.kind != *b"mdat" {
                return Err(unexpected(mdat.kind, mdat.start));
            }
            complete(input, mdat)?;
            let parsed = fragment(
                input,
                atom,
                mdat,
                &initialization,
                MAX_SAMPLES - total_samples,
            )?;
            total_samples += parsed.samples;
            if fragments == 0 {
                first = parsed.first;
                reconstruction.first_sample_duration = parsed.first_sample_duration;
                reconstruction.first_sequence = parsed.sequence;
            } else if parsed.first != end {
                return Err(error(Failure::Clock, atom.start));
            }
            if let Some(previous) = previous_sequence {
                if previous.checked_add(1) != Some(parsed.sequence) {
                    return Err(error(Failure::Malformed, atom.start));
                }
            }
            previous_sequence = Some(parsed.sequence);
            reconstruction.last_sequence = parsed.sequence;
            reconstruction.all_fragments_default_base_is_moof &= parsed.default_base_is_moof;
            count = count
                .checked_add(parsed.samples)
                .ok_or(error(Failure::Limit, atom.start))?;
            if count > MAX_SAMPLES {
                return Err(error(Failure::Limit, atom.start));
            }
            fragments = fragments
                .checked_add(1)
                .ok_or(error(Failure::Limit, atom.start))?;
            pending_padding = false;
            last = parsed.last;
            end = parsed.end;
            at = mdat.end;
        }
    }
}

#[cfg(test)]
pub(super) fn authored_audio() -> Vec<u8> {
    tests::authored_audio()
}

#[cfg(test)]
pub(super) fn authored_audio_durations(tick: u64, durations: &[u32]) -> Vec<u8> {
    tests::authored_audio_durations(tick, durations)
}

#[cfg(test)]
mod tests {
    use super::super::{scan, ClockRelation, InitRelation, Outcome, Terminal, Unit};
    use super::*;
    use std::{io::Cursor, sync::atomic::AtomicBool};

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend(kind);
        bytes.extend(body);
        bytes
    }
    fn joined(items: &[Vec<u8>]) -> Vec<u8> {
        items.concat()
    }
    fn descriptor_bytes(tag: u8, data: &[u8]) -> Vec<u8> {
        assert!(data.len() < 128);
        let mut b = vec![tag, data.len() as u8];
        b.extend(data);
        b
    }
    fn fixture_init(padded: bool, external_reference: bool) -> Vec<u8> {
        fixture_init_durations(padded, external_reference, 0, [0; 3])
    }
    fn fixture_init_durations(
        padded: bool,
        external_reference: bool,
        version: u8,
        durations: [u64; 3],
    ) -> Vec<u8> {
        let mut movie = vec![0; if version == 0 { 100 } else { 112 }];
        let mut track = vec![0; if version == 0 { 84 } else { 96 }];
        let mut header = vec![0; if version == 0 { 24 } else { 36 }];
        let scale_at = if version == 0 { 12 } else { 20 };
        movie[scale_at..scale_at + 4].copy_from_slice(&1000u32.to_be_bytes());
        track[3] = 3;
        track[scale_at..scale_at + 4].copy_from_slice(&7u32.to_be_bytes());
        header[scale_at..scale_at + 4].copy_from_slice(&44_100u32.to_be_bytes());
        for (bytes, value, offset) in [
            (&mut movie, durations[0], if version == 0 { 16 } else { 24 }),
            (&mut track, durations[1], if version == 0 { 20 } else { 28 }),
            (
                &mut header,
                durations[2],
                if version == 0 { 16 } else { 24 },
            ),
        ] {
            bytes[0] = version;
            if version == 0 {
                bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_be_bytes());
            } else {
                bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
            }
        }
        let mut handler = vec![0; 24];
        handler[8..12].copy_from_slice(b"soun");
        let url = if external_reference {
            atom(b"url ", b"\0\0\0\0http://bad\0")
        } else {
            atom(b"url ", &[0, 0, 0, 1])
        };
        let mut dref = vec![0, 0, 0, 0, 0, 0, 0, 1];
        dref.extend(url);
        let mut config = vec![0x12, 0x10];
        if padded {
            config.extend([0; 14]);
        }
        let mut decoder = vec![0x40, 0x15];
        decoder.extend([0; 11]);
        decoder.extend(descriptor_bytes(5, &config));
        let mut es = vec![0, 7, 0];
        es.extend(descriptor_bytes(4, &decoder));
        es.extend([6, 1, 2]);
        let mut esds = vec![0; 4];
        esds.extend(descriptor_bytes(3, &es));
        let mut sample = vec![0; 28];
        sample[7] = 1;
        sample[17] = 2;
        sample[19] = 16;
        sample[24..28].copy_from_slice(&(44_100u32 << 16).to_be_bytes());
        sample.extend(atom(b"esds", &esds));
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(atom(b"mp4a", &sample));
        let stbl = atom(
            b"stbl",
            &joined(&[
                atom(b"stsd", &stsd),
                atom(b"stts", &[0; 8]),
                atom(b"stsc", &[0; 8]),
                atom(b"stsz", &[0; 12]),
                atom(b"stco", &[0; 8]),
            ]),
        );
        let minf = atom(
            b"minf",
            &joined(&[
                atom(b"smhd", &[0; 8]),
                atom(b"dinf", &atom(b"dref", &dref)),
                stbl,
            ]),
        );
        let mdia = atom(
            b"mdia",
            &joined(&[atom(b"mdhd", &header), atom(b"hdlr", &handler), minf]),
        );
        let mut trex = vec![0; 24];
        trex[4..8].copy_from_slice(&7u32.to_be_bytes());
        trex[8..12].copy_from_slice(&1u32.to_be_bytes());
        trex[12..16].copy_from_slice(&1024u32.to_be_bytes());
        trex[16..20].copy_from_slice(&4u32.to_be_bytes());
        let moov = atom(
            b"moov",
            &joined(&[
                atom(b"mvhd", &movie),
                atom(b"trak", &joined(&[atom(b"tkhd", &track), mdia])),
                atom(b"mvex", &atom(b"trex", &trex)),
            ]),
        );
        joined(&[atom(b"ftyp", b"iso5\0\0\0\0iso6"), moov])
    }
    fn media(tick: u64, sequence: u32, samples: u32) -> Vec<u8> {
        media_durations(tick, sequence, samples, None, None)
    }
    fn media_durations(tick: u64, sequence: u32, samples: u32, default: Option<u32>, durations: Option<&[u32]>) -> Vec<u8> {
        let mut mfhd = vec![0; 4];
        mfhd.extend(sequence.to_be_bytes());
        let mut tfhd = vec![0, 2, 0, 0];
        tfhd.extend(7u32.to_be_bytes());
        if let Some(duration) = default {
            tfhd[3] |= 8;
            tfhd.extend(duration.to_be_bytes());
        }
        let mut tfdt = vec![1, 0, 0, 0];
        tfdt.extend(tick.to_be_bytes());
        let mut trun = vec![0, 0, 0, 1];
        trun.extend(samples.to_be_bytes());
        trun.extend([0; 4]);
        if let Some(durations) = durations {
            assert_eq!(durations.len(), samples as usize);
            trun[2] |= 1;
            for duration in durations { trun.extend(duration.to_be_bytes()); }
        }
        let build = |run: &[u8]| {
            atom(
                b"moof",
                &joined(&[
                    atom(b"mfhd", &mfhd),
                    atom(
                        b"traf",
                        &joined(&[
                            atom(b"tfhd", &tfhd),
                            atom(b"tfdt", &tfdt),
                            atom(b"trun", run),
                        ]),
                    ),
                ]),
            )
        };
        let moof = build(&trun);
        trun[8..12].copy_from_slice(&((moof.len() + 8) as u32).to_be_bytes());
        let mut payload = vec![0; samples as usize * 4];
        // A complete-looking header inside encoded payload must not split it.
        if payload.len() >= 8 {
            payload[..8].copy_from_slice(b"\0\0\0\x08ftyp");
        }
        joined(&[build(&trun), atom(b"mdat", &payload)])
    }
    fn fixture(padded: bool, tick: u64) -> Vec<u8> {
        joined(&[fixture_init(padded, false), media(tick, 1, 3)])
    }
    pub(super) fn authored_audio() -> Vec<u8> {
        fixture(false, 0)
    }
    pub(super) fn authored_audio_durations(tick: u64, durations: &[u32]) -> Vec<u8> {
        joined(&[fixture_init(false, false), media_durations(tick, 1, durations.len() as u32, None, Some(durations))])
    }
    fn inspect(bytes: &[u8]) -> (Outcome, Vec<Unit>) {
        let mut units = Vec::new();
        let result = scan(
            Format::AacMp4,
            &mut Cursor::new(bytes),
            bytes.len() as u64,
            44_100,
            &AtomicBool::new(false),
            &mut |unit| {
                units.push(unit);
                Ok(())
            },
        );
        (result, units)
    }
    fn location(bytes: &[u8], tag: &[u8; 4]) -> usize {
        bytes.windows(4).position(|x| x == tag).unwrap() - 4
    }
    fn failure(bytes: &[u8]) -> Failure {
        match inspect(bytes).0.terminal {
            Terminal::Failed(e) => e.kind,
            other => panic!("expected failure, got {other:?}"),
        }
    }
    fn reconstruction(unit: &Unit) -> AacReconstruction {
        match unit.facts.reconstruction {
            ReconstructionFacts::Aac(facts) => facts,
            _ => panic!("wrong reconstruction track"),
        }
    }

    #[test]
    fn first_actual_sample_duration_resolves_trun_tfhd_then_trex() {
        let initialization = fixture_init(false, false);
        let mut config = None;
        for (default, explicit, expected_first, expected_last, expected_end) in [
            (None, None, 1024, 1024, 3072),
            (Some(777), None, 777, 777, 2331),
            (Some(777), Some([123, 456, 789]), 123, 789, 1368),
            (None, Some([789, 456, 123]), 789, 123, 1368),
        ] {
            let bytes = joined(&[initialization.clone(), media_durations(100, 1, 3, default, explicit.as_ref().map(|x| x.as_slice()))]);
            let (outcome, units) = inspect(&bytes);
            assert_eq!(outcome.terminal, Terminal::Complete);
            let unit = &units[0];
            assert_eq!(reconstruction(unit).first_sample_duration, expected_first);
            assert_eq!(unit.facts.clock.end - unit.facts.clock.last, expected_last);
            assert_eq!(unit.facts.clock.end, 100 + expected_end);
            if let Some(config) = config { assert_eq!(unit.configuration_sha256, config); }
            config = Some(unit.configuration_sha256);
        }
        let bytes = joined(&[initialization.clone(), media_durations(0, 1, 3, None, Some(&[123,456,789])), media(1368, 2, 1)]);
        let (outcome, units) = inspect(&bytes);
        assert_eq!(outcome.terminal, Terminal::Complete);
        assert_eq!(reconstruction(&units[0]).first_sample_duration, 123);
        assert_eq!(units[0].facts.clock.end - units[0].facts.clock.last, 1024);
        for durations in [[0,456,789], [123,456,0]] {
            assert_eq!(failure(&joined(&[initialization.clone(), media_durations(0,1,3,None,Some(&durations))])), Failure::Malformed);
        }
    }

    #[test]
    fn aac_initialization_keeps_native_config_and_asc_padding_equivalence() {
        use super::super::inspect_aac_initialization;
        let cancel = AtomicBool::new(false);
        let plain = fixture_init(false, false);
        let padded = fixture_init(true, false);
        let a = inspect_aac_initialization(&plain, &cancel).unwrap();
        let b = inspect_aac_initialization(&padded, &cancel).unwrap();
        assert_eq!(a.timebase(), 44_100);
        assert_eq!(a.track(), crate::platform_media::youtube_spool::Track::Audio);
        assert_eq!(a.configuration_sha256(), b.configuration_sha256());
        assert_ne!(a.init_sha256(), b.init_sha256());
        assert_eq!(a.len(), plain.len());
        assert_eq!(b.len(), padded.len());
        let ftyp_end = u32::from_be_bytes(plain[..4].try_into().unwrap()) as usize;
        let before_moov = [&plain[..ftyp_end], atom(b"skip", &[0; 8]).as_slice(), &plain[ftyp_end..]].concat();
        assert_eq!(inspect_aac_initialization(&before_moov, &cancel).unwrap().configuration_sha256(), a.configuration_sha256());
    }

    #[test]
    fn aac_initialization_rejects_partial_trailing_media_and_nonzero_durations() {
        use super::super::inspect_aac_initialization;
        let cancel = AtomicBool::new(false);
        let valid = fixture_init(false, false);
        for end in 0..valid.len() {
            assert!(inspect_aac_initialization(&valid[..end], &cancel).is_err(), "prefix {end}");
        }
        for tail in [vec![0], atom(b"skip", &[0; 8]), media(0, 1, 3), valid.clone()] {
            assert!(inspect_aac_initialization(&[valid.clone(), tail].concat(), &cancel).is_err());
        }
        for version in [0, 1] {
            for field in 0..3 {
                for value in [1, if version == 0 { u32::MAX as u64 } else { u64::MAX }] {
                    let mut durations = [0; 3]; durations[field] = value;
                    assert!(inspect_aac_initialization(&fixture_init_durations(false, false, version, durations), &cancel).is_err());
                }
            }
        }
    }

    #[test]
    fn aac_initialization_preserves_codec_rate_channel_dref_and_encryption_guards() {
        use super::super::inspect_aac_initialization;
        let cancel = AtomicBool::new(false);
        let valid = fixture_init(false, false);
        assert!(inspect_aac_initialization(&fixture_init(false, true), &cancel).is_err());
        let mut wrong_codec = valid.clone();
        let at = location(&wrong_codec, b"mp4a");
        wrong_codec[at + 4..at + 8].copy_from_slice(b"avc1");
        let mut wrong_rate = valid.clone();
        let at = location(&wrong_rate, b"mdhd");
        wrong_rate[at + 20..at + 24].copy_from_slice(&48_000u32.to_be_bytes());
        let mut wrong_channels = valid.clone();
        let at = location(&wrong_channels, b"mp4a");
        wrong_channels[at + 25] = 6;
        let mut encrypted = valid;
        let at = location(&encrypted, b"esds");
        encrypted[at + 4..at + 8].copy_from_slice(b"sinf");
        for bytes in [wrong_codec, wrong_rate, wrong_channels, encrypted] {
            assert!(inspect_aac_initialization(&bytes, &cancel).is_err());
        }
    }

    #[test]
    fn reconstruction_moof_base_is_reported_per_unit_without_changing_admission() {
        for (first_explicit, second_explicit) in
            [(true, true), (false, false), (true, false), (false, true)]
        {
            let mut first = media(0, 6, 3);
            let mut second = media(3072, 7, 2);
            for (bytes, explicit) in [(&mut first, first_explicit), (&mut second, second_explicit)]
            {
                let at = location(bytes, b"tfhd");
                bytes[at + 9] = if explicit { 2 } else { 0 };
            }
            let bytes = joined(&[
                fixture_init(false, false),
                first,
                atom(b"skip", &[0; 8]),
                second,
            ]);
            let (outcome, units) = inspect(&bytes);
            assert_eq!(outcome.terminal, Terminal::Complete);
            assert_eq!(
                reconstruction(&units[0]),
                AacReconstruction {
                    first_sample_duration: 1024,
                    all_fragments_default_base_is_moof: first_explicit && second_explicit,
                    first_sequence: 6,
                    last_sequence: 7,
                    movie_duration: HeaderDuration::Zero,
                    track_duration: HeaderDuration::Zero,
                    media_duration: HeaderDuration::Zero,
                }
            );
            assert_eq!(units[0].facts.end, bytes.len() as u64);
            assert_eq!(units[0].facts.clock.end, 5120);
            assert!(units[0].codec_packet_validation_pending);
        }
        let mut absolute = fixture(false, 0);
        let at = location(&absolute, b"tfhd");
        absolute[at + 11] |= 1;
        assert_eq!(failure(&absolute), Failure::Unsupported);
    }

    #[test]
    fn reconstruction_header_durations_use_each_versions_actual_width() {
        for version in [0, 1] {
            let sentinel = if version == 0 {
                u32::MAX as u64
            } else {
                u64::MAX
            };
            for slot in 0..3 {
                for (value, expected) in [
                    (0, HeaderDuration::Zero),
                    (sentinel, HeaderDuration::UnknownSentinel),
                    (12345, HeaderDuration::Finite(12345)),
                ] {
                    let mut durations = [0; 3];
                    durations[slot] = value;
                    let bytes = joined(&[
                        fixture_init_durations(false, false, version, durations),
                        media(0, 1, 3),
                    ]);
                    let (outcome, units) = inspect(&bytes);
                    assert_eq!(outcome.terminal, Terminal::Complete);
                    let facts = reconstruction(&units[0]);
                    let actual = [
                        facts.movie_duration,
                        facts.track_duration,
                        facts.media_duration,
                    ];
                    let mut want = [HeaderDuration::Zero; 3];
                    want[slot] = expected;
                    assert_eq!(actual, want, "version {version}, slot {slot}");
                    assert_eq!(units[0].facts.clock.end, 3072);
                    assert!(units[0].codec_packet_validation_pending);
                }
            }
        }
        // A 32-bit all-ones value is finite when stored in a 64-bit duration.
        let bytes = joined(&[
            fixture_init_durations(false, false, 1, [u32::MAX as u64; 3]),
            media(0, 1, 3),
        ]);
        let (outcome, units) = inspect(&bytes);
        assert_eq!(outcome.terminal, Terminal::Complete);
        let facts = reconstruction(&units[0]);
        assert_eq!(
            facts.movie_duration,
            HeaderDuration::Finite(u32::MAX as u64)
        );
        assert_eq!(facts.track_duration, facts.movie_duration);
        assert_eq!(facts.media_duration, facts.movie_duration);
    }

    #[test]
    fn reconstruction_sequences_and_flags_reset_at_repeated_init_not_media_time() {
        let first = joined(&[fixture_init(false, false), media(0, 9, 3)]);
        let mut second = fixture(false, 3072);
        let at = location(&second, b"tfhd");
        second[at + 9] = 0;
        let (outcome, units) = inspect(&joined(&[first.clone(), second.clone()]));
        assert_eq!(outcome.terminal, Terminal::Complete);
        assert_eq!(units[1].init_relation, InitRelation::IdenticalValidatedInit);
        assert_eq!(units[1].clock_relation, ClockRelation::Contiguous);
        assert_eq!(reconstruction(&units[0]).first_sequence, 9);
        assert_eq!(reconstruction(&units[0]).last_sequence, 9);
        assert!(reconstruction(&units[0]).all_fragments_default_base_is_moof);
        assert_eq!(reconstruction(&units[1]).first_sequence, 1);
        assert_eq!(reconstruction(&units[1]).last_sequence, 1);
        assert!(!reconstruction(&units[1]).all_fragments_default_base_is_moof);
        let (outcome, units) = inspect(&joined(&[first, second[..second.len() - 1].to_vec()]));
        assert!(matches!(outcome.terminal, Terminal::PartialTail { .. }));
        assert_eq!(units.len(), 1);
        assert!(units[0].codec_packet_validation_pending);
    }

    #[test]
    fn complete_aac_unit_preserves_integer_ticks_and_raw_boundaries() {
        let bytes = fixture(false, 2_063_879_393);
        let (outcome, units) = inspect(&bytes);
        assert_eq!(outcome.terminal, Terminal::Complete);
        assert_eq!(units.len(), 1);
        let unit = &units[0];
        assert_eq!((unit.facts.start, unit.facts.end), (0, bytes.len() as u64));
        assert_eq!(unit.facts.init_end, fixture_init(false, false).len() as u64);
        assert_eq!(unit.facts.clock.first, 2_063_879_393);
        assert_eq!(unit.facts.clock.end, 2_063_882_465);
        assert_eq!(unit.facts.items, 3);
        assert!(unit.codec_packet_validation_pending);
    }
    #[test]
    fn identical_and_padded_inits_are_separate_with_unproved_difference() {
        let bytes = joined(&[fixture(false, 0), fixture(false, 3072), fixture(true, 6144)]);
        let (outcome, units) = inspect(&bytes);
        assert_eq!(outcome.terminal, Terminal::Complete);
        assert_eq!(units.len(), 3);
        assert_eq!(units[1].init_relation, InitRelation::IdenticalValidatedInit);
        assert_eq!(units[2].init_relation, InitRelation::UnprovedInitDifference);
        assert_eq!(units[2].clock_relation, ClockRelation::Contiguous);
        assert_ne!(units[1].facts.init_sha256, units[2].facts.init_sha256);
        assert_eq!(units[1].configuration_sha256, units[2].configuration_sha256);
    }
    #[test]
    fn fragments_require_contiguous_clocks_and_sequences_inside_unit() {
        let init = fixture_init(false, false);
        for (tick, sequence, kind) in [(3073, 2, Failure::Clock), (3072, 3, Failure::Malformed)] {
            assert_eq!(
                failure(&joined(&[
                    init.clone(),
                    media(0, 1, 3),
                    media(tick, sequence, 2)
                ])),
                kind
            );
        }
        assert_eq!(
            inspect(&joined(&[init, media(0, 1, 3), media(3072, 2, 2)]))
                .0
                .terminal,
            Terminal::Complete
        );
    }
    #[test]
    fn cross_unit_gap_and_reset_are_facts_not_repaired() {
        let (_, units) = inspect(&joined(&[
            fixture(false, 0),
            fixture(false, 4000),
            fixture(false, 0),
        ]));
        assert_eq!(units[1].clock_relation, ClockRelation::Gap);
        assert_eq!(units[2].clock_relation, ClockRelation::OverlapOrReset);
    }
    #[test]
    fn unsafe_dref_and_protection_are_not_recoverable_tails() {
        assert_eq!(
            failure(&joined(&[fixture_init(false, true), media(0, 1, 3)])),
            Failure::Unsafe
        );
        let mut bytes = fixture(false, 0);
        let at = location(&bytes, b"mp4a");
        bytes[at + 4..at + 8].copy_from_slice(b"enca");
        assert_eq!(failure(&bytes), Failure::Unsafe);
    }
    #[test]
    fn invalid_box_lengths_versions_and_nonempty_sample_tables_fail() {
        for variant in 0..4 {
            let mut bytes = fixture(false, 0);
            match variant {
                0 => {
                    let at = location(&bytes, b"tfdt");
                    bytes[at..at + 4].copy_from_slice(&9u32.to_be_bytes());
                }
                1 => {
                    let at = location(&bytes, b"tfdt");
                    bytes[at + 8] = 2;
                }
                2 => {
                    let at = location(&bytes, b"stts");
                    bytes[at + 15] = 1;
                }
                _ => {
                    let at = location(&bytes, b"tfhd");
                    bytes[at + 15] = 8;
                }
            }
            assert!(matches!(
                failure(&bytes),
                Failure::Malformed | Failure::Unsupported
            ));
        }
    }
    #[test]
    fn extents_must_exhaust_mdat_without_gap_overlap_or_extra_bytes() {
        for variant in 0..3 {
            let mut bytes = fixture(false, 0);
            let at = location(&bytes, b"trun");
            if variant < 2 {
                let offset = u32::from_be_bytes(bytes[at + 16..at + 20].try_into().unwrap());
                bytes[at + 16..at + 20].copy_from_slice(
                    &(if variant == 0 { offset + 1 } else { offset - 1 }).to_be_bytes(),
                );
            } else {
                let at = location(&bytes, b"mdat");
                let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
                bytes[at..at + 4].copy_from_slice(&(length + 1).to_be_bytes());
                bytes.push(0);
            }
            assert_eq!(failure(&bytes), Failure::Malformed);
        }
    }
    #[test]
    fn exact_eof_shortage_is_partial_but_size_zero_is_unsupported() {
        let bytes = fixture(false, 0);
        assert!(matches!(
            inspect(&bytes[..bytes.len() - 1]).0.terminal,
            Terminal::PartialTail { .. }
        ));
        let mut zero = bytes.clone();
        let at = location(&zero, b"mdat");
        zero[at..at + 4].fill(0);
        assert_eq!(failure(&zero), Failure::Unsupported);
    }
    #[test]
    fn complete_padding_between_init_and_fragments_preserves_init_and_ticks() {
        let init = fixture_init(false, false);
        let mut padding = vec![0; 328];
        padding[..8].copy_from_slice(b"\0\0\0\x08ftyp");
        let bytes = joined(&[
            init.clone(),
            atom(b"skip", &padding),
            media(0, 1, 3),
            atom(b"free", &[0; 8]),
            media(3072, 2, 2),
        ]);
        let (outcome, units) = inspect(&bytes);
        assert_eq!(outcome.terminal, Terminal::Complete);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].facts.init_end, init.len() as u64);
        let expected_init_hash: [u8; 32] = Sha256::digest(&init).into();
        assert_eq!(units[0].facts.init_sha256, expected_init_hash);
        assert_eq!(units[0].facts.end, bytes.len() as u64);
        assert_eq!(units[0].facts.items, 5);
        assert_eq!(units[0].facts.fragments, 2);
        assert_eq!(units[0].facts.clock.end, 5120);
    }
    #[test]
    fn padding_body_shortage_and_pending_eof_are_not_complete() {
        let init = fixture_init(false, false);
        let padding = atom(b"skip", &[0; 328]);
        for prefix in [init, fixture(false, 0)] {
            for tail in [
                padding[..padding.len() - 1].to_vec(),
                padding.clone(),
                joined(&[padding.clone(), atom(b"moof", &[0; 4])[..10].to_vec()]),
            ] {
                let (outcome, units) = inspect(&joined(&[prefix.clone(), tail]));
                assert!(matches!(outcome.terminal, Terminal::PartialTail { .. }));
                assert!(units.is_empty());
            }
        }
    }
    #[test]
    fn padding_limits_apply_before_eof_shortage_and_across_boxes() {
        let init = fixture_init(false, false);
        let at_limit = atom(b"skip", &vec![0; MAX_PADDING_BYTES as usize - 8]);
        assert_eq!(
            inspect(&joined(&[init.clone(), at_limit.clone(), media(0, 1, 3)]))
                .0
                .terminal,
            Terminal::Complete
        );
        let mut oversized_header = (MAX_PADDING_BYTES as u32 + 1).to_be_bytes().to_vec();
        oversized_header.extend(b"skip");
        assert_eq!(
            failure(&joined(&[init.clone(), oversized_header])),
            Failure::Limit
        );
        assert_eq!(
            failure(&joined(&[
                init.clone(),
                at_limit,
                atom(b"free", &[]),
                media(0, 1, 3)
            ])),
            Failure::Limit
        );
        let padding = atom(b"free", &[]).repeat(MAX_PADDING_BOXES);
        let complete = joined(&[init.clone(), padding.clone(), media(0, 1, 3)]);
        assert_eq!(inspect(&complete).0.terminal, Terminal::Complete);
        assert_eq!(
            failure(&joined(&[
                init,
                padding,
                atom(b"skip", &[]),
                media(0, 1, 3)
            ])),
            Failure::Limit
        );
        assert_eq!(
            inspect(&joined(&[complete.clone(), complete])).0.terminal,
            Terminal::Complete
        );
    }
    #[test]
    fn padding_cannot_separate_moof_from_its_mdat() {
        let mut bytes = fixture(false, 0);
        let at = location(&bytes, b"mdat");
        bytes.splice(at..at, atom(b"skip", &[0; 8]));
        assert_eq!(failure(&bytes), Failure::Unsupported);
    }
    #[test]
    fn complete_unit_remains_provisional_when_successor_moov_is_partial_or_invalid() {
        let first = fixture(false, 0);
        let next_init = fixture_init(false, false);
        let moov = location(&next_init, b"moov");
        let bytes = joined(&[first.clone(), next_init[..moov + 12].to_vec()]);
        let (outcome, units) = inspect(&bytes);
        assert_eq!(outcome.complete_units, 1);
        assert_eq!(units[0].facts.end, first.len() as u64);
        assert!(matches!(outcome.terminal, Terminal::PartialTail { .. }));
        let invalid = joined(&[
            first.clone(),
            next_init[..moov].to_vec(),
            atom(b"pssh", &[0; 4]),
        ]);
        let (outcome, units) = inspect(&invalid);
        assert_eq!(outcome.complete_units, 1);
        assert_eq!(units.len(), 1);
        assert!(matches!(
            outcome.terminal,
            Terminal::Failed(super::super::Error {
                kind: Failure::Unsafe,
                ..
            })
        ));
        let (outcome, units) = inspect(&joined(&[first, next_init[..moov - 1].to_vec()]));
        assert_eq!(outcome.complete_units, 0);
        assert!(units.is_empty());
        assert!(matches!(outcome.terminal, Terminal::PartialTail { .. }));
    }
    #[test]
    fn one_byte_reads_and_sixty_four_bit_top_headers_are_supported() {
        struct Short(Cursor<Vec<u8>>);
        impl Read for Short {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                let n = b.len().min(1);
                self.0.read(&mut b[..n])
            }
        }
        impl Seek for Short {
            fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
                self.0.seek(p)
            }
        }
        let mut bytes = fixture(false, 0);
        let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as u64;
        bytes[..4].copy_from_slice(&1u32.to_be_bytes());
        bytes.splice(8..8, (length + 8).to_be_bytes());
        let expected = inspect(&bytes);
        let mut actual = Vec::new();
        let result = scan(
            Format::AacMp4,
            &mut Short(Cursor::new(bytes.clone())),
            bytes.len() as u64,
            44_100,
            &AtomicBool::new(false),
            &mut |u| {
                actual.push(u);
                Ok(())
            },
        );
        assert_eq!(result, expected.0);
        assert_eq!(actual, expected.1);
        assert_eq!(result.terminal, Terminal::Complete);
    }
    #[test]
    fn cancellation_and_sink_failure_never_become_complete() {
        let bytes = fixture(false, 0);
        let cancelled = scan(
            Format::AacMp4,
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            44_100,
            &AtomicBool::new(true),
            &mut |_| Ok(()),
        );
        assert!(matches!(
            cancelled.terminal,
            Terminal::Failed(super::super::Error {
                kind: Failure::Cancelled,
                ..
            })
        ));
        let sink = scan(
            Format::AacMp4,
            &mut Cursor::new(&bytes),
            bytes.len() as u64,
            44_100,
            &AtomicBool::new(false),
            &mut |_| Err(()),
        );
        assert!(matches!(
            sink.terminal,
            Terminal::Failed(super::super::Error {
                kind: Failure::Sink,
                ..
            })
        ));
    }
}
