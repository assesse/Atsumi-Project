//! Move only proven immutable ranges off the capture SSD. Raw fragments remain
//! on the archive volume until the ordinary final decode/proof/cleanup succeeds.
//! Both range receipts are durable before any SSD copy is removed.
use super::*;

pub(super) const INDEX: &str = "ssd-ranges.json";
const INDEX_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Range {
    first: u64,
    end: u64,
    proof_sha256: String,
    source_bytes: u64,
    files: Vec<Copied>,
    cleanup_complete: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Index {
    owner: Owner,
    ranges: Vec<Range>,
}

fn owner(record: &BrowserRecording) -> Result<Owner, StreamError> {
    let a = record
        .archive
        .as_ref()
        .filter(|a| a.rolling && a.valid(&record.id))
        .ok_or_else(invalid)?;
    Ok(Owner {
        version: 1,
        id: record.id.clone(),
        token: a.token.clone(),
        source: a.source_dir.clone(),
    })
}

/// Does not create/adopt folders; resolution is safe on a read-only path.
fn saved_root(record: &BrowserRecording) -> Result<PathBuf, StreamError> {
    let a = record.archive.as_ref().ok_or_else(invalid)?;
    let path = checked_directory(Path::new(&a.destination_root))?
        .join("CHZZK")
        .join("BrowserCapture")
        .join(&record.id);
    let root = owned_root(&path.to_string_lossy(), &record.id)?;
    let actual: Owner = read_json(&root.join(MARKER), MAX_METADATA)?;
    if actual != owner(record)? {
        return Err(invalid());
    }
    Ok(root)
}

fn index(record: &BrowserRecording) -> Result<Index, StreamError> {
    let root = owned_root(&record.output_dir, &record.id)?;
    let expected = owner(record)?;
    let path = root.join(INDEX);
    if !path.try_exists().map_err(|_| storage())? {
        return Ok(Index {
            owner: expected,
            ranges: vec![],
        });
    }
    let index: Index = read_json(&path, INDEX_LIMIT)?;
    let parts = parts::load(record)?;
    if index.owner != expected || index.ranges.len() > parts.len() {
        return Err(invalid());
    }
    let mut total = 0usize;
    for (range, part) in index.ranges.iter().zip(parts.iter()) {
        let count = usize::try_from(part.end - part.first).map_err(|_| invalid())?;
        total = total.checked_add(range.files.len()).ok_or_else(invalid)?;
        if total > MAX_FILES
            || range.first != part.first
            || range.end != part.end
            || range.proof_sha256 != part.proof_sha256
            || range.files.len() != count + 3
        {
            return Err(invalid());
        }
        for (offset, file) in range.files[..count].iter().enumerate() {
            if file.name != segment_name(part.first + offset as u64, extension(&record.mime_type))
                || file.bytes == 0
                || file.bytes > MAX_SEGMENT
                || !cleanup::valid_hash(&file.sha256)
            {
                return Err(invalid());
            }
        }
        let source_bytes = range.files[..count]
            .iter()
            .try_fold(0u64, |sum, f| sum.checked_add(f.bytes))
            .ok_or_else(invalid)?;
        if range.source_bytes != source_bytes || source_bytes > record.bytes_written {
            return Err(invalid());
        }
        for (file, name) in
            range.files[count..]
                .iter()
                .zip([&part.file, &part.timeline_file, &part.proof_file])
        {
            if &file.name != name
                || file.bytes == 0
                || file.bytes > 256 * 1024 * 1024
                || !cleanup::valid_hash(&file.sha256)
            {
                return Err(invalid());
            }
        }
        if range
            .files
            .last()
            .is_none_or(|f| f.sha256 != part.proof_sha256)
            || range.files[count].bytes != part.bytes
        {
            return Err(invalid());
        }
    }
    Ok(index)
}

pub(crate) struct Resolver {
    local: PathBuf,
    remote: Option<PathBuf>,
    files: HashMap<String, u64>,
}
impl Resolver {
    pub(crate) fn new(record: &BrowserRecording) -> Result<Self, StreamError> {
        let local = owned_root(&record.output_dir, &record.id)?;
        if !record
            .archive
            .as_ref()
            .is_some_and(|a| a.rolling && record.output_dir == a.source_dir)
        {
            return Ok(Self {
                local,
                remote: None,
                files: HashMap::new(),
            });
        }
        let index = index(record)?;
        let remote = (!index.ranges.is_empty())
            .then(|| saved_root(record))
            .transpose()?;
        let files = index
            .ranges
            .into_iter()
            .flat_map(|r| r.files)
            .map(|f| (f.name, f.bytes))
            .collect();
        Ok(Self {
            local,
            remote,
            files,
        })
    }
    pub(crate) fn resolve(&self, name: &str) -> Result<PathBuf, StreamError> {
        let relative = relative(name)?;
        // Prefer the already-durable D copy, including while C unlink is pending.
        if let Some(bytes) = self.files.get(name) {
            let path = self.remote.as_ref().ok_or_else(invalid)?.join(relative);
            if open_regular(&path, *bytes)?
                .metadata()
                .map_err(|_| storage())?
                .len()
                != *bytes
            {
                return Err(invalid());
            }
            return Ok(path);
        }
        Ok(self.local.join(relative))
    }
}

/// Startup/recovery verifies remote file sizes once. Live success uses its
/// already-verified receipt instead of stat'ing all historical ranges again.
pub(crate) fn evicted(record: &BrowserRecording) -> Result<(u64, u64), StreamError> {
    if !record
        .archive
        .as_ref()
        .is_some_and(|a| a.rolling && record.output_dir == a.source_dir)
    {
        return Ok((0, 0));
    }
    let source = owned_root(&record.output_dir, &record.id)?;
    let index = index(record)?;
    if index.ranges.is_empty() {
        return Ok((0, 0));
    }
    let target = saved_root(record)?;
    let (mut bytes, mut end) = (0u64, 0u64);
    for range in &index.ranges {
        // A crash during unlink must not release reservation for resident bytes.
        if !range.cleanup_complete {
            break;
        }
        if range.files.iter().any(|f| source.join(&f.name).exists()) {
            return Err(invalid());
        }
        for file in &range.files {
            if open_regular(&target.join(&file.name), file.bytes)?
                .metadata()
                .map_err(|_| storage())?
                .len()
                != file.bytes
            {
                return Err(invalid());
            }
        }
        bytes = bytes.checked_add(range.source_bytes).ok_or_else(invalid)?;
        end = range.end;
    }
    Ok((bytes, end))
}

/// Also fence the cleanup-pending phase after D publication: otherwise final
/// cleanup could remove D originals before the remaining C copies are verified.
pub(crate) fn needs_finalization(record: &BrowserRecording) -> bool {
    record
        .archive
        .as_ref()
        .is_some_and(|a| a.rolling && a.status != ArchiveStatus::Complete)
}

pub(super) fn retained_files(
    record: &BrowserRecording,
    cancel: &AtomicBool,
) -> Result<Vec<Copied>, StreamError> {
    if !record.archive.as_ref().is_some_and(|a| a.rolling) {
        return Ok(vec![]);
    }
    let index = index(record)?;
    if index.ranges.is_empty() {
        return Ok(vec![]);
    }
    let root = saved_root(record)?;
    let _dirs = cleanup::pin_directories(&root)?;
    // Range-index publication itself may have been interrupted between D and C.
    // It is native metadata, not user media: normalize it to the validated C
    // index rather than treating a harmless receipt phase difference as collision.
    write_json(&root, INDEX, &index)?;
    let mut files = Vec::new();
    for range in index.ranges {
        for file in range.files {
            let mut input = cleanup::verification_guard(&root.join(&file.name))?;
            if input.metadata().map_err(|_| storage())?.len() != file.bytes
                || digest(&mut input, cancel)? != file.sha256
            {
                return Err(invalid());
            }
            files.push(file);
        }
    }
    Ok(files)
}

fn publish(source: &Path, target: &Path, index: &Index) -> Result<(), StreamError> {
    write_json(target, INDEX, index)?;
    write_json(source, INDEX, index)
}

struct EvictionGuard {
    store: BrowserCaptureStore,
    id: String,
}
impl Drop for EvictionGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.store.lock() {
            state.range_evictions.remove(&self.id);
        }
    }
}
fn fence_eviction(
    store: &BrowserCaptureStore,
    job: &BrowserMergeJob,
) -> Result<EvictionGuard, StreamError> {
    let mut state = store.lock()?;
    if state.merge_job.as_ref() != Some(&(job.recording.id.clone(), job.token.clone()))
        || state.closing
    {
        return Err(inactive());
    }
    if state
        .replay_leases
        .get(&job.recording.id)
        .is_some_and(|lease| Arc::strong_count(lease) > 1)
    {
        return Err(failed(
            "다시보기에서 사용 중인 SSD 구간은 유지합니다. 종료 후 이동을 마무리합니다.",
        ));
    }
    state.range_evictions.insert(job.recording.id.clone());
    Ok(EvictionGuard {
        store: store.clone(),
        id: job.recording.id.clone(),
    })
}

fn evacuate(
    store: &BrowserCaptureStore,
    job: &BrowserMergeJob,
    cancel: &AtomicBool,
) -> Result<(u64, u64), StreamError> {
    let record = &job.recording;
    let source = owned_root(&record.output_dir, &record.id)?;
    let target = destination(record)?;
    let _source_dirs = cleanup::pin_directories(&source)?;
    let _target_dirs = cleanup::pin_directories(&target)?;
    let mut index = index(record)?;
    let unfinished = index.ranges.iter().position(|r| !r.cleanup_complete);
    let range_index = if let Some(position) = unfinished {
        position
    } else {
        let parts = parts::load(record)?;
        let Some(part) = parts.get(index.ranges.len()) else {
            return evicted(record);
        };
        let verified = cleanup::verified_part_files(record, part, cancel)?;
        let needed = verified
            .iter()
            .filter(|(name, _, _)| !target.join(name).exists())
            .map(|(_, bytes, _)| *bytes)
            .sum();
        ensure_space(&target, needed)?;
        let mut files = Vec::new();
        for (name, bytes, expected) in verified {
            let hash = copy_file(
                &source.join(&name),
                &target.join(&name),
                bytes,
                &index.owner.token,
                cancel,
            )?;
            if hash != expected {
                return Err(invalid());
            }
            files.push(Copied {
                name,
                bytes,
                sha256: hash,
            });
        }
        let (sources, _) = parts::range_sources(record, part)?;
        index.ranges.push(Range {
            first: part.first,
            end: part.end,
            proof_sha256: part.proof_sha256.clone(),
            source_bytes: sources.iter().map(|s| s.bytes).sum(),
            files,
            cleanup_complete: false,
        });
        publish(&source, &target, &index)?;
        index.ranges.len() - 1
    };
    // Atomically reject an existing reader and fence new reader leases for this
    // short eviction phase. No HDD hash/copy holds the capture state lock.
    let _eviction = fence_eviction(store, job)?;
    // Resume after any crash/failed unlink without copying or deleting an
    // unknown file. Keep each destination pinned throughout source deletion.
    for file in &index.ranges[range_index].files {
        check(cancel)?;
        let mut output = cleanup::verification_guard(&target.join(&file.name))?;
        if output.metadata().map_err(|_| storage())?.len() != file.bytes
            || digest(&mut output, cancel)? != file.sha256
        {
            return Err(invalid());
        }
        let input = source.join(&file.name);
        if input.try_exists().map_err(|_| storage())? {
            unlink_verified(&input, Some((file, cancel)))?;
        }
    }
    index.ranges[range_index].cleanup_complete = true;
    publish(&source, &target, &index)?;
    let mut result = (0u64, 0u64);
    for range in index.ranges.iter().take_while(|r| r.cleanup_complete) {
        result.0 = result
            .0
            .checked_add(range.source_bytes)
            .ok_or_else(invalid)?;
        result.1 = range.end;
    }
    Ok(result)
}

impl BrowserCaptureStore {
    pub(crate) fn take_offload_job(&self) -> Result<Option<BrowserMergeJob>, StreamError> {
        let mut state = self.lock()?;
        if state.closing || state.merge_job.is_some() {
            return Ok(None);
        }
        let count = state.recordings.len();
        let start = state
            .last_offload_recording
            .as_ref()
            .and_then(|id| state.recordings.iter().position(|r| &r.id == id))
            .map_or(0, |i| i + 1);
        let Some(record) = (0..count)
            .map(|step| &state.recordings[(start + step) % count])
            .find(|r| {
                !r.deletion_pending
                    && r.media_removed_at.is_none()
                    && !state.unverified.contains(&r.id)
                    && state
                        .replay_leases
                        .get(&r.id)
                        .is_none_or(|lease| Arc::strong_count(lease) == 1)
                    && r.archive.as_ref().is_some_and(|a| {
                        a.rolling
                            && a.valid(&r.id)
                            && a.status != ArchiveStatus::Complete
                            && a.retry_at <= now_ms()
                    })
                    && (state.active.is_empty() || r.status == BrowserRecordingStatus::Recording)
                    && (r.status == BrowserRecordingStatus::Recording
                        || r.archive.as_ref().is_some_and(|a| {
                            !crate::streaming::replay_assets::pending(Path::new(&a.source_dir))
                        }))
                    && (r.status != BrowserRecordingStatus::Recording
                        || r.progressive
                            .as_ref()
                            .is_some_and(|p| p.segment_count > p.offloaded_segment_count))
            })
            .cloned()
        else {
            return Ok(None);
        };
        state.last_offload_recording = Some(record.id.clone());
        let token = uuid::Uuid::new_v4().simple().to_string();
        let job = BrowserMergeJob {
            recording: record,
            token: token.clone(),
            journal_len: 0,
            journal_modified: std::time::UNIX_EPOCH,
            part: None,
        };
        state.merge_job = Some((job.recording.id.clone(), token));
        Ok(Some(job))
    }

    fn finish_offload(
        &self,
        job: &BrowserMergeJob,
        result: Result<(u64, u64), StreamError>,
    ) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        if state.merge_job.as_ref() != Some(&(job.recording.id.clone(), job.token.clone())) {
            return Err(inactive());
        }
        let record = state
            .recordings
            .iter_mut()
            .find(|r| r.id == job.recording.id)
            .ok_or_else(invalid)?;
        let archive = record.archive.as_mut().ok_or_else(invalid)?;
        match result {
            Ok((bytes, segments)) => {
                if let Some(p) = record.progressive.as_mut() {
                    p.offloaded_bytes = bytes;
                    p.offloaded_segment_count = segments;
                }
                archive.status = ArchiveStatus::Pending;
                archive.last_error = None;
                archive.retry_at = 0;
            }
            Err(cause) => {
                archive.status = ArchiveStatus::Blocked;
                archive.last_error = Some(bounded_text(&cause.message, 512));
                archive.retry_at = now_ms().saturating_add(RETRY_MS);
            }
        }
        // The growing tail is taken from the current record, never overwritten
        // by the worker snapshot. Only small SSD metadata is saved in this lock.
        let result = save_metadata(record);
        state.merge_job = None;
        result
    }
}

pub(crate) fn run(store: &BrowserCaptureStore, job: &BrowserMergeJob, cancel: &AtomicBool) {
    if job.recording.status != BrowserRecordingStatus::Recording {
        // Coalesce the bounded tail and mutable metadata onto D before the
        // existing final merge/proof/cleanup path is admitted there.
        super::run(store, job, cancel);
        return;
    }
    let result = evacuate(store, job, cancel);
    if let Err(cause) = &result {
        tracing::warn!(recording_id=%job.recording.id, code=%cause.code, "SSD immutable-range offload deferred");
    }
    if let Err(cause) = store.finish_offload(job, result) {
        tracing::warn!(code=%cause.code, "SSD range state could not be persisted");
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const WEBM: &[u8] = &[
        0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 1, 0, 0, 0, 0, 0, 0, 0,
    ];
    struct Fixture {
        _dir: tempfile::TempDir,
        app: PathBuf,
        store: BrowserCaptureStore,
        record: BrowserRecording,
        target: PathBuf,
    }
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let app = child_directory(dir.path(), "app").unwrap();
        let work = child_directory(dir.path(), "work").unwrap();
        let target = child_directory(dir.path(), "archive").unwrap();
        let store = BrowserCaptureStore::new(&app).unwrap();
        let record = store
            .begin_with_storage(
                &work,
                Some(&target),
                CHANNEL,
                "rolling fixture",
                "video/webm",
                true,
            )
            .unwrap();
        for i in 0..3 {
            append(&store, &record.id, i);
        }
        let job = store.take_part_job().unwrap().unwrap();
        parts::tests::publish_fixture(&store, &job);
        let record = current(&store, &record.id);
        Fixture {
            _dir: dir,
            app,
            store,
            record,
            target,
        }
    }
    fn append(store: &BrowserCaptureStore, id: &str, index: u64) {
        store.append(id, index, 0, WEBM).unwrap();
        store.finish_segment(id, index, 60.0).unwrap();
    }
    fn current(store: &BrowserCaptureStore, id: &str) -> BrowserRecording {
        store
            .snapshot()
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap()
    }
    fn copy_range(record: &BrowserRecording) -> (PathBuf, Index) {
        let cancel = AtomicBool::new(false);
        let part = parts::load(record).unwrap().remove(0);
        let root = destination(record).unwrap();
        let files = cleanup::verified_part_files(record, &part, &cancel)
            .unwrap()
            .into_iter()
            .map(|(name, bytes, sha256)| {
                assert_eq!(
                    copy_file(
                        &Path::new(&record.output_dir).join(&name),
                        &root.join(&name),
                        bytes,
                        &record.archive.as_ref().unwrap().token,
                        &cancel
                    )
                    .unwrap(),
                    sha256
                );
                Copied {
                    name,
                    bytes,
                    sha256,
                }
            })
            .collect();
        let index = Index {
            owner: owner(record).unwrap(),
            ranges: vec![Range {
                first: 0,
                end: 3,
                source_bytes: 3 * WEBM.len() as u64,
                proof_sha256: part.proof_sha256,
                files,
                cleanup_complete: false,
            }],
        };
        (root, index)
    }
    fn consolidate(store: &BrowserCaptureStore, id: &str) -> BrowserRecording {
        let job = store.take_offload_job().unwrap().unwrap();
        assert_eq!(job.recording.id, id);
        run(store, &job, &AtomicBool::new(false));
        let record = current(store, id);
        assert_eq!(
            record.archive.as_ref().unwrap().status,
            ArchiveStatus::Complete,
            "{:?}",
            record.archive
        );
        record
    }

    #[test]
    fn range_offload_preserves_raw_on_archive_and_append_tail_then_finalizes_there() {
        let f = fixture();
        let job = f.store.take_offload_job().unwrap().unwrap();
        // Copy worker snapshots must never overwrite this newly acknowledged tail.
        append(&f.store, &f.record.id, 3);
        run(&f.store, &job, &AtomicBool::new(false));
        let r = current(&f.store, &f.record.id);
        assert_eq!(r.segment_count, 4);
        assert_eq!(r.progressive.as_ref().unwrap().offloaded_segment_count, 3);
        assert_eq!(
            r.progressive.as_ref().unwrap().offloaded_bytes,
            3 * WEBM.len() as u64
        );
        let source = Path::new(&r.output_dir);
        for i in 0..3 {
            assert!(!source.join(segment_name(i, "webm")).exists());
        }
        assert!(source.join(segment_name(3, "webm")).is_file());
        let replay = f.store.replay_source(&r.id).unwrap();
        assert_eq!(replay.parts.len(), 1);
        drop(replay);
        fs::write(source.join("chat.jsonl"), b"{\"fixture\":true}\n").unwrap();
        f.store.finish(&r.id, false, None).unwrap();
        assert!(f.store.take_merge_job().unwrap().is_none());
        let archived = consolidate(&f.store, &r.id);
        assert_ne!(archived.output_dir, r.output_dir);
        let final_job = f.store.take_merge_job().unwrap().unwrap();
        let segments = read_merge_segments(&final_job).unwrap();
        assert_eq!(
            parts::export_segments(&final_job.recording, &segments)
                .unwrap()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            cleanup::verified_range_inputs(&final_job, &AtomicBool::new(false))
                .unwrap()
                .0
                .len(),
            3
        );
        for s in segments {
            assert_eq!(
                fs::read(Path::new(&archived.output_dir).join(s.file)).unwrap(),
                WEBM
            );
        }
        assert!(Path::new(&archived.output_dir).join("chat.jsonl").is_file());
        assert!(f.target.join("CHZZK/BrowserCapture").is_dir());
    }

    #[test]
    fn receipt_crash_boundaries_and_partial_unlink_recover_twice_then_consolidate() {
        for phase in 0..3 {
            let f = fixture();
            let (target, mut receipt) = copy_range(&f.record);
            write_json(&target, INDEX, &receipt).unwrap();
            if phase > 0 {
                write_json(Path::new(&f.record.output_dir), INDEX, &receipt).unwrap();
                let to_remove = if phase == 1 {
                    1
                } else {
                    receipt.ranges[0].files.len()
                };
                for file in receipt.ranges[0].files.iter().take(to_remove) {
                    unlink_verified(
                        &Path::new(&f.record.output_dir).join(&file.name),
                        Some((file, &AtomicBool::new(false))),
                    )
                    .unwrap();
                }
                if phase == 2 {
                    receipt.ranges[0].cleanup_complete = true;
                    write_json(&target, INDEX, &receipt).unwrap(); // C receipt still false.
                }
            }
            drop(f.store);
            for _ in 0..2 {
                let reopened = BrowserCaptureStore::new(&f.app).unwrap();
                let r = current(&reopened, &f.record.id);
                assert_eq!(r.segment_count, 3, "phase {phase}");
                assert_ne!(
                    r.status,
                    BrowserRecordingStatus::Failed,
                    "phase {phase}: {:?}",
                    r.last_error
                );
                assert_eq!(r.progressive.as_ref().unwrap().offloaded_bytes, 0);
            }
            let reopened = BrowserCaptureStore::new(&f.app).unwrap();
            let archived = consolidate(&reopened, &f.record.id);
            for i in 0..3 {
                assert_eq!(
                    fs::read(Path::new(&archived.output_dir).join(segment_name(i, "webm")))
                        .unwrap(),
                    WEBM
                );
            }
            assert!(reopened.take_merge_job().unwrap().is_some());
        }
    }

    #[test]
    fn cancellation_collision_and_changed_source_preserve_ssd_originals() {
        for scenario in 0..3 {
            let f = fixture();
            if scenario == 1 {
                let target = destination(&f.record).unwrap();
                fs::write(target.join(segment_name(0, "webm")), b"not the same source").unwrap();
            }
            if scenario == 2 {
                fs::write(
                    Path::new(&f.record.output_dir).join(segment_name(0, "webm")),
                    b"changed source",
                )
                .unwrap();
            }
            let job = f.store.take_offload_job().unwrap().unwrap();
            run(&f.store, &job, &AtomicBool::new(scenario == 0));
            let r = current(&f.store, &f.record.id);
            assert_eq!(r.archive.as_ref().unwrap().status, ArchiveStatus::Blocked);
            assert_eq!(r.progressive.as_ref().unwrap().offloaded_bytes, 0);
            for i in 0..3 {
                assert!(Path::new(&r.output_dir)
                    .join(segment_name(i, "webm"))
                    .is_file());
            }
            append(&f.store, &r.id, 3);
        }
    }

    #[test]
    fn replay_race_prevents_eviction_and_phase_fence_rejects_new_readers() {
        let f = fixture();
        let job = f.store.take_offload_job().unwrap().unwrap();
        let replay = f.store.replay_source(&f.record.id).unwrap();
        assert!(fence_eviction(&f.store, &job).is_err());
        let result = evacuate(&f.store, &job, &AtomicBool::new(false));
        assert!(result.is_err());
        assert!(Path::new(&f.record.output_dir)
            .join(segment_name(0, "webm"))
            .exists());
        drop(replay);
        let fence = fence_eviction(&f.store, &job).unwrap();
        assert_eq!(
            f.store.replay_source(&f.record.id).err().unwrap().code,
            "BROWSER_REPLAY_BUSY"
        );
        append(&f.store, &f.record.id, 3); // the phase fence is not the append mutex.
        drop(fence);
        let result = evacuate(&f.store, &job, &AtomicBool::new(false));
        f.store.finish_offload(&job, result).unwrap();
        assert_eq!(current(&f.store, &f.record.id).segment_count, 4);
    }

    #[test]
    fn malformed_range_byte_totals_fail_closed_without_overflow() {
        let f = fixture();
        let (_, mut receipt) = copy_range(&f.record);
        receipt.ranges[0].files[0].bytes = u64::MAX;
        receipt.ranges[0].files[1].bytes = u64::MAX;
        write_json(Path::new(&f.record.output_dir), INDEX, &receipt).unwrap();
        assert!(index(&f.record).is_err());
        assert!(Path::new(&f.record.output_dir)
            .join(segment_name(0, "webm"))
            .exists());
    }

    #[test]
    fn published_archive_waits_for_remaining_ssd_cleanup_before_final_merge() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = fixture();
        let job = f.store.take_offload_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        append(&f.store, &f.record.id, 3);
        f.store.finish(&f.record.id, false, None).unwrap();
        let busy = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(Path::new(&f.record.output_dir).join(segment_name(3, "webm")))
            .unwrap();
        let job = f.store.take_offload_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        let r = current(&f.store, &f.record.id);
        assert_ne!(r.output_dir, f.record.output_dir);
        assert_eq!(
            r.archive.as_ref().unwrap().status,
            ArchiveStatus::CleanupPending
        );
        assert!(f.store.take_merge_job().unwrap().is_none());
        assert!(Path::new(&r.output_dir)
            .join(segment_name(3, "webm"))
            .is_file());
        drop(busy);
        f.store.retry_loaded_merges(Some(&r.id)).unwrap();
        consolidate(&f.store, &r.id);
        assert!(f.store.take_merge_job().unwrap().is_some());
    }

    #[test]
    fn preempted_archive_is_pending_but_shutdown_retains_retryable_failure_semantics() {
        for preempted in [false, true] {
            let f = fixture();
            f.store.finish(&f.record.id, false, None).unwrap();
            let job = f.store.take_offload_job().unwrap().unwrap();
            super::super::run_background(
                &f.store,
                &job,
                &AtomicBool::new(true),
                &AtomicBool::new(preempted),
            );
            let record = current(&f.store, &f.record.id);
            let archive = record.archive.unwrap();
            assert_eq!(
                archive.status,
                if preempted {
                    ArchiveStatus::Pending
                } else {
                    ArchiveStatus::Blocked
                }
            );
            assert_eq!(archive.retry_at == 0, preempted);
            for i in 0..3 {
                assert!(Path::new(&record.output_dir)
                    .join(segment_name(i, "webm"))
                    .exists());
            }
        }
    }

    #[test]
    fn legacy_archive_metadata_does_not_opt_in_to_rolling() {
        let f = fixture();
        let mut value = serde_json::to_value(f.record.archive.as_ref().unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("rolling");
        let archive: ArchiveTransfer = serde_json::from_value(value).unwrap();
        assert!(!archive.rolling);
    }

    #[test]
    fn archive_root_alias_is_canonicalized_before_read_only_resolution() {
        let f = fixture();
        let (target, _) = copy_range(&f.record);
        let mut alias = f.record.clone();
        let archive = alias.archive.as_mut().unwrap();
        archive.destination_root = archive.destination_root.strip_prefix(r"\\?\").unwrap_or(&archive.destination_root).to_owned();
        assert_eq!(saved_root(&alias).unwrap(), target);
    }
}
