//! Small structural WebM subset. Encoded blocks stay opaque; no decoder or mux.
use super::*;
use std::io::{Read, Seek};

const EBML: u64 = 0x1a45dfa3;
const SEGMENT: u64 = 0x18538067;
const INFO: u64 = 0x1549a966;
const TRACKS: u64 = 0x1654ae6b;
const CLUSTER: u64 = 0x1f43b675;
#[derive(Clone, Copy, Debug)]
struct Element {
    id: u64,
    at: u64,
    body: u64,
    end: Option<u64>,
}

fn width(first: u8, maximum: u8, at: u64) -> Result<u8> {
    let n = first.leading_zeros() as u8 + 1;
    if n > maximum {
        Err(error(Failure::Malformed, at))
    } else {
        Ok(n)
    }
}
fn vint(bytes: &[u8], identifier: bool, at: u64) -> Result<(u64, usize, bool)> {
    let first = *bytes.first().ok_or(error(Failure::Malformed, at))?;
    let n = width(first, if identifier { 4 } else { 8 }, at)? as usize;
    if bytes.len() < n {
        return Err(error(Failure::Malformed, at));
    }
    let mut value = if identifier {
        first as u64
    } else {
        (first as u64) & (0xffu64 >> n)
    };
    for byte in &bytes[1..n] {
        value = (value << 8) | *byte as u64;
    }
    let unknown = !identifier && value == ((1u64 << (7 * n)) - 1);
    Ok((value, n, unknown))
}
fn header<R: Read + Seek>(r: &mut Input<'_, R>, at: u64, parent: Option<u64>) -> Result<Element> {
    if parent.is_some_and(|end| at >= end) {
        return Err(error(Failure::Malformed, at));
    }
    let first = r.bytes(at, 1)?[0];
    let iw = width(first, 4, at)? as usize;
    if parent.is_some_and(|end| at.checked_add(iw as u64).is_none_or(|v| v > end)) {
        return Err(error(Failure::Malformed, at));
    }
    let id = vint(&r.bytes(at, iw)?, true, at)?.0;
    let size_at = at
        .checked_add(iw as u64)
        .ok_or(error(Failure::Malformed, at))?;
    if parent.is_some_and(|end| size_at >= end) {
        return Err(error(Failure::Malformed, at));
    }
    let size_first = r.bytes(size_at, 1)?[0];
    let sw = width(size_first, 8, size_at)? as usize;
    if parent.is_some_and(|end| size_at.checked_add(sw as u64).is_none_or(|v| v > end)) {
        return Err(error(Failure::Malformed, at));
    }
    let (size, _, unknown) = vint(&r.bytes(size_at, sw)?, false, size_at)?;
    let body = size_at
        .checked_add(sw as u64)
        .ok_or(error(Failure::Malformed, at))?;
    if parent.is_some_and(|end| body > end) {
        return Err(error(Failure::Malformed, at));
    }
    if unknown && !matches!(id, SEGMENT | CLUSTER) {
        return Err(error(Failure::Unsupported, at));
    }
    let end = if unknown {
        None
    } else {
        Some(
            body.checked_add(size)
                .ok_or(error(Failure::Malformed, at))?,
        )
    };
    if matches!(
        id,
        0x6d80 | 0x6240 | 0x5034 | 0x5035 | 0x3cb923 | 0x3eb923 | 0x3c83ab | 0x3e83bb | 0x1941a469
    ) {
        return Err(error(Failure::Unsafe, at));
    }
    if !matches!(
        id,
        EBML | SEGMENT
            | INFO
            | TRACKS
            | CLUSTER
            | 0xa1
            | 0xa3
            | 0xa0
            | 0xe7
            | 0x9b
            | 0xfb
            | 0xec
            | 0xbf
            | 0xa7
            | 0xab
    ) {
        return Err(error(Failure::Unsupported, at));
    }
    let maximum = match id {
        EBML | INFO | TRACKS => METADATA as u64,
        0xa1 | 0xa3 => MAX_ITEM,
        0xa0 => MAX_ITEM + METADATA as u64,
        _ => MAX_RAW,
    };
    if !unknown && (size > maximum || end.is_some_and(|end| end > MAX_RAW)) {
        return Err(error(Failure::Limit, at));
    }
    if id == 0xbf && size != 4 {
        return Err(error(Failure::Malformed, at));
    }
    if end.zip(parent).is_some_and(|(end, parent)| end > parent) {
        return Err(error(Failure::Malformed, at));
    }
    if end.is_some_and(|end| end > r.len) {
        return Err(error(Failure::Tail, at));
    }
    Ok(Element { id, at, body, end })
}
fn end(e: Element) -> Result<u64> {
    e.end.ok_or(error(Failure::Unsupported, e.at))
}
fn payload<R: Read + Seek>(r: &mut Input<'_, R>, e: Element, limit: usize) -> Result<Vec<u8>> {
    let length = end(e)? - e.body;
    if length > limit as u64 {
        return Err(error(Failure::Limit, e.at));
    }
    r.bytes(e.body, length as usize)
}
fn uint(bytes: &[u8], at: u64) -> Result<u64> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(error(Failure::Malformed, at));
    }
    Ok(bytes.iter().fold(0u64, |n, b| (n << 8) | *b as u64))
}
fn signed(bytes: &[u8], at: u64) -> Result<i64> {
    let value = uint(bytes, at)?;
    let shift = 64 - bytes.len() * 8;
    Ok(((value << shift) as i64) >> shift)
}

/// Metadata is bounded before allocation; every leaf is inside its own parent.
struct Meta<'a> {
    bytes: &'a [u8],
    base: u64,
    nodes: usize,
}
#[derive(Clone, Copy)]
struct Node<'a> {
    id: u64,
    bytes: &'a [u8],
    at: u64,
}
impl<'a> Meta<'a> {
    fn children(&mut self, bytes: &'a [u8], base: u64, depth: usize) -> Result<Vec<Node<'a>>> {
        if depth > 8 {
            return Err(error(Failure::Limit, base));
        }
        let mut at = 0usize;
        let mut values = Vec::new();
        while at < bytes.len() {
            self.nodes += 1;
            if self.nodes > 512 {
                return Err(error(Failure::Limit, base + at as u64));
            }
            let (id, iw, _) = vint(&bytes[at..], true, base + at as u64)?;
            let size_at = at.checked_add(iw).ok_or(error(Failure::Malformed, base))?;
            let (size, sw, unknown) = vint(
                bytes
                    .get(size_at..)
                    .ok_or(error(Failure::Malformed, base))?,
                false,
                base + size_at as u64,
            )?;
            if unknown {
                return Err(error(Failure::Unsupported, base + at as u64));
            }
            let body = size_at
                .checked_add(sw)
                .ok_or(error(Failure::Malformed, base))?;
            let finish = body
                .checked_add(usize::try_from(size).map_err(|_| error(Failure::Limit, base))?)
                .ok_or(error(Failure::Malformed, base))?;
            let value = bytes
                .get(body..finish)
                .ok_or(error(Failure::Malformed, base + at as u64))?;
            values.push(Node {
                id,
                bytes: value,
                at: base + body as u64,
            });
            at = finish;
        }
        Ok(values)
    }
    fn root(&mut self) -> Result<Vec<Node<'a>>> {
        self.children(self.bytes, self.base, 0)
    }
}
fn unique<'a>(nodes: &[Node<'a>], id: u64, at: u64) -> Result<Option<Node<'a>>> {
    let mut found = nodes.iter().filter(|node| node.id == id);
    let value = found.next().copied();
    if found.next().is_some() {
        return Err(error(Failure::Malformed, at));
    }
    Ok(value)
}
fn known(nodes: &[Node<'_>], allowed: &[u64]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for n in nodes {
        if n.id != 0xec && !seen.insert(n.id) {
            return Err(error(Failure::Malformed, n.at));
        }
        if n.id == 0xbf && n.bytes.len() != 4 {
            return Err(error(Failure::Malformed, n.at));
        }
        if !allowed.contains(&n.id) {
            let unsafe_id = matches!(
                n.id,
                0x6d80
                    | 0x6240
                    | 0x5034
                    | 0x5035
                    | 0x3cb923
                    | 0x3eb923
                    | 0x3c83ab
                    | 0x3e83bb
                    | 0x4444
                    | 0x1ba5387a
                    | 0x1941a469
            );
            return Err(error(
                if unsafe_id {
                    Failure::Unsafe
                } else {
                    Failure::Unsupported
                },
                n.at,
            ));
        }
    }
    Ok(())
}
fn number(nodes: &[Node<'_>], id: u64, default: Option<u64>, at: u64) -> Result<u64> {
    match unique(nodes, id, at)? {
        Some(n) => uint(n.bytes, n.at),
        None => default.ok_or(error(Failure::Malformed, at)),
    }
}
fn ebml_header<R: Read + Seek>(
    r: &mut Input<'_, R>,
    e: Element,
    nodes_used: &mut usize,
) -> Result<()> {
    if e.id != EBML {
        return Err(error(Failure::Malformed, e.at));
    }
    let data = payload(r, e, METADATA)?;
    let mut m = Meta {
        bytes: &data,
        base: e.body,
        nodes: *nodes_used,
    };
    let nodes = m.root()?;
    *nodes_used = m.nodes;
    known(
        &nodes,
        &[0x4286, 0x42f7, 0x42f2, 0x42f3, 0x4282, 0x4287, 0x4285, 0xec],
    )?;
    if unique(&nodes, 0x4282, e.at)?.is_none_or(|n| n.bytes != b"webm")
        || number(&nodes, 0x4286, Some(1), e.at)? != 1
        || number(&nodes, 0x42f7, Some(1), e.at)? != 1
        || number(&nodes, 0x42f2, Some(4), e.at)? != 4
        || number(&nodes, 0x42f3, Some(8), e.at)? != 8
        || !(1..=4).contains(&number(&nodes, 0x4287, Some(1), e.at)?)
        || !(1..=2).contains(&number(&nodes, 0x4285, Some(1), e.at)?)
    {
        return Err(error(Failure::Unsupported, e.at));
    }
    Ok(())
}
fn info<R: Read + Seek>(
    r: &mut Input<'_, R>,
    e: Element,
    nodes_used: &mut usize,
) -> Result<(u64, bool)> {
    let data = payload(r, e, METADATA)?;
    let mut m = Meta {
        bytes: &data,
        base: e.body,
        nodes: *nodes_used,
    };
    let nodes = m.root()?;
    *nodes_used = m.nodes;
    known(
        &nodes,
        &[0x2ad7b1, 0x4489, 0x4461, 0x4d80, 0x5741, 0x73a4, 0xec, 0xbf],
    )?;
    let scale = number(&nodes, 0x2ad7b1, Some(1_000_000), e.at)?;
    if scale == 0 || scale > 1_000_000_000 {
        return Err(error(Failure::Clock, e.at));
    }
    Ok((scale, nodes.iter().any(|node| node.id == 0x4489)))
}
fn tracks<R: Read + Seek>(
    r: &mut Input<'_, R>,
    e: Element,
    scale: u64,
    nodes_used: &mut usize,
) -> Result<Configuration> {
    let data = payload(r, e, METADATA)?;
    let mut m = Meta {
        bytes: &data,
        base: e.body,
        nodes: *nodes_used,
    };
    let nodes = m.root()?;
    known(&nodes, &[0xae, 0xec, 0xbf])?;
    let track = unique(&nodes, 0xae, e.at)?.ok_or(error(Failure::Malformed, e.at))?;
    let fields = m.children(track.bytes, track.at, 1)?;
    known(
        &fields,
        &[
            0xd7, 0x73c5, 0x83, 0xb9, 0x88, 0x55aa, 0x9c, 0x23e383, 0x23314f, 0x56aa, 0x56bb, 0x86,
            0x63a2, 0xe0, 0x536e, 0x22b59c, 0x22b59d, 0xec, 0xbf,
        ],
    )?;
    let id = number(&fields, 0xd7, None, e.at)?;
    if id == 0
        || number(&fields, 0x83, None, e.at)? != 1
        || number(&fields, 0xb9, Some(1), e.at)? != 1
        || number(&fields, 0x56aa, Some(0), e.at)? != 0
        || number(&fields, 0x56bb, Some(0), e.at)? != 0
        || unique(&fields, 0x86, e.at)?.is_none_or(|n| n.bytes != b"V_VP9")
    {
        return Err(error(Failure::Unsupported, e.at));
    }
    if let Some(n) = unique(&fields, 0x23314f, e.at)? {
        let one = match n.bytes.len() {
            4 => {
                f32::from_be_bytes(
                    n.bytes
                        .try_into()
                        .map_err(|_| error(Failure::Malformed, n.at))?,
                ) == 1.0
            }
            8 => {
                f64::from_be_bytes(
                    n.bytes
                        .try_into()
                        .map_err(|_| error(Failure::Malformed, n.at))?,
                ) == 1.0
            }
            _ => false,
        };
        if !one {
            return Err(error(Failure::Unsupported, n.at));
        }
    }
    let default = unique(&fields, 0x23e383, e.at)?
        .map(|n| uint(n.bytes, n.at))
        .transpose()?;
    if default.is_some_and(|v| v == 0 || v > 1_000_000_000) {
        return Err(error(Failure::Unsupported, e.at));
    }
    let video = unique(&fields, 0xe0, e.at)?.ok_or(error(Failure::Malformed, e.at))?;
    let video_fields = m.children(video.bytes, video.at, 2)?;
    known(
        &video_fields,
        &[
            0xb0, 0xba, 0x54b0, 0x54ba, 0x54b2, 0x54b3, 0x54aa, 0x54bb, 0x54cc, 0x54dd, 0x53b8,
            0x9a, 0x9d, 0x55b0, 0xec, 0xbf,
        ],
    )?;
    let w = number(&video_fields, 0xb0, None, e.at)?;
    let h = number(&video_fields, 0xba, None, e.at)?;
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(error(Failure::Unsupported, e.at));
    }
    if let Some(colour) = unique(&video_fields, 0x55b0, e.at)? {
        let colours = m.children(colour.bytes, colour.at, 3)?;
        known(
            &colours,
            &[
                0x55b1, 0x55b2, 0x55b3, 0x55b4, 0x55b5, 0x55b6, 0x55b7, 0x55b8, 0x55b9, 0x55ba,
                0x55bb, 0x55bc, 0x55bd,
            ],
        )?;
        for field in colours {
            let _ = uint(field.bytes, field.at)?;
        }
    }
    let mut semantic = Sha256::new();
    semantic.update(b"webm-container-v1\0");
    // Decoder/display fields remain byte-exact here. A different representation
    // can conservatively produce a different fingerprint, never false equality.
    for field in &fields {
        if matches!(
            field.id,
            0x73c5 | 0x63a2 | 0xe0 | 0x9c | 0x23314f | 0x56aa | 0x56bb
        ) {
            if field.bytes.len() > 4096 {
                return Err(error(Failure::Limit, field.at));
            }
            semantic.update(field.id.to_be_bytes());
            semantic.update((field.bytes.len() as u64).to_be_bytes());
            semantic.update(field.bytes);
        }
    }
    *nodes_used = m.nodes;
    Ok(Configuration {
        format: Format::Vp9Webm,
        track_id: id,
        timebase: scale,
        width: w as u32,
        height: h as u32,
        default_duration_ns: default,
        sample_rate: 0,
        channels: 0,
        semantic_sha256: semantic.finalize().into(),
    })
}

/// Initial metadata for a future raw writer. Existing full-unit scanners keep
/// their own acceptance; this stricter subset matches reconstructible starts.
pub(super) fn initialization<R: Read + Seek>(r: &mut Input<'_, R>) -> Result<Configuration> {
    r.checkpoint(0)?;
    let mut metadata_nodes = 0;
    let ebml = header(r, 0, None)?;
    ebml_header(r, ebml, &mut metadata_nodes)?;
    let segment = header(r, end(ebml)?, None)?;
    if segment.id != SEGMENT || segment.end.is_some() {
        return Err(error(Failure::Unsupported, segment.at));
    }
    let mut at = segment.body;
    let mut scale = None;
    let mut config = None;
    while at < r.len {
        r.checkpoint(at)?;
        let child = header(r, at, None)?;
        match child.id {
            INFO => {
                if scale.is_some() || config.is_some() {
                    return Err(error(Failure::Malformed, at));
                }
                let (value, duration) = info(r, child, &mut metadata_nodes)?;
                if duration {
                    return Err(error(Failure::Unsupported, at));
                }
                scale = Some(value);
            }
            TRACKS => {
                if config.is_some() {
                    return Err(error(Failure::Malformed, at));
                }
                config = Some(tracks(
                    r, child, scale.ok_or(error(Failure::Malformed, at))?, &mut metadata_nodes,
                )?);
            }
            0xec => (),
            // In particular no Cluster or Segment CRC is begin metadata.
            _ => return Err(error(Failure::Unsupported, at)),
        }
        metadata_nodes += 1;
        if metadata_nodes > 512 {
            return Err(error(Failure::Limit, at));
        }
        at = end(child)?;
    }
    r.checkpoint(at)?;
    config.ok_or(error(Failure::Malformed, at))
}

#[derive(Default)]
struct Stats {
    count: u64,
    first: Option<i128>,
    last: Option<i128>,
    end: i128,
    rap: bool,
    references: u64,
    clusters: u64,
}
struct Block {
    ticks: i128,
    rap: bool,
    at: u64,
    payload_start: u64,
    payload_end: u64,
}
fn block<R: Read + Seek>(
    r: &mut Input<'_, R>,
    e: Element,
    config: &Configuration,
    timestamp: u64,
    simple: bool,
) -> Result<Block> {
    let length = end(e)? - e.body;
    if length < 5 {
        return Err(error(Failure::Malformed, e.at));
    }
    if length > MAX_ITEM {
        return Err(error(Failure::Limit, e.at));
    }
    let prefix = r.bytes(e.body, length.min(12) as usize)?;
    let (track, n, unknown) = vint(&prefix, false, e.body)?;
    if unknown || track != config.track_id || length <= n as u64 + 3 {
        return Err(error(Failure::Malformed, e.at));
    }
    let relative = i16::from_be_bytes([prefix[n], prefix[n + 1]]) as i128;
    let flags = prefix[n + 2];
    if flags & 6 != 0 {
        return Err(error(Failure::Unsupported, e.at));
    }
    if flags & if simple { 0x70 } else { 0xf1 } != 0 {
        return Err(error(Failure::Malformed, e.at));
    }
    let ticks = (timestamp as i128)
        .checked_add(relative)
        .ok_or(error(Failure::Clock, e.at))?;
    if ticks < 0 {
        return Err(error(Failure::Clock, e.at));
    }
    r.checkpoint(e.at)?;
    Ok(Block {
        ticks,
        rap: simple && flags & 0x80 != 0,
        at: e.at,
        payload_start: e.body + n as u64 + 3,
        payload_end: end(e)?,
    })
}
fn inspect<R: Read + Seek>(
    r: &mut Input<'_, R>,
    block: &Block,
    config: &Configuration,
    observer: &mut Option<&mut super::super::vp9_admission::Frames>,
) -> Result<()> {
    if let Some(observer) = observer.as_deref_mut() {
        observer.packet(r, config, block.payload_start, block.payload_end, block.rap)?;
    }
    Ok(())
}
fn record(
    stats: &mut Stats,
    b: Block,
    duration: Option<u64>,
    references: u64,
    config: &Configuration,
) -> Result<()> {
    let ns = b
        .ticks
        .checked_mul(config.timebase as i128)
        .ok_or(error(Failure::Clock, b.at))?;
    let duration = match duration {
        Some(ticks) => ticks.checked_mul(config.timebase),
        None => config.default_duration_ns,
    }
    .filter(|n| *n > 0)
    .ok_or(error(Failure::Unsupported, b.at))? as i128;
    let end = ns
        .checked_add(duration)
        .ok_or(error(Failure::Clock, b.at))?;
    if stats.last.is_some_and(|v| ns < v) {
        return Err(error(Failure::Clock, b.at));
    }
    stats.count = stats
        .count
        .checked_add(1)
        .filter(|n| *n <= MAX_ITEMS)
        .ok_or(error(Failure::Limit, b.at))?;
    if stats.first.is_none() {
        stats.first = Some(ns);
        stats.rap = b.rap;
    }
    stats.last = Some(ns);
    stats.end = stats.end.max(end);
    stats.references = stats
        .references
        .checked_add(references)
        .ok_or(error(Failure::Limit, b.at))?;
    Ok(())
}
fn group<R: Read + Seek>(
    r: &mut Input<'_, R>,
    e: Element,
    config: &Configuration,
    timestamp: u64,
    stats: &mut Stats,
    nodes: &mut usize,
    observer: &mut Option<&mut super::super::vp9_admission::Frames>,
) -> Result<()> {
    let finish = end(e)?;
    let mut at = e.body;
    let mut found = None;
    let mut duration = None;
    let mut refs = 0u64;
    while at < finish {
        *nodes += 1;
        if *nodes > 512 {
            return Err(error(Failure::Limit, at));
        }
        let child = header(r, at, Some(finish))?;
        match child.id {
            0xa1 => {
                if found.is_some() {
                    return Err(error(Failure::Malformed, at));
                }
                found = Some(block(r, child, config, timestamp, false)?);
            }
            0x9b => {
                if duration.is_some() {
                    return Err(error(Failure::Malformed, at));
                }
                duration = Some(uint(&payload(r, child, 8)?, at)?);
            }
            0xfb => {
                let _ = signed(&payload(r, child, 8)?, at)?;
                refs += 1;
                if refs > 8 {
                    return Err(error(Failure::Limit, at));
                }
            }
            0xec => {}
            _ => return Err(error(Failure::Unsupported, at)),
        }
        at = end(child)?;
    }
    let mut found = found.ok_or(error(Failure::Malformed, e.at))?;
    found.rap = refs == 0;
    inspect(r, &found, config, observer)?;
    record(stats, found, duration, refs, config)
}
fn cluster<R: Read + Seek>(
    r: &mut Input<'_, R>,
    e: Element,
    segment_end: Option<u64>,
    config: &Configuration,
    stats: &mut Stats,
    reconstruction: &mut WebmReconstruction,
    observer: &mut Option<&mut super::super::vp9_admission::Frames>,
) -> Result<u64> {
    let parent = e.end.or(segment_end);
    let finish = parent.unwrap_or(r.len);
    let mut at = e.body;
    let mut timestamp = None;
    let mut nodes = 0;
    stats.clusters = stats
        .clusters
        .checked_add(1)
        .filter(|n| *n <= MAX_ITEMS)
        .ok_or(error(Failure::Limit, e.at))?;
    while at < finish {
        nodes += 1;
        if nodes > 512 {
            return Err(error(Failure::Limit, at));
        }
        r.checkpoint(at)?;
        let child = header(r, at, parent)?;
        if e.end.is_none() && matches!(child.id, CLUSTER | EBML) {
            if timestamp.is_none() {
                return Err(error(Failure::Malformed, e.at));
            }
            return Ok(at);
        }
        match child.id {
            0xe7 => {
                if timestamp.is_some() {
                    return Err(error(Failure::Malformed, at));
                }
                timestamp = Some(uint(&payload(r, child, 8)?, at)?);
            }
            0xa3 => {
                let t = timestamp.ok_or(error(Failure::Malformed, at))?;
                let b = block(r, child, config, t, true)?;
                inspect(r, &b, config, observer)?;
                record(stats, b, None, 0, config)?;
            }
            0xa0 => group(
                r,
                child,
                config,
                timestamp.ok_or(error(Failure::Malformed, at))?,
                stats,
                &mut nodes,
                observer,
            )?,
            0xec | 0xbf | 0xa7 | 0xab => {
                let size = end(child)? - child.body;
                if size > METADATA as u64 {
                    return Err(error(Failure::Limit, at));
                }
                match child.id {
                    0xbf if e.end.is_none() => reconstruction.unknown_cluster_crc = true,
                    0xbf => reconstruction.finite_cluster_crc = true,
                    0xa7 => reconstruction.cluster_position = true,
                    0xab => reconstruction.cluster_prev_size = true,
                    _ => {}
                }
            }
            _ => return Err(error(Failure::Unsupported, at)),
        }
        at = end(child)?;
    }
    if timestamp.is_none() {
        return Err(error(Failure::Malformed, e.at));
    }
    Ok(at)
}
pub(super) fn run<R: Read + Seek>(
    r: &mut Input<'_, R>,
    emit: &mut Emit<'_>,
    expected: u64,
) -> Result<()> {
    run_observed(r, emit, expected, 0, &mut None)
}

/// Separate native header-proof path. The ordinary structural index passes no
/// observer and retains its original acceptance and pending-codec semantics.
pub(super) fn run_observed<R: Read + Seek>(
    r: &mut Input<'_, R>,
    emit: &mut Emit<'_>,
    expected: u64,
    start_offset: u64,
    observer: &mut Option<&mut super::super::vp9_admission::Frames>,
) -> Result<()> {
    let mut at = start_offset;
    while at < r.len {
        let start = at;
        let eh = header(r, at, None)?;
        let mut metadata_nodes = 0;
        ebml_header(r, eh, &mut metadata_nodes)?;
        at = end(eh)?;
        let segment = header(r, at, None)?;
        if segment.id != SEGMENT {
            return Err(error(Failure::Malformed, at));
        }
        at = segment.body;
        let mut scale = None;
        let mut config = None;
        let mut init_end = None;
        let mut stats = Stats::default();
        let mut reconstruction = WebmReconstruction {
            unknown_segment: segment.end.is_none(),
            ..WebmReconstruction::default()
        };
        let finish = segment.end.unwrap_or(r.len);
        let mut boundary = if segment.end.is_some() {
            Boundary::FiniteEnd
        } else {
            Boundary::CommittedEof
        };
        while at < finish {
            r.checkpoint(at)?;
            let child = header(r, at, segment.end)?;
            if child.id == EBML && segment.end.is_none() {
                // A syntactically complete EBML header establishes the prior
                // unknown segment boundary; the successor still needs its own
                // Info/Tracks validation and can subsequently fail the scan.
                ebml_header(r, child, &mut 0)?;
                boundary = Boundary::NextInitialization;
                break;
            }
            match child.id {
                INFO => {
                    if scale.is_some() || config.is_some() || init_end.is_some() {
                        return Err(error(Failure::Malformed, at));
                    }
                    let (value, duration) = info(r, child, &mut metadata_nodes)?;
                    if value != expected {
                        return Err(error(Failure::Clock, at));
                    }
                    scale = Some(value);
                    reconstruction.info_duration = duration;
                }
                TRACKS => {
                    if config.is_some() || init_end.is_some() {
                        return Err(error(Failure::Malformed, at));
                    }
                    config = Some(tracks(
                        r,
                        child,
                        scale.ok_or(error(Failure::Malformed, at))?,
                        &mut metadata_nodes,
                    )?);
                }
                CLUSTER => {
                    let c = config.as_ref().ok_or(error(Failure::Malformed, at))?;
                    init_end.get_or_insert(at);
                    at = cluster(r, child, segment.end, c, &mut stats, &mut reconstruction, observer)?;
                    continue;
                }
                0xec | 0xbf => {
                    if end(child)? - child.body > METADATA as u64 {
                        return Err(error(Failure::Limit, at));
                    }
                    reconstruction.segment_crc |= child.id == 0xbf;
                }
                _ => {
                    return Err(error(
                        if matches!(child.id, 0x1941a469 | 0x3cb923 | 0x3eb923) {
                            Failure::Unsafe
                        } else {
                            Failure::Unsupported
                        },
                        at,
                    ))
                }
            }
            metadata_nodes += 1;
            if metadata_nodes > 512 {
                return Err(error(Failure::Limit, at));
            }
            at = end(child)?;
            if init_end.is_none() && at - start > METADATA as u64 {
                return Err(error(Failure::Limit, start));
            }
        }
        let init_end = init_end.ok_or(error(Failure::Unsupported, start))?;
        let configuration = config.ok_or(error(Failure::Malformed, start))?;
        let first = stats.first.ok_or(error(Failure::Unsupported, start))?;
        let last = stats.last.ok_or(error(Failure::Unsupported, start))?;
        if init_end - start > METADATA as u64 {
            return Err(error(Failure::Limit, start));
        }
        let init_sha256 = r.hash_range(start, init_end)?;
        r.checkpoint(start)?;
        emit.unit(UnitDraft {
            start,
            init_end,
            end: at,
            boundary,
            init_sha256,
            configuration,
            clock: Clock {
                first,
                last,
                end: stats.end,
                ticks_per_second: 1_000_000_000,
            },
            items: stats.count,
            fragments: stats.clusters,
            rap_candidate: stats.rap,
            references: stats.references,
            reconstruction: ReconstructionFacts::Webm(reconstruction),
        })?;
    }
    Ok(())
}
