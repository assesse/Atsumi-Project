//! YouTube raw-only approximate timing. A receipt hint is not a source clock.
use super::{files::{Directories, GuardedFile}, index, packet_facts::Rational, schema::{self, Receipt}, Ack, Chunk, Error, Result, SpoolIdentity, Track};
use crate::platform_media::chat_status::{self, Marker};
use serde::{Deserialize, Serialize};
use std::{io::{Read, Write}, path::Path, sync::atomic::{AtomicBool, Ordering}, time::{Instant, SystemTime, UNIX_EPOCH}};

const ANCHOR: &str = "youtube-raw-anchor.json";
const TIMELINE: &str = "chat-timeline.json";
const CAP: u64 = 4096;
const FIRST_BYTES: u64 = 32 * 1024 * 1024;
const START_MS: u64 = 15_000;
const MAX_DELTA_MS: i128 = 86_520_000;
fn wall(value: u64) -> bool { (1_000_000_000_000..=9_000_000_000_000).contains(&value) }
fn checkpoint(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) { Err(Error::Cancelled) } else { Ok(()) }
}
fn now() -> Result<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok()
        .and_then(|value| value.as_millis().try_into().ok()).filter(|value| wall(*value)).ok_or(Error::Proof)
}
fn directory_identity(directory: &Path, identity: &SpoolIdentity) -> Result<()> {
    identity.validate()?;
    if directory.file_name().and_then(|name| name.to_str()) != Some(identity.job_id.as_str()) {
        return Err(Error::Identity);
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    version: u8,
    identity: SpoolIdentity,
    first_unit_end: u64,
    sequence: u64,
    track: String,
    append_index: u64,
    chunk_index: u32,
    offset: u64,
    length: u32,
    final_chunk: bool,
    payload_sha256: String,
    received_at_ms: u64,
    committed_at_ms: u64,
    observed_elapsed_ms: u64,
    committed_elapsed_ms: u64,
}
impl Record {
    fn validate(&self) -> Result<()> {
        self.identity.validate()?;
        let end = self.offset.checked_add(self.length as u64).ok_or(Error::Bounds)?;
        if self.version != 1 || self.track != "video" || self.sequence >= 1_100_000
            || self.length == 0 || self.length > 131072 || (!self.final_chunk && self.length != 131072)
            || self.chunk_index >= 256 || self.append_index >= 1_100_000
            || !(1..=FIRST_BYTES).contains(&self.first_unit_end)
            || self.first_unit_end > end || self.offset.saturating_sub(self.first_unit_end) > 16
            || !schema::digest_valid(&self.payload_sha256)
            || !wall(self.received_at_ms) || !wall(self.committed_at_ms)
            || self.committed_at_ms < self.received_at_ms
            || self.observed_elapsed_ms > START_MS || self.committed_elapsed_ms < self.observed_elapsed_ms
            || self.committed_elapsed_ms > 86_400_000
        { return Err(Error::Proof); }
        Ok(())
    }
    fn matches(&self, row: &Receipt) -> bool {
        row.track == Track::Video && row.sequence == self.sequence
            && row.append_index == self.append_index && row.chunk_index == self.chunk_index
            && row.offset == self.offset && row.length == self.length
            && row.final_chunk == self.final_chunk && row.payload_sha256 == self.payload_sha256
    }
}

/// Capture once before disk work, then consume only with that append's real ACK.
pub(in crate::platform_media) struct BoundaryObservation {
    begin: Instant,
    first_unit_end: u64,
    append_index: u64,
    chunk_index: u32,
    length: u32,
    final_chunk: bool,
    payload_sha256: String,
    received_at_ms: u64,
    observed_elapsed_ms: u64,
}
impl BoundaryObservation {
    pub(in crate::platform_media) fn capture(begin: Instant, first_unit_end: u64, chunk: &Chunk) -> Result<Self> {
        let received_at_ms = now()?;
        let observed_elapsed_ms = u64::try_from(begin.elapsed().as_millis()).map_err(|_| Error::Bounds)?;
        if chunk.track != Track::Video || chunk.bytes.is_empty() || chunk.bytes.len() > 131072
            || observed_elapsed_ms > START_MS || !(1..=FIRST_BYTES).contains(&first_unit_end)
        { return Err(Error::Proof); }
        Ok(Self { begin, first_unit_end, append_index: chunk.append_index, chunk_index: chunk.chunk_index,
            length: chunk.bytes.len() as u32, final_chunk: chunk.final_chunk,
            payload_sha256: schema::hash(&chunk.bytes), received_at_ms, observed_elapsed_ms })
    }
    pub(in crate::platform_media) fn persist(self, directory: &Path, identity: &SpoolIdentity, ack: &Ack, cancel: &AtomicBool) -> Result<()> {
        checkpoint(cancel)?;
        directory_identity(directory, identity)?;
        if ack.duplicate || ack.track != Track::Video || ack.append_index != self.append_index || ack.chunk_index != self.chunk_index {
            return Err(Error::Identity);
        }
        let record = Record { version: 1, identity: identity.clone(), first_unit_end: self.first_unit_end,
            sequence: ack.sequence, track: "video".into(), append_index: self.append_index, chunk_index: self.chunk_index,
            offset: ack.committed_end.checked_sub(self.length as u64).ok_or(Error::Bounds)?, length: self.length,
            final_chunk: self.final_chunk, payload_sha256: self.payload_sha256, received_at_ms: self.received_at_ms,
            committed_at_ms: now()?, observed_elapsed_ms: self.observed_elapsed_ms,
            committed_elapsed_ms: u64::try_from(self.begin.elapsed().as_millis()).map_err(|_| Error::Bounds)? };
        record.validate()?;
        create(directory, ANCHOR, &record, cancel)
    }
}

fn create(directory: &Path, name: &str, value: &impl Serialize, cancel: &AtomicBool) -> Result<()> {
    checkpoint(cancel)?;
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Proof)?;
    if bytes.is_empty() || bytes.len() as u64 > CAP { return Err(Error::Bounds); }
    let dirs = Directories::hold(directory)?;
    let mut target = GuardedFile::open(&directory.join(name), true)?;
    checkpoint(cancel)?;
    target.file.write_all(&bytes).map_err(|_| Error::Io)?;
    target.file.sync_all().map_err(|_| Error::Io)?;
    if target.check()? != bytes.len() as u64 { return Err(Error::Corrupt); }
    dirs.check()?;
    checkpoint(cancel)
}
fn load(directory: &Path, name: &str) -> Result<(GuardedFile, Directories, Vec<u8>)> {
    let dirs = Directories::hold(directory)?;
    let mut file = GuardedFile::open(&directory.join(name), false)?;
    let length = file.check()?;
    if length == 0 || length > CAP { return Err(Error::Bounds); }
    let mut bytes = vec![0; length as usize];
    file.file.read_exact(&mut bytes).map_err(|_| Error::Io)?;
    if file.check()? != length { return Err(Error::Corrupt); }
    dirs.check()?;
    Ok((file, dirs, bytes))
}

/// Optional held evidence. Caller must discard on corrupt source/failed scans.
pub(super) struct PendingAnchor {
    file: GuardedFile,
    dirs: Directories,
    length: u64,
    record: Record,
    receipt_matched: bool,
    invalid: bool,
    first_clock: Option<index::Clock>,
}
impl PendingAnchor {
    pub(super) fn open(directory: &Path, identity: &SpoolIdentity) -> Result<Self> {
        directory_identity(directory, identity)?;
        let (file, dirs, bytes) = load(directory, ANCHOR)?;
        let record: Record = serde_json::from_slice(&bytes).map_err(|_| Error::Proof)?;
        record.validate()?;
        if &record.identity != identity { return Err(Error::Identity); }
        Ok(Self { file, dirs, length: bytes.len() as u64, record, receipt_matched: false, invalid: false, first_clock: None })
    }
    /// Call only after journal ordering, chain and exact raw payload validation.
    pub(super) fn observe_receipt(&mut self, receipt: &Receipt) {
        if receipt.sequence == self.record.sequence {
            if self.receipt_matched || !self.record.matches(receipt) { self.invalid = true; }
            else { self.receipt_matched = true; }
        }
    }
    /// Only the successfully completed source scan supplies this native unit.
    pub(super) fn observe_first_unit(&mut self, unit: &index::Unit) {
        let facts = &unit.facts;
        if self.first_clock.is_some() || unit.ordinal != 0 || facts.start != 0
            || facts.end != self.record.first_unit_end || facts.configuration.format != index::Format::Vp9Webm
            || facts.clock.ticks_per_second != 1_000_000_000 || facts.clock.first < 0
            || facts.clock.last < facts.clock.first || facts.clock.end <= facts.clock.last
            || facts.clock.end.checked_sub(facts.clock.first).is_none_or(|v| v > 120_000_000_000)
        { self.invalid = true; return; }
        self.first_clock = Some(facts.clock);
    }
    pub(super) fn check(&self, cancel: &AtomicBool) -> Result<()> {
        checkpoint(cancel)?;
        self.dirs.check()?;
        if self.file.check()? != self.length || self.invalid { return Err(Error::Proof); }
        checkpoint(cancel)
    }
    pub(super) fn timeline(&self, common_origin: Rational, cancel: &AtomicBool) -> Result<RawTimelineFacts> {
        self.check(cancel)?;
        let clock = self.first_clock.ok_or(Error::Proof)?;
        if !self.receipt_matched { return Err(Error::Proof); }
        let evidence = Timing { record: self.record.clone(), native_end: clock.end,
            native_timescale: clock.ticks_per_second, common_numerator: common_origin.numerator,
            common_denominator: common_origin.denominator };
        let origin_ms = evidence.origin()?;
        self.check(cancel)?;
        Ok(RawTimelineFacts { evidence, origin_ms })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Timing {
    record: Record,
    native_end: i128,
    native_timescale: u64,
    common_numerator: i128,
    common_denominator: i128,
}
impl Timing {
    fn origin(&self) -> Result<u64> {
        self.record.validate()?;
        if self.native_end < 0 || self.native_timescale != 1_000_000_000
            || self.common_numerator < 0 || !(1..=1_000_000_000).contains(&self.common_denominator)
        { return Err(Error::Proof); }
        let scale = self.native_timescale as i128;
        let numerator = self.native_end.checked_mul(self.common_denominator)
            .and_then(|n| self.common_numerator.checked_mul(scale).and_then(|c| n.checked_sub(c)))
            .and_then(|n| n.checked_mul(1000)).ok_or(Error::Bounds)?;
        let denominator = scale.checked_mul(self.common_denominator).ok_or(Error::Bounds)?;
        let magnitude = numerator.checked_abs().ok_or(Error::Bounds)?;
        let rounded = magnitude.checked_add(denominator / 2).ok_or(Error::Bounds)? / denominator;
        if rounded > MAX_DELTA_MS { return Err(Error::Bounds); }
        let delta = if numerator < 0 { -rounded } else { rounded };
        let origin = (self.record.received_at_ms as i128).checked_sub(delta)
            .and_then(|v| u64::try_from(v).ok()).filter(|v| wall(*v)).ok_or(Error::Proof)?;
        Ok(origin)
    }
}

/// Reporting data derived from held receipt+unit facts and verified file origin.
pub(in crate::platform_media) struct RawTimelineFacts { evidence: Timing, origin_ms: u64 }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Timeline {
    version: u8,
    provider: String,
    timeline_quality: String,
    anchor_method: String,
    marker: Marker,
    origin_ms: u64,
    evidence: Timing,
}
impl RawTimelineFacts {
    pub(in crate::platform_media) fn origin_ms(&self) -> u64 { self.origin_ms }
    pub(in crate::platform_media) fn publish(&self, directory: &Path, marker: &Marker, cancel: &AtomicBool) -> Result<u64> {
        checkpoint(cancel)?;
        directory_identity(directory, &self.evidence.record.identity)?;
        chat_status::identity(marker, "youtube", &self.evidence.record.identity.job_id).map_err(|_| Error::Identity)?;
        if chat_status::read_marker(directory).map_err(|_| Error::Proof)?.as_ref() != Some(marker)
            || self.evidence.origin()? != self.origin_ms { return Err(Error::Proof); }
        let (_held, _dirs, source) = load(directory, ANCHOR)?;
        let record: Record = serde_json::from_slice(&source).map_err(|_| Error::Proof)?;
        if record != self.evidence.record { return Err(Error::Proof); }
        let value = Timeline { version: 2, provider: "youtube".into(), timeline_quality: "receive-time-approximate".into(),
            anchor_method: "native-raw-boundary".into(), marker: marker.clone(), origin_ms: self.origin_ms,
            evidence: self.evidence.clone() };
        create(directory, TIMELINE, &value, cancel)?;
        Ok(self.origin_ms)
    }
}

/// Never fall back to the legacy end-minus-duration formula after this fails.
pub(in crate::platform_media) fn replay_origin(bytes: &[u8], directory: &Path, provider: &str, job_id: &str) -> Result<u64> {
    if bytes.is_empty() || bytes.len() as u64 > CAP { return Err(Error::Bounds); }
    let timeline: Timeline = serde_json::from_slice(bytes).map_err(|_| Error::Proof)?;
    if timeline.version != 2 || provider != "youtube" || timeline.provider != provider
        || timeline.timeline_quality != "receive-time-approximate" || timeline.anchor_method != "native-raw-boundary"
        || timeline.evidence.record.identity.job_id != job_id
        || timeline.origin_ms != timeline.evidence.origin()? { return Err(Error::Proof); }
    directory_identity(directory, &timeline.evidence.record.identity)?;
    chat_status::identity(&timeline.marker, provider, job_id).map_err(|_| Error::Identity)?;
    if chat_status::read_marker(directory).map_err(|_| Error::Proof)?.as_ref() != Some(&timeline.marker) {
        return Err(Error::Identity);
    }
    // The original native observation is retained; a foreign timeline cannot
    // bind itself merely by copying the current attachment UUID.
    let (_held, _dirs, source) = load(directory, ANCHOR)?;
    let record: Record = serde_json::from_slice(&source).map_err(|_| Error::Proof)?;
    if record != timeline.evidence.record { return Err(Error::Proof); }
    Ok(timeline.origin_ms)
}

// Standalone debug QA only: reuse the exact release parser and validation. This
// projection is not source admission and cannot publish or repair a timeline.
#[cfg(debug_assertions)]
pub(super) fn inspect_qa_timeline(bytes: &[u8], directory: &Path, job_id: &str) -> Result<(Rational, RawTimelineFacts)> {
    let origin_ms = replay_origin(bytes, directory, "youtube", job_id)?;
    let timeline: Timeline = serde_json::from_slice(bytes).map_err(|_| Error::Proof)?;
    let common = Rational::new(timeline.evidence.common_numerator, timeline.evidence.common_denominator)
        .map_err(|_| Error::Proof)?;
    Ok((common, RawTimelineFacts { evidence: timeline.evidence, origin_ms }))
}
#[cfg(debug_assertions)]
impl RawTimelineFacts {
    pub(super) fn qa_matches(&self, other: &Self) -> bool {
        self.origin_ms == other.origin_ms && self.evidence == other.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    const WALL: u64 = 1_700_000_000_000;
    fn identity() -> SpoolIdentity {
        SpoolIdentity { job_id: uuid::Uuid::new_v4().to_string(), request_id: uuid::Uuid::new_v4().to_string(),
            receiver_id: uuid::Uuid::new_v4().to_string(), source_id: uuid::Uuid::new_v4().to_string(),
            document_id: uuid::Uuid::new_v4().to_string(), document_epoch: 1, source_generation: 1,
            video_id: "abcdefghijk".into(), channel_id: format!("UC{}", "a".repeat(22)), owner: super::super::Owner::Manual }
    }
    fn record(identity: &SpoolIdentity) -> Record {
        Record { version: 1, identity: identity.clone(), first_unit_end: 100, sequence: 2, track: "video".into(),
            append_index: 1, chunk_index: 0, offset: 100, length: 8, final_chunk: true,
            payload_sha256: schema::hash(&[1; 8]), received_at_ms: WALL, committed_at_ms: WALL + 9000,
            observed_elapsed_ms: 5000, committed_elapsed_ms: 14000 }
    }
    fn receipt(record: &Record) -> Receipt {
        Receipt { sequence: record.sequence, track: Track::Video, append_index: record.append_index,
            chunk_index: record.chunk_index, offset: record.offset, length: record.length,
            final_chunk: record.final_chunk, payload_sha256: record.payload_sha256.clone(), elapsed_ms: 14000,
            previous_record_sha256: "0".repeat(64) }
    }
    fn unit() -> index::Unit {
        index::Unit { ordinal: 0, configuration_sha256: [0; 32], init_relation: index::InitRelation::First,
            clock_relation: index::ClockRelation::First, codec_packet_validation_pending: true,
            facts: index::UnitDraft { start: 0, init_end: 30, end: 100,
                boundary: index::Boundary::NextInitialization, init_sha256: [0; 32],
                configuration: index::Configuration { format: index::Format::Vp9Webm, track_id: 1, timebase: 1_000_000,
                    width: 854, height: 480, default_duration_ns: Some(33_333_333), sample_rate: 0, channels: 0, semantic_sha256: [0;32] },
                clock: index::Clock { first: 100_000_000_000, last: 104_966_666_667, end: 105_000_000_000, ticks_per_second: 1_000_000_000 },
                items: 150, fragments: 1, rap_candidate: true, references: 1,
                reconstruction: index::ReconstructionFacts::Webm(index::WebmReconstruction::default()) } }
    }
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, SpoolIdentity, Record) {
        let temp = tempfile::tempdir().unwrap();
        let id = identity(); let directory = temp.path().join(&id.job_id);
        fs::create_dir(&directory).unwrap();
        let record = record(&id);
        create(&directory, ANCHOR, &record, &AtomicBool::new(false)).unwrap();
        (temp, directory, id, record)
    }
    fn matched(directory: &Path, id: &SpoolIdentity, record: &Record) -> PendingAnchor {
        let mut pending = PendingAnchor::open(directory, id).unwrap();
        pending.observe_receipt(&receipt(record)); pending.observe_first_unit(&unit());
        pending
    }
    fn origin(numerator: i128, denominator: i128) -> Rational { Rational::new(numerator, denominator).unwrap() }

    #[test]
    fn delayed_ack_and_remux_do_not_move_native_observation() {
        let (_temp, directory, id, record) = fixture();
        let pending = matched(&directory, &id, &record);
        let facts = pending.timeline(origin(100, 1), &AtomicBool::new(false)).unwrap();
        assert_eq!(facts.origin_ms(), WALL - 5000);
        assert_eq!(facts.evidence.record.committed_at_ms, WALL + 9000);
        assert_eq!(pending.timeline(origin(110, 1), &AtomicBool::new(false)).unwrap().origin_ms(), WALL + 5000);
        assert_eq!(pending.timeline(origin(99_985_236, 1_000_000), &AtomicBool::new(false)).unwrap().origin_ms(), WALL - 5015);
    }
    #[test]
    fn native_receipt_and_structural_unit_are_both_required() {
        let (_temp, directory, id, record) = fixture();
        let cancel = AtomicBool::new(false);
        let mut pending = PendingAnchor::open(&directory, &id).unwrap();
        assert!(pending.timeline(origin(100, 1), &cancel).is_err());
        pending.observe_receipt(&receipt(&record));
        assert!(pending.timeline(origin(100, 1), &cancel).is_err());
        pending.observe_first_unit(&unit());
        assert!(pending.timeline(origin(100, 1), &cancel).is_ok());
        pending.observe_receipt(&receipt(&record));
        assert!(pending.timeline(origin(100, 1), &cancel).is_err());
    }
    #[test]
    fn receipt_and_unit_mismatches_latch_invalid() {
        let (_temp, directory, id, record) = fixture();
        for case in 0..6 {
            let mut pending = PendingAnchor::open(&directory, &id).unwrap();
            let mut row = receipt(&record);
            let mut native = unit();
            match case { 0 => row.track = Track::Audio, 1 => row.payload_sha256 = "0".repeat(64),
                2 => row.offset += 1, 3 => native.ordinal = 1, 4 => native.facts.end -= 1,
                _ => native.facts.clock.end = native.facts.clock.first }
            pending.observe_receipt(&row); pending.observe_first_unit(&native);
            assert!(pending.timeline(origin(100, 1), &AtomicBool::new(false)).is_err());
        }
    }
    #[test]
    fn malformed_and_implausible_time_has_no_fallback() {
        let (_temp, directory, id, record) = fixture();
        let pending = matched(&directory, &id, &record);
        for common in [origin(-1, 1), origin(200_000, 1), Rational { numerator: i128::MAX, denominator: 1 },
            Rational { numerator: 100, denominator: 0 }] {
            assert!(pending.timeline(common, &AtomicBool::new(false)).is_err());
        }
        assert!(pending.timeline(origin(100, 1), &AtomicBool::new(true)).is_err());
        let mut altered = record.clone(); altered.first_unit_end = altered.offset - 16; assert!(altered.validate().is_ok());
        altered.first_unit_end -= 1; assert!(altered.validate().is_err());
        altered = record.clone(); altered.first_unit_end = altered.offset + altered.length as u64 + 1; assert!(altered.validate().is_err());
        altered = record.clone(); altered.observed_elapsed_ms = START_MS + 1; assert!(altered.validate().is_err());
        altered = record; altered.committed_at_ms = WALL - 1; assert!(altered.validate().is_err());
    }
    #[test]
    fn observation_is_video_only_one_shot_and_duplicate_ack_cannot_refresh() {
        let temp = tempfile::tempdir().unwrap(); let id = identity();
        let directory = temp.path().join(&id.job_id); fs::create_dir(&directory).unwrap();
        let mut chunk = Chunk { track: Track::Video, append_index: 0, chunk_index: 0, final_chunk: true, bytes: vec![1; 8] };
        let mut ack = Ack { sequence: 0, track: Track::Video, append_index: 0, chunk_index: 0, committed_end: 8, duplicate: true };
        let cancel = AtomicBool::new(false);
        let observation = BoundaryObservation::capture(Instant::now(), 8, &chunk).unwrap();
        assert!(observation.persist(&directory, &id, &ack, &cancel).is_err());
        assert!(!directory.join(ANCHOR).exists());
        ack.duplicate = false;
        BoundaryObservation::capture(Instant::now(), 8, &chunk).unwrap().persist(&directory, &id, &ack, &cancel).unwrap();
        let before = fs::read(directory.join(ANCHOR)).unwrap();
        assert!(BoundaryObservation::capture(Instant::now(), 8, &chunk).unwrap().persist(&directory, &id, &ack, &cancel).is_err());
        assert_eq!(fs::read(directory.join(ANCHOR)).unwrap(), before);
        chunk.track = Track::Audio; assert!(BoundaryObservation::capture(Instant::now(), 8, &chunk).is_err());
        chunk.track = Track::Video;
        assert!(BoundaryObservation::capture(Instant::now() - std::time::Duration::from_secs(16), 8, &chunk).is_err());
    }
    #[test]
    fn sidecar_identity_schema_and_size_are_closed() {
        let (_temp, directory, id, record) = fixture();
        let mut wrong = id.clone(); wrong.receiver_id = uuid::Uuid::new_v4().to_string();
        assert!(PendingAnchor::open(&directory, &wrong).is_err());
        let mut json = serde_json::to_value(&record).unwrap(); json["unknown"] = true.into();
        fs::write(directory.join(ANCHOR), serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(PendingAnchor::open(&directory, &id).is_err());
        fs::write(directory.join(ANCHOR), vec![b' '; CAP as usize + 1]).unwrap();
        assert!(PendingAnchor::open(&directory, &id).is_err());
    }
    #[test]
    fn v2_binds_marker_source_receipt_and_formula() {
        let (_temp, directory, id, record) = fixture();
        let cancel = AtomicBool::new(false);
        let pending = matched(&directory, &id, &record);
        let facts = pending.timeline(origin(100, 1), &cancel).unwrap();
        let marker = chat_status::begin_multipart(&directory, "youtube", &id.job_id).unwrap();
        assert_eq!(facts.publish(&directory, &marker, &cancel).unwrap(), WALL - 5000);
        let bytes = fs::read(directory.join(TIMELINE)).unwrap();
        assert_eq!(replay_origin(&bytes, &directory, "youtube", &id.job_id).unwrap(), WALL - 5000);
        assert!(replay_origin(&bytes, &directory, "soop", &id.job_id).is_err());
        assert!(replay_origin(&bytes, &directory, "youtube", &uuid::Uuid::new_v4().to_string()).is_err());
        for case in 0..5 {
            let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            match case { 0 => changed["originMs"] = WALL.into(), 1 => changed["version"] = 3.into(),
                2 => changed["marker"]["attachmentId"] = uuid::Uuid::new_v4().to_string().into(),
                3 => changed["evidence"]["record"]["payloadSha256"] = "0".repeat(64).into(),
                _ => changed["extra"] = true.into() }
            assert!(replay_origin(&serde_json::to_vec(&changed).unwrap(), &directory, "youtube", &id.job_id).is_err());
        }
        assert!(facts.publish(&directory, &marker, &cancel).is_err(), "no overwrite on replay/retry");
    }
}
