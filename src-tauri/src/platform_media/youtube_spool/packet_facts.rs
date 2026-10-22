//! Fixed FFprobe projection evidence only: no runner, file access or media admission.
use super::Track;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    InvalidRow,
    UnsupportedSideData,
    UnsupportedFlags,
    InvalidValue,
    Bounds,
    UnknownStream,
    Truncated,
    Empty,
    CountMismatch,
    PayloadMismatch,
    FlagsMismatch,
    SideDataMismatch,
    ArithmeticOverflow,
    Sink,
    Terminal,
}
type Result<T> = std::result::Result<T, Fault>;
const CHUNK: usize = 128 * 1024;
const LINE: usize = 2048;
const PACKET: u64 = 32 * 1024 * 1024;
pub(super) const ROWS: u64 = 20_000_000;
const TEXT: u64 = 4 * 1024 * 1024 * 1024;
const PAYLOAD: u64 = 128 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SkipSamples {
    skip: u32,
    discard: u32,
    skip_reason: u8,
    discard_reason: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PacketFact {
    track: Track,
    pts: i64,
    dts: i64,
    duration: i64,
    size: u64,
    key: bool,
    hash: [u8; 32],
    side: Option<SkipSamples>,
}

impl PacketFact {
    /// Versioned by the enclosing tape. Never serialize a Rust struct's layout.
    pub(super) fn encode_record(self) -> Result<[u8; 80]> {
        self.validate_record()?;
        let mut out = [0; 80];
        out[0] = self.track.index() as u8;
        out[1] = u8::from(self.key);
        out[2] = u8::from(self.side.is_some());
        out[4..12].copy_from_slice(&self.pts.to_be_bytes());
        out[12..20].copy_from_slice(&self.dts.to_be_bytes());
        out[20..28].copy_from_slice(&self.duration.to_be_bytes());
        out[28..36].copy_from_slice(&self.size.to_be_bytes());
        out[36..68].copy_from_slice(&self.hash);
        if let Some(side) = self.side {
            out[68..72].copy_from_slice(&side.skip.to_be_bytes());
            out[72..76].copy_from_slice(&side.discard.to_be_bytes());
            out[76] = side.skip_reason;
            out[77] = side.discard_reason;
        }
        Ok(out)
    }

    pub(super) fn decode_record(bytes: &[u8; 80]) -> Result<Self> {
        let track = match bytes[0] {
            0 => Track::Video,
            1 => Track::Audio,
            _ => return Err(Fault::UnknownStream),
        };
        if bytes[1] > 1 || bytes[2] > 1 || bytes[3] != 0 || bytes[78..] != [0, 0] {
            return Err(Fault::InvalidValue);
        }
        let side = match bytes[2] {
            0 if bytes[68..78].iter().all(|&byte| byte == 0) => None,
            0 => return Err(Fault::InvalidValue),
            _ => Some(SkipSamples {
                skip: u32::from_be_bytes(bytes[68..72].try_into().unwrap()),
                discard: u32::from_be_bytes(bytes[72..76].try_into().unwrap()),
                skip_reason: bytes[76],
                discard_reason: bytes[77],
            }),
        };
        let fact = Self {
            track,
            pts: i64::from_be_bytes(bytes[4..12].try_into().unwrap()),
            dts: i64::from_be_bytes(bytes[12..20].try_into().unwrap()),
            duration: i64::from_be_bytes(bytes[20..28].try_into().unwrap()),
            size: u64::from_be_bytes(bytes[28..36].try_into().unwrap()),
            key: bytes[1] == 1,
            hash: bytes[36..68].try_into().unwrap(),
            side,
        };
        fact.validate_record()?;
        Ok(fact)
    }

    fn validate_record(self) -> Result<()> {
        if self.pts == i64::MIN
            || self.dts == i64::MIN
            || self.duration <= 0
            || !(1..=PACKET).contains(&self.size)
        {
            Err(Fault::InvalidValue)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Streams([Option<(u8, Track)>; 2]);
impl Streams {
    pub(super) fn new(entries: &[(u8, Track)]) -> Result<Self> {
        if entries.is_empty() || entries.len() > 2 {
            return Err(Fault::Bounds);
        }
        let mut slots = [None; 2];
        for (i, &(id, track)) in entries.iter().enumerate() {
            if id > 31
                || entries[..i]
                    .iter()
                    .any(|&(other, kind)| other == id || kind == track)
            {
                return Err(Fault::InvalidValue);
            }
            slots[i] = Some((id, track));
        }
        Ok(Self(slots))
    }
    fn track(self, id: u8) -> Result<Track> {
        self.0
            .into_iter()
            .flatten()
            .find_map(|(candidate, track)| (candidate == id).then_some(track))
            .ok_or(Fault::UnknownStream)
    }
}

/// The callback consumes each fact synchronously. A terminal decoder cannot be reused.
pub(super) struct Decoder {
    streams: Streams,
    line: Vec<u8>,
    bytes: u64,
    rows: u64,
    payload_bytes: [u64; 2],
    terminal: bool,
    row_cap: u64,
    byte_cap: u64,
    payload_cap: u64,
}
impl Decoder {
    pub(super) fn new(streams: Streams) -> Self {
        Self {
            streams,
            line: Vec::with_capacity(LINE),
            bytes: 0,
            rows: 0,
            payload_bytes: [0; 2],
            terminal: false,
            row_cap: ROWS,
            byte_cap: TEXT,
            payload_cap: PAYLOAD,
        }
    }
    pub(super) fn feed(
        &mut self,
        bytes: &[u8],
        mut emit: impl FnMut(PacketFact) -> Result<()>,
    ) -> Result<()> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        let result = self.feed_inner(bytes, &mut emit);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }
    fn feed_inner(
        &mut self,
        bytes: &[u8],
        emit: &mut impl FnMut(PacketFact) -> Result<()>,
    ) -> Result<()> {
        if bytes.len() > CHUNK {
            return Err(Fault::Bounds);
        }
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= self.byte_cap)
            .ok_or(Fault::Bounds)?;
        for &byte in bytes {
            if self.line.len() == LINE {
                return Err(Fault::Bounds);
            }
            self.line.push(byte);
            if byte == b'\n' {
                self.rows = self
                    .rows
                    .checked_add(1)
                    .filter(|n| *n <= self.row_cap)
                    .ok_or(Fault::Bounds)?;
                let fact = parse(&self.line, self.streams)?;
                let total = &mut self.payload_bytes[fact.track.index()];
                *total = total
                    .checked_add(fact.size)
                    .filter(|n| *n <= self.payload_cap)
                    .ok_or(Fault::Bounds)?;
                emit(fact)?;
                self.line.clear();
            }
        }
        Ok(())
    }
    pub(super) fn finish(&mut self) -> Result<u64> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        self.terminal = true;
        if !self.line.is_empty() {
            return Err(Fault::Truncated);
        }
        if self.rows == 0 {
            return Err(Fault::Empty);
        }
        Ok(self.rows)
    }
}

const KEYS: [&str; 12] = [
    "stream_index",
    "pts",
    "dts",
    "duration",
    "size",
    "flags",
    "data_hash",
    "side_datum/skip_samples:side_data_type",
    "side_datum/skip_samples:skip_samples",
    "side_datum/skip_samples:discard_padding",
    "side_datum/skip_samples:skip_reason",
    "side_datum/skip_samples:discard_reason",
];
fn unsigned(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Fault::InvalidValue);
    }
    value.parse().map_err(|_| Fault::InvalidValue)
}
fn signed(value: &str) -> Result<i64> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Fault::InvalidValue);
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|n| *n != i64::MIN)
        .ok_or(Fault::InvalidValue)
}
fn digest(value: &str) -> Result<[u8; 32]> {
    let hex = value
        .strip_prefix("SHA256:")
        .filter(|s| s.len() == 64)
        .ok_or(Fault::InvalidValue)?;
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(Fault::InvalidValue),
    };
    let mut out = [0; 32];
    for (i, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        out[i] = nibble(pair[0])? * 16 + nibble(pair[1])?;
    }
    Ok(out)
}
fn parse(line: &[u8], streams: Streams) -> Result<PacketFact> {
    let body = line.strip_suffix(b"\n").ok_or(Fault::Truncated)?;
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    if !body.iter().all(|&b| (32..=126).contains(&b) && b != b'\\') {
        return Err(Fault::InvalidRow);
    }
    let body = std::str::from_utf8(body).map_err(|_| Fault::InvalidRow)?;
    let mut fields = [None; 12];
    let mut parts = body.split('|');
    if parts.next() != Some("packet") {
        return Err(Fault::InvalidRow);
    }
    for part in parts {
        let (key, value) = part.split_once('=').ok_or(Fault::InvalidRow)?;
        let slot = KEYS
            .iter()
            .position(|candidate| *candidate == key)
            .ok_or_else(|| {
                if key.starts_with("side_") {
                    Fault::UnsupportedSideData
                } else {
                    Fault::InvalidRow
                }
            })?;
        if fields[slot].replace(value).is_some() {
            return Err(if slot >= 7 {
                Fault::UnsupportedSideData
            } else {
                Fault::InvalidRow
            });
        }
    }
    let get = |i: usize| fields[i].ok_or(Fault::InvalidRow);
    let id = u8::try_from(unsigned(get(0)?)?)
        .ok()
        .filter(|n| *n <= 31)
        .ok_or(Fault::InvalidValue)?;
    let track = streams.track(id)?;
    let pts = signed(get(1)?)?;
    let dts = signed(get(2)?)?;
    let duration = signed(get(3)?)?;
    if duration <= 0 {
        return Err(Fault::InvalidValue);
    }
    let size = unsigned(get(4)?)?;
    if !(1..=PACKET).contains(&size) {
        return Err(Fault::Bounds);
    }
    let flags = get(5)?.as_bytes();
    if flags.len() != 3
        || !matches!(flags[0], b'K' | b'_')
        || !matches!(flags[1], b'D' | b'_')
        || !matches!(flags[2], b'C' | b'_')
    {
        return Err(Fault::InvalidValue);
    }
    if flags[1] != b'_' || flags[2] != b'_' {
        return Err(Fault::UnsupportedFlags);
    }
    let side = if fields[7..].iter().all(Option::is_none) {
        None
    } else {
        if fields[7..].iter().any(Option::is_none) || fields[7] != Some("Skip Samples") {
            return Err(Fault::UnsupportedSideData);
        }
        Some(SkipSamples {
            skip: u32::try_from(unsigned(get(8)?)?).map_err(|_| Fault::InvalidValue)?,
            discard: u32::try_from(unsigned(get(9)?)?).map_err(|_| Fault::InvalidValue)?,
            skip_reason: u8::try_from(unsigned(get(10)?)?).map_err(|_| Fault::InvalidValue)?,
            discard_reason: u8::try_from(unsigned(get(11)?)?).map_err(|_| Fault::InvalidValue)?,
        })
    };
    Ok(PacketFact {
        track,
        pts,
        dts,
        duration,
        size,
        key: flags[0] == b'K',
        hash: digest(get(6)?)?,
        side,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Times {
    pub pts: i64,
    pub dts: i64,
    pub duration: i64,
}
impl From<PacketFact> for Times {
    fn from(p: PacketFact) -> Self {
        Self {
            pts: p.pts,
            dts: p.dts,
            duration: p.duration,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FoldResult {
    pub track: Track,
    pub count: u64,
    pub bytes: u64,
    pub digest: [u8; 32],
    pub first: Times,
    pub last: Times,
    pub min: Times,
    pub max: Times,
}
pub(super) struct TrackFold {
    track: Track,
    count: u64,
    bytes: u64,
    hash: Sha256,
    first: Option<Times>,
    last: Option<Times>,
    min: Option<Times>,
    max: Option<Times>,
    terminal: bool,
}
impl TrackFold {
    pub(super) fn new(track: Track) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"Atsumi/YouTube/packet-fold/v1\0");
        hash.update([track.index() as u8]);
        Self {
            track,
            count: 0,
            bytes: 0,
            hash,
            first: None,
            last: None,
            min: None,
            max: None,
            terminal: false,
        }
    }
    pub(super) fn push(&mut self, packet: PacketFact) -> Result<()> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        let result = self.push_inner(packet);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }
    fn push_inner(&mut self, packet: PacketFact) -> Result<()> {
        if packet.track != self.track {
            return Err(Fault::UnknownStream);
        }
        let count = self
            .count
            .checked_add(1)
            .filter(|n| *n <= ROWS)
            .ok_or(Fault::Bounds)?;
        let bytes = self
            .bytes
            .checked_add(packet.size)
            .filter(|n| *n <= PAYLOAD)
            .ok_or(Fault::Bounds)?;
        self.hash.update(b"packet\0");
        self.hash.update(self.count.to_be_bytes());
        self.hash.update(packet.size.to_be_bytes());
        self.hash.update(packet.hash);
        self.hash.update([packet.key as u8]);
        match packet.side {
            None => self.hash.update([0]),
            Some(side) => {
                self.hash.update([1]);
                self.hash.update(side.skip.to_be_bytes());
                self.hash.update(side.discard.to_be_bytes());
                self.hash.update([side.skip_reason, side.discard_reason]);
            }
        }
        let t = Times::from(packet);
        self.first.get_or_insert(t);
        self.last = Some(t);
        self.min = Some(self.min.map_or(t, |old| Times {
            pts: old.pts.min(t.pts),
            dts: old.dts.min(t.dts),
            duration: old.duration.min(t.duration),
        }));
        self.max = Some(self.max.map_or(t, |old| Times {
            pts: old.pts.max(t.pts),
            dts: old.dts.max(t.dts),
            duration: old.duration.max(t.duration),
        }));
        self.count = count;
        self.bytes = bytes;
        Ok(())
    }
    pub(super) fn finish(&mut self) -> Result<FoldResult> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        self.terminal = true;
        let first = self.first.ok_or(Fault::Empty)?;
        self.hash.update(b"end\0");
        self.hash.update(self.count.to_be_bytes());
        self.hash.update(self.bytes.to_be_bytes());
        Ok(FoldResult {
            track: self.track,
            count: self.count,
            bytes: self.bytes,
            digest: self.hash.clone().finalize().into(),
            first,
            last: self.last.ok_or(Fault::Empty)?,
            min: self.min.ok_or(Fault::Empty)?,
            max: self.max.ok_or(Fault::Empty)?,
        })
    }
}

/// Local checked rational arithmetic. These values report facts, not an accepted tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rational {
    pub numerator: i128,
    pub denominator: i128,
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}
impl Rational {
    pub(super) fn new(mut numerator: i128, mut denominator: i128) -> Result<Self> {
        if denominator == 0 {
            return Err(Fault::InvalidValue);
        }
        if denominator < 0 {
            numerator = numerator.checked_neg().ok_or(Fault::ArithmeticOverflow)?;
            denominator = denominator.checked_neg().ok_or(Fault::ArithmeticOverflow)?;
        }
        let divisor = gcd(numerator.unsigned_abs(), denominator as u128) as i128;
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
    fn sub(self, other: Self) -> Result<Self> {
        let divisor = gcd(self.denominator as u128, other.denominator as u128) as i128;
        let left = other.denominator / divisor;
        let right = self.denominator / divisor;
        let num = self
            .numerator
            .checked_mul(left)
            .and_then(|a| {
                other
                    .numerator
                    .checked_mul(right)
                    .and_then(|b| a.checked_sub(b))
            })
            .ok_or(Fault::ArithmeticOverflow)?;
        let den = self
            .denominator
            .checked_mul(left)
            .ok_or(Fault::ArithmeticOverflow)?;
        Self::new(num, den)
    }
    fn abs(self) -> Result<Self> {
        Ok(Self {
            numerator: self
                .numerator
                .checked_abs()
                .ok_or(Fault::ArithmeticOverflow)?,
            denominator: self.denominator,
        })
    }
    fn cmp(self, other: Self) -> Result<Ordering> {
        if self.numerator.signum() != other.numerator.signum() {
            return Ok(self.numerator.cmp(&other.numerator));
        }
        if self.numerator == 0 {
            return Ok(Ordering::Equal);
        }
        let denominators = gcd(self.denominator as u128, other.denominator as u128) as i128;
        let numerators = gcd(
            self.numerator.unsigned_abs(),
            other.numerator.unsigned_abs(),
        );
        // Both numerators can be i128::MIN only in same-sign comparisons.
        if numerators > i128::MAX as u128 {
            return Ok(self.denominator.cmp(&other.denominator));
        }
        let numerators = numerators as i128;
        let a = (self.numerator / numerators)
            .checked_mul(other.denominator / denominators)
            .ok_or(Fault::ArithmeticOverflow)?;
        let b = (other.numerator / numerators)
            .checked_mul(self.denominator / denominators)
            .ok_or(Fault::ArithmeticOverflow)?;
        Ok(a.cmp(&b))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TimeBase(Rational);
impl TimeBase {
    pub(super) fn new(numerator: u32, denominator: u32) -> Result<Self> {
        if !(1..=1_000_000_000).contains(&numerator) || !(1..=1_000_000_000).contains(&denominator)
        {
            return Err(Fault::InvalidValue);
        }
        Ok(Self(Rational::new(numerator as i128, denominator as i128)?))
    }
    fn ticks(self, value: i64) -> Result<Rational> {
        let divisor = gcd(value.unsigned_abs() as u128, self.0.denominator as u128) as i128;
        Rational::new(
            (value as i128 / divisor)
                .checked_mul(self.0.numerator)
                .ok_or(Fault::ArithmeticOverflow)?,
            self.0.denominator / divisor,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PairReport {
    pub track: Track,
    pub count: u64,
    pub bytes: u64,
    pub source_tick: Rational,
    pub output_tick: Rational,
    pub max_pts_error: Rational,
    pub max_dts_error: Rational,
    pub max_duration_error: Rational,
}
pub(super) struct PairComparison {
    report: PairReport,
    source: TimeBase,
    output: TimeBase,
    origin: Rational,
    terminal: bool,
}
impl PairComparison {
    pub(super) fn new(
        track: Track,
        source: TimeBase,
        output: TimeBase,
        origin: Rational,
    ) -> Result<Self> {
        // Revalidate the origin because its fields are exposed for reporting.
        let origin = Rational::new(origin.numerator, origin.denominator)?;
        let zero = Rational::new(0, 1)?;
        Ok(Self {
            report: PairReport {
                track,
                count: 0,
                bytes: 0,
                source_tick: source.0,
                output_tick: output.0,
                max_pts_error: zero,
                max_dts_error: zero,
                max_duration_error: zero,
            },
            source,
            output,
            origin,
            terminal: false,
        })
    }
    pub(super) fn push(&mut self, source: PacketFact, output: PacketFact) -> Result<()> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        let result = self.push_inner(source, output);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }
    fn push_inner(&mut self, source: PacketFact, output: PacketFact) -> Result<()> {
        if source.track != self.report.track || output.track != self.report.track {
            return Err(Fault::UnknownStream);
        }
        if source.size != output.size || source.hash != output.hash {
            return Err(Fault::PayloadMismatch);
        }
        if source.key != output.key {
            return Err(Fault::FlagsMismatch);
        }
        if source.side != output.side {
            return Err(Fault::SideDataMismatch);
        }
        let count = self
            .report
            .count
            .checked_add(1)
            .filter(|n| *n <= ROWS)
            .ok_or(Fault::Bounds)?;
        let bytes = self
            .report
            .bytes
            .checked_add(source.size)
            .filter(|n| *n <= PAYLOAD)
            .ok_or(Fault::Bounds)?;
        let difference = |a, b, with_origin| {
            let mut a = self.source.ticks(a)?;
            if with_origin {
                a = a.sub(self.origin)?;
            }
            a.sub(self.output.ticks(b)?)?.abs()
        };
        let pts = difference(source.pts, output.pts, true)?;
        let dts = difference(source.dts, output.dts, true)?;
        let duration = difference(source.duration, output.duration, false)?;
        for (target, value) in [
            (&mut self.report.max_pts_error, pts),
            (&mut self.report.max_dts_error, dts),
            (&mut self.report.max_duration_error, duration),
        ] {
            if value.cmp(*target)? == Ordering::Greater {
                *target = value;
            }
        }
        self.report.count = count;
        self.report.bytes = bytes;
        Ok(())
    }
    pub(super) fn finish(&mut self) -> Result<PairReport> {
        if self.terminal {
            return Err(Fault::Terminal);
        }
        self.terminal = true;
        if self.report.count == 0 {
            return Err(Fault::Empty);
        }
        Ok(self.report.clone())
    }
}

/// Both iterators must be complete, per-track ordered projections of the retained range.
pub(super) fn compare_iters(
    mut comparison: PairComparison,
    source: impl IntoIterator<Item = Result<PacketFact>>,
    output: impl IntoIterator<Item = Result<PacketFact>>,
) -> Result<PairReport> {
    let mut source = source.into_iter();
    let mut output = output.into_iter();
    loop {
        match (source.next(), output.next()) {
            (Some(a), Some(b)) => comparison.push(a?, b?)?,
            (None, None) => return comparison.finish(),
            (Some(Err(error)), _) | (_, Some(Err(error))) => return Err(error),
            _ => return Err(Fault::CountMismatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_record_layout_roundtrip_and_side_presence() {
        let mut value = fact();
        value.pts = -7;
        value.dts = 19;
        let plain = value.encode_record().unwrap();
        assert_eq!(&plain[..4], &[1, 1, 0, 0]);
        assert_eq!(&plain[4..12], &(-7_i64).to_be_bytes());
        assert_eq!(&plain[12..20], &19_i64.to_be_bytes());
        assert_eq!(&plain[20..28], &1024_i64.to_be_bytes());
        assert_eq!(&plain[28..36], &372_u64.to_be_bytes());
        assert_eq!(&plain[36..68], &value.hash);
        assert_eq!(PacketFact::decode_record(&plain).unwrap(), value);
        value.side = Some(SkipSamples {
            skip: 0,
            discard: 0,
            skip_reason: 0,
            discard_reason: 0,
        });
        let zero = value.encode_record().unwrap();
        assert_ne!(plain, zero);
        assert_eq!(zero[2], 1);
        assert_eq!(PacketFact::decode_record(&zero).unwrap(), value);
        value.track = Track::Video;
        value.key = false;
        value.pts = i64::MAX;
        value.dts = i64::MIN + 1;
        value.duration = i64::MAX;
        value.size = PACKET;
        value.side = Some(SkipSamples {
            skip: u32::MAX,
            discard: u32::MAX,
            skip_reason: u8::MAX,
            discard_reason: u8::MAX,
        });
        assert_eq!(
            PacketFact::decode_record(&value.encode_record().unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn binary_record_rejects_invalid_tags_reserved_and_absent_side_bytes() {
        let valid = fact().encode_record().unwrap();
        for (at, byte) in [(0, 2), (1, 2), (2, 2), (3, 1), (78, 1), (79, 1)] {
            let mut changed = valid;
            changed[at] = byte;
            assert!(PacketFact::decode_record(&changed).is_err(), "offset {at}");
        }
        for at in 68..78 {
            let mut changed = valid;
            changed[at] = 1;
            assert_eq!(
                PacketFact::decode_record(&changed),
                Err(Fault::InvalidValue)
            );
        }
    }

    #[test]
    fn binary_record_revalidates_all_numeric_boundaries() {
        let valid = fact().encode_record().unwrap();
        for (at, number) in [(4, i64::MIN), (12, i64::MIN), (20, 0), (20, -1)] {
            let mut changed = valid;
            changed[at..at + 8].copy_from_slice(&number.to_be_bytes());
            assert_eq!(
                PacketFact::decode_record(&changed),
                Err(Fault::InvalidValue)
            );
        }
        for number in [0, PACKET + 1, u64::MAX] {
            let mut changed = valid;
            changed[28..36].copy_from_slice(&number.to_be_bytes());
            assert_eq!(
                PacketFact::decode_record(&changed),
                Err(Fault::InvalidValue)
            );
        }
        let mut invalid = fact();
        invalid.duration = 0;
        assert_eq!(invalid.encode_record(), Err(Fault::InvalidValue));
    }
    const GOOD: &str = "packet|stream_index=1|pts=0|dts=0|duration=1024|size=372|flags=K__|data_hash=SHA256:4e419162bff08af7166a0ed3f8eb532bb24e2cc47ed43d89c85ce1066366970c\n";
    const SIDE: &str = "|side_datum/skip_samples:side_data_type=Skip Samples|side_datum/skip_samples:skip_samples=651|side_datum/skip_samples:discard_padding=0|side_datum/skip_samples:skip_reason=0|side_datum/skip_samples:discard_reason=0";
    fn streams() -> Streams {
        Streams::new(&[(0, Track::Video), (1, Track::Audio)]).unwrap()
    }
    fn fact() -> PacketFact {
        parse(GOOD.as_bytes(), streams()).unwrap()
    }
    fn decoder_result(bytes: &[u8]) -> Result<u64> {
        let mut decoder = Decoder::new(streams());
        decoder.feed(bytes, |_| Ok(()))?;
        decoder.finish()
    }
    fn comparison(track: Track) -> PairComparison {
        PairComparison::new(
            track,
            TimeBase::new(1, 44100).unwrap(),
            TimeBase::new(1, 44100).unwrap(),
            Rational::new(0, 1).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn pinned_normal_and_skip_sample_rows_and_every_split_boundary() {
        let bad = format!(
            "{}{}\n",
            GOOD.trim_end()
                .replace("pts=0", "pts=-651")
                .replace("dts=0", "dts=-651"),
            SIDE
        );
        for line in [GOOD.to_owned(), GOOD.replace('\n', "\r\n"), bad] {
            let expected = parse(line.as_bytes(), streams()).unwrap();
            for split in 0..=line.len() {
                let mut decoder = Decoder::new(streams());
                let mut seen = None;
                decoder
                    .feed(&line.as_bytes()[..split], |p| {
                        assert!(seen.replace(p).is_none());
                        Ok(())
                    })
                    .unwrap();
                decoder
                    .feed(&line.as_bytes()[split..], |p| {
                        assert!(seen.replace(p).is_none());
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(decoder.finish(), Ok(1));
                assert_eq!(seen, Some(expected));
                assert_eq!(decoder.feed(b"", |_| Ok(())), Err(Fault::Terminal));
                assert_eq!(decoder.finish(), Err(Fault::Terminal));
            }
        }
        let mut fields: Vec<_> = GOOD.trim_end().split('|').skip(1).collect();
        fields.reverse();
        assert_eq!(
            parse(
                format!("packet|{}\n", fields.join("|")).as_bytes(),
                streams()
            ),
            Ok(fact())
        );
    }

    #[test]
    fn side_data_is_closed_and_absence_is_not_zero_or_trim() {
        let bad = parse(
            format!("{}{}\n", GOOD.trim_end(), SIDE).as_bytes(),
            streams(),
        )
        .unwrap();
        assert_eq!(bad.side.unwrap().skip, 651);
        assert_eq!(
            comparison(Track::Audio).push(fact(), bad),
            Err(Fault::SideDataMismatch)
        );
        let zero = parse(
            format!("{}{}\n", GOOD.trim_end(), SIDE.replace("=651", "=0")).as_bytes(),
            streams(),
        )
        .unwrap();
        assert_eq!(
            comparison(Track::Audio).push(fact(), zero),
            Err(Fault::SideDataMismatch)
        );
        for side in [
            SIDE.replace("skip_samples:skip_samples", "other:skip_samples"),
            SIDE.replace("Skip Samples", "Other"),
            SIDE.replace("|side_datum/skip_samples:discard_reason=0", ""),
            format!("{SIDE}|side_datum/skip_samples:skip_samples=0"),
            "|side_data_type=Skip Samples".into(),
        ] {
            assert_eq!(
                decoder_result(format!("{}{}\n", GOOD.trim_end(), side).as_bytes()),
                Err(Fault::UnsupportedSideData)
            );
        }
    }

    #[test]
    fn malformed_values_sections_and_rows_fail_without_diagnostic_echo() {
        for changed in [
            GOOD.replace("pts=0", "pts=N/A"),
            GOOD.replace("dts=0", "dts=-9223372036854775808"),
            GOOD.replace("duration=1024", "duration=0"),
            GOOD.replace("duration=1024", "duration=-1"),
            GOOD.replace("size=372", "size=+1"),
            GOOD.replace("size=372", "size=1.0"),
            GOOD.replace("size=372", "size=18446744073709551616"),
            GOOD.replace("size=372", "size=0"),
            GOOD.replace("size=372", "size=33554433"),
            GOOD.replace("stream_index=1", "stream_index=32"),
            GOOD.replace("packet|", "stream|"),
            GOOD.replace("|dts=0", ""),
            GOOD.replace("|dts=0", "|dts=0|dts=0"),
            GOOD.replace("SHA256:", "MD5:"),
            GOOD.replace("4e4191", "ze4191"),
            GOOD.replace("K__", "K___"),
            GOOD.replace("|pts=0", "|pts= 0"),
            GOOD.replace("|pts=0", "|pts=\\0"),
            GOOD.replace("|pts=0", "|pts=\r0"),
            GOOD.replace("|pts=0", "|pts=\u{0}0"),
            GOOD.replace("|pts=0", "|pts=한글"),
            GOOD.replace("\n", "|tag=test\n"),
            GOOD.replace("\n", "\r\r\n"),
            "\n".into(),
        ] {
            assert!(decoder_result(changed.as_bytes()).is_err());
        }
        assert_eq!(
            decoder_result(GOOD.replace("K__", "KD_").as_bytes()),
            Err(Fault::UnsupportedFlags)
        );
        assert_eq!(
            decoder_result(GOOD.replace("K__", "K_C").as_bytes()),
            Err(Fault::UnsupportedFlags)
        );
        assert_eq!(
            decoder_result(GOOD.replace("stream_index=1", "stream_index=3").as_bytes()),
            Err(Fault::UnknownStream)
        );
        assert_eq!(
            decoder_result(GOOD.trim_end().as_bytes()),
            Err(Fault::Truncated)
        );
        assert_eq!(decoder_result(b""), Err(Fault::Empty));
        assert!(Streams::new(&[]).is_err());
        assert!(Streams::new(&[(0, Track::Audio), (0, Track::Video)]).is_err());
        assert!(Streams::new(&[(0, Track::Audio), (1, Track::Audio)]).is_err());
    }

    #[test]
    fn decoder_limits_and_callback_failure_latch_and_memory_stays_bounded() {
        let mut d = Decoder::new(streams());
        d.byte_cap = GOOD.len() as u64;
        d.row_cap = 1;
        d.feed(GOOD.as_bytes(), |_| Ok(())).unwrap();
        assert_eq!(d.finish(), Ok(1));
        let mut d = Decoder::new(streams());
        d.row_cap = 1;
        d.feed(GOOD.as_bytes(), |_| Ok(())).unwrap();
        assert_eq!(d.feed(GOOD.as_bytes(), |_| Ok(())), Err(Fault::Bounds));
        assert_eq!(d.finish(), Err(Fault::Terminal));
        let mut d = Decoder::new(streams());
        d.byte_cap = GOOD.len() as u64 - 1;
        assert_eq!(d.feed(GOOD.as_bytes(), |_| Ok(())), Err(Fault::Bounds));
        let mut d = Decoder::new(streams());
        d.payload_cap = 372;
        d.feed(GOOD.as_bytes(), |_| Ok(())).unwrap();
        assert_eq!(d.feed(GOOD.as_bytes(), |_| Ok(())), Err(Fault::Bounds));
        let mut d = Decoder::new(streams());
        assert_eq!(
            d.feed(&vec![b'x'; CHUNK + 1], |_| Ok(())),
            Err(Fault::Bounds)
        );
        assert!(d.line.is_empty());
        let mut d = Decoder::new(streams());
        assert_eq!(d.feed(&vec![b'x'; LINE], |_| Ok(())), Ok(()));
        assert_eq!(d.feed(b"\n", |_| Ok(())), Err(Fault::Bounds));
        assert_eq!(d.line.len(), LINE);
        let padding = "0".repeat(LINE - GOOD.len());
        let exact = GOOD.replace("size=372", &format!("size={padding}372"));
        assert_eq!(exact.len(), LINE);
        assert_eq!(decoder_result(exact.as_bytes()), Ok(1));
        let mut d = Decoder::new(streams());
        assert_eq!(
            d.feed(GOOD.as_bytes(), |_| Err(Fault::PayloadMismatch)),
            Err(Fault::PayloadMismatch)
        );
        assert_eq!(
            d.feed(GOOD.as_bytes(), |_| panic!("must not emit")),
            Err(Fault::Terminal)
        );
    }

    #[test]
    fn signed_overflow_plus_and_wrong_digest_lengths_fail_closed() {
        for key in ["pts", "dts"] {
            for value in ["9223372036854775808", "-9223372036854775809", "+1", "+0"] {
                let line = GOOD.replace(&format!("|{key}=0|"), &format!("|{key}={value}|"));
                let mut decoder = Decoder::new(streams());
                assert_eq!(
                    decoder.feed(line.as_bytes(), |_| panic!("invalid row must not emit")),
                    Err(Fault::InvalidValue)
                );
                assert_eq!(decoder.finish(), Err(Fault::Terminal));
            }
        }
        let (prefix, _) = GOOD.split_once("SHA256:").unwrap();
        for length in [0, 63, 65] {
            let line = format!("{prefix}SHA256:{}\n", "a".repeat(length));
            let mut decoder = Decoder::new(streams());
            assert_eq!(
                decoder.feed(line.as_bytes(), |_| panic!("invalid digest must not emit")),
                Err(Fault::InvalidValue)
            );
            assert_eq!(decoder.finish(), Err(Fault::Terminal));
        }
    }

    #[test]
    fn interleaved_streams_route_into_two_fixed_ordered_folds() {
        let audio = GOOD.to_owned();
        let video = GOOD.replace("stream_index=1", "stream_index=0");
        let second_audio = GOOD
            .replace("|pts=0|", "|pts=1024|")
            .replace("|dts=0|", "|dts=1024|");
        let second_video = video
            .replace("|pts=0|", "|pts=1024|")
            .replace("|dts=0|", "|dts=1024|");
        let mut folds = [TrackFold::new(Track::Video), TrackFold::new(Track::Audio)];
        let mut decoder = Decoder::new(streams());
        for row in [&video, &audio, &second_audio, &second_video] {
            decoder
                .feed(row.as_bytes(), |packet| {
                    folds[packet.track.index()].push(packet)
                })
                .unwrap();
        }
        assert_eq!(decoder.finish(), Ok(4));
        for (track, expected_rows) in [
            (Track::Video, [&video, &second_video]),
            (Track::Audio, [&audio, &second_audio]),
        ] {
            let mut expected = TrackFold::new(track);
            for row in expected_rows {
                expected
                    .push(parse(row.as_bytes(), streams()).unwrap())
                    .unwrap();
            }
            let actual = folds[track.index()].finish().unwrap();
            assert_eq!((actual.track, actual.count, actual.bytes), (track, 2, 744));
            assert_eq!(actual, expected.finish().unwrap());
        }
    }

    #[test]
    fn ordered_folds_bind_track_order_payload_flags_and_explicit_side_data() {
        let first = fact();
        let mut second = first;
        second.hash[0] ^= 1;
        let fold = |values: &[PacketFact]| {
            let mut f = TrackFold::new(Track::Audio);
            for &p in values {
                f.push(p).unwrap();
            }
            f.finish().unwrap()
        };
        assert_ne!(fold(&[first, second]).digest, fold(&[second, first]).digest);
        let a = fold(&[first, second]);
        assert_eq!((a.count, a.bytes), (2, 744));
        let mut changed = first;
        changed.key = false;
        assert_ne!(fold(&[first]).digest, fold(&[changed]).digest);
        let mut changed = first;
        changed.side = Some(SkipSamples {
            skip: 0,
            discard: 0,
            skip_reason: 0,
            discard_reason: 0,
        });
        assert_ne!(fold(&[first]).digest, fold(&[changed]).digest);
        let mut f = TrackFold::new(Track::Video);
        assert_eq!(f.push(first), Err(Fault::UnknownStream));
        assert_eq!(f.finish(), Err(Fault::Terminal));
        let mut f = TrackFold::new(Track::Audio);
        assert_eq!(f.finish(), Err(Fault::Empty));
        let mut f = TrackFold::new(Track::Audio);
        f.count = ROWS;
        assert_eq!(f.push(first), Err(Fault::Bounds));
        let mut f = TrackFold::new(Track::Audio);
        f.bytes = PAYLOAD;
        assert_eq!(f.push(first), Err(Fault::Bounds));
        let mut f = TrackFold::new(Track::Audio);
        f.push(first).unwrap();
        f.finish().unwrap();
        assert_eq!(f.push(first), Err(Fault::Terminal));
    }

    #[test]
    fn pair_helper_checks_order_both_ends_and_faults_without_ignoring_last_packet() {
        let a = fact();
        let mut b = a;
        b.hash[0] ^= 1;
        assert_eq!(
            compare_iters(comparison(Track::Audio), [Ok(a), Ok(b)], [Ok(a)]),
            Err(Fault::CountMismatch)
        );
        assert_eq!(
            compare_iters(comparison(Track::Audio), [Ok(a)], [Ok(a), Ok(b)]),
            Err(Fault::CountMismatch)
        );
        assert_eq!(
            compare_iters(comparison(Track::Audio), [Ok(a), Ok(b)], [Ok(b), Ok(a)]),
            Err(Fault::PayloadMismatch)
        );
        assert_eq!(
            compare_iters(comparison(Track::Audio), [], []),
            Err(Fault::Empty)
        );
        assert_eq!(
            compare_iters(
                comparison(Track::Audio),
                [Ok(a), Err(Fault::Truncated)],
                [Ok(a)]
            ),
            Err(Fault::Truncated)
        );
        assert_eq!(
            compare_iters(
                comparison(Track::Audio),
                [Ok(a)],
                [Ok(a), Err(Fault::Truncated)]
            ),
            Err(Fault::Truncated)
        );
        for fault in [
            Fault::FlagsMismatch,
            Fault::PayloadMismatch,
            Fault::UnknownStream,
        ] {
            let mut changed = a;
            match fault {
                Fault::FlagsMismatch => changed.key = !a.key,
                Fault::PayloadMismatch => changed.size += 1,
                _ => changed.track = Track::Video,
            }
            let mut c = comparison(Track::Audio);
            assert_eq!(c.push(a, changed), Err(fault));
            assert_eq!(c.finish(), Err(Fault::Terminal));
        }
    }

    #[test]
    fn exact_common_origin_audio_and_reported_video_quantization_are_facts_not_tolerance() {
        // Synthetic ordered fixtures using measured clocks/counts, NOT captured packet transcripts.
        let origin = Rational::new(2063879393, 44100).unwrap();
        let mut audio = PairComparison::new(
            Track::Audio,
            TimeBase::new(1, 44100).unwrap(),
            TimeBase::new(1, 44100).unwrap(),
            origin,
        )
        .unwrap();
        for i in 0..216 {
            let mut source = fact();
            source.pts = 2063879393 + i * 1024;
            source.dts = source.pts;
            let mut output = source;
            output.pts -= 2063879393;
            output.dts = output.pts;
            audio.push(source, output).unwrap();
        }
        let report = audio.finish().unwrap();
        assert_eq!(report.count, 216);
        assert_eq!(report.max_pts_error.numerator, 0);
        assert_eq!(report.max_dts_error.numerator, 0);
        assert_eq!(report.max_duration_error.numerator, 0);
        let mut video = PairComparison::new(
            Track::Video,
            TimeBase::new(1, 1000).unwrap(),
            TimeBase::new(1, 16000).unwrap(),
            origin,
        )
        .unwrap();
        for i in 0..150 {
            let mut source = fact();
            source.track = Track::Video;
            source.pts = 46800001 + i * 33;
            source.dts = source.pts;
            source.duration = 34;
            let mut output = source;
            output.pts = 240 + i * 528;
            output.dts = output.pts;
            output.duration = 533;
            video.push(source, output).unwrap();
        }
        let report = video.finish().unwrap();
        assert_eq!(report.count, 150);
        assert_eq!(report.max_pts_error, Rational::new(26, 110250).unwrap());
        assert_eq!(report.max_duration_error, Rational::new(11, 16000).unwrap());
        assert_eq!(report.source_tick, Rational::new(1, 1000).unwrap());
    }

    #[test]
    fn rational_reduction_sign_zero_bounds_and_overflow_never_use_float() {
        assert_eq!(Rational::new(0, -5), Rational::new(0, 1));
        assert_eq!(Rational::new(6, -8), Rational::new(-3, 4));
        assert_eq!(
            Rational::new(1, 3)
                .unwrap()
                .sub(Rational::new(1, 6).unwrap()),
            Rational::new(1, 6)
        );
        assert_eq!(Rational::new(1, 0), Err(Fault::InvalidValue));
        assert_eq!(Rational::new(1, i128::MIN), Err(Fault::ArithmeticOverflow));
        assert_eq!(
            Rational::new(i128::MIN, 1).unwrap().abs(),
            Err(Fault::ArithmeticOverflow)
        );
        assert_eq!(
            Rational::new(i128::MAX, 1)
                .unwrap()
                .sub(Rational::new(-1, 1).unwrap()),
            Err(Fault::ArithmeticOverflow)
        );
        let a = Rational::new(9_007_199_254_740_993, 1).unwrap();
        let b = Rational::new(9_007_199_254_740_992, 1).unwrap();
        assert_eq!(a.sub(b), Rational::new(1, 1));
        assert_eq!(a.cmp(b), Ok(Ordering::Greater));
        assert_eq!(
            Rational::new(i128::MIN, 1)
                .unwrap()
                .cmp(Rational::new(i128::MIN, 3).unwrap()),
            Ok(Ordering::Less)
        );
        assert_eq!(
            TimeBase::new(1, 1).unwrap().ticks(i64::MAX),
            Rational::new(i64::MAX as i128, 1)
        );
        assert_eq!(
            TimeBase::new(1, 1).unwrap().ticks(-i64::MAX),
            Rational::new(-i64::MAX as i128, 1)
        );
        assert!(TimeBase::new(0, 1).is_err());
        assert!(TimeBase::new(1, 1_000_000_001).is_err());
        let mut c = PairComparison::new(
            Track::Audio,
            TimeBase::new(1, 1).unwrap(),
            TimeBase::new(1, 1).unwrap(),
            Rational::new(i128::MIN, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(c.push(fact(), fact()), Err(Fault::ArithmeticOverflow));
        assert_eq!(c.finish(), Err(Fault::Terminal));
    }
}
