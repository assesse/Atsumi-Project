//! Restartable SSD -> archive transfer. Bulk I/O never holds a capture/catalog
//! lock. Copy + flush + read-back verification precede catalog publication;
//! publication precedes unlink. The original metadata/receipt remain as evidence.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

const RECEIPT: &str = "ssd-archive-receipt.json";
const MARKER: &str = "ssd-archive-owner.json";
const MAX_FILES: usize = 65_536;
const RETRY_MS: u64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveStatus {
    Pending,
    Copying,
    CleanupPending,
    Complete,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveTransfer {
    pub status: ArchiveStatus,
    pub source_dir: String,
    pub destination_root: String,
    pub token: String,
    #[serde(default)]
    pub retry_at: u64,
    pub last_error: Option<String>,
}
impl ArchiveTransfer {
    pub(super) fn new(source: &Path, destination: &Path) -> Self {
        Self {
            status: ArchiveStatus::Pending,
            source_dir: source.to_string_lossy().into_owned(),
            destination_root: destination.to_string_lossy().into_owned(),
            token: uuid::Uuid::new_v4().simple().to_string(),
            retry_at: 0,
            last_error: None,
        }
    }
    pub(super) fn valid(&self, id: &str) -> bool {
        valid_id(&self.token)
            && valid_root_shape(Path::new(&self.source_dir), id)
            && Path::new(&self.destination_root).is_absolute()
            && !Path::new(&self.destination_root)
                .components()
                .any(|c| matches!(c, Component::ParentDir))
            && self
                .last_error
                .as_ref()
                .is_none_or(|s| s.chars().count() <= 512)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    version: u32,
    id: String,
    token: String,
    source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Copied {
    name: String,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    owner: Owner,
    files: Vec<Copied>,
}

fn failed(message: &str) -> StreamError {
    error("BROWSER_ARCHIVE_BLOCKED", message, true)
}
fn check(cancel: &AtomicBool) -> Result<(), StreamError> {
    if cancel.load(Ordering::Acquire) {
        Err(failed(
            "보관 이동을 일시 중지했습니다. 다음 실행에서 다시 시도합니다.",
        ))
    } else {
        Ok(())
    }
}

impl BrowserCaptureStore {
    pub(crate) fn take_archive_job(&self) -> Result<Option<BrowserMergeJob>, StreamError> {
        let mut state = self.lock()?;
        if state.closing || state.merge_job.is_some() {
            return Ok(None);
        }
        let Some(index) = state.recordings.iter().position(|r| {
            r.status != BrowserRecordingStatus::Recording
                && !r.deletion_pending
                && r.media_removed_at.is_none()
                && !state.active.contains_key(&r.id)
                && !state.unverified.contains(&r.id)
                && state
                    .replay_leases
                    .get(&r.id)
                    .is_none_or(|lease| Arc::strong_count(lease) == 1)
                && r.archive.as_ref().is_some_and(|a| {
                    a.valid(&r.id) && a.status != ArchiveStatus::Complete && a.retry_at <= now_ms()
                })
                && r.merge.as_ref().is_some_and(|m| {
                    m.status == BrowserMergeStatus::Complete
                        && m.source_cleanup
                            .as_ref()
                            .is_some_and(|c| c.status == BrowserSourceCleanupStatus::Complete)
                })
        }) else {
            return Ok(None);
        };
        let recording = &mut state.recordings[index];
        let a = recording.archive.as_mut().unwrap();
        a.status = if recording.output_dir == a.source_dir {
            ArchiveStatus::Copying
        } else {
            ArchiveStatus::CleanupPending
        };
        a.last_error = None;
        // Recording metadata is saved by the worker, outside this lock.
        let token = uuid::Uuid::new_v4().simple().to_string();
        let job = BrowserMergeJob {
            recording: recording.clone(),
            token: token.clone(),
            journal_len: 0,
            journal_modified: std::time::UNIX_EPOCH,
            part: None,
        };
        state.merge_job = Some((job.recording.id.clone(), token));
        Ok(Some(job))
    }

    fn publish_archive(
        &self,
        job: &BrowserMergeJob,
        next: &BrowserRecording,
    ) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        if state.merge_job.as_ref() != Some(&(job.recording.id.clone(), job.token.clone()))
            || state.closing
        {
            return Err(inactive());
        }
        // A viewer opened on the SSD during copying: keep its paths intact and
        // retry publication after it closes. Its existing playback never stops.
        if state
            .replay_leases
            .get(&job.recording.id)
            .is_some_and(|lease| Arc::strong_count(lease) > 1)
        {
            return Err(failed(
                "다시보기 종료 후 보관 이동을 마무리합니다. SSD 저장본은 유지됩니다.",
            ));
        }
        let mut catalog = state.catalog.clone();
        if state.recordings.iter().find(|r| r.id == job.recording.id) != Some(&job.recording) {
            return Err(failed(
                "녹화 정보가 갱신되어 최신 정보를 반영한 뒤 보관 이동을 다시 시도합니다.",
            ));
        }
        let entry = catalog
            .iter_mut()
            .find(|r| r.id == next.id)
            .ok_or_else(invalid)?;
        if entry.output_dir != job.recording.output_dir {
            return Err(inactive());
        }
        entry.output_dir = next.output_dir.clone();
        // Catalog is on app-data SSD; no archive HDD flush under this mutex.
        save_catalog(&self.catalog_root, &catalog)?;
        state.catalog = catalog;
        *state
            .recordings
            .iter_mut()
            .find(|r| r.id == next.id)
            .ok_or_else(invalid)? = next.clone();
        Ok(())
    }

    fn finish_archive(
        &self,
        job: &BrowserMergeJob,
        result: Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        let mut next = {
            let state = self.lock()?;
            if state.merge_job.as_ref() != Some(&(job.recording.id.clone(), job.token.clone())) {
                return Err(inactive());
            }
            state
                .recordings
                .iter()
                .find(|r| r.id == job.recording.id)
                .cloned()
                .ok_or_else(invalid)?
        };
        let a = next.archive.as_mut().ok_or_else(invalid)?;
        match result {
            Ok(()) => {
                a.status = ArchiveStatus::Complete;
                a.last_error = None;
                a.retry_at = 0;
            }
            Err(cause) => {
                a.status = if next.output_dir == a.source_dir {
                    ArchiveStatus::Blocked
                } else {
                    ArchiveStatus::CleanupPending
                };
                a.last_error = Some(bounded_text(&cause.message, 512));
                a.retry_at = now_ms().saturating_add(RETRY_MS);
            }
        }
        let saved = save_metadata(&next); // no global locks; may be a slow HDD
        let mut state = self.lock()?;
        if state.merge_job.as_ref() == Some(&(job.recording.id.clone(), job.token.clone())) {
            let index = state
                .recordings
                .iter()
                .position(|r| r.id == next.id)
                .ok_or_else(invalid)?;
            state.recordings[index] = next;
            state.merge_job = None;
        }
        saved
    }
}

pub(crate) fn run(store: &BrowserCaptureStore, job: &BrowserMergeJob, cancel: &AtomicBool) {
    let result = (|| {
        check(cancel)?;
        let a = job
            .recording
            .archive
            .as_ref()
            .filter(|a| a.valid(&job.recording.id))
            .ok_or_else(invalid)?;
        if job.recording.output_dir == a.source_dir {
            save_metadata(&job.recording)?;
            let next = copy_recording(job, cancel)?;
            check(cancel)?;
            store.publish_archive(job, &next)?;
            cleanup_source(&next, cancel)
        } else {
            cleanup_source(&job.recording, cancel)
        }
    })();
    if let Err(cause) = &result {
        tracing::warn!(recording_id=%job.recording.id, code=%cause.code, reason=%cause.message, "SSD archive transfer deferred");
    }
    if let Err(cause) = store.finish_archive(job, result) {
        tracing::warn!(code=%cause.code, "SSD archive transfer state could not be persisted");
    }
}

fn relative(name: &str) -> Result<PathBuf, StreamError> {
    let path = Path::new(name);
    if name.is_empty()
        || name.len() > 1024
        || path.is_absolute()
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || name.contains(':')
        || name.contains('\0')
    {
        return Err(invalid());
    }
    Ok(path.to_owned())
}

// Snapshot the recording directory, not a whole drive. Never follow reparse
// points, and bound both recursion and inventory size.
fn inventory(root: &Path) -> Result<Vec<(String, u64, std::time::SystemTime)>, StreamError> {
    fn visit(
        root: &Path,
        dir: &Path,
        depth: usize,
        entries: &mut Vec<(String, u64, std::time::SystemTime)>,
        seen: &mut usize,
    ) -> Result<(), StreamError> {
        *seen += 1;
        if depth > 12 || *seen > MAX_FILES {
            return Err(invalid());
        }
        if checked_directory(dir)? != dir {
            return Err(invalid());
        }
        for entry in fs::read_dir(dir).map_err(|_| storage())? {
            let path = entry.map_err(|_| storage())?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| storage())?;
            if is_link(&metadata) {
                return Err(failed("녹화 폴더에 링크가 있어 자동 이동하지 않았습니다."));
            }
            if metadata.is_dir() {
                visit(root, &path, depth + 1, entries, seen)?;
                continue;
            }
            if !safe_regular(&metadata) {
                return Err(storage());
            }
            *seen += 1;
            if *seen > MAX_FILES {
                return Err(invalid());
            }
            let name = path
                .strip_prefix(root)
                .map_err(|_| invalid())?
                .to_str()
                .ok_or_else(invalid)?
                .to_owned();
            relative(&name)?;
            if matches!(name.as_str(), "recording.json" | RECEIPT | MARKER) {
                continue;
            }
            entries.push((
                name,
                metadata.len(),
                metadata.modified().map_err(|_| storage())?,
            ));
        }
        Ok(())
    }
    let mut result = Vec::new();
    visit(root, root, 0, &mut result, &mut 0)?;
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn nested(root: &Path, name: &str, create: bool) -> Result<PathBuf, StreamError> {
    let rel = relative(name)?;
    let mut parent = root.to_owned();
    for component in rel.parent().ok_or_else(invalid)?.components() {
        let Component::Normal(name) = component else {
            return Err(invalid());
        };
        parent = if create {
            child_directory(&parent, name.to_str().ok_or_else(invalid)?)?
        } else {
            let path = parent.join(name);
            if checked_directory(&path)? != path {
                return Err(storage());
            }
            path
        };
    }
    Ok(parent.join(rel.file_name().ok_or_else(invalid)?))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T, StreamError> {
    serde_json::from_reader(BufReader::new(open_regular(path, limit)?)).map_err(|_| invalid())
}
fn write_json(root: &Path, name: &str, value: &impl Serialize) -> Result<(), StreamError> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(invalid());
    }
    atomic_write(root, name, &bytes)
}

fn digest(file: &mut File, cancel: &AtomicBool) -> Result<String, StreamError> {
    let budget = crate::storage_io_budget::BulkReadBudget::for_file(file);
    file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
    let mut sha = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        check(cancel)?;
        let started = std::time::Instant::now();
        let count = file.read(&mut buffer).map_err(|_| storage())?;
        if count == 0 {
            break;
        }
        if !budget.account(count, started.elapsed(), || cancel.load(Ordering::Acquire)) {
            return Err(inactive());
        }
        sha.update(&buffer[..count]);
    }
    Ok(format!("{:x}", sha.finalize()))
}

fn temporary_copy(destination: &Path, token: &str) -> PathBuf {
    let name_hash = format!(
        "{:x}",
        Sha256::digest(destination.to_string_lossy().as_bytes())
    );
    destination.with_file_name(format!(".atsumi-copy-{token}-{}.partial", &name_hash[..16]))
}

fn copy_file(
    source: &Path,
    destination: &Path,
    bytes: u64,
    token: &str,
    cancel: &AtomicBool,
) -> Result<String, StreamError> {
    let _source_dirs = cleanup::pin_directories(source.parent().ok_or_else(invalid)?)?;
    let _target_dirs = cleanup::pin_directories(destination.parent().ok_or_else(invalid)?)?;
    let mut input = cleanup::verification_guard(source)?;
    if input.metadata().map_err(|_| storage())?.len() != bytes {
        return Err(storage());
    }
    if destination.try_exists().map_err(|_| storage())? {
        let mut output = cleanup::verification_guard(destination)?;
        let hash = digest(&mut input, cancel)?;
        if output.metadata().map_err(|_| storage())?.len() != bytes
            || digest(&mut output, cancel)? != hash
        {
            return Err(failed(
                "보관 위치에 다른 파일이 있어 덮어쓰지 않았습니다. SSD 원본은 유지됩니다.",
            ));
        }
        return Ok(hash);
    }
    let temporary = temporary_copy(destination, token);
    // Only our owner-token-specific unfinished copy is replaceable. Never
    // truncate through a symlink, or overwrite a completed destination.
    if temporary.try_exists().map_err(|_| storage())? {
        unlink_temporary(&temporary)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| storage())?;
    let mut buffer = vec![0; 256 * 1024];
    let mut hash = Sha256::new();
    let mut written = 0u64;
    let budget = crate::storage_io_budget::BulkReadBudget::for_path(destination.parent().unwrap());
    loop {
        check(cancel)?;
        let started = std::time::Instant::now();
        let count = input.read(&mut buffer).map_err(|_| storage())?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| failed("보관 폴더에 복사하지 못했습니다. SSD 원본은 유지됩니다."))?;
        if !budget.account(count, started.elapsed(), || cancel.load(Ordering::Acquire)) {
            return Err(inactive());
        }
        hash.update(&buffer[..count]);
        written = written.saturating_add(count as u64);
    }
    if written != bytes {
        return Err(storage());
    }
    output.sync_all().map_err(|_| storage())?;
    drop(output);
    let hash = format!("{:x}", hash.finalize());
    let mut verify = cleanup::verification_guard(&temporary)?;
    if verify.metadata().map_err(|_| storage())?.len() != bytes
        || digest(&mut verify, cancel)? != hash
    {
        return Err(storage());
    }
    drop(verify);
    rename_closed(&temporary, destination, false)?;
    Ok(hash)
}

fn destination(recording: &BrowserRecording) -> Result<PathBuf, StreamError> {
    let a = recording
        .archive
        .as_ref()
        .filter(|a| a.valid(&recording.id))
        .ok_or_else(invalid)?;
    let base = checked_directory(Path::new(&a.destination_root))?;
    let parent = child_directory(&child_directory(&base, "CHZZK")?, "BrowserCapture")?;
    let root = parent.join(&recording.id);
    if root.to_string_lossy() == a.source_dir {
        return Err(invalid());
    }
    let expected = Owner {
        version: 1,
        id: recording.id.clone(),
        token: a.token.clone(),
        source: a.source_dir.clone(),
    };
    match fs::create_dir(&root) {
        Ok(()) => {
            write_json(&root, MARKER, &expected)?;
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {
            if checked_directory(&root)? != root {
                return Err(storage());
            }
            // A crash between mkdir and marker creation leaves an empty owned
            // candidate only; never adopt a nonempty folder without our marker.
            if fs::read_dir(&root).map_err(|_| storage())?.next().is_none() {
                write_json(&root, MARKER, &expected)?;
            }
            let owner: Owner = read_json(&root.join(MARKER), MAX_METADATA)?;
            if owner != expected {
                return Err(failed(
                    "보관 폴더가 이미 사용 중입니다. 기존 파일과 SSD 원본을 보존했습니다.",
                ));
            }
        }
        Err(_) => return Err(storage()),
    }
    owned_root(&root.to_string_lossy(), &recording.id)
}

/// Explicit user deletion includes this transfer's secondary copy. Unknown
/// collisions are not owned by us and must never be swept up with the recording.
pub(super) fn extra_delete_roots(
    recording: &BrowserRecording,
) -> Result<Vec<PathBuf>, StreamError> {
    let Some(a) = recording
        .archive
        .as_ref()
        .filter(|a| a.valid(&recording.id))
    else {
        return Ok(Vec::new());
    };
    let expected = Owner {
        version: 1,
        id: recording.id.clone(),
        token: a.token.clone(),
        source: a.source_dir.clone(),
    };
    let destination = Path::new(&a.destination_root)
        .join("CHZZK")
        .join("BrowserCapture")
        .join(&recording.id);
    let mut roots = Vec::new();
    for candidate in [PathBuf::from(&a.source_dir), destination] {
        if candidate.to_string_lossy() == recording.output_dir
            || !candidate.try_exists().map_err(|_| storage())?
        {
            continue;
        }
        let root = owned_root(&candidate.to_string_lossy(), &recording.id)?;
        let owned = if candidate.to_string_lossy() == a.source_dir {
            read_json::<BrowserRecording>(&root.join("recording.json"), MAX_METADATA).is_ok_and(
                |r| {
                    r.id == recording.id
                        && r.output_dir == a.source_dir
                        && r.archive.as_ref().is_some_and(|old| old.token == a.token)
                },
            )
        } else {
            read_json::<Owner>(&root.join(MARKER), MAX_METADATA)
                .is_ok_and(|owner| owner == expected)
        };
        if owned {
            roots.push(root);
        }
    }
    Ok(roots)
}

fn copy_recording(
    job: &BrowserMergeJob,
    cancel: &AtomicBool,
) -> Result<BrowserRecording, StreamError> {
    let recording = &job.recording;
    let a = recording.archive.as_ref().ok_or_else(invalid)?;
    let source = owned_root(&a.source_dir, &recording.id)?;
    let root = destination(recording)?;
    let _source_dirs = cleanup::pin_directories(&source)?;
    let _target_dirs = cleanup::pin_directories(&root)?;
    let entries = inventory(&source)?;
    let mut needed = 0u64;
    for (name, size, _) in &entries {
        // Reclaim only this owned transfer's incomplete copy before reserving
        // space. Otherwise a partial left by a crash could prevent its retry.
        let temporary = temporary_copy(&root.join(relative(name)?), &a.token);
        if temporary.try_exists().map_err(|_| storage())? {
            let _directories = cleanup::pin_directories(temporary.parent().ok_or_else(invalid)?)?;
            unlink_temporary(&temporary)?;
        }
        if !root
            .join(relative(name)?)
            .try_exists()
            .map_err(|_| storage())?
        {
            needed = needed.checked_add(*size).ok_or_else(invalid)?;
        }
    }
    ensure_space(&root, needed)?;
    let mut files = Vec::new();
    for (name, size, _) in &entries {
        check(cancel)?;
        let input = nested(&source, name, false)?;
        let output = nested(&root, name, true)?;
        files.push(Copied {
            name: name.clone(),
            bytes: *size,
            sha256: copy_file(&input, &output, *size, &a.token, cancel)?,
        });
    }
    if inventory(&source)? != entries {
        return Err(failed(
            "녹화 부가정보 저장이 진행 중입니다. 완료 후 이동을 다시 시도합니다.",
        ));
    }
    let receipt = Receipt {
        owner: Owner {
            version: 1,
            id: recording.id.clone(),
            token: a.token.clone(),
            source: a.source_dir.clone(),
        },
        files,
    };
    write_json(&root, RECEIPT, &receipt)?;
    write_json(&source, RECEIPT, &receipt)?;
    let mut next = recording.clone();
    next.output_dir = root.to_string_lossy().into_owned();
    next.archive.as_mut().unwrap().status = ArchiveStatus::CleanupPending;
    next.archive.as_mut().unwrap().retry_at = 0;
    save_metadata(&next)?;
    Ok(next)
}

fn cleanup_source(recording: &BrowserRecording, cancel: &AtomicBool) -> Result<(), StreamError> {
    let a = recording
        .archive
        .as_ref()
        .filter(|a| a.valid(&recording.id))
        .ok_or_else(invalid)?;
    let source = owned_root(&a.source_dir, &recording.id)?;
    let target = owned_root(&recording.output_dir, &recording.id)?;
    if source == target {
        return Err(invalid());
    }
    let receipt: Receipt = read_json(&target.join(RECEIPT), 32 * 1024 * 1024)?;
    let expected = Owner {
        version: 1,
        id: recording.id.clone(),
        token: a.token.clone(),
        source: a.source_dir.clone(),
    };
    if receipt.owner != expected || receipt.files.len() > MAX_FILES {
        return Err(invalid());
    }
    let _source_dirs = cleanup::pin_directories(&source)?;
    let _target_dirs = cleanup::pin_directories(&target)?;
    for copied in receipt.files {
        check(cancel)?;
        if !cleanup::valid_hash(&copied.sha256)
            || matches!(copied.name.as_str(), "recording.json" | RECEIPT | MARKER)
        {
            return Err(invalid());
        }
        let input = nested(&source, &copied.name, false)?;
        if !input.try_exists().map_err(|_| storage())? {
            continue;
        }
        let output = nested(&target, &copied.name, false)?;
        let _input_dirs = cleanup::pin_directories(input.parent().ok_or_else(invalid)?)?;
        let _output_dirs = cleanup::pin_directories(output.parent().ok_or_else(invalid)?)?;
        // Verify and pin the destination while unlinking the matching source.
        let mut saved = cleanup::verification_guard(&output)?;
        if saved.metadata().map_err(|_| storage())?.len() != copied.bytes
            || digest(&mut saved, cancel)? != copied.sha256
        {
            return Err(failed(
                "보관 파일 검증에 실패해 SSD 원본을 정리하지 않았습니다.",
            ));
        }
        unlink_verified(&input, Some((&copied, cancel)))?;
    }
    // Keep the tiny source metadata + transfer receipt for recovery/diagnosis.
    // Empty asset directories are harmless; no recursive directory deletion.
    Ok(())
}

fn unlink_temporary(path: &Path) -> Result<(), StreamError> {
    unlink_verified(path, None)
}
fn unlink_verified(path: &Path, proof: Option<(&Copied, &AtomicBool)>) -> Result<(), StreamError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x80000000 | 0x10000)
            .share_mode(1)
            .custom_flags(0x00200000);
    }
    let mut file = options
        .open(path)
        .map_err(|_| failed("사용 중인 SSD 파일은 유지했습니다. 다음 시도에서 정리합니다."))?;
    if !safe_regular(&file.metadata().map_err(|_| storage())?)
        || fs::canonicalize(path).map_err(|_| storage())? != path
    {
        return Err(storage());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| storage())?;
        if info.nNumberOfLinks != 1 {
            return Err(storage());
        }
    }
    if let Some((proof, cancel)) = proof {
        if file.metadata().map_err(|_| storage())?.len() != proof.bytes
            || digest(&mut file, cancel)? != proof.sha256
        {
            return Err(failed("SSD 원본이 변경되어 자동 정리를 중단했습니다."));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
            },
        };
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                &disposition as *const _ as *const _,
                std::mem::size_of_val(&disposition) as u32,
            )
        }
        .map_err(|_| storage())?;
    }
    #[cfg(not(windows))]
    {
        fs::remove_file(path).map_err(|_| storage())?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const WEBM: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    struct Fixture {
        _dir: tempfile::TempDir,
        app: PathBuf,
        store: BrowserCaptureStore,
        id: String,
        source: PathBuf,
        media: String,
        target: PathBuf,
    }
    // Storage transaction fixtures only. Real media decode remains the merge
    // worker's responsibility and is covered by browser_merge FFmpeg tests.
    fn fixture() -> Fixture {
        fixture_at(None)
    }
    fn fixture_at(archive_root: Option<&Path>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let app = child_directory(dir.path(), "app").unwrap();
        let work = child_directory(dir.path(), "work").unwrap();
        let target = archive_root
            .map(|root| checked_directory(root).unwrap())
            .unwrap_or_else(|| child_directory(dir.path(), "archive").unwrap());
        let store = BrowserCaptureStore::new(&app).unwrap();
        let record = store
            .begin_with_storage(
                &work,
                Some(&target),
                CHANNEL,
                "archive fixture",
                "video/webm",
                false,
            )
            .unwrap();
        for index in 0..2 {
            store.append(&record.id, index, 0, WEBM).unwrap();
            store.finish_segment(&record.id, index, 1.0).unwrap();
        }
        assert!(store.take_archive_job().unwrap().is_none());
        store.finish(&record.id, false, None).unwrap();
        assert!(store.take_archive_job().unwrap().is_none());
        let job = store.take_merge_job().unwrap().unwrap();
        let source = PathBuf::from(&record.output_dir);
        let media = format!("merged-{}.webm", job.token);
        let timeline = format!("merged-{}.timeline.jsonl", job.token);
        fs::write(source.join(&media), WEBM).unwrap();
        fs::write(source.join(&timeline), b"synthetic timeline\n").unwrap();
        fs::write(source.join("chat.jsonl"), b"{\"msg\":\"preserve chat\"}\n").unwrap();
        fs::write(source.join("viewer-metrics.jsonl"), b"{\"viewers\":42}\n").unwrap();
        fs::write(
            source.join("channel-profile.json"),
            b"{\"name\":\"saved profile\"}\n",
        )
        .unwrap();
        fs::create_dir(source.join("chat-assets")).unwrap();
        fs::write(source.join("chat-assets/badge.png"), b"synthetic badge").unwrap();
        let hashes = vec![format!("{:x}", Sha256::digest(WEBM)); 2];
        let proof = cleanup::write_proof(
            &job,
            &hashes,
            &source.join(&media),
            &source.join(&timeline),
            &AtomicBool::new(false),
        )
        .unwrap();
        store
            .complete_merge(
                &job,
                BrowserMergedOutput {
                    file: media.clone(),
                    timeline_file: timeline,
                    bytes: WEBM.len() as u64,
                    duration_seconds: 2.0,
                    cleanup: Some(proof),
                },
            )
            .unwrap();
        assert!(store.take_archive_job().unwrap().is_none());
        let clean = store.take_cleanup_job().unwrap().unwrap();
        let outcome = cleanup::remove_verified_sources(&clean, &AtomicBool::new(false));
        store.finish_cleanup(&clean, outcome).unwrap();
        Fixture {
            _dir: dir,
            app,
            store,
            id: record.id,
            source,
            media,
            target,
        }
    }
    fn record(f: &Fixture) -> BrowserRecording {
        f.store
            .lock()
            .unwrap()
            .recordings
            .iter()
            .find(|r| r.id == f.id)
            .unwrap()
            .clone()
    }

    #[test]
    fn transfers_all_assets_and_switches_catalog_before_removing_source() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        let r = record(&f);
        assert_eq!(r.archive.as_ref().unwrap().status, ArchiveStatus::Complete);
        assert_ne!(r.output_dir, f.source.to_string_lossy());
        assert_eq!(fs::read(f.store.merged_file(&f.id).unwrap()).unwrap(), WEBM);
        for name in [
            "chat.jsonl",
            "viewer-metrics.jsonl",
            "channel-profile.json",
            "chat-assets/badge.png",
        ] {
            assert!(
                Path::new(&r.output_dir).join(name).is_file(),
                "missing {name}"
            );
            assert!(!f.source.join(name).exists(), "duplicate {name}");
        }
        assert!(!f.source.join(&f.media).exists());
        assert!(f.source.join("recording.json").is_file());
        assert!(f.source.join(RECEIPT).is_file());
        let reopened = BrowserCaptureStore::new(&f.app).unwrap();
        assert_eq!(
            reopened.merged_file(&f.id).unwrap(),
            f.store.merged_file(&f.id).unwrap()
        );
        assert!(reopened.take_archive_job().unwrap().is_none());
    }

    #[test]
    fn crash_after_verified_copy_before_catalog_publication_reuses_copy() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        save_metadata(&job.recording).unwrap();
        let next = copy_recording(&job, &AtomicBool::new(false)).unwrap();
        assert_eq!(record(&f).output_dir, f.source.to_string_lossy());
        assert!(f.source.join(&f.media).exists());
        let reopened = BrowserCaptureStore::new(&f.app).unwrap();
        let retry = reopened.take_archive_job().unwrap().unwrap();
        run(&reopened, &retry, &AtomicBool::new(false));
        assert_eq!(
            reopened.merged_file(&f.id).unwrap(),
            Path::new(&next.output_dir).join(&f.media)
        );
        assert!(!f.source.join(&f.media).exists());
    }

    #[test]
    fn crash_after_catalog_publication_resumes_cleanup_without_duplicate_recording() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        let next = copy_recording(&job, &AtomicBool::new(false)).unwrap();
        f.store.publish_archive(&job, &next).unwrap();
        assert!(f.source.join(&f.media).exists());
        let reopened = BrowserCaptureStore::new(&f.app).unwrap();
        let retry = reopened.take_archive_job().unwrap().unwrap();
        assert_eq!(
            retry.recording.archive.as_ref().unwrap().status,
            ArchiveStatus::CleanupPending
        );
        run(&reopened, &retry, &AtomicBool::new(false));
        assert_eq!(reopened.lock().unwrap().recordings.len(), 1);
        assert!(!f.source.join(&f.media).exists());
    }

    #[test]
    fn cancellation_keeps_source_and_does_not_publish_destination() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(true));
        assert!(f.source.join(&f.media).exists());
        assert_eq!(record(&f).output_dir, f.source.to_string_lossy());
        assert_eq!(record(&f).archive.unwrap().status, ArchiveStatus::Blocked);
        assert!(f.store.take_archive_job().unwrap().is_none());
        f.store.retry_loaded_merges(Some(&f.id)).unwrap();
        assert!(f.store.take_archive_job().unwrap().is_some());
    }

    #[test]
    fn existing_foreign_destination_and_corrupt_copy_never_delete_or_overwrite() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        let root = destination(&job.recording).unwrap();
        fs::write(root.join(&f.media), b"unrelated data").unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        assert_eq!(fs::read(root.join(&f.media)).unwrap(), b"unrelated data");
        assert!(f.source.join(&f.media).exists());
        assert_eq!(record(&f).archive.unwrap().status, ArchiveStatus::Blocked);
        assert_eq!(record(&f).output_dir, f.source.to_string_lossy());
    }

    #[test]
    fn destination_corruption_after_copy_prevents_source_unlink() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        let next = copy_recording(&job, &AtomicBool::new(false)).unwrap();
        f.store.publish_archive(&job, &next).unwrap();
        fs::write(Path::new(&next.output_dir).join(&f.media), b"corrupt").unwrap();
        assert!(cleanup_source(&next, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(f.source.join(&f.media)).unwrap(), WEBM);
    }

    #[test]
    fn in_app_replay_defers_transfer_and_opening_during_copy_defers_publication() {
        let f = fixture();
        let replay = f.store.replay_source(&f.id).unwrap();
        assert!(f.store.take_archive_job().unwrap().is_none());
        drop(replay);
        let job = f.store.take_archive_job().unwrap().unwrap();
        let next = copy_recording(&job, &AtomicBool::new(false)).unwrap();
        let replay = f.store.replay_source(&f.id).unwrap();
        assert!(f.store.publish_archive(&job, &next).is_err());
        assert!(f.source.join(&f.media).exists());
        assert_eq!(record(&f).output_dir, f.source.to_string_lossy());
        drop(replay);
        f.store.publish_archive(&job, &next).unwrap();
    }

    #[test]
    fn missing_destination_retries_without_losing_source_and_hdd_only_records_are_untouched() {
        let f = fixture();
        fs::remove_dir(&f.target).unwrap(); // exact empty test-only directory
        let job = f.store.take_archive_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        assert!(f.source.join(&f.media).exists());
        assert_eq!(record(&f).archive.unwrap().status, ArchiveStatus::Blocked);
        let mut state = f.store.lock().unwrap();
        state.recordings.front_mut().unwrap().archive = None;
        drop(state);
        assert!(f.store.take_archive_job().unwrap().is_none());
    }

    #[test]
    fn relative_inventory_paths_cannot_escape_or_target_alternate_streams() {
        for name in [
            "../video.mp4",
            "C:\\video.mp4",
            "video:stream",
            "",
            "a/../../outside",
        ] {
            assert!(relative(name).is_err(), "{name}");
        }
        assert!(relative("chat-assets/badge.png").is_ok());
    }

    #[test]
    fn explicit_deletion_cleans_both_owned_locations_after_transfer() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        let destination = PathBuf::from(record(&f).output_dir);
        let delete = f.store.prepare_delete(&f.id).unwrap();
        let result = deletion::remove_files(&delete);
        f.store.finish_delete(&delete, result).unwrap();
        assert!(!f.source.exists());
        assert!(!destination.exists());
        assert!(f.store.snapshot().unwrap().is_empty());
    }

    #[test]
    fn interrupted_partial_copy_is_reclaimed_and_metadata_updates_are_not_lost() {
        let f = fixture();
        let job = f.store.take_archive_job().unwrap().unwrap();
        let root = destination(&job.recording).unwrap();
        let partial = temporary_copy(
            &root.join(&f.media),
            &job.recording.archive.as_ref().unwrap().token,
        );
        fs::write(&partial, b"unfinished copy").unwrap();
        let next = copy_recording(&job, &AtomicBool::new(false)).unwrap();
        assert!(!partial.exists());
        assert_eq!(fs::read(root.join(&f.media)).unwrap(), WEBM);
        f.store
            .lock()
            .unwrap()
            .recordings
            .front_mut()
            .unwrap()
            .title = "updated after copy".into();
        assert!(f.store.publish_archive(&job, &next).is_err());
        assert_eq!(record(&f).title, "updated after copy");
        assert!(f.source.join(&f.media).exists());
    }

    #[test]
    #[ignore = "opt-in tiny cross-volume fixture; set ATSUMI_ARCHIVE_TEST_ROOT"]
    fn cross_volume_archive_smoke() {
        let parent = PathBuf::from(
            std::env::var_os("ATSUMI_ARCHIVE_TEST_ROOT")
                .expect("explicit test destination required"),
        );
        let destination = tempfile::Builder::new()
            .prefix("atsumi-ssd-archive-smoke-")
            .tempdir_in(&parent)
            .unwrap();
        let f = fixture_at(Some(destination.path()));
        assert_eq!(
            crate::storage_io_budget::is_solid_state(&f.source),
            Some(true)
        );
        assert_eq!(
            crate::storage_io_budget::is_solid_state(&f.target),
            Some(false)
        );
        let job = f.store.take_archive_job().unwrap().unwrap();
        run(&f.store, &job, &AtomicBool::new(false));
        let r = record(&f);
        assert_eq!(r.archive.as_ref().unwrap().status, ArchiveStatus::Complete);
        assert!(Path::new(&r.output_dir).starts_with(&f.target));
        assert_eq!(fs::read(f.store.merged_file(&f.id).unwrap()).unwrap(), WEBM);
        assert_eq!(
            fs::read(Path::new(&r.output_dir).join("chat-assets/badge.png")).unwrap(),
            b"synthetic badge"
        );
        assert!(!f.source.join(&f.media).exists());
        println!(
            "SSD -> HDD verified: media, chat, metrics, profile, assets, catalog and SSD cleanup"
        );
    }
}
