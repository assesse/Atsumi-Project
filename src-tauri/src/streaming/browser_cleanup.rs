//! Two-phase, recording-local cleanup. The completed merge and its proof are
//! durable before the first unlink. A crash leaves Pending, never a lost merge.
//! No directory enumeration, recursive removal, or remote-provided path is used.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PROOF: u64 = 64 * 1024 * 1024;
const CLEANUP_ERROR: &str = "병합 영상은 저장되었습니다. 사용 중이거나 변경된 원본 조각은 보존했습니다. 원본 정리를 다시 시도할 수 있습니다.";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSourceCleanupStatus {
    Pending,
    Complete,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSourceCleanup {
    pub status: BrowserSourceCleanupStatus,
    pub deleted_segments: u64,
    pub proof_file: String,
    pub proof_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

pub(crate) struct CleanupProofReference {
    pub file: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Proof {
    version: u32,
    recording_id: String,
    media_sha256: String,
    timeline_sha256: String,
    journal_sha256: String,
    segments: Vec<SourceProof>,
    #[serde(default)]
    derivatives: Vec<DerivativeProof>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DerivativeProof {
    file: String,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceProof {
    index: u64,
    bytes: u64,
    sha256: String,
}

pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn valid_summary(value: Option<&BrowserSourceCleanup>, token: &str, count: u64) -> bool {
    value.is_none_or(|c| {
        c.proof_file == format!("merged-{token}.cleanup.json")
            && valid_hash(&c.proof_sha256)
            && c.deleted_segments <= count
            && (c.status != BrowserSourceCleanupStatus::Complete || c.deleted_segments == count)
            && c.last_error
                .as_ref()
                .is_none_or(|s| s.chars().count() <= 512)
    })
}
pub(super) fn pending_proof(
    root: &Path,
    token: &str,
    proof: CleanupProofReference,
) -> Result<BrowserSourceCleanup, StreamError> {
    let result = BrowserSourceCleanup {
        status: BrowserSourceCleanupStatus::Pending,
        deleted_segments: 0,
        proof_file: proof.file,
        proof_sha256: proof.sha256,
        last_error: None,
    };
    if !valid_summary(Some(&result), token, 0) {
        return Err(invalid());
    }
    open_regular(&root.join(&result.proof_file), MAX_PROOF)?;
    Ok(result)
}

/// Read-only handles deny writes and replacement on Windows. They also keep
/// existing merged-file replay handles usable while originals are removed.
fn pinned_file(path: &Path, limit: u64) -> Result<File, StreamError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000); // READ, OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|_| storage())?;
    let metadata = file.metadata().map_err(|_| storage())?;
    if !safe_regular(&metadata)
        || metadata.len() > limit
        || fs::canonicalize(path).map_err(|_| storage())? != path
    {
        return Err(storage());
    }
    Ok(file)
}

pub(crate) fn verification_guard(path: &Path) -> Result<File, StreamError> {
    pinned_file(path, u64::MAX)
}

fn hash_file(file: &mut File, cancel: &AtomicBool) -> Result<String, StreamError> {
    file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(inactive());
        }
        let count = file.read(&mut buffer).map_err(|_| storage())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn source_hash(
    path: &Path,
    bytes: u64,
    cancel: &AtomicBool,
) -> Result<String, StreamError> {
    let mut file = pinned_file(path, MAX_SEGMENT)?;
    if file.metadata().map_err(|_| storage())?.len() != bytes {
        return Err(invalid());
    }
    hash_file(&mut file, cancel)
}

/// Bind a final export to the exact ranges and raw inputs verified earlier.
/// Changed ranges or changed originals never authorize source cleanup.
pub(crate) fn verified_range_inputs(
    job: &BrowserMergeJob,
    cancel: &AtomicBool,
) -> Result<(Vec<String>, Vec<DerivativeProof>), StreamError> {
    if job.part.is_some() || job.recording.progressive.is_none() {
        return Ok((vec![], vec![]));
    }
    let root = validate_merge_generation(job)?;
    let mut hashes = Vec::new();
    let mut derivatives = Vec::new();
    for part in parts::load(&job.recording)? {
        let mut file = pinned_file(&root.join(&part.proof_file), MAX_PROOF)?;
        if hash_file(&mut file, cancel)? != part.proof_sha256 {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
        let proof: Proof = serde_json::from_reader(file).map_err(|_| invalid())?;
        if proof.version != 1
            || proof.recording_id != job.recording.id
            || !proof.derivatives.is_empty()
            || proof.segments.len() as u64 != part.end - part.first
            || hashes.len() as u64 != part.first
        {
            return Err(invalid());
        }
        for (offset, source) in proof.segments.iter().enumerate() {
            if source.index != part.first + offset as u64 || !valid_hash(&source.sha256) {
                return Err(invalid());
            }
            hashes.push(source.sha256.clone());
        }
        for (name, expected) in [
            (&part.file, &proof.media_sha256),
            (&part.timeline_file, &proof.timeline_sha256),
        ] {
            let mut file = pinned_file(&root.join(name), 256 * 1024 * 1024)?;
            let bytes = file.metadata().map_err(|_| storage())?.len();
            if name == &part.file && bytes != part.bytes
                || hash_file(&mut file, cancel)? != *expected
            {
                return Err(invalid());
            }
            derivatives.push(DerivativeProof {
                file: name.clone(),
                bytes,
                sha256: expected.clone(),
            });
        }
    }
    if hashes.len() as u64 != job.recording.segment_count {
        return Err(invalid());
    }
    Ok((hashes, derivatives))
}

/// Called only after full A/V decoding and duration/codec verification succeed.
/// Re-read each source so a changed input cannot authorize later deletion.
pub(crate) fn write_proof(
    job: &BrowserMergeJob,
    hashes: &[String],
    media: &Path,
    timeline: &Path,
    cancel: &AtomicBool,
) -> Result<CleanupProofReference, StreamError> {
    let root = validate_merge_generation(job)?;
    let segments = read_merge_segments(job)?;
    if segments.len() != hashes.len() {
        return Err(invalid());
    }
    let mut sources = Vec::with_capacity(segments.len());
    for (segment, expected) in segments.iter().zip(hashes) {
        if source_hash(&root.join(&segment.file), segment.bytes, cancel)? != *expected {
            return Err(invalid());
        }
        sources.push(SourceProof {
            index: segment.index,
            bytes: segment.bytes,
            sha256: expected.clone(),
        });
    }
    let (_, derivatives) = verified_range_inputs(job, cancel)?;
    let journal_sha256 = if let Some(plan) = &job.part {
        validate_merge_generation(job)?;
        plan.prefix_sha256.clone()
    } else {
        hash_file(
            &mut pinned_file(&root.join("segments.jsonl"), MAX_JOURNAL)?,
            cancel,
        )?
    };
    let proof = Proof {
        version: 1,
        recording_id: job.recording.id.clone(),
        media_sha256: hash_file(&mut pinned_file(media, u64::MAX)?, cancel)?,
        timeline_sha256: hash_file(&mut pinned_file(timeline, 128 * 1024 * 1024)?, cancel)?,
        journal_sha256,
        segments: sources,
        derivatives,
    };
    let bytes = serde_json::to_vec(&proof).map_err(|_| storage())?;
    if bytes.len() as u64 > MAX_PROOF {
        return Err(invalid());
    }
    let name = format!("merged-{}.cleanup.json", job.token);
    let partial = root.join(format!("{name}.partial"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|_| storage())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| storage())?;
    drop(file);
    validate_merge_generation(job)?;
    rename_closed(&partial, &root.join(&name), false)?;
    Ok(CleanupProofReference {
        file: name,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}

pub(super) fn merged_files_exist(root: &Path, merge: &BrowserMerge) -> bool {
    merge.file.as_ref().is_some_and(|name| {
        open_regular(&root.join(name), u64::MAX)
            .and_then(|file| file.metadata().map_err(|_| storage()))
            .is_ok_and(|m| Some(m.len()) == merge.bytes)
    }) && merge
        .timeline_file
        .as_ref()
        .is_some_and(|name| open_regular(&root.join(name), 128 * 1024 * 1024).is_ok())
}

impl BrowserCaptureStore {
    pub(crate) fn take_cleanup_job(&self) -> Result<Option<BrowserMergeJob>, StreamError> {
        let mut state = self.lock()?;
        if state.closing || state.merge_job.is_some() {
            return Ok(None);
        }
        let Some(index) = state.recordings.iter().position(|r| {
            r.status != BrowserRecordingStatus::Recording
                && !r.deletion_pending
                && !state.active.contains_key(&r.id)
                && r.merge.as_ref().is_some_and(|m| {
                    m.status == BrowserMergeStatus::Complete
                        && m.source_cleanup
                            .as_ref()
                            .is_some_and(|c| c.status == BrowserSourceCleanupStatus::Pending)
                })
        }) else {
            return Ok(None);
        };
        let prepared = (|| -> Result<BrowserMergeJob, StreamError> {
            let recording = &state.recordings[index];
            let root = owned_root(&recording.output_dir, &recording.id)?;
            let merge = recording.merge.as_ref().unwrap();
            if !valid_merge(Some(merge), recording.segment_count, &recording.mime_type) {
                return Err(invalid());
            }
            let token = merge.file.as_ref().unwrap()[7..39].to_owned();
            let metadata = open_regular(&root.join("segments.jsonl"), MAX_JOURNAL)?
                .metadata()
                .map_err(|_| storage())?;
            Ok(BrowserMergeJob {
                recording: recording.clone(),
                token,
                journal_len: metadata.len(),
                journal_modified: metadata.modified().map_err(|_| storage())?,
                part: None,
            })
        })();
        let job = match prepared {
            Ok(job) => job,
            Err(_) => {
                let recording = &mut state.recordings[index];
                let cleanup = recording
                    .merge
                    .as_mut()
                    .unwrap()
                    .source_cleanup
                    .as_mut()
                    .unwrap();
                cleanup.status = BrowserSourceCleanupStatus::Blocked;
                cleanup.last_error = Some(CLEANUP_ERROR.into());
                let _ = save_metadata(recording);
                return Ok(None);
            }
        };
        state.merge_job = Some((job.recording.id.clone(), job.token.clone()));
        Ok(Some(job))
    }

    pub(crate) fn finish_cleanup(
        &self,
        job: &BrowserMergeJob,
        outcome: CleanupOutcome,
    ) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        let index = merge_job_index(&state, job)?;
        let recording = &mut state.recordings[index];
        let cleanup = recording
            .merge
            .as_mut()
            .filter(|m| m.status == BrowserMergeStatus::Complete)
            .and_then(|m| m.source_cleanup.as_mut())
            .ok_or_else(invalid)?;
        cleanup.deleted_segments = cleanup.deleted_segments.max(outcome.deleted);
        cleanup.status = if outcome.complete {
            BrowserSourceCleanupStatus::Complete
        } else {
            BrowserSourceCleanupStatus::Blocked
        };
        cleanup.last_error = (!outcome.complete).then(|| CLEANUP_ERROR.to_owned());
        // If this write fails, the durable Pending marker permits idempotent
        // recovery. The successful merge is never downgraded or overwritten.
        let result = save_metadata(recording);
        if result.is_err() {
            let cleanup = recording
                .merge
                .as_mut()
                .unwrap()
                .source_cleanup
                .as_mut()
                .unwrap();
            cleanup.status = BrowserSourceCleanupStatus::Blocked;
            cleanup.last_error = Some("병합본은 저장되었지만 원본 정리 상태를 저장하지 못했습니다. 남은 조각과 병합본을 보존하고 다음 실행에서 확인합니다.".into());
        }
        state.merge_job = None;
        result
    }
}

pub(crate) struct CleanupOutcome {
    pub deleted: u64,
    pub complete: bool,
}

pub(crate) fn remove_verified_sources(
    job: &BrowserMergeJob,
    cancel: &AtomicBool,
) -> CleanupOutcome {
    let mut deleted = 0;
    let result = (|| -> Result<(), StreamError> {
        let root = validate_merge_generation(job)?;
        let _directories = pin_directories(&root)?;
        let merge = job.recording.merge.as_ref().ok_or_else(invalid)?;
        if merge.status != BrowserMergeStatus::Complete
            || !valid_merge(
                Some(merge),
                job.recording.segment_count,
                &job.recording.mime_type,
            )
        {
            return Err(invalid());
        }
        let cleanup = merge.source_cleanup.as_ref().ok_or_else(invalid)?;
        let mut proof_file = pinned_file(&root.join(&cleanup.proof_file), MAX_PROOF)?;
        if hash_file(&mut proof_file, cancel)? != cleanup.proof_sha256 {
            return Err(invalid());
        }
        proof_file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
        let proof: Proof = serde_json::from_reader(&proof_file).map_err(|_| invalid())?;
        let segments = read_merge_segments(job)?;
        if proof.version != 1
            || proof.recording_id != job.recording.id
            || proof.segments.len() != segments.len()
        {
            return Err(invalid());
        }
        let mut media = pinned_file(&root.join(merge.file.as_ref().unwrap()), u64::MAX)?;
        let mut timeline = pinned_file(
            &root.join(merge.timeline_file.as_ref().unwrap()),
            128 * 1024 * 1024,
        )?;
        let mut journal = pinned_file(&root.join("segments.jsonl"), MAX_JOURNAL)?;
        if media.metadata().map_err(|_| storage())?.len() != merge.bytes.unwrap()
            || hash_file(&mut media, cancel)? != proof.media_sha256
            || hash_file(&mut timeline, cancel)? != proof.timeline_sha256
            || hash_file(&mut journal, cancel)? != proof.journal_sha256
        {
            return Err(invalid());
        }
        // Validate the complete manifest before touching even the first file.
        for (segment, source) in segments.iter().zip(&proof.segments) {
            if source.index != segment.index
                || source.bytes != segment.bytes
                || !valid_hash(&source.sha256)
            {
                return Err(invalid());
            }
        }
        let range_files: Vec<String> = if job.recording.progressive.is_some() {
            parts::load(&job.recording)?
                .into_iter()
                .flat_map(|p| [p.file, p.timeline_file])
                .collect()
        } else {
            vec![]
        };
        if range_files.len() != proof.derivatives.len()
            || range_files
                .iter()
                .zip(&proof.derivatives)
                .any(|(name, p)| name != &p.file || p.bytes == 0 || !valid_hash(&p.sha256))
        {
            return Err(invalid());
        }
        for (segment, source) in segments.iter().zip(&proof.segments) {
            if cancel.load(Ordering::Acquire) {
                return Err(inactive());
            }
            validate_merge_generation(job)?;
            let path = root.join(&segment.file); // exact journal-generated basename
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    deleted += 1;
                    continue;
                }
                Ok(metadata)
                    if safe_regular(&metadata)
                        && metadata.len() == source.bytes
                        && !metadata.permissions().readonly() => {}
                _ => return Err(storage()),
            }
            remove_one(&path, source, cancel)?;
            deleted += 1;
        }
        // Finished-prefix readers retain their already-open handles. Only the
        // exact proven derivatives are retired after the final file is durable.
        for derivative in &proof.derivatives {
            if cancel.load(Ordering::Acquire) {
                return Err(inactive());
            }
            let path = root.join(&derivative.file);
            if !path.try_exists().map_err(|_| storage())? {
                continue;
            }
            remove_one(
                &path,
                &SourceProof {
                    index: 0,
                    bytes: derivative.bytes,
                    sha256: derivative.sha256.clone(),
                },
                cancel,
            )?;
        }
        Ok(())
    })();
    #[cfg(test)]
    if let Err(error) = &result {
        eprintln!(
            "source cleanup blocked after {deleted} source files: {}",
            error.code
        );
    }
    CleanupOutcome {
        deleted,
        complete: result.is_ok(),
    }
}

#[cfg(windows)]
pub(super) fn pin_directories(root: &Path) -> Result<Vec<File>, StreamError> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut handles = Vec::new();
    // Pin every ancestor without DELETE sharing, preventing directory/junction
    // swaps between validation and handle-based deletion below.
    for path in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let file = OpenOptions::new()
            .access_mode(0x80)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
            .map_err(|_| storage())?;
        let metadata = file.metadata().map_err(|_| storage())?;
        if !metadata.is_dir()
            || is_link(&metadata)
            || fs::canonicalize(path).map_err(|_| storage())? != path
        {
            return Err(storage());
        }
        handles.push(file);
    }
    Ok(handles)
}

#[cfg(windows)]
fn remove_one(path: &Path, source: &SourceProof, cancel: &AtomicBool) -> Result<(), StreamError> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
            BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO,
        },
    };
    let mut file = OpenOptions::new()
        .access_mode(0x80000000 | 0x10000)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|_| storage())?;
    let metadata = file.metadata().map_err(|_| storage())?;
    let handle = HANDLE(file.as_raw_handle());
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut information) }.map_err(|_| storage())?;
    if !safe_regular(&metadata)
        || metadata.len() != source.bytes
        || metadata.permissions().readonly()
        || information.nNumberOfLinks != 1
        || fs::canonicalize(path).map_err(|_| storage())? != path
        || hash_file(&mut file, cancel)? != source.sha256
    {
        return Err(invalid());
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // Delete the verified open file, never a newly substituted path. Existing
    // readers lacking DELETE sharing cause a safe retryable failure on open.
    unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            &disposition as *const _ as *const _,
            std::mem::size_of_val(&disposition) as u32,
        )
    }
    .map_err(|_| storage())?;
    Ok(())
}

// This desktop feature is Windows-only. Fail closed on another platform until
// equivalent no-follow, directory-relative handle deletion is implemented.
#[cfg(not(windows))]
pub(super) fn pin_directories(_: &Path) -> Result<Vec<File>, StreamError> {
    Err(storage())
}
#[cfg(not(windows))]
fn remove_one(_: &Path, _: &SourceProof, _: &AtomicBool) -> Result<(), StreamError> {
    Err(storage())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    const WEBM: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    fn fixture() -> (tempfile::TempDir, BrowserCaptureStore, BrowserRecording) {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let recording = store
            .begin(
                dir.path(),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "temporary cleanup fixture",
                "video/webm",
            )
            .unwrap();
        for index in 0..2 {
            store.append(&recording.id, index, 0, WEBM).unwrap();
            store.finish_segment(&recording.id, index, 1.0).unwrap();
        }
        store.finish(&recording.id, false, None).unwrap();
        (dir, store, recording)
    }
    // Deliberately synthetic storage tests, NOT a claim these dummy bytes decode.
    // Real full A/V decode + cleanup is covered by the explicit FFmpeg fixtures.
    fn publish(store: &BrowserCaptureStore) -> BrowserMergeJob {
        let job = store.take_merge_job().unwrap().unwrap();
        let root = Path::new(&job.recording.output_dir);
        let media = format!("merged-{}.webm", job.token);
        let timeline = format!("merged-{}.timeline.jsonl", job.token);
        fs::write(root.join(&media), WEBM).unwrap();
        fs::write(root.join(&timeline), b"synthetic timeline\n").unwrap();
        let hashes = vec![format!("{:x}", Sha256::digest(WEBM)); 2];
        let proof = write_proof(
            &job,
            &hashes,
            &root.join(&media),
            &root.join(&timeline),
            &AtomicBool::new(false),
        )
        .unwrap();
        store
            .complete_merge(
                &job,
                BrowserMergedOutput {
                    file: media,
                    timeline_file: timeline,
                    bytes: WEBM.len() as u64,
                    duration_seconds: 2.0,
                    cleanup: Some(proof),
                },
            )
            .unwrap();
        job
    }
    fn clean(store: &BrowserCaptureStore) -> CleanupOutcome {
        let job = store.take_cleanup_job().unwrap().unwrap();
        let result = remove_verified_sources(&job, &AtomicBool::new(false));
        let returned = CleanupOutcome {
            deleted: result.deleted,
            complete: result.complete,
        };
        store.finish_cleanup(&job, result).unwrap();
        returned
    }
    fn source(root: &Path, index: u64) -> PathBuf {
        root.join(segment_name(index, "webm"))
    }

    #[test]
    fn cleanup_removes_only_proven_fragments_and_replay_handles_survive_reopen() {
        let (dir, store, recording) = fixture();
        let root = Path::new(&recording.output_dir);
        for name in ["chat.jsonl", "init.mp4", "assets.bin", "unowned.webm"] {
            fs::write(root.join(name), b"retain").unwrap();
        }
        let job = publish(&store);
        let mut replay = store.replay_source(&recording.id).unwrap();
        let result = clean(&store);
        assert!(result.complete);
        assert_eq!(result.deleted, 2);
        for index in 0..2 {
            assert!(!source(root, index).exists());
        }
        for name in ["chat.jsonl", "init.mp4", "assets.bin", "unowned.webm"] {
            assert_eq!(fs::read(root.join(name)).unwrap(), b"retain");
        }
        for name in [
            "segments.jsonl".to_owned(),
            format!("merged-{}.timeline.jsonl", job.token),
            format!("merged-{}.cleanup.json", job.token),
        ] {
            assert!(root.join(name).is_file());
        }
        let mut media = Vec::new();
        replay.media.read_to_end(&mut media).unwrap();
        assert_eq!(media, WEBM);
        drop(replay);
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        let record = reopened.snapshot().unwrap().remove(0);
        assert_eq!(record.status, BrowserRecordingStatus::Stopped);
        assert_eq!(record.segment_count, 2);
        assert_eq!(
            record.merge.unwrap().source_cleanup.unwrap().status,
            BrowserSourceCleanupStatus::Complete
        );
        assert!(reopened.replay_source(&recording.id).is_ok());
        assert_eq!(reopened.retry_merges(Some(&recording.id)).unwrap(), 0);
        assert!(reopened.take_cleanup_job().unwrap().is_none());
    }

    #[test]
    fn cleanup_crash_after_one_temporary_unlink_resumes_idempotently_and_keeps_partial() {
        let (dir, store, recording) = fixture();
        let root = Path::new(&recording.output_dir);
        publish(&store);
        fs::remove_file(source(root, 0)).unwrap(); // simulate crash after the first committed unlink
        let unfinished = root.join(partial_name(2, "webm"));
        fs::write(&unfinished, WEBM).unwrap();
        drop(store);
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        assert_ne!(
            reopened.snapshot().unwrap()[0].status,
            BrowserRecordingStatus::Failed
        );
        let result = clean(&reopened);
        assert!(result.complete);
        assert_eq!(result.deleted, 2);
        assert_eq!(fs::read(unfinished).unwrap(), WEBM);
        assert!(reopened.merged_file(&recording.id).is_ok());
    }

    #[test]
    fn cleanup_rejects_corrupt_same_length_output_proof_and_source() {
        for target in ["media", "timeline", "proof", "source"] {
            let (_dir, store, recording) = fixture();
            let job = publish(&store);
            let root = Path::new(&recording.output_dir);
            let path = match target {
                "media" => root.join(format!("merged-{}.webm", job.token)),
                "timeline" => root.join(format!("merged-{}.timeline.jsonl", job.token)),
                "proof" => root.join(format!("merged-{}.cleanup.json", job.token)),
                _ => source(root, 0),
            };
            let original = fs::read(&path).unwrap();
            let mut changed = original.clone();
            let middle = changed.len() / 2;
            changed[middle] ^= 1;
            fs::write(&path, changed).unwrap();
            assert!(!clean(&store).complete, "{target}");
            assert!(source(root, 0).is_file() && source(root, 1).is_file());
            let merge = store.snapshot().unwrap().remove(0).merge.unwrap();
            assert_eq!(merge.status, BrowserMergeStatus::Complete);
            assert_eq!(
                merge.source_cleanup.unwrap().status,
                BrowserSourceCleanupStatus::Blocked
            );
            fs::write(path, original).unwrap();
            assert_eq!(store.retry_merges(Some(&recording.id)).unwrap(), 1);
            assert!(clean(&store).complete);
        }
    }

    #[test]
    fn cleanup_keeps_busy_source_reader_and_retries_without_remerging() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_dir, store, recording) = fixture();
        publish(&store);
        let root = Path::new(&recording.output_dir);
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(source(root, 0))
            .unwrap();
        assert!(!clean(&store).complete);
        assert!(store.merged_file(&recording.id).is_ok());
        assert_eq!(fs::read(source(root, 0)).unwrap(), WEBM);
        drop(reader);
        store.retry_merges(Some(&recording.id)).unwrap();
        assert!(clean(&store).complete);
    }

    #[test]
    fn cleanup_completion_metadata_failure_reopens_durable_pending_without_losing_merge() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, store, recording) = fixture();
        publish(&store);
        let root = Path::new(&recording.output_dir);
        let job = store.take_cleanup_job().unwrap().unwrap();
        let result = remove_verified_sources(&job, &AtomicBool::new(false));
        assert!(result.complete);
        let metadata_reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(root.join("recording.json"))
            .unwrap();
        assert!(store.finish_cleanup(&job, result).is_err());
        assert_eq!(
            store.snapshot().unwrap()[0].merge.as_ref().unwrap().status,
            BrowserMergeStatus::Complete
        );
        assert!(store.merged_file(&recording.id).is_ok());
        drop(metadata_reader);
        drop(store);
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        assert_eq!(
            reopened.snapshot().unwrap()[0]
                .merge
                .as_ref()
                .unwrap()
                .source_cleanup
                .as_ref()
                .unwrap()
                .status,
            BrowserSourceCleanupStatus::Pending
        );
        assert!(reopened.replay_source(&recording.id).is_ok());
        assert!(clean(&reopened).complete);
    }

    #[test]
    fn cleanup_missing_journal_reports_blocked_instead_of_stalling_pending() {
        let (_dir, store, recording) = fixture();
        publish(&store);
        let root = Path::new(&recording.output_dir);
        fs::rename(
            root.join("segments.jsonl"),
            root.join("saved-test-journal.jsonl"),
        )
        .unwrap();
        assert!(store.take_cleanup_job().unwrap().is_none());
        let merge = store.snapshot().unwrap().remove(0).merge.unwrap();
        assert_eq!(merge.status, BrowserMergeStatus::Complete);
        assert_eq!(
            merge.source_cleanup.unwrap().status,
            BrowserSourceCleanupStatus::Blocked
        );
        assert!(source(root, 0).exists());
    }

    #[test]
    fn cleanup_rejects_hardlink_and_readonly_sources_without_touching_outside_file() {
        for hardlink in [true, false] {
            let (dir, store, recording) = fixture();
            publish(&store);
            let root = Path::new(&recording.output_dir);
            let path = source(root, 0);
            let outside = dir.path().join("outside-recording.webm");
            let original_permissions = fs::metadata(&path).unwrap().permissions();
            if hardlink {
                fs::hard_link(&path, &outside).unwrap();
            } else {
                let mut permissions = fs::metadata(&path).unwrap().permissions();
                permissions.set_readonly(true);
                fs::set_permissions(&path, permissions).unwrap();
            }
            assert!(!clean(&store).complete);
            assert_eq!(fs::read(&path).unwrap(), WEBM);
            assert!(store.merged_file(&recording.id).is_ok());
            if hardlink {
                assert_eq!(fs::read(outside).unwrap(), WEBM);
            } else {
                fs::set_permissions(&path, original_permissions).unwrap();
            }
        }
    }

    #[test]
    fn cleanup_rejects_manifest_traversal_and_cancellation_before_deletion() {
        let (_dir, store, recording) = fixture();
        publish(&store);
        let job = store.take_cleanup_job().unwrap().unwrap();
        assert!(!remove_verified_sources(&job, &AtomicBool::new(true)).complete);
        let root = Path::new(&recording.output_dir);
        assert!(source(root, 0).exists());
        let mut forged = job.clone();
        forged
            .recording
            .merge
            .as_mut()
            .unwrap()
            .source_cleanup
            .as_mut()
            .unwrap()
            .proof_file = "../outside.json".into();
        assert!(!remove_verified_sources(&forged, &AtomicBool::new(false)).complete);
        forged.recording.output_dir = root
            .join("..")
            .join(&recording.id)
            .to_string_lossy()
            .into_owned();
        assert!(!remove_verified_sources(&forged, &AtomicBool::new(false)).complete);
        assert!(source(root, 0).exists() && source(root, 1).exists());
    }
}
