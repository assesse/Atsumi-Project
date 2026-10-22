//! Bounded typed packet evidence over borrowed I/O, not durable/source authority.
//! The caller must prove complete successful probing and retain its own guards.
use super::{
    packet_facts::{Fault, PacketFact, TrackFold},
    stream_facts::ProbeFacts,
    Track,
};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};

const MAGIC: &[u8; 8] = b"ATYPKT01";
const RECORD: u64 = 80;
const ROWS: u64 = 20_000_000;
const MAX_BYTES: u64 = 8 + ROWS * RECORD;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Io,
    Truncated,
    Header,
    Bounds,
    SummaryMismatch,
    Packet(Fault),
    Terminal,
}
type Result<T> = std::result::Result<T, Error>;
impl From<Fault> for Error {
    fn from(value: Fault) -> Self {
        Self::Packet(value)
    }
}

/// Constructed only after a nonempty tape agrees with supplied successful facts.
/// This does not certify process success, disk durability or media publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Summary {
    facts: ProbeFacts,
    bytes: u64,
    hash: [u8; 32],
}
impl Summary {
    pub(super) fn facts(&self) -> &ProbeFacts {
        &self.facts
    }
}

pub(super) struct Writer<'a, W: Write + ?Sized> {
    output: &'a mut W,
    track: Track,
    count: u64,
    hash: Sha256,
    fold: TrackFold,
    terminal: bool,
    row_cap: u64,
}
impl<'a, W: Write + ?Sized> Writer<'a, W> {
    pub(super) fn new(output: &'a mut W, track: Track) -> Result<Self> {
        output.write_all(MAGIC).map_err(|_| Error::Io)?;
        Ok(Self {
            output,
            track,
            count: 0,
            hash: Sha256::new_with_prefix(MAGIC),
            fold: TrackFold::new(track),
            terminal: false,
            row_cap: ROWS,
        })
    }

    pub(super) fn push(&mut self, fact: PacketFact) -> Result<()> {
        if self.terminal {
            return Err(Error::Terminal);
        }
        let result = self.push_inner(fact);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }

    fn push_inner(&mut self, fact: PacketFact) -> Result<()> {
        let next = self
            .count
            .checked_add(1)
            .filter(|n| *n <= self.row_cap)
            .ok_or(Error::Bounds)?;
        tape_bytes(next)?;
        let bytes = fact.encode_record()?;
        self.fold.push(fact)?;
        self.output.write_all(&bytes).map_err(|_| Error::Io)?;
        self.hash.update(bytes);
        self.count = next;
        Ok(())
    }

    /// Call only with the full successful probe result, after its outer guards.
    pub(super) fn finish(&mut self, facts: &ProbeFacts) -> Result<Summary> {
        if self.terminal {
            return Err(Error::Terminal);
        }
        self.terminal = true;
        let actual = self.fold.finish()?;
        if facts.stream.track != self.track || actual != facts.packets {
            return Err(Error::SummaryMismatch);
        }
        let bytes = tape_bytes(self.count)?;
        self.output.flush().map_err(|_| Error::Io)?;
        Ok(Summary {
            facts: facts.clone(),
            bytes,
            hash: self.hash.clone().finalize().into(),
        })
    }
}

fn tape_bytes(count: u64) -> Result<u64> {
    if count == 0 || count > ROWS {
        return Err(Error::Bounds);
    }
    count
        .checked_mul(RECORD)
        .and_then(|n| n.checked_add(8))
        .filter(|n| *n <= MAX_BYTES)
        .ok_or(Error::Bounds)
}

pub(super) struct Reader<'a, R: Read + ?Sized> {
    input: &'a mut R,
    summary: &'a Summary,
    count: u64,
    hash: Sha256,
    fold: TrackFold,
    complete: bool,
    failed: bool,
}
impl<'a, R: Read + ?Sized> Reader<'a, R> {
    pub(super) fn new(input: &'a mut R, summary: &'a Summary) -> Result<Self> {
        let facts = &summary.facts;
        if tape_bytes(facts.packets.count)? != summary.bytes
            || facts.stream.track != facts.packets.track
        {
            return Err(Error::SummaryMismatch);
        }
        let mut magic = [0; 8];
        exact(input, &mut magic)?;
        if &magic != MAGIC {
            return Err(Error::Header);
        }
        Ok(Self {
            input,
            summary,
            count: 0,
            hash: Sha256::new_with_prefix(MAGIC),
            fold: TrackFold::new(facts.packets.track),
            complete: false,
            failed: false,
        })
    }

    /// None means validated physical EOF, never a failed read or a missing row.
    pub(super) fn next_fact(&mut self) -> Result<Option<PacketFact>> {
        if self.failed {
            return Err(Error::Terminal);
        }
        if self.complete {
            return Ok(None);
        }
        let result = self.next_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn next_inner(&mut self) -> Result<Option<PacketFact>> {
        if self.count == self.summary.facts.packets.count {
            let mut extra = [0];
            loop {
                match self.input.read(&mut extra) {
                    Ok(0) => break,
                    Ok(_) => return Err(Error::SummaryMismatch),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(Error::Io),
                }
            }
            let hash: [u8; 32] = self.hash.clone().finalize().into();
            if tape_bytes(self.count)? != self.summary.bytes
                || hash != self.summary.hash
                || self.fold.finish()? != self.summary.facts.packets
            {
                return Err(Error::SummaryMismatch);
            }
            self.complete = true;
            return Ok(None);
        }
        let mut bytes = [0; 80];
        exact(self.input, &mut bytes)?;
        let fact = PacketFact::decode_record(&bytes)?;
        self.fold.push(fact)?;
        self.hash.update(bytes);
        self.count = self
            .count
            .checked_add(1)
            .filter(|n| *n <= ROWS)
            .ok_or(Error::Bounds)?;
        Ok(Some(fact))
    }
}

fn exact(input: &mut (impl Read + ?Sized), bytes: &mut [u8]) -> Result<()> {
    input.read_exact(bytes).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Error::Truncated
        } else {
            Error::Io
        }
    })
}

impl<R: Read + ?Sized> Iterator for Reader<'_, R> {
    type Item = std::result::Result<PacketFact, Fault>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_fact() {
            Ok(Some(fact)) => Some(Ok(fact)),
            Ok(None) => None,
            Err(Error::Packet(fault)) => Some(Err(fault)),
            Err(_) => Some(Err(Fault::Sink)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_media::youtube_spool::{
        packet_facts::{compare_iters, Decoder, PairComparison, Rational, Streams, TimeBase},
        stream_facts::StreamFacts,
    };
    use std::io::Cursor;

    fn packets() -> Vec<PacketFact> {
        let mut decoder = Decoder::new(Streams::new(&[(0, Track::Audio)]).unwrap());
        let mut out = Vec::new();
        for (pts, hash) in [(0, '1'), (1024, '2')] {
            let row = format!("packet|stream_index=0|pts={pts}|dts={pts}|duration=1024|size=372|flags=K__|data_hash=SHA256:{}\n", hash.to_string().repeat(64));
            decoder
                .feed(row.as_bytes(), |packet| {
                    out.push(packet);
                    Ok(())
                })
                .unwrap();
        }
        decoder.finish().unwrap();
        out
    }
    fn facts(packets: &[PacketFact], track: Track) -> ProbeFacts {
        let mut fold = TrackFold::new(track);
        for &packet in packets {
            fold.push(packet).unwrap();
        }
        ProbeFacts {
            packets: fold.finish().unwrap(),
            stream: StreamFacts {
                track,
                time_base: TimeBase::new(1, 44100).unwrap(),
                width: 0,
                height: 0,
                sample_rate: 44100,
                channels: 2,
            },
        }
    }
    fn tape() -> (Vec<u8>, Summary, Vec<PacketFact>) {
        let packets = packets();
        let facts = facts(&packets, Track::Audio);
        let mut data = Vec::new();
        let mut writer = Writer::new(&mut data, Track::Audio).unwrap();
        for &packet in &packets {
            writer.push(packet).unwrap();
        }
        let summary = writer.finish(&facts).unwrap();
        drop(writer);
        (data, summary, packets)
    }
    fn drain(reader: &mut Reader<'_, impl Read>) -> Result<()> {
        while reader.next_fact()?.is_some() {}
        Ok(())
    }

    #[test]
    fn roundtrip_and_pair_comparison_exhaust_validated_eof() {
        let (data, summary, packets) = tape();
        assert_eq!(data.len(), 8 + 2 * 80);
        assert_eq!(summary.facts(), &facts(&packets, Track::Audio));
        let mut input = Cursor::new(&data);
        let mut reader = Reader::new(&mut input, &summary).unwrap();
        assert_eq!(reader.next_fact().unwrap(), Some(packets[0]));
        assert_eq!(reader.next_fact().unwrap(), Some(packets[1]));
        assert_eq!(reader.next_fact(), Ok(None));
        assert_eq!(reader.next_fact(), Ok(None));
        let mut a = Cursor::new(&data);
        let mut b = Cursor::new(&data);
        let report = compare_iters(
            PairComparison::new(
                Track::Audio,
                summary.facts.stream.time_base,
                summary.facts.stream.time_base,
                Rational::new(0, 1).unwrap(),
            )
            .unwrap(),
            Reader::new(&mut a, &summary).unwrap(),
            Reader::new(&mut b, &summary).unwrap(),
        )
        .unwrap();
        assert_eq!(report.count, 2);
        assert_eq!(report.max_pts_error, Rational::new(0, 1).unwrap());
    }

    struct ShortWriter {
        data: Vec<u8>,
        step: usize,
        stop: Option<usize>,
        flush_error: bool,
    }
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.stop.is_some_and(|n| self.data.len() >= n) {
                return Err(io::Error::other("fixture"));
            }
            let n = bytes
                .len()
                .min(self.step)
                .min(self.stop.unwrap_or(usize::MAX) - self.data.len());
            self.data.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.flush_error {
                Err(io::Error::other("fixture"))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn writer_handles_short_writes_and_poisoned_record_or_flush() {
        let packets = packets();
        let expected = facts(&packets, Track::Audio);
        for stop in [None, Some(0), Some(7), Some(8), Some(43)] {
            let mut sink = ShortWriter {
                data: Vec::new(),
                step: 1,
                stop,
                flush_error: false,
            };
            let initial = Writer::new(&mut sink, Track::Audio);
            if stop.is_some_and(|n| n < 8) {
                assert!(matches!(initial, Err(Error::Io)));
                continue;
            }
            let mut writer = initial.unwrap();
            if stop.is_some() {
                assert_eq!(writer.push(packets[0]), Err(Error::Io));
                assert_eq!(writer.push(packets[0]), Err(Error::Terminal));
                assert!(matches!(writer.finish(&expected), Err(Error::Terminal)));
            } else {
                for &p in &packets {
                    writer.push(p).unwrap();
                }
                writer.finish(&expected).unwrap();
            }
        }
        let mut sink = ShortWriter {
            data: Vec::new(),
            step: 80,
            stop: None,
            flush_error: true,
        };
        let mut writer = Writer::new(&mut sink, Track::Audio).unwrap();
        for &p in &packets {
            writer.push(p).unwrap();
        }
        assert!(matches!(writer.finish(&expected), Err(Error::Io)));
        assert!(matches!(writer.finish(&expected), Err(Error::Terminal)));
        let mut zero = ShortWriter {
            data: Vec::new(),
            step: 0,
            stop: None,
            flush_error: false,
        };
        assert!(matches!(
            Writer::new(&mut zero, Track::Audio),
            Err(Error::Io)
        ));
    }

    #[test]
    fn writer_rejects_empty_wrong_track_wrong_fold_and_bounds() {
        let packets = packets();
        let expected = facts(&packets, Track::Audio);
        let mut data = Vec::new();
        let mut writer = Writer::new(&mut data, Track::Audio).unwrap();
        assert!(matches!(
            writer.finish(&expected),
            Err(Error::Packet(Fault::Empty))
        ));
        let mut data = Vec::new();
        let mut writer = Writer::new(&mut data, Track::Video).unwrap();
        assert_eq!(
            writer.push(packets[0]),
            Err(Error::Packet(Fault::UnknownStream))
        );
        let mut data = Vec::new();
        let mut writer = Writer::new(&mut data, Track::Audio).unwrap();
        writer.push(packets[0]).unwrap();
        assert!(matches!(
            writer.finish(&expected),
            Err(Error::SummaryMismatch)
        ));
        let mut data = Vec::new();
        let mut writer = Writer::new(&mut data, Track::Audio).unwrap();
        writer.row_cap = 1;
        writer.push(packets[0]).unwrap();
        assert_eq!(writer.push(packets[1]), Err(Error::Bounds));
        assert_eq!(writer.push(packets[1]), Err(Error::Terminal));
        assert_eq!(tape_bytes(ROWS), Ok(MAX_BYTES));
        for count in [0, ROWS + 1, u64::MAX] {
            assert_eq!(tape_bytes(count), Err(Error::Bounds));
        }
    }

    #[test]
    fn writer_payload_ceiling_is_enforced_before_extra_record() {
        let mut bytes = packets()[0].encode_record().unwrap();
        bytes[28..36].copy_from_slice(&(32_u64 * 1024 * 1024).to_be_bytes());
        let large = PacketFact::decode_record(&bytes).unwrap();
        let mut output = Vec::new();
        let mut writer = Writer::new(&mut output, Track::Audio).unwrap();
        for _ in 0..4096 {
            writer.push(large).unwrap();
        }
        assert_eq!(writer.push(large), Err(Error::Packet(Fault::Bounds)));
        assert_eq!(writer.push(large), Err(Error::Terminal));
        drop(writer);
        assert_eq!(output.len(), 8 + 4096 * 80);
    }

    struct ShortReader<'a> {
        bytes: &'a [u8],
        at: usize,
        step: usize,
        fail_at: Option<usize>,
    }
    impl Read for ShortReader<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.fail_at.is_some_and(|n| self.at >= n) {
                return Err(io::Error::other("fixture"));
            }
            let n = out
                .len()
                .min(self.step)
                .min(self.bytes.len() - self.at)
                .min(self.fail_at.unwrap_or(usize::MAX) - self.at);
            out[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }
    #[test]
    fn every_partial_record_boundary_fails_and_all_short_reads_work() {
        let (data, summary, _) = tape();
        for end in 0..data.len() {
            let mut input = Cursor::new(&data[..end]);
            match Reader::new(&mut input, &summary) {
                Err(Error::Truncated) => {}
                Ok(mut reader) => {
                    assert_eq!(drain(&mut reader), Err(Error::Truncated), "end {end}");
                    assert_eq!(reader.next_fact(), Err(Error::Terminal));
                }
                _ => panic!("unexpected header result"),
            }
        }
        for step in 1..=80 {
            let mut input = ShortReader {
                bytes: &data,
                at: 0,
                step,
                fail_at: None,
            };
            drain(&mut Reader::new(&mut input, &summary).unwrap()).unwrap();
        }
    }

    #[test]
    fn actual_read_error_even_at_final_eof_is_not_success_or_none() {
        let (data, summary, _) = tape();
        for fail_at in [0, 7, 8, 43, 88, data.len()] {
            let mut input = ShortReader {
                bytes: &data,
                at: 0,
                step: 3,
                fail_at: Some(fail_at),
            };
            match Reader::new(&mut input, &summary) {
                Err(Error::Io) => {}
                Ok(mut reader) => {
                    assert_eq!(drain(&mut reader), Err(Error::Io));
                    assert_eq!(reader.next_fact(), Err(Error::Terminal));
                    assert_eq!(reader.next(), Some(Err(Fault::Sink)));
                }
                _ => panic!("unexpected result"),
            }
        }
    }

    #[test]
    fn reader_rejects_header_extra_bytes_corruption_and_swapped_rows() {
        let (data, summary, _) = tape();
        for at in 0..8 {
            let mut corrupt = data.clone();
            corrupt[at] ^= 1;
            assert!(matches!(
                Reader::new(&mut Cursor::new(corrupt), &summary),
                Err(Error::Header)
            ));
        }
        let mut variations = vec![];
        let mut extra = data.clone();
        extra.push(0);
        variations.push(extra);
        let mut payload = data.clone();
        payload[8 + 36] ^= 1;
        variations.push(payload);
        let mut time = data.clone();
        time[8 + 11] ^= 1;
        variations.push(time);
        let mut reserved = data.clone();
        reserved[8 + 78] = 1;
        variations.push(reserved);
        let mut swapped = data.clone();
        swapped[8..88].copy_from_slice(&data[88..168]);
        swapped[88..168].copy_from_slice(&data[8..88]);
        variations.push(swapped);
        let mut duplicated = data.clone();
        duplicated[88..168].copy_from_slice(&data[8..88]);
        variations.push(duplicated);
        for altered in variations {
            let mut input = Cursor::new(altered);
            let mut reader = Reader::new(&mut input, &summary).unwrap();
            assert!(drain(&mut reader).is_err());
            assert_eq!(reader.next_fact(), Err(Error::Terminal));
        }
    }

    #[test]
    fn private_summary_count_hash_size_and_fold_are_all_checked() {
        let (data, summary, _) = tape();
        let mut variants = Vec::new();
        let mut s = summary.clone();
        s.bytes += 1;
        variants.push(s);
        let mut s = summary.clone();
        s.facts.packets.count = 0;
        variants.push(s);
        let mut s = summary.clone();
        s.facts.packets.count += 1;
        s.bytes += 80;
        variants.push(s);
        let mut s = summary.clone();
        s.hash[0] ^= 1;
        variants.push(s);
        let mut s = summary.clone();
        s.facts.packets.digest[0] ^= 1;
        variants.push(s);
        let mut s = summary.clone();
        s.facts.stream.track = Track::Video;
        variants.push(s);
        for changed in variants {
            let mut input = Cursor::new(&data);
            if let Ok(mut reader) = Reader::new(&mut input, &changed) {
                assert!(drain(&mut reader).is_err());
            };
        }
    }

    #[test]
    fn comparison_cannot_ignore_error_after_last_packet() {
        let (data, summary, _) = tape();
        let mut good = Cursor::new(&data);
        let mut bad = ShortReader {
            bytes: &data,
            at: 0,
            step: 80,
            fail_at: Some(data.len()),
        };
        let result = compare_iters(
            PairComparison::new(
                Track::Audio,
                summary.facts.stream.time_base,
                summary.facts.stream.time_base,
                Rational::new(0, 1).unwrap(),
            )
            .unwrap(),
            Reader::new(&mut good, &summary).unwrap(),
            Reader::new(&mut bad, &summary).unwrap(),
        );
        assert_eq!(result, Err(Fault::Sink));
    }
}
