//! Verified immutable ranges of a growing recording. The append journal is never
//! rewritten; a range job fences only its committed prefix, not the growing tail.
use super::*;
use sha2::{Digest, Sha256};

const TARGET_SECONDS: f64 = 180.0;
const TARGET_BYTES: u64 = 48 * 1024 * 1024;
const MAX_PARTS: usize = 4096;
const INDEX_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveSummary {
    pub part_count: u64,
    pub segment_count: u64,
    pub duration_seconds: f64,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct PartPlan {
    pub first: u64,
    pub end: u64,
    prefix_len: u64,
    pub prefix_sha256: String,
    pub offset: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Part {
    pub first: u64,
    pub end: u64,
    pub file: String,
    pub timeline_file: String,
    pub bytes: u64,
    pub duration: f64,
    pub offset: f64,
    pub source_start: Option<f64>,
    pub source_end: Option<f64>,
    pub proof_file: String,
    pub proof_sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    recording_id: String,
    parts: Vec<Part>,
}
fn part_name(name: &str, suffix: &str) -> bool {
    name.strip_prefix("merged-")
        .and_then(|s| s.strip_suffix(suffix))
        .is_some_and(valid_id)
}
fn summary(parts: &[Part]) -> ProgressiveSummary {
    ProgressiveSummary {
        part_count: parts.len() as u64,
        segment_count: parts.last().map_or(0, |p| p.end),
        duration_seconds: parts.last().map_or(0.0, |p| p.offset + p.duration),
        last_error: None,
    }
}
pub(crate) fn load(record: &BrowserRecording) -> Result<Vec<Part>, StreamError> {
    let root = owned_root(&record.output_dir, &record.id)?;
    let path = root.join("progressive.json");
    if !path.try_exists().map_err(|_| storage())? {
        return if record
            .progressive
            .as_ref()
            .is_some_and(|p| p.part_count > 0)
        {
            Err(invalid())
        } else {
            Ok(vec![])
        };
    }
    let index: Index = serde_json::from_reader(BufReader::new(open_regular(&path, INDEX_LIMIT)?))
        .map_err(|_| invalid())?;
    if index.version != 1 || index.recording_id != record.id || index.parts.len() > MAX_PARTS {
        return Err(invalid());
    }
    let mut end = 0;
    let mut offset = 0.0;
    for part in &index.parts {
        if part.first != end
            || part.end <= end
            || part.end > record.segment_count
            || !part.offset.is_finite()
            || (part.offset - offset).abs() > 0.001
            || !part.duration.is_finite()
            || part.duration <= 0.0
            || part.duration > TARGET_SECONDS + MAX_DURATION + 1.0
            || part.bytes == 0
            || part.bytes > 256 * 1024 * 1024
            || !part_name(
                &part.file,
                if record.mime_type.starts_with("video/webm") {
                    ".webm"
                } else {
                    ".mp4"
                },
            )
            || !part_name(&part.timeline_file, ".timeline.jsonl")
            || part.file.get(7..39) != part.timeline_file.get(7..39)
            || !part_name(&part.proof_file, ".cleanup.json")
            || part.file.get(7..39) != part.proof_file.get(7..39)
            || !cleanup::valid_hash(&part.proof_sha256)
            || !valid_source_range(part.source_start, part.source_end, part.duration)
        {
            return Err(invalid());
        }
        end = part.end;
        offset += part.duration;
    }
    Ok(index.parts)
}

fn prefix(
    record: &BrowserRecording,
    first: u64,
    stop: Option<u64>,
) -> Result<(Vec<BrowserSegment>, u64, String), StreamError> {
    let root = owned_root(&record.output_dir, &record.id)?;
    let mut reader = BufReader::new(open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?);
    let mut hash = Sha256::new();
    let (mut length, mut index, mut bytes, mut seconds) = (0, 0, 0, 0.0);
    let mut selected = Vec::new();
    while index < record.segment_count {
        let Some(line) = read_line(&mut reader)? else {
            return Err(invalid());
        };
        hash.update(&line);
        length += line.len() as u64;
        let segment: BrowserSegment = serde_json::from_slice(&line).map_err(|_| invalid())?;
        if segment.index != index
            || segment.file != segment_name(index, extension(&record.mime_type))
            || segment.bytes == 0
            || segment.bytes > MAX_SEGMENT
            || !valid_duration(segment.duration_seconds)
            || !valid_source_range(
                segment.source_start_seconds,
                segment.source_end_seconds,
                segment.duration_seconds,
            )
        {
            return Err(invalid());
        }
        index += 1;
        if segment.index >= first {
            bytes += segment.bytes;
            seconds += segment.duration_seconds;
            selected.push(segment);
        }
        if stop.is_some_and(|end| index == end)
            || stop.is_none()
                && !selected.is_empty()
                && (seconds >= TARGET_SECONDS || bytes >= TARGET_BYTES)
        {
            break;
        }
    }
    Ok((selected, length, format!("{:x}", hash.finalize())))
}
pub(crate) fn validate_prefix(
    record: &BrowserRecording,
    plan: &PartPlan,
) -> Result<PathBuf, StreamError> {
    let root = owned_root(&record.output_dir, &record.id)?;
    if plan.first >= plan.end || plan.end > record.segment_count || plan.prefix_len > MAX_JOURNAL {
        return Err(invalid());
    }
    let mut file = open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?;
    if file.metadata().map_err(|_| storage())?.len() < plan.prefix_len {
        return Err(invalid());
    }
    let mut hash = Sha256::new();
    let mut left = plan.prefix_len;
    let mut buffer = [0u8; 32768];
    while left > 0 {
        let count = left.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..count])
            .map_err(|_| storage())?;
        hash.update(&buffer[..count]);
        left -= count as u64;
    }
    if format!("{:x}", hash.finalize()) != plan.prefix_sha256 {
        return Err(invalid());
    }
    Ok(root)
}
pub(crate) fn read_part_segments(
    record: &BrowserRecording,
    plan: &PartPlan,
) -> Result<Vec<BrowserSegment>, StreamError> {
    validate_prefix(record, plan)?;
    let (segments, length, digest) = prefix(record, plan.first, Some(plan.end))?;
    if length != plan.prefix_len
        || digest != plan.prefix_sha256
        || segments.len() as u64 != plan.end - plan.first
    {
        return Err(invalid());
    }
    Ok(segments)
}

impl BrowserCaptureStore {
    pub(crate) fn begin_progressive(
        &self,
        root: &Path,
        channel: &str,
        title: &str,
        mime: &str,
    ) -> Result<BrowserRecording, StreamError> {
        self.begin_with_progressive(root, channel, title, mime, true)
    }
    pub(crate) fn take_part_job(&self) -> Result<Option<BrowserMergeJob>, StreamError> {
        self.take_part_job_at(TARGET_SECONDS)
    }
    #[cfg(test)]
    pub(crate) fn take_short_part_job(&self) -> Result<Option<BrowserMergeJob>, StreamError> {
        self.take_part_job_at(1.0)
    }
    fn take_part_job_at(
        &self,
        target_seconds: f64,
    ) -> Result<Option<BrowserMergeJob>, StreamError> {
        let mut state = self.lock()?;
        if state.closing || state.merge_job.is_some() {
            return Ok(None);
        }
        for index in 0..state.recordings.len() {
            let r = &state.recordings[index];
            if r.deletion_pending
                || r.media_removed_at.is_some()
                || state.unverified.contains(&r.id)
                || r.progressive
                    .as_ref()
                    .is_none_or(|p| p.segment_count >= r.segment_count || p.last_error.is_some())
                || r.merge
                    .as_ref()
                    .is_some_and(|m| m.status == BrowserMergeStatus::Complete)
            {
                continue;
            }
            let prepared = (|| {
                let parts = load(r)?;
                if parts.len() >= MAX_PARTS {
                    return Err(invalid());
                }
                let saved = summary(&parts);
                let (segments, prefix_len, digest) = prefix(r, saved.segment_count, None)?;
                let duration: f64 = segments.iter().map(|s| s.duration_seconds).sum();
                let bytes: u64 = segments.iter().map(|s| s.bytes).sum();
                if segments.is_empty()
                    || r.status == BrowserRecordingStatus::Recording
                        && duration < target_seconds
                        && bytes < TARGET_BYTES
                {
                    return Ok(None);
                }
                let root = owned_root(&r.output_dir, &r.id)?;
                let modified = fs::metadata(root.join("segments.jsonl"))
                    .map_err(|_| storage())?
                    .modified()
                    .map_err(|_| storage())?;
                Ok(Some(BrowserMergeJob {
                    recording: r.clone(),
                    token: uuid::Uuid::new_v4().simple().to_string(),
                    journal_len: prefix_len,
                    journal_modified: modified,
                    part: Some(PartPlan {
                        first: saved.segment_count,
                        end: segments.last().unwrap().index + 1,
                        prefix_len,
                        prefix_sha256: digest,
                        offset: saved.duration_seconds,
                    }),
                }))
            })();
            match prepared {
                Ok(Some(job)) => {
                    state.merge_job = Some((job.recording.id.clone(), job.token.clone()));
                    return Ok(Some(job));
                }
                Ok(None) => {}
                Err(_) => {
                    state.recordings[index]
                        .progressive
                        .as_mut()
                        .unwrap()
                        .last_error =
                        Some("구간 병합 목록을 확인하지 못했습니다. 원본은 보존됩니다.".into());
                    let _ = save_metadata(&state.recordings[index]);
                }
            }
        }
        Ok(None)
    }
    pub(crate) fn finish_part_job(
        &self,
        job: &BrowserMergeJob,
        result: Result<BrowserMergedOutput, StreamError>,
    ) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        if state.merge_job.as_ref() != Some(&(job.recording.id.clone(), job.token.clone())) {
            return Err(invalid());
        }
        let index = state
            .recordings
            .iter()
            .position(|r| r.id == job.recording.id && !r.deletion_pending)
            .ok_or_else(invalid)?;
        let r = &mut state.recordings[index];
        let result = result.and_then(|output| {
            let plan = job.part.as_ref().ok_or_else(invalid)?;
            let root = validate_prefix(&job.recording, plan)?;
            let mut parts = load(r)?;
            if parts.last().map_or(0, |p| p.end) != plan.first {
                return Err(invalid());
            }
            let segments = read_part_segments(&job.recording, plan)?;
            let proof = output.cleanup.ok_or_else(invalid)?;
            if output.file != format!("merged-{}.{}", job.token, extension(&r.mime_type))
                || output.timeline_file != format!("merged-{}.timeline.jsonl", job.token)
                || proof.file != format!("merged-{}.cleanup.json", job.token)
                || !cleanup::valid_hash(&proof.sha256)
                || !output.duration_seconds.is_finite()
                || output.duration_seconds <= 0.0
                || output.duration_seconds > TARGET_SECONDS + MAX_DURATION + 1.0
                || output.bytes == 0
                || open_regular(&root.join(&output.file), 256 * 1024 * 1024)?
                    .metadata()
                    .map_err(|_| storage())?
                    .len()
                    != output.bytes
            {
                return Err(invalid());
            }
            open_regular(&root.join(&output.timeline_file), MAX_JOURNAL)?;
            open_regular(&root.join(&proof.file), MAX_JOURNAL)?;
            parts.push(Part {
                first: plan.first,
                end: plan.end,
                file: output.file,
                timeline_file: output.timeline_file,
                bytes: output.bytes,
                duration: output.duration_seconds,
                offset: plan.offset,
                source_start: segments.first().and_then(|s| s.source_start_seconds),
                source_end: segments.last().and_then(|s| s.source_end_seconds),
                proof_file: proof.file,
                proof_sha256: proof.sha256,
            });
            let next = summary(&parts);
            let data = serde_json::to_vec(&Index {
                version: 1,
                recording_id: r.id.clone(),
                parts,
            })
            .map_err(|_| storage())?;
            if data.len() as u64 > INDEX_LIMIT {
                return Err(storage());
            }
            atomic_write(&root, "progressive.json", &data)?;
            r.progressive = Some(next);
            save_metadata(r)
        });
        if let Err(cause) = &result {
            if cause.code != "BROWSER_MERGE_CANCELLED" {
                r.progressive.as_mut().unwrap().last_error =
                    Some(bounded_text(&cause.message, 512));
            }
            let _ = save_metadata(r);
        }
        state.merge_job = None;
        result
    }
}

pub(crate) fn recover_summary(record: &mut BrowserRecording) {
    if record.progressive.is_none() {
        return;
    }
    match load(record) {
        Ok(parts) => record.progressive = Some(summary(&parts)),
        Err(_) => {
            record.progressive.as_mut().unwrap().last_error =
                Some("구간 병합 기록을 확인하지 못했습니다. 원본은 보존됩니다.".into())
        }
    }
}

/// Final export consumes each verified range once, rather than re-probing all
/// tiny source segments. The original journal remains the cleanup authority.
pub(crate) fn export_segments(
    record: &BrowserRecording,
) -> Result<Option<Vec<BrowserSegment>>, StreamError> {
    if record.progressive.is_none() {
        return Ok(None);
    }
    let parts = load(record)?;
    if parts.is_empty() || summary(&parts).segment_count != record.segment_count {
        return Err(invalid());
    }
    Ok(Some(
        parts
            .iter()
            .enumerate()
            .map(|(i, p)| BrowserSegment {
                index: i as u64,
                file: p.file.clone(),
                bytes: p.bytes,
                duration_seconds: p.duration,
                source_start_seconds: p.source_start,
                source_end_seconds: p.source_end,
            })
            .collect(),
    ))
}

/// Preserve each original segment's clock even when a final export consumes
/// multi-minute files. The existing replay index intentionally rejects huge rows.
pub(crate) fn timeline_rows(
    root: &Path,
    part: &Part,
    offset: f64,
    duration: f64,
) -> Result<Vec<u8>, StreamError> {
    let mut reader = BufReader::new(open_regular(&root.join(&part.timeline_file), MAX_JOURNAL)?);
    let mut bytes = Vec::new();
    let mut rows = 0;
    let mut previous_end = 0.0;
    while let Some(line) = read_line(&mut reader)? {
        let mut row: serde_json::Value = serde_json::from_slice(&line).map_err(|_| invalid())?;
        let start = row["mergedStartSeconds"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or_else(invalid)?;
        let mut length = row["mergedDurationSeconds"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_DURATION + 2.0)
            .ok_or_else(invalid)?;
        if rows >= part.end - part.first
            || (start - previous_end).abs() > 0.1
            || row["segmentIndex"].as_u64() != Some(part.first + rows)
            || row["chatRewritten"].as_bool() != Some(false)
        {
            return Err(invalid());
        }
        if rows + 1 == part.end - part.first {
            if (start + length - duration).abs() > 0.5 || duration <= start {
                return Err(invalid());
            }
            length = duration - start;
        }
        previous_end = start + length;
        row["mergedStartSeconds"] = serde_json::json!(start + offset);
        row["mergedDurationSeconds"] = serde_json::json!(length);
        bytes.extend(json_line(&row)?);
        rows += 1;
        if bytes.len() as u64 > MAX_JOURNAL {
            return Err(invalid());
        }
    }
    if rows != part.end - part.first {
        return Err(invalid());
    }
    Ok(bytes)
}

pub(crate) fn replay_source(record: &BrowserRecording) -> Result<BrowserReplaySource, StreamError> {
    let parts = load(record)?;
    if parts.is_empty() {
        return Err(invalid());
    }
    let root = owned_root(&record.output_dir, &record.id)?;
    let mut media = Vec::new();
    let mut combined = Vec::new();
    for part in &parts {
        let file = open_regular(&root.join(&part.file), 256 * 1024 * 1024)?;
        if file.metadata().map_err(|_| storage())?.len() != part.bytes {
            return Err(storage());
        }
        media.push((file, part.clone()));
        combined.extend(timeline_rows(&root, part, part.offset, part.duration)?);
        if combined.len() as u64 > MAX_JOURNAL {
            return Err(invalid());
        }
    }
    let first = &parts[0];
    let chat_path = root.join("chat.jsonl");
    let chat = if chat_path.try_exists().map_err(|_| storage())? {
        Some(open_regular(&chat_path, 512 * 1024 * 1024)?)
    } else {
        None
    };
    Ok(BrowserReplaySource {
        recording: record.clone(),
        media: open_regular(&root.join(&first.file), 256 * 1024 * 1024)?,
        timeline: open_regular(&root.join(&first.timeline_file), MAX_JOURNAL)?,
        duration: summary(&parts).duration_seconds,
        chat,
        parts: media,
        combined_timeline: Some(combined),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const WEBM: &[u8] = &[
        0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 1, 0, 0, 0, 0, 0, 0, 0,
    ];
    fn append(store: &BrowserCaptureStore, id: &str, first: u64, count: u64) {
        for index in first..first + count {
            store.append(id, index, 0, WEBM).unwrap();
            store.finish_segment(id, index, 60.0).unwrap();
        }
    }
    pub(crate) fn publish_fixture(store: &BrowserCaptureStore, job: &BrowserMergeJob) {
        let plan = job.part.as_ref().unwrap();
        let root = Path::new(&job.recording.output_dir);
        let file = format!("merged-{}.webm", job.token);
        let timeline_file = format!("merged-{}.timeline.jsonl", job.token);
        fs::write(root.join(&file), WEBM).unwrap();
        let timeline = (plan.first..plan.end).map(|index| format!("{{\"segmentIndex\":{index},\"mergedStartSeconds\":{},\"mergedDurationSeconds\":60,\"mediaDurationSeconds\":60,\"recordedDurationSeconds\":60,\"sourceStartSeconds\":null,\"sourceEndSeconds\":null,\"clock\":\"media_duration\",\"chatClock\":\"original_recording_receive_time\",\"chatRewritten\":false}}\n", (index - plan.first) * 60)).collect::<String>();
        fs::write(root.join(&timeline_file), timeline).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let hashes: Vec<_> = read_merge_segments(job)
            .unwrap()
            .iter()
            .map(|s| cleanup::source_hash(&root.join(&s.file), s.bytes, &cancel).unwrap())
            .collect();
        let proof = cleanup::write_proof(
            job,
            &hashes,
            &root.join(&file),
            &root.join(&timeline_file),
            &cancel,
        )
        .unwrap();
        store
            .finish_part_job(
                job,
                Ok(BrowserMergedOutput {
                    file,
                    timeline_file,
                    bytes: WEBM.len() as u64,
                    duration_seconds: (plan.end - plan.first) as f64 * 60.0,
                    cleanup: Some(proof),
                }),
            )
            .unwrap();
    }
    #[test]
    fn live_range_accepts_append_only_tail_and_never_remerges_a_committed_range() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 2);
        assert!(store.take_part_job().unwrap().is_none());
        append(&store, &r.id, 2, 1);
        let job = store.take_part_job().unwrap().unwrap();
        append(&store, &r.id, 3, 1);
        assert!(validate_merge_generation(&job).is_ok());
        publish_fixture(&store, &job);
        let r = store.snapshot().unwrap().remove(0);
        assert_eq!(r.status, BrowserRecordingStatus::Recording);
        assert_eq!(r.segment_count, 4);
        assert_eq!(r.progressive.unwrap().segment_count, 3);
        assert!(store.has_active());
        assert!(store.take_part_job().unwrap().is_none());
        store.finish(&r.id, false, None).unwrap();
        let tail = store.take_part_job().unwrap().unwrap();
        assert_eq!(tail.part.as_ref().unwrap().first, 3);
        publish_fixture(&store, &tail);
        assert!(store.take_part_job().unwrap().is_none());
        let final_job = store.take_merge_job().unwrap().unwrap();
        assert!(final_job.part.is_none());
        assert_eq!(
            export_segments(&final_job.recording)
                .unwrap()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(read_merge_segments(&final_job).unwrap().len(), 4);
    }
    #[test]
    fn range_prefix_mutation_rejects_publication_without_stopping_capture() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 3);
        let job = store.take_part_job().unwrap().unwrap();
        let path = Path::new(&r.output_dir).join("segments.jsonl");
        let mut journal = fs::read(&path).unwrap();
        journal[2] ^= 1;
        fs::write(path, journal).unwrap();
        assert!(validate_merge_generation(&job).is_err());
        assert!(store.has_active());
        assert!(Path::new(&r.output_dir)
            .join("segment-000000000000.webm")
            .exists());
    }
    #[test]
    fn restart_recovers_manifest_ahead_of_metadata_without_duplicate_work() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 3);
        let job = store.take_part_job().unwrap().unwrap();
        publish_fixture(&store, &job);
        let mut snapshot = store.snapshot().unwrap().remove(0);
        snapshot.progressive = Some(ProgressiveSummary::default());
        save_metadata(&snapshot).unwrap();
        drop(store);
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        assert_eq!(
            reopened.snapshot().unwrap()[0]
                .progressive
                .as_ref()
                .unwrap()
                .part_count,
            1
        );
        assert!(reopened.take_part_job().unwrap().is_none());
    }
    #[test]
    fn cancellation_and_retry_leave_originals_and_do_not_claim_a_final_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 3);
        let job = store.take_part_job().unwrap().unwrap();
        assert!(store
            .finish_part_job(&job, Err(error("BROWSER_MERGE_CANCELLED", "cancel", true)))
            .is_err());
        let next = store.take_part_job().unwrap().unwrap();
        assert_ne!(job.token, next.token);
        assert!(store.snapshot().unwrap()[0].merge.is_none());
        assert!(store.finish_part_job(&next, Err(storage())).is_err());
        assert!(store.take_part_job().unwrap().is_none());
        store.retry_merges(Some(&r.id)).unwrap();
        assert!(store.take_part_job().unwrap().is_some());
    }
    #[test]
    fn malformed_manifest_cannot_escape_the_recording_or_open_unverified_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 3);
        let job = store.take_part_job().unwrap().unwrap();
        publish_fixture(&store, &job);
        let path = Path::new(&r.output_dir).join("progressive.json");
        let mut index: Index = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        index.parts[0].file = "../outside.webm".into();
        fs::write(path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(store.replay_source(&r.id).is_err());
        assert!(store.has_active());
    }
    #[test]
    fn legacy_recordings_keep_the_original_single_file_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin(dir.path(), CHANNEL, "legacy", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 1);
        store.finish(&r.id, false, None).unwrap();
        assert!(store.take_part_job().unwrap().is_none());
        assert!(store.take_merge_job().unwrap().is_some());
    }
    #[test]
    fn missing_committed_index_blocks_instead_of_rebuilding_or_forgetting_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 3);
        let job = store.take_part_job().unwrap().unwrap();
        publish_fixture(&store, &job);
        fs::remove_file(Path::new(&r.output_dir).join("progressive.json")).unwrap();
        assert!(store.replay_source(&r.id).is_err());
        drop(store);
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        let summary = reopened.snapshot().unwrap()[0].progressive.clone().unwrap();
        assert_eq!(summary.part_count, 1);
        assert!(summary.last_error.is_some());
    }
    #[test]
    fn modified_range_cannot_be_used_to_export_or_authorize_source_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin_progressive(dir.path(), CHANNEL, "range", "video/webm")
            .unwrap();
        append(&store, &r.id, 0, 3);
        let job = store.take_part_job().unwrap().unwrap();
        publish_fixture(&store, &job);
        store.finish(&r.id, false, None).unwrap();
        let final_job = store.take_merge_job().unwrap().unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(
            cleanup::verified_range_inputs(&final_job, &cancel)
                .unwrap()
                .0
                .len(),
            3
        );
        let range = Path::new(&r.output_dir).join(format!("merged-{}.webm", job.token));
        let mut data = WEBM.to_vec();
        data[15] = 1;
        fs::write(range, data).unwrap();
        assert!(cleanup::verified_range_inputs(&final_job, &cancel).is_err());
        assert!(Path::new(&r.output_dir)
            .join("segment-000000000000.webm")
            .exists());
    }
}
