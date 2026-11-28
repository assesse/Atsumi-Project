//! SOOP-only retention of complete samples rejected by strict normal finish.
//! Retention files never enter the normal manifest or authorize publication.
//! A separately validated overlap derivative can use normal Writer.commit.
use super::*;

pub(super) const BUNDLE: &str = "encoded-pending-tail.bin";
pub(super) const MANIFEST: &str = "encoded-pending-tail.json";
const PARTIAL: &str = "encoded-pending-tail.mp4";
const OVERLAP: &str = "encoded-tail-overlap.json";
const MAGIC: &[u8; 16] = b"ATSUMI-SOOP-TAIL";
const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
const MAX_SAMPLES: usize = 120_000;
// Each of the at most two original inputs admits64KiB; the merged init also
// contains a small native ftyp/moov wrapper. Do not reject valid two-input data.
const MAX_INIT: usize = 2 * 64 * 1024 + 1024;
const MAX_BUNDLE: usize = MAX_PAYLOAD + MAX_SAMPLES * 24 + MAX_INIT + 40;

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Report {
    state: &'static str,
    sample_count: u64,
    sample_bytes: u64,
    partial_mp4: &'static str,
}
impl Report {
    pub(super) fn unavailable() -> Self {
        Self { state: "unavailable", sample_count: 0, sample_bytes: 0, partial_mp4: "unavailable" }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileDigest { file: String, bytes: u64, sha256: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TailBinding { version: u32, job_id: String, owner_sha256: String, manifest_sha256: Option<String> }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrackSummary { timescale: u32, samples: u64, sample_bytes: u64, first_dts: Option<u64>, last_end: Option<u64> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetainedRecord {
    version: u32, kind: String, state: String, binding: TailBinding, reason: String,
    format: String, bundle: FileDigest, video: TrackSummary, audio: TrackSummary,
    partial_mp4: serde_json::Value, normal_segment: bool, publication_authorized: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    samples: u64, sample_bytes: u64, last_end: u64,
    excluded_samples: u64, excluded_bytes: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OverlapProof {
    version: u32, job_id: String, owner_sha256: String,
    publication_authorized: bool,
    retention_sha256: String, bundle_sha256: String,
    parent_sha256: Option<String>, parent_bytes: u64, parent_segments: u64,
    video: Selection, audio: Selection, appended: SegmentProof,
    result_sha256: String, result_bytes: u64, result_segments: u64,
}

fn exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => { storage::regular(path)?; Ok(true) },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ERROR.into()),
    }
}
fn read_held(path: &Path, limit: u64) -> Result<(fs::File, Vec<u8>), String> {
    use std::io::Read;
    let mut file = pin(path)?;
    let length = file.metadata().map_err(|_| ERROR)?.len();
    if length == 0 || length > limit { return Err(ERROR.into()); }
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes).map_err(|_| ERROR)?;
    if bytes.len() as u64 != length { return Err(ERROR.into()); }
    Ok((file, bytes))
}
fn pin(path: &Path) -> Result<fs::File, String> {
    // Validate the original spelling and every ancestor before canonicalizing;
    // the existing native guard also requires Windows' canonical spelling.
    storage::regular(path)?;
    let canonical = fs::canonicalize(path).map_err(|_| ERROR)?;
    storage::safe_path(&canonical)?;
    crate::streaming::browser_store::cleanup::verification_guard(&canonical).map_err(|_| ERROR.into())
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn retained(job: &MediaJob, directory: &Path, cancel: &AtomicBool) -> Result<(RetainedRecord, String, fs::File, fs::File), String> {
    validate_proof(job, directory)?;
    let (manifest, bytes) = read_held(&directory.join(MANIFEST), 8192)?;
    let record: RetainedRecord = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    if record.version != 1 || record.kind != "soop-pending-tail" || record.state != "retained"
        || record.reason != "incomplete_audio_tail" || record.format != "atsumi-soop-pending-v1"
        || record.normal_segment || record.publication_authorized || record.binding.version != 1
        || record.binding.job_id != job.job_id
        || record.binding.owner_sha256 != storage::hash(&directory.join(PROOF), cancel)?
        || record.bundle.file != BUNDLE || record.bundle.bytes == 0 || record.bundle.bytes > MAX_BUNDLE as u64
        || record.video.samples.checked_add(record.audio.samples).is_none_or(|n|n > MAX_SAMPLES as u64)
        || record.video.sample_bytes.checked_add(record.audio.sample_bytes).is_none_or(|n|n > MAX_PAYLOAD as u64)
    { return Err(ERROR.into()); }
    for track in [&record.video, &record.audio] {
        if track.timescale == 0 || (track.samples == 0) != track.first_dts.is_none()
            || (track.samples == 0) != track.last_end.is_none()
            || (track.samples > 0 && (track.sample_bytes == 0 || track.last_end <= track.first_dts))
        { return Err(ERROR.into()); }
    }
    let bundle = pin(&directory.join(BUNDLE))?;
    if bundle.metadata().map_err(|_| ERROR)?.len() != record.bundle.bytes
        || storage::hash(&directory.join(BUNDLE), cancel)? != record.bundle.sha256 { return Err(ERROR.into()); }
    Ok((record, digest(&bytes), manifest, bundle))
}

fn selection(track: &fmp4::RetainedTrack, selected: usize) -> Selection {
    Selection { samples: selected as u64, sample_bytes: track.samples[..selected].iter().map(|s|s.data.len() as u64).sum(),
        last_end: track.samples[selected-1].dts + u64::from(track.samples[selected-1].duration),
        excluded_samples: (track.samples.len()-selected) as u64,
        excluded_bytes: track.samples[selected..].iter().map(|s|s.data.len() as u64).sum() }
}
fn valid_selection(selected: &Selection, original: &TrackSummary) -> bool {
    selected.samples > 0 && selected.sample_bytes > 0
        && selected.samples.checked_add(selected.excluded_samples) == Some(original.samples)
        && selected.sample_bytes.checked_add(selected.excluded_bytes) == Some(original.sample_bytes)
        && original.first_dts.is_some_and(|start| selected.last_end > start)
        && original.last_end.is_some_and(|end| selected.last_end <= end)
        && (selected.excluded_samples == 0) == (selected.excluded_bytes == 0)
}

/// Called by both normal finalization and restart recovery, BEFORE STOP-missing
/// can mean complete. A reserved/torn/removed proof never authorizes recovery.
pub(super) fn validate_overlap(job: &MediaJob, directory: &Path, cancel: &AtomicBool)
    -> Result<Option<([u64; 2], [u64; 2])>, String>
{
    let has_tail = exists(&directory.join(MANIFEST))?;
    let has_overlap = exists(&directory.join(OVERLAP))?;
    let has_blob = exists(&directory.join(BUNDLE))?;
    let has_partial = exists(&directory.join(PARTIAL))?;
    if !has_tail && !has_overlap && !has_blob && !has_partial { return Ok(None); }
    if !has_tail || cancel.load(Ordering::Acquire) { return Err(ERROR.into()); }
    let (record, retention_sha, _retention, _bundle) = retained(job, directory, cancel)?;
    let normal_path = directory.join(super::super::MANIFEST);
    let normal = if exists(&normal_path)? { read_held(&normal_path, 32*1024*1024)?.1 } else { Vec::new() };
    if !has_overlap {
        let current = (!normal.is_empty()).then(||digest(&normal));
        if current != record.binding.manifest_sha256 { return Err(ERROR.into()); }
        return Ok(Some(([0;2], [0;2])));
    }
    let (_proof_file, bytes) = read_held(&directory.join(OVERLAP), 8192)?;
    let proof: OverlapProof = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    let parent_len = usize::try_from(proof.parent_bytes).map_err(|_| ERROR)?;
    if proof.version != 1 || proof.publication_authorized || proof.job_id != job.job_id || proof.owner_sha256 != record.binding.owner_sha256
        || proof.retention_sha256 != retention_sha || proof.bundle_sha256 != record.bundle.sha256
        || proof.parent_sha256 != record.binding.manifest_sha256 || parent_len >= normal.len()
        || proof.result_bytes != normal.len() as u64 || proof.result_sha256 != digest(&normal)
        || proof.parent_segments >= MAX_SEGMENTS as u64 || proof.result_segments != proof.parent_segments + 1
        || proof.parent_sha256 != (parent_len > 0).then(||digest(&normal[..parent_len]))
        || !valid_selection(&proof.video, &record.video) || !valid_selection(&proof.audio, &record.audio)
        || u128::from(proof.video.last_end) * u128::from(record.audio.timescale)
            > u128::from(proof.audio.last_end) * u128::from(record.video.timescale)
    { return Err(ERROR.into()); }
    let parent = std::str::from_utf8(&normal[..parent_len]).map_err(|_| ERROR)?;
    let appended = std::str::from_utf8(&normal[parent_len..]).map_err(|_| ERROR)?;
    if (!parent.is_empty() && !parent.ends_with('\n')) || parent.lines().count() as u64 != proof.parent_segments
        || !appended.ends_with('\n') || appended.lines().count() != 1 { return Err(ERROR.into()); }
    let part: SegmentProof = serde_json::from_str(appended.trim_end()).map_err(|_| ERROR)?;
    let expected = &proof.appended;
    if segment_sequence(&part.name) != Some(proof.parent_segments) || part.name != expected.name
        || part.sha256 != expected.sha256 || part.bytes != expected.bytes
        || part.duration != expected.duration || part.source_start != expected.source_start || part.source_end != expected.source_end
    { return Err(ERROR.into()); }
    // This also validates every normal part's owned filename, bounds and hash.
    if recovery_segments_cancellable(job, directory, cancel)?.len() as u64 != proof.result_segments { return Err(ERROR.into()); }
    Ok(Some(([proof.video.excluded_samples, proof.audio.excluded_samples], [proof.video.excluded_bytes, proof.audio.excluded_bytes])))
}

/// Admission is possible only with this turn's freshly retained snapshot. The
/// on-disk bundle is not a general restore endpoint or a publication authority.
pub(super) fn commit_overlap(writer: &mut Writer, snapshot: &fmp4::PendingSnapshot) -> Result<bool, String> {
    owner(&writer.lease.service, &writer.job, &writer.cancel)?;
    let Some(candidate) = snapshot.complete_overlap().map_err(|e|e.message)? else { return Ok(false); };
    let (record, retention_sha, _retention, _bundle) = retained(&writer.job, &writer.directory, &writer.cancel)?;
    // Bind the exact in-memory snapshot to the durable lossless bundle without
    // allocating a second serialization-sized buffer.
    struct HashOutput(Sha256);
    impl std::io::Write for HashOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0.update(bytes); Ok(bytes.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let check = || owner(&writer.lease.service, &writer.job, &writer.cancel);
    let mut hash = HashOutput(Sha256::new());
    let (_, _, length) = write_bundle(&mut hash, snapshot, &check)?;
    if record.bundle.bytes != length as u64 || record.bundle.sha256 != format!("{:x}", hash.0.finalize()) { return Err(ERROR.into()); }
    let parent_path = writer.directory.join(super::super::MANIFEST);
    let parent = if exists(&parent_path)? { read_held(&parent_path,32*1024*1024)?.1 } else { Vec::new() };
    let parent_sha = (!parent.is_empty()).then(||digest(&parent));
    if parent_sha != record.binding.manifest_sha256
        || std::str::from_utf8(&parent).map_err(|_| ERROR)?.lines().count() != writer.segments.len()
    { return Err(ERROR.into()); }
    if !writer.segments.is_empty() && !same_segments(&recovery_segments_cancellable(&writer.job,&writer.directory,&writer.cancel)?,&writer.segments) {
        return Err(ERROR.into());
    }
    let mut proof = OverlapProof { version:1, publication_authorized:false, job_id:writer.job.job_id.clone(), owner_sha256:record.binding.owner_sha256,
        retention_sha256:retention_sha, bundle_sha256:record.bundle.sha256,
        parent_sha256:parent_sha, parent_bytes:parent.len() as u64, parent_segments:writer.segments.len() as u64,
        video:selection(&snapshot.video,candidate.video_samples), audio:selection(&snapshot.audio,candidate.audio_samples),
        appended:SegmentProof { name:format!("e{:08}.mp4",writer.segments.len()), duration:candidate.segment.duration_seconds,
            source_start:candidate.segment.source_start_seconds, source_end:candidate.segment.source_end_seconds,
            bytes:candidate.segment.bytes.len() as u64, sha256:digest(&candidate.segment.bytes) },
        result_sha256:String::new(), result_bytes:0, result_segments:writer.segments.len() as u64+1 };
    // Reserve BEFORE the append. An empty file after a crash is a persistent
    // fail-closed marker, not a signal to repeat or skip this operation.
    let mut reserved = create(&writer.directory.join(OVERLAP))?;
    reserved.sync_all().map_err(|_| ERROR)?;
    owner(&writer.lease.service,&writer.job,&writer.cancel)?;
    writer.commit(vec![candidate.segment])?;
    owner(&writer.lease.service,&writer.job,&writer.cancel)?;
    let result = read_held(&parent_path,32*1024*1024)?.1;
    if !result.starts_with(&parent) || result.len() <= parent.len() || writer.segments.len() as u64 != proof.result_segments { return Err(ERROR.into()); }
    proof.result_bytes=result.len() as u64; proof.result_sha256=digest(&result);
    let bytes=serde_json::to_vec(&proof).map_err(|_| ERROR)?;
    if bytes.len()>8192 { return Err(ERROR.into()); }
    reserved.write_all(&bytes).and_then(|_|reserved.sync_all()).map_err(|_| ERROR)?;
    hash_written(&mut reserved,bytes.len() as u64,&||owner(&writer.lease.service,&writer.job,&writer.cancel))?;
    drop(reserved);
    validate_overlap(&writer.job,&writer.directory,&writer.cancel)?;
    Ok(true)
}

fn lengths(snapshot: &fmp4::PendingSnapshot) -> Result<(usize, usize, usize), String> {
    let count = snapshot.video.samples.len().checked_add(snapshot.audio.samples.len()).ok_or(ERROR)?;
    let payload = snapshot.video.samples.iter().chain(&snapshot.audio.samples)
        .try_fold(0usize, |sum, sample| {
            if sample.duration == 0 || sample.data.is_empty() || sample.dts.checked_add(u64::from(sample.duration)).is_none() {
                return Err(ERROR);
            }
            sum.checked_add(sample.data.len()).ok_or(ERROR)
        })?;
    let bytes = count.checked_mul(24).and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(snapshot.init.len())).and_then(|n| n.checked_add(40)).ok_or(ERROR)?;
    if count == 0 || count > MAX_SAMPLES || payload > MAX_PAYLOAD || snapshot.init.is_empty()
        || snapshot.init.len() > MAX_INIT || bytes > MAX_BUNDLE
        || snapshot.video.timescale == 0 || snapshot.audio.timescale == 0
    { return Err(ERROR.into()); }
    Ok((count, payload, bytes))
}

// All fields are fixed-width big-endian integers. Track order is video, audio.
// No floats, paths or deserialized caller bytes enter this native-only format.
fn write_bundle(
    output: &mut impl std::io::Write, snapshot: &fmp4::PendingSnapshot,
    check: &impl Fn() -> Result<(), String>,
) -> Result<(usize, usize, usize), String> {
    let bounds = lengths(snapshot)?;
    check()?;
    let mut since_check = 0usize;
    let mut write = |bytes: &[u8]| -> Result<(), String> {
        for chunk in bytes.chunks(64 * 1024) {
            since_check = since_check.saturating_add(chunk.len());
            if since_check >= 64 * 1024 { check()?; since_check = 0; }
            output.write_all(chunk).map_err(|_| ERROR)?;
        }
        Ok(())
    };
    write(MAGIC)?;
    write(&1u32.to_be_bytes())?;
    write(&(snapshot.init.len() as u32).to_be_bytes())?;
    for track in [&snapshot.video, &snapshot.audio] {
        write(&track.timescale.to_be_bytes())?;
        write(&(track.samples.len() as u32).to_be_bytes())?;
    }
    write(&snapshot.init)?;
    for track in [&snapshot.video, &snapshot.audio] {
        for sample in &track.samples {
            let mut header = [0u8;24];
            header[..8].copy_from_slice(&sample.dts.to_be_bytes());
            header[8..12].copy_from_slice(&sample.duration.to_be_bytes());
            header[12..16].copy_from_slice(&sample.flags.to_be_bytes());
            header[16..20].copy_from_slice(&sample.cts.to_be_bytes());
            header[20..24].copy_from_slice(&(sample.data.len() as u32).to_be_bytes());
            write(&header)?;
            write(&sample.data)?;
        }
    }
    check()?;
    Ok(bounds)
}

fn create(path: &Path) -> Result<fs::File, String> {
    storage::safe_path(path)?;
    let mut options = fs::OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // Other readers only while this attempt writes.
    }
    options.open(path).map_err(|_| ERROR.into())
}

// Keep the create_new write/delete exclusion until the digest is complete.
// Reopening via storage::regular while this writable handle exists conflicts
// with Windows' strict read-only sharing, even when the bytes were synced.
fn hash_written(
    file: &mut fs::File, expected: u64, check: &impl Fn() -> Result<(), String>,
) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    check()?;
    if expected > (MAX_BUNDLE + 1024) as u64 || file.metadata().map_err(|_| ERROR)?.len() != expected {
        return Err(ERROR.into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| ERROR)?;
    let mut hasher = Sha256::new();
    let mut remaining = expected;
    let mut bytes = [0u8; 64 * 1024];
    while remaining > 0 {
        check()?;
        let limit = remaining.min(bytes.len() as u64) as usize;
        let count = file.read(&mut bytes[..limit]).map_err(|_| ERROR)?;
        if count == 0 { return Err(ERROR.into()); }
        hasher.update(&bytes[..count]);
        remaining -= count as u64;
    }
    check()?;
    if file.read(&mut bytes[..1]).map_err(|_| ERROR)? != 0 || file.metadata().map_err(|_| ERROR)?.len() != expected {
        return Err(ERROR.into());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn track_summary(track: &fmp4::RetainedTrack) -> serde_json::Value {
    serde_json::json!({"timescale":track.timescale,"samples":track.samples.len(),
        "sampleBytes":track.samples.iter().map(|s| s.data.len() as u64).sum::<u64>(),
        "firstDts":track.samples.first().map(|s|s.dts),
        "lastEnd":track.samples.last().and_then(|s|s.dts.checked_add(u64::from(s.duration)))})
}

pub(super) fn preserve(writer: &Writer, snapshot: &fmp4::PendingSnapshot) -> Result<Report, String> {
    let check = || owner(&writer.lease.service, &writer.job, &writer.cancel);
    check()?;
    // The first pending tail may fail before a prefix manifest exists. Retain
    // it under its native encoded-owner proof without inventing a STOP proof.
    validate_proof(&writer.job, &writer.directory)?;
    let prefix = writer.directory.join(super::super::MANIFEST);
    let manifest_sha = if prefix.exists() {
        if storage::regular(&prefix)?.len() > 32 * 1024 * 1024 { return Err(ERROR.into()); }
        Some(storage::hash(&prefix, &writer.cancel)?)
    } else { None };
    let proof = serde_json::json!({"version":1,"job_id":writer.job.job_id,
        "owner_sha256":storage::hash(&writer.directory.join(PROOF),&writer.cancel)?,
        "manifest_sha256":manifest_sha});
    lengths(snapshot)?;
    let blob_path = writer.directory.join(BUNDLE);
    let mut blob = create(&blob_path)?;
    let (count, payload, expected_bytes) = write_bundle(&mut blob, snapshot, &check)?;
    blob.sync_all().map_err(|_| ERROR)?;
    if blob.metadata().map_err(|_| ERROR)?.len() != expected_bytes as u64 { return Err(ERROR.into()); }
    check()?;
    let blob_sha = hash_written(&mut blob, expected_bytes as u64, &check)?;
    let mut derivative = serde_json::json!({"state":"unavailable"});
    // A decodable-looking derivative is still unverified and incomplete. Its
    // failure cannot destroy the independently retained lossless sample bundle.
    if let Ok(segment) = snapshot.partial_mp4() {
        let attempt = (|| -> Result<serde_json::Value, String> {
            check()?;
            let path = writer.directory.join(PARTIAL);
            let mut file = create(&path)?;
            for chunk in segment.bytes.chunks(64 * 1024) { check()?; file.write_all(chunk).map_err(|_| ERROR)?; }
            file.sync_all().map_err(|_| ERROR)?;
            check()?;
            let sha = hash_written(&mut file, segment.bytes.len() as u64, &check)?;
            Ok(serde_json::json!({"state":"unverified","file":PARTIAL,"bytes":segment.bytes.len(),
                "sha256":sha,"durationSeconds":segment.duration_seconds,
                "sourceStartSeconds":segment.source_start_seconds,"sourceEndSeconds":segment.source_end_seconds}))
        })();
        if let Ok(value) = attempt { derivative = value; }
    }
    check()?;
    let partial_state = if derivative["state"] == "unverified" { "unverified" } else { "unavailable" };
    let manifest = serde_json::json!({"version":1,"kind":"soop-pending-tail","state":"retained",
        "binding":proof,"reason":"incomplete_audio_tail","format":"atsumi-soop-pending-v1",
        "bundle":{"file":BUNDLE,"bytes":expected_bytes,"sha256":blob_sha},
        "video":track_summary(&snapshot.video),"audio":track_summary(&snapshot.audio),"partialMp4":derivative,
        "normalSegment":false,"publicationAuthorized":false});
    let bytes = serde_json::to_vec(&manifest).map_err(|_| ERROR)?;
    if bytes.len() > 8192 { return Err(ERROR.into()); }
    let mut file = create(&writer.directory.join(MANIFEST))?;
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|_| ERROR)?;
    check()?;
    Ok(Report { state: "retained", sample_count: count as u64, sample_bytes: payload as u64, partial_mp4: partial_state })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Bounded reader exists only for pinned fixture verification, never as an
    // automatic archive restore/publication endpoint.
    fn read_bundle(bytes: &[u8]) -> Result<fmp4::PendingSnapshot,String> {
        use std::io::Read;
        if bytes.len()>MAX_BUNDLE || bytes.get(..16)!=Some(MAGIC.as_slice()) {return Err(ERROR.into());}
        let mut cursor=std::io::Cursor::new(&bytes[16..]);
        fn word(cursor:&mut impl std::io::Read)->Result<u32,String> {
            let mut b=[0;4];cursor.read_exact(&mut b).map_err(|_|ERROR)?;Ok(u32::from_be_bytes(b))
        }
        if word(&mut cursor)?!=1 {return Err(ERROR.into());}
        let init_len=word(&mut cursor)? as usize;
        let vs=word(&mut cursor)?;let vn=word(&mut cursor)? as usize;
        let aus=word(&mut cursor)?;let an=word(&mut cursor)? as usize;
        if init_len==0 || init_len>MAX_INIT || vn.checked_add(an).is_none_or(|n|n>MAX_SAMPLES) {return Err(ERROR.into());}
        let mut init=vec![0;init_len];cursor.read_exact(&mut init).map_err(|_|ERROR)?;
        let mut payload=0usize;
        let mut track=|scale,count|->Result<fmp4::RetainedTrack,String> {
            let mut samples=Vec::new();
            for _ in 0..count {
                let mut b=[0;8];cursor.read_exact(&mut b).map_err(|_|ERROR)?;let dts=u64::from_be_bytes(b);
                let duration=word(&mut cursor)?;let flags=word(&mut cursor)?;let cts=word(&mut cursor)? as i32;
                let length=word(&mut cursor)? as usize;
                payload=payload.checked_add(length).filter(|n|*n<=MAX_PAYLOAD).ok_or(ERROR)?;
                if length==0 || length>bytes.len().saturating_sub(16+cursor.position() as usize) {return Err(ERROR.into());}
                let mut data=vec![0;length];cursor.read_exact(&mut data).map_err(|_|ERROR)?;
                samples.push(fmp4::RetainedSample {dts,duration,flags,cts,data});
            }
            Ok(fmp4::RetainedTrack {timescale:scale,samples})
        };
        let snapshot=fmp4::PendingSnapshot {init,video:track(vs,vn)?,audio:track(aus,an)?};
        if cursor.position() as usize+16!=bytes.len() {return Err(ERROR.into());}
        lengths(&snapshot)?;
        // Full continuity/CTS/clock bounds, before using the trusted codec init.
        snapshot.complete_overlap().map_err(|_|ERROR)?;
        Ok(snapshot)
    }
    fn snapshot() -> fmp4::PendingSnapshot {
        let mut muxer = fmp4::EncodedMuxer::new(fmp4::fixtures::inputs()).unwrap();
        muxer.push(0, &fmp4::fixtures::fragment(0, 0, 4, true)).unwrap();
        muxer.push(1, &fmp4::fixtures::fragment(1, 0, 2, true)).unwrap();
        muxer.pending_snapshot().unwrap()
    }
    #[test]
    fn lossless_bundle_retains_exact_samples_and_integer_fields() {
        let mut source = snapshot();
        source.video.samples[1].cts = -31;
        let mut bytes = Vec::new();
        let (_, _, length) = write_bundle(&mut bytes, &source, &||Ok(())).unwrap();
        assert_eq!(bytes.len(),length);
        assert_eq!(&bytes[..16],MAGIC);
        let mut cursor = 16;
        fn word(bytes:&[u8], cursor:&mut usize) -> u32 { let n=u32::from_be_bytes(bytes[*cursor..*cursor+4].try_into().unwrap());*cursor+=4;n }
        assert_eq!(word(&bytes,&mut cursor),1);
        let init=word(&bytes,&mut cursor) as usize;
        for track in [&source.video,&source.audio] {
            assert_eq!(word(&bytes,&mut cursor),track.timescale);
            assert_eq!(word(&bytes,&mut cursor),track.samples.len() as u32);
        }
        assert_eq!(&bytes[cursor..cursor+init],source.init);cursor+=init;
        for track in [&source.video,&source.audio] { for sample in &track.samples {
            let dts=u64::from_be_bytes(bytes[cursor..cursor+8].try_into().unwrap());cursor+=8;
            assert_eq!(dts,sample.dts);assert_eq!(word(&bytes,&mut cursor),sample.duration);
            assert_eq!(word(&bytes,&mut cursor),sample.flags);assert_eq!(word(&bytes,&mut cursor) as i32,sample.cts);
            let length=word(&bytes,&mut cursor) as usize;
            assert_eq!(&bytes[cursor..cursor+length],sample.data);cursor+=length;
        }}
        assert_eq!(cursor,bytes.len());
    }
    #[test]
    fn bundle_bounds_and_cancellation_do_not_claim_retention() {
        let mut source=snapshot();let mut bytes=Vec::new();
        assert!(write_bundle(&mut bytes,&source,&||Err("cancelled".into())).is_err());assert!(bytes.is_empty());
        source.video.timescale=0;assert!(write_bundle(&mut bytes,&source,&||Ok(())).is_err());
        source.video.timescale=90000;source.video.samples[0].duration=0;
        assert!(write_bundle(&mut bytes,&source,&||Ok(())).is_err());assert!(bytes.is_empty());
    }
    #[test]
    fn retained_bundle_reader_rejects_truncation_trailing_data_and_unknown_schema() {
        let original=snapshot();let mut bytes=Vec::new();write_bundle(&mut bytes,&original,&||Ok(())).unwrap();
        assert_eq!(read_bundle(&bytes).unwrap(),original);
        assert!(read_bundle(&bytes[..bytes.len()-1]).is_err());
        let mut trailing=bytes.clone();trailing.push(0);assert!(read_bundle(&trailing).is_err());
        bytes[19]=2;assert!(read_bundle(&bytes).is_err());
    }
    #[test]
    fn retention_files_are_write_once_and_regular() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join(BUNDLE);
        let mut first=create(&path).unwrap();first.write_all(b"original").unwrap();first.sync_all().unwrap();drop(first);
        assert!(create(&path).is_err());assert_eq!(fs::read(path).unwrap(),b"original");
    }
    #[test]
    fn synced_write_handle_is_hashed_without_reopening_or_dropping_exclusion() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join(BUNDLE);
        let mut file=create(&path).unwrap();file.write_all(b"retained samples").unwrap();file.sync_all().unwrap();
        assert_eq!(hash_written(&mut file,16,&||Ok(())).unwrap(),format!("{:x}",Sha256::digest(b"retained samples")));
        assert!(hash_written(&mut file,15,&||Ok(())).is_err());
        assert!(hash_written(&mut file,16,&||Err("cancelled".into())).is_err());
        assert!(create(&path).is_err());
        // A second read repeats from byte0 on the same, still-held handle.
        assert_eq!(hash_written(&mut file,16,&||Ok(())).unwrap(),format!("{:x}",Sha256::digest(b"retained samples")));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Exact retained real-network tail; read-only33 originals, new isolated overlap/full-merge derivatives only"]
    fn actual_retained_overlap_and_merged_derivative_decode_without_source_changes() -> Result<(),String> {
        const ID:&str="b3debf7a-89bd-458a-9aec-d8c56d58e740";
        if std::env::var("ATSUMI_SOOP_OVERLAP_QA").as_deref()!=Ok(ID) {return Err("QA_OPT_IN".into());}
        let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
        let audit=workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
        let cancel=Arc::new(AtomicBool::new(false));
        let evidence_path=audit.join("soop-retained-actual-overlap-analysis.json");
        let (_evidence,bytes)=read_held(&evidence_path,64*1024)?;
        if digest(&bytes)!="1b0f08ca7c364dcfe0df213b07766d2051905f64fe57914f2ecd05b14af6243c" {return Err("QA_EVIDENCE_CHANGED".into());}
        let evidence:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|"QA_EVIDENCE_SCHEMA")?;
        let root=workspace.join(".runtime/soop-original-qa/2b23c8f07d8c4965a896157526afffa6");
        storage::safe_path(&root)?;
        let canonical=fs::canonicalize(&root).map_err(|_|"QA_SOURCE_PATH")?;
        let _directories=crate::streaming::browser_store::cleanup::pin_directories(&canonical).map_err(|_|"QA_SOURCE_PIN")?;
        let entries=evidence["originals"]["files"].as_array().filter(|x|x.len()==33).ok_or("QA_SOURCE_COUNT")?;
        let mut originals=Vec::new();
        for entry in entries {
            let relative=Path::new(entry["file"].as_str().ok_or("QA_SOURCE_PATH")?);
            if relative.is_absolute() || relative.components().any(|p|!matches!(p,std::path::Component::Normal(_))) {return Err("QA_SOURCE_PATH".into());}
            let path=root.join(relative);let held=pin(&path)?;let meta=held.metadata().map_err(|_|"QA_SOURCE_STAT")?;
            let expected=entry["sha256"].as_str().filter(|s|s.len()==64).ok_or("QA_SOURCE_DIGEST")?;
            if Some(meta.len())!=entry["bytes"].as_u64() || storage::hash(&path,&cancel)?!=expected {return Err("QA_SOURCE_CHANGED".into());}
            originals.push((path,held,meta.len(),meta.modified().map_err(|_|"QA_SOURCE_STAT")?,expected.to_string()));
        }
        let source=root.join(format!("media/SOOP/{ID}"));
        let (_manifest,bytes)=read_held(&source.join(MANIFEST),8192)?;
        let record:RetainedRecord=serde_json::from_slice(&bytes).map_err(|_|"QA_TAIL_SCHEMA")?;
        if digest(&bytes)!="f9cd1ed4c4f465b8749233dce53259bd371006c92091432c19f3f1641b030f23"
            || record.binding.job_id!=ID || record.binding.owner_sha256!=storage::hash(&source.join(PROOF),&cancel)?
            || record.binding.manifest_sha256.as_ref()!=Some(&storage::hash(&source.join(super::super::super::MANIFEST),&cancel)?)
        {return Err("QA_TAIL_BINDING".into());}
        let (_bundle,bytes)=read_held(&source.join(BUNDLE),MAX_BUNDLE as u64)?;
        if digest(&bytes)!=record.bundle.sha256 || bytes.len() as u64!=record.bundle.bytes {return Err("QA_BUNDLE_HASH".into());}
        let snapshot=read_bundle(&bytes)?;
        if snapshot.video.samples.len()!=600 || snapshot.audio.samples.len()!=466 {return Err("QA_EXPECTED_SAMPLES".into());}
        // Validate the actual retained init and every complete encoded sample
        // through the normal parser; compare clocks modulo the documented mux
        // origin, payload, flags and durations without printing any media bytes.
        let partial=snapshot.partial_mp4().map_err(|_|"QA_PARTIAL_MUX")?;
        let input=||vec![fmp4::EncodedTrackInput {track_index:0,
            mime_type:"video/mp4; codecs=\"avc1.640028,mp4a.40.2\"".into(),init:snapshot.init.clone()}];
        let mut parsed=fmp4::EncodedMuxer::new(input()).map_err(|_|"QA_CODEC_INIT")?;
        if !parsed.push(0,&partial.bytes[snapshot.init.len()..]).map_err(|_|"QA_CODEC_SAMPLES")?.is_empty() {return Err("QA_EXPECTED_PENDING".into());}
        let validated=parsed.pending_snapshot().map_err(|_|"QA_CODEC_SNAPSHOT")?;
        let video_base=snapshot.video.samples[0].dts;
        let audio_base=u64::try_from(u128::from(video_base)*u128::from(snapshot.audio.timescale)/u128::from(snapshot.video.timescale)).map_err(|_|"QA_CLOCK")?;
        for (saved,copy,base) in [(&snapshot.video,&validated.video,video_base),(&snapshot.audio,&validated.audio,audio_base)] {
            if saved.timescale!=copy.timescale || saved.samples.len()!=copy.samples.len()
                || !saved.samples.iter().zip(&copy.samples).all(|(a,b)|a.dts.checked_sub(base)==Some(b.dts)
                    && a.duration==b.duration && a.flags==b.flags && a.cts==b.cts && a.data==b.data)
            {return Err("QA_CODEC_ROUNDTRIP".into());}
        }
        if parsed.finish().is_ok() {return Err("QA_STRICT_FINISH_UNEXPECTED".into());}
        let candidate=snapshot.complete_overlap().map_err(|_|"QA_OVERLAP")?.ok_or("QA_NO_OVERLAP")?;
        if candidate.video_samples!=597 || candidate.audio_samples!=466
            || (candidate.segment.duration_seconds-9.957333333333338).abs()>1e-9 {return Err("QA_EXPECTED_OVERLAP".into());}
        let mut strict=fmp4::EncodedMuxer::new(input()).map_err(|_|"QA_CODEC_INIT")?;
        if !strict.push(0,&candidate.segment.bytes[snapshot.init.len()..]).map_err(|_|"QA_PREFIX_PARSE")?.is_empty()
            || strict.finish().map_err(|_|"QA_PREFIX_STRICT_FINISH")?.len()!=1 {return Err("QA_PREFIX_SEGMENT".into());}
        let output=audit.join(format!("soop-tail-overlap-fixture-{}",uuid::Uuid::new_v4().simple()));
        fs::create_dir(&output).map_err(|_|"QA_OUTPUT_CREATE")?;
        let candidate_name="complete-overlap.mp4";
        let mut file=create(&output.join(candidate_name))?;
        file.write_all(&candidate.segment.bytes).and_then(|_|file.sync_all()).map_err(|_|"QA_OUTPUT_SYNC")?;drop(file);
        let tools=workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        let decode=|name:&str|->Result<(),String> {
            let result=process::run(&tools.join("ffmpeg.exe"),&["-hide_banner","-nostdin","-v","error","-xerror","-threads","2",
                "-protocol_whitelist","file,pipe","-i",name,"-map","0:v:0","-map","0:a:0","-f","null","-"].map(String::from),
                &output,&cancel,Duration::from_secs(60),1024)?;
            if result.success {Ok(())}else{Err("QA_FULL_DECODE_FAILED".into())}
        };
        decode(candidate_name)?;
        let mut concat=String::new();
        for at in 0..9 {
            let name=source.join(format!("e{at:08}.mp4")).to_string_lossy().replace('\\',"/");
            if name.contains('\'') {return Err("QA_CONCAT_PATH".into());}
            concat.push_str(&format!("file '{name}'\n"));
        }
        concat.push_str("file 'complete-overlap.mp4'\n");
        let mut file=create(&output.join("concat.txt"))?;file.write_all(concat.as_bytes()).and_then(|_|file.sync_all()).map_err(|_|"QA_OUTPUT_SYNC")?;drop(file);
        let merged=process::run(&tools.join("ffmpeg.exe"),&["-hide_banner","-nostdin","-v","error","-n","-protocol_whitelist","file,pipe",
            "-f","concat","-safe","0","-i","concat.txt","-map","0:v:0","-map","0:a:0","-c","copy","candidate-main.mp4"].map(String::from),
            &output,&cancel,Duration::from_secs(60),1024)?;
        if !merged.success {return Err("QA_MERGE_FAILED".into());}decode("candidate-main.mp4")?;
        let measured=process::run(&tools.join("ffprobe.exe"),&["-v","error","-show_entries","format=duration","-of","json","candidate-main.mp4"].map(String::from),
            &output,&cancel,Duration::from_secs(15),2048)?;
        let metadata:serde_json::Value=serde_json::from_slice(&measured.stdout).map_err(|_|"QA_DURATION")?;
        let duration=metadata["format"]["duration"].as_str().and_then(|n|n.parse::<f64>().ok()).filter(|n|n.is_finite()).ok_or("QA_DURATION")?;
        if !measured.success || (duration-(108.208667+candidate.segment.duration_seconds)).abs()>0.1 {return Err("QA_MERGED_DURATION".into());}
        for (path,held,length,modified,expected) in &originals {
            let meta=held.metadata().map_err(|_|"QA_SOURCE_STAT")?;
            if meta.len()!=*length || meta.modified().map_err(|_|"QA_SOURCE_STAT")?!=*modified || storage::hash(path,&cancel)?!=*expected {
                return Err("QA_ORIGINAL_CHANGED".into());
            }
        }
        let report=serde_json::json!({"scope":"Actual retained sample derivative only; original archive not retrofitted or published",
            "sourceJob":ID,"originalFiles":33,"originalsUnchanged":true,"sourceBundleSha256":record.bundle.sha256,
            "selectedVideoSamples":597,"selectedAudioSamples":466,"excludedVideoSamples":3,"excludedVideoBytes":4634,
            "completeOriginalTailRetained":true,"strictOriginalFinishStillRejected":true,"strictPrefixFinishPassed":true,
            "prefixFullDecodePassed":true,"mergedFullDecodePassed":true,"mergedDurationSeconds":duration,
            "prefixSha256":storage::hash(&output.join(candidate_name),&cancel)?,"mergedSha256":storage::hash(&output.join("candidate-main.mp4"),&cancel)?,
            "freshProductionCaptureVerified":false,"publicationAuthorized":false});
        let mut file=create(&output.join("result.json"))?;
        file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_|"QA_REPORT")?).and_then(|_|file.sync_all()).map_err(|_|"QA_REPORT")?;
        println!("QA_SOOP_OVERLAP_RESULT={}",output.join("result.json").display());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Read-only exact actual codec fixture; opt-in, local derivative and bounded full decode only"]
    fn actual_avc_aac_partial_derivative_decodes_without_original_changes() -> Result<(), String> {
        use std::io::Read;
        use std::os::windows::fs::MetadataExt;
        const ID:&str="8fd00700-81d4-401f-a083-0d13fcf286ea";
        const SHA:&str="861f08cf68605ff4e7393e652a149d137552aaef4093b981637dc6e43911d088";
        if std::env::var("ATSUMI_SOOP_TAIL_DECODE_QA").as_deref()!=Ok(ID) {return Err("QA_OPT_IN".into());}
        let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
        let source=workspace.join(format!(".runtime/soop-original-qa/baa5338478ee4c8cb1cdda57d278d918/media/SOOP/{ID}/e00000008.mp4"));
        for part in source.ancestors() {
            let meta=fs::symlink_metadata(part).map_err(|_|"QA_SOURCE_PATH")?;
            if meta.file_type().is_symlink()||meta.file_attributes()&0x400!=0 {return Err("QA_SOURCE_LINK".into());}
        }
        let source=fs::canonicalize(source).map_err(|_|"QA_SOURCE_PATH")?;
        let _directories=crate::streaming::browser_store::cleanup::pin_directories(source.parent().ok_or("QA_SOURCE_PARENT")?)
            .map_err(|_|"QA_SOURCE_PIN")?;
        let mut held=crate::streaming::browser_store::cleanup::verification_guard(&source).map_err(|_|"QA_SOURCE_PIN")?;
        if held.metadata().map_err(|_|"QA_SOURCE_STAT")?.len()!=1_789_078 {return Err("QA_SOURCE_SIZE".into());}
        let cancel=Arc::new(AtomicBool::new(false));
        if storage::hash(&source,&cancel)?!=SHA {return Err("QA_SOURCE_HASH".into());}
        let mut bytes=Vec::new();(&mut held).take(2*1024*1024).read_to_end(&mut bytes).map_err(|_|"QA_SOURCE_READ")?;
        let mut split=0usize;
        for kind in [b"ftyp",b"moov"] {
            let header=bytes.get(split..split+8).ok_or("QA_SOURCE_BOX")?;
            if &header[4..8]!=kind {return Err("QA_SOURCE_BOX".into());}
            let length=u32::from_be_bytes(header[..4].try_into().map_err(|_|"QA_SOURCE_BOX")?) as usize;
            if length<8 {return Err("QA_SOURCE_BOX".into());}
            split=split.checked_add(length).filter(|n|*n<=bytes.len()).ok_or("QA_SOURCE_BOX")?;
        }
        let mut muxer=fmp4::EncodedMuxer::new(vec![fmp4::EncodedTrackInput {track_index:0,
            mime_type:"video/mp4; codecs=\"avc1.640028,mp4a.40.2\"".into(),init:bytes[..split].to_vec()}])
            .map_err(|_|"QA_SOURCE_INIT")?;
        let emitted=muxer.push(0,&bytes[split..]).map_err(|_|"QA_SOURCE_SAMPLES")?;
        if !emitted.is_empty() {return Err("QA_EXPECTED_SINGLE_PENDING_TAIL".into());}
        let mut snapshot=muxer.pending_snapshot().map_err(|_|"QA_SOURCE_SNAPSHOT")?;
        let original_audio=snapshot.audio.samples.len();
        if original_audio<4 {return Err("QA_SOURCE_AUDIO".into());}
        // Test fixture mutation ONLY: simulate three AAC packets not received.
        // This does not reconstruct the already discarded real capture tail.
        snapshot.audio.samples.truncate(original_audio-3);
        let partial=snapshot.partial_mp4().map_err(|_|"QA_PARTIAL_EXPORT")?;
        let output=workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume")
            .join(format!("soop-tail-codec-fixture-{}",uuid::Uuid::new_v4().simple()));
        fs::create_dir(&output).map_err(|_|"QA_OUTPUT_CREATE")?;
        let mut bundle=create(&output.join(BUNDLE))?;write_bundle(&mut bundle,&snapshot,&||Ok(()))?;
        bundle.sync_all().map_err(|_|"QA_OUTPUT_SYNC")?;drop(bundle);
        let mut derivative=create(&output.join(PARTIAL))?;
        derivative.write_all(&partial.bytes).and_then(|_|derivative.sync_all()).map_err(|_|"QA_OUTPUT_SYNC")?;
        drop(derivative);
        let tools=workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        let decoded=crate::platform_media::process::run(&tools.join("ffmpeg.exe"),
            &["-hide_banner","-nostdin","-v","error","-xerror","-threads","2","-protocol_whitelist","file,pipe",
                "-i",PARTIAL,"-map","0:v:0","-map","0:a:0","-f","null","-"].map(String::from),
            &output,&cancel,Duration::from_secs(30),1024)?;
        if !decoded.success {return Err("QA_PARTIAL_DECODE_FAILED".into());}
        if storage::hash(&source,&cancel)?!=SHA {return Err("QA_SOURCE_CHANGED".into());}
        let report=serde_json::json!({"scope":"actual-codec fixture with3 AAC packets omitted in memory only",
            "actualNetworkTailRecovered":false,"normalPublication":false,"sourceSha256":SHA,"originalUnchanged":true,
            "videoSamples":snapshot.video.samples.len(),"audioSamples":snapshot.audio.samples.len(),"omittedFixtureAudioPackets":3,
            "bundleSha256":storage::hash(&output.join(BUNDLE),&cancel)?,"partialSha256":storage::hash(&output.join(PARTIAL),&cancel)?,
            "fullDecodePassed":true,"durationSeconds":partial.duration_seconds,"stillPartial":true});
        let mut file=create(&output.join("result.json"))?;
        file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_|"QA_REPORT")?).and_then(|_|file.sync_all()).map_err(|_|"QA_REPORT")?;
        println!("QA_SOOP_TAIL_CODEC_RESULT={}",output.join("result.json").display());
        Ok(())
    }
}
