//! Explicit library deletion, separate from automatic post-merge source cleanup.
//! Catalog tombstones precede file removal and survive partial failure/restart.
//! All media I/O is outside capture/store locks; only catalog IDs cross IPC.
use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFailure {
    pub id: String,
    pub error: StreamError,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReport {
    pub deleted_ids: Vec<String>,
    pub failures: Vec<DeleteFailure>,
}

#[derive(Debug)]
pub(crate) struct DeleteJob {
    pub id: String,
    root: PathBuf,
    token: String,
}

pub(crate) fn validate_ids(ids: &[String]) -> Result<(), StreamError> {
    let unique = ids.iter().collect::<std::collections::HashSet<_>>();
    if ids.is_empty()
        || ids.len() > MAX_RECORDINGS
        || unique.len() != ids.len()
        || ids.iter().any(|id| !valid_id(id))
    {
        return Err(invalid());
    }
    Ok(())
}

fn blocked() -> StreamError {
    error("BROWSER_DELETE_FILES_BUSY", "파일이 사용 중이거나 저장 폴더에 접근할 수 없습니다. 외부 플레이어와 폴더 내 파일을 닫고 저장 장치를 확인한 뒤 다시 삭제해 주세요. 일부 파일은 이미 삭제되었을 수 있습니다.", true)
}

impl BrowserCaptureStore {
    pub(crate) fn prepare_delete(&self, id: &str) -> Result<DeleteJob, StreamError> {
        validate_id(id)?;
        let mut state = self.lock()?;
        let index = state
            .recordings
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(invalid)?;
        let record = &state.recordings[index];
        if state.closing
            || state.active.contains_key(id)
            || state.deleting.contains_key(id)
            || record.status == BrowserRecordingStatus::Recording
            || state
                .merge_job
                .as_ref()
                .is_some_and(|(running, _)| running == id)
            || record
                .merge
                .as_ref()
                .is_some_and(|merge| merge.status == BrowserMergeStatus::Merging)
        {
            return Err(error(
                "BROWSER_DELETE_BUSY",
                "녹화·병합·파일 정리가 끝난 뒤 삭제해 주세요.",
                true,
            ));
        }
        let root = PathBuf::from(&record.output_dir);
        if !valid_root_shape(&root, id) {
            return Err(invalid());
        }
        // Only a durable previous deletion can explain an already absent root.
        if !record.deletion_pending {
            owned_root(&record.output_dir, id)?;
        }
        let mut catalog = state.catalog.clone();
        catalog
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(invalid)?
            .deletion_pending = true;
        save_catalog(&self.catalog_root, &catalog)?;
        state.catalog = catalog;
        state.recordings[index].deletion_pending = true;
        let token = uuid::Uuid::new_v4().simple().to_string();
        state.deleting.insert(id.to_owned(), token.clone());
        Ok(DeleteJob {
            id: id.to_owned(),
            root,
            token,
        })
    }

    pub(crate) fn finish_delete(
        &self,
        job: &DeleteJob,
        outcome: Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        let mut state = self.lock()?;
        if state.deleting.get(&job.id) != Some(&job.token) {
            return Err(invalid());
        }
        let result = outcome.and_then(|()| {
            let catalog = state
                .catalog
                .iter()
                .filter(|r| r.id != job.id)
                .cloned()
                .collect::<Vec<_>>();
            save_catalog(&self.catalog_root, &catalog)?;
            state.catalog = catalog;
            state.recordings.retain(|r| r.id != job.id);
            Ok(())
        });
        state.deleting.remove(&job.id);
        if let Err(cause) = &result {
            if let Some(record) = state.recordings.iter_mut().find(|r| r.id == job.id) {
                record.last_error = Some(format!("삭제 미완료 · {}", cause.message));
            }
        }
        result
    }
}

/// No symlinks/junctions, directory replacement, broad roots, or recursive
/// path-based delete calls. Every unlink uses the verified Windows handle.
#[cfg(windows)]
mod files {
    use super::*;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
            BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO,
        },
    };

    fn open(path: &Path, directory: bool) -> Result<File, StreamError> {
        let file = OpenOptions::new()
            .access_mode(0x80 | 0x10000) // READ_ATTRIBUTES | DELETE
            .share_mode(if directory { 3 } else { 1 }) // Pin names; deny active file writers.
            .custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 })
            .open(path)
            .map_err(|_| blocked())?;
        let metadata = file.metadata().map_err(|_| blocked())?;
        if is_link(&metadata)
            || metadata.is_dir() != directory
            || (!directory && (!metadata.is_file() || metadata.permissions().readonly()))
            || fs::canonicalize(path).map_err(|_| blocked())? != path
        {
            return Err(blocked());
        }
        if !directory {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
                .map_err(|_| blocked())?;
            if info.nNumberOfLinks != 1 {
                return Err(blocked());
            }
        }
        Ok(file)
    }

    fn unlink(file: File) -> Result<(), StreamError> {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                &disposition as *const _ as *const _,
                std::mem::size_of_val(&disposition) as u32,
            )
        }
        .map_err(|_| blocked())?;
        Ok(())
    }

    fn plan(
        root: &Path,
        path: &Path,
        depth: usize,
        directories: &mut Vec<File>,
        paths: &mut Vec<PathBuf>,
        count: &mut usize,
    ) -> Result<(), StreamError> {
        *count += 1;
        if depth > 16 || *count > MAX_SEGMENTS as usize * 4 + 1024 || !path.starts_with(root) {
            return Err(blocked());
        }
        directories.push(open(path, true)?);
        for entry in fs::read_dir(path).map_err(|_| blocked())? {
            let entry = entry.map_err(|_| blocked())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| blocked())?;
            if is_link(&metadata) {
                return Err(blocked());
            }
            if metadata.is_dir() {
                plan(root, &path, depth + 1, directories, paths, count)?;
            } else {
                *count += 1;
                if *count > MAX_SEGMENTS as usize * 4 + 1024 {
                    return Err(blocked());
                }
                // Preflight before the first deletion, without retaining an
                // unbounded number of media handles or reading video contents.
                drop(open(&path, false)?);
                paths.push(path);
            }
        }
        Ok(())
    }

    pub(super) fn recording(job: &DeleteJob) -> Result<(), StreamError> {
        if !valid_root_shape(&job.root, &job.id) {
            return Err(invalid());
        }
        let _ancestors = cleanup::pin_directories(job.root.parent().ok_or_else(invalid)?)?;
        match fs::symlink_metadata(&job.root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(blocked()),
            Ok(_) => {}
        }
        let mut directories = Vec::new();
        let mut paths = Vec::new();
        plan(
            &job.root,
            &job.root,
            0,
            &mut directories,
            &mut paths,
            &mut 0,
        )?;
        for path in paths {
            unlink(open(&path, false)?)?;
        }
        // New/unverified files created during deletion make directory unlink
        // fail safely. Never fall back to remove_dir_all.
        for directory in directories.into_iter().rev() {
            unlink(directory)?;
        }
        Ok(())
    }

    pub(super) fn replay_cache(root: &Path, id: &str) -> Result<(), StreamError> {
        let _ancestors = cleanup::pin_directories(root)?;
        let prefix = format!("index-v2-{id}-");
        let mut paths = Vec::new();
        for entry in fs::read_dir(root).map_err(|_| blocked())? {
            let entry = entry.map_err(|_| blocked())?;
            let name = entry.file_name();
            let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(&prefix)) else {
                continue;
            };
            let hash = suffix
                .strip_suffix(".sqlite")
                .or_else(|| suffix.strip_suffix(".sqlite-journal"));
            if !hash.is_some_and(|hash| {
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                continue;
            }
            let path = entry.path();
            drop(open(&path, false)?);
            paths.push(path);
        }
        for path in paths {
            unlink(open(&path, false)?)?;
        }
        Ok(())
    }
}

pub(crate) fn remove_files(job: &DeleteJob) -> Result<(), StreamError> {
    #[cfg(windows)]
    {
        files::recording(job)
    }
    #[cfg(not(windows))]
    {
        let _ = job;
        Err(blocked())
    }
}

pub(crate) fn remove_replay_cache(root: &Path, id: &str) -> Result<(), StreamError> {
    validate_id(id)?;
    #[cfg(windows)]
    {
        files::replay_cache(root, id)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Err(blocked())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
    const WEBM: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0];
    fn fixture() -> (tempfile::TempDir, BrowserCaptureStore, BrowserRecording) {
        let temp = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(temp.path()).unwrap();
        let recording = store
            .begin(
                temp.path(),
                CHANNEL,
                "temporary deletion fixture",
                "video/webm",
            )
            .unwrap();
        (temp, store, recording)
    }
    fn delete(store: &BrowserCaptureStore, id: &str) -> Result<(), StreamError> {
        let job = store.prepare_delete(id)?;
        let result = remove_files(&job);
        store.finish_delete(&job, result)
    }
    #[test]
    fn deletion_validates_bounded_unique_catalog_ids() {
        let (_temp, store, rec) = fixture();
        assert!(validate_ids(&[]).is_err());
        assert!(validate_ids(&["../".into()]).is_err());
        assert!(validate_ids(&[rec.id.clone(), rec.id.clone()]).is_err());
        assert!(validate_ids(&vec![rec.id.clone(); 257]).is_err());
        assert!(store.prepare_delete(&"f".repeat(32)).is_err());
        assert!(!store.snapshot().unwrap()[0].deletion_pending);
    }
    #[test]
    fn deletion_protects_capture_and_actual_merge_reservations() {
        let (_temp, store, rec) = fixture();
        assert_eq!(
            store.prepare_delete(&rec.id).unwrap_err().code,
            "BROWSER_DELETE_BUSY"
        );
        store.append(&rec.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&rec.id, 0, 15.0).unwrap();
        store.finish(&rec.id, false, None).unwrap();
        let _merge = store.take_merge_job().unwrap().unwrap();
        assert_eq!(
            store.prepare_delete(&rec.id).unwrap_err().code,
            "BROWSER_DELETE_BUSY"
        );
        assert!(!store.snapshot().unwrap()[0].deletion_pending);
    }
    #[test]
    fn deletion_removes_only_selected_owned_folder_and_persists_catalog() {
        let (temp, store, rec) = fixture();
        let other = store
            .begin(temp.path(), CHANNEL, "keep", "video/webm")
            .unwrap();
        store.finish(&rec.id, false, None).unwrap();
        let root = Path::new(&rec.output_dir);
        fs::create_dir(root.join("replay-assets")).unwrap();
        fs::write(root.join("replay-assets/profile.json"), b"{}").unwrap();
        fs::write(root.join("chat.jsonl"), b"chat").unwrap();
        fs::write(root.join("video.webm"), WEBM).unwrap();
        let unrelated = root.parent().unwrap().join("keep.txt");
        fs::write(&unrelated, b"keep").unwrap();
        delete(&store, &rec.id).unwrap();
        assert!(!root.exists());
        assert!(Path::new(&other.output_dir).exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"keep");
        assert_eq!(store.snapshot().unwrap().len(), 1);
        let reopened = BrowserCaptureStore::new(temp.path()).unwrap();
        assert_eq!(reopened.snapshot().unwrap()[0].id, other.id);
    }
    #[test]
    fn deletion_tombstone_blocks_merges_and_survives_crash_without_automatic_removal() {
        let (temp, store, rec) = fixture();
        store.append(&rec.id, 0, 0, WEBM).unwrap();
        store.finish_segment(&rec.id, 0, 15.0).unwrap();
        store.finish(&rec.id, false, None).unwrap();
        let job = store.prepare_delete(&rec.id).unwrap();
        assert!(store.prepare_delete(&rec.id).is_err());
        assert!(store.take_merge_job().unwrap().is_none());
        assert!(store.take_cleanup_job().unwrap().is_none());
        assert_eq!(store.retry_merges(None).unwrap(), 0);
        assert!(store
            .finish_with_chat(&rec.id, false, None, false, "disabled", 0)
            .is_err());
        assert!(store.finish_delete(&job, Err(blocked())).is_err());
        drop(store);
        let reopened = BrowserCaptureStore::new(temp.path()).unwrap();
        assert!(reopened.snapshot().unwrap()[0].deletion_pending);
        assert!(Path::new(&rec.output_dir).join("segments.jsonl").exists());
        assert!(reopened.take_merge_job().unwrap().is_none());
        delete(&reopened, &rec.id).unwrap();
    }
    #[test]
    fn deletion_crash_after_file_removal_can_finish_catalog_on_manual_retry() {
        let (temp, store, rec) = fixture();
        store.finish(&rec.id, false, None).unwrap();
        let job = store.prepare_delete(&rec.id).unwrap();
        remove_files(&job).unwrap(); // Simulated crash before catalog publication.
        drop(store);
        let reopened = BrowserCaptureStore::new(temp.path()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().len(), 1);
        delete(&reopened, &rec.id).unwrap();
        assert!(BrowserCaptureStore::new(temp.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn deletion_catalog_failure_remains_retryable_after_files_removed() {
        let (_temp, store, rec) = fixture();
        store.finish(&rec.id, false, None).unwrap();
        let job = store.prepare_delete(&rec.id).unwrap();
        remove_files(&job).unwrap();
        let catalog = store.catalog_root.join("catalog.jsonl");
        let backup = store.catalog_root.join("test-catalog-backup");
        fs::rename(&catalog, &backup).unwrap();
        fs::create_dir(&catalog).unwrap();
        assert!(store.finish_delete(&job, Ok(())).is_err());
        assert!(store.snapshot().unwrap()[0].deletion_pending);
        fs::remove_dir(&catalog).unwrap();
        fs::rename(backup, catalog).unwrap();
        delete(&store, &rec.id).unwrap();
    }
    #[test]
    fn deletion_preflight_preserves_files_while_an_external_reader_locks_video() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_temp, store, rec) = fixture();
        store.finish(&rec.id, false, None).unwrap();
        let video = Path::new(&rec.output_dir).join("video.webm");
        fs::write(&video, WEBM).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&video)
            .unwrap();
        assert!(delete(&store, &rec.id).is_err());
        assert!(Path::new(&rec.output_dir).join("recording.json").exists());
        assert_eq!(fs::read(&video).unwrap(), WEBM);
        drop(reader);
        delete(&store, &rec.id).unwrap();
    }
    #[test]
    fn deletion_refuses_hardlinks_readonly_and_broad_catalog_roots() {
        let (temp, store, rec) = fixture();
        store.finish(&rec.id, false, None).unwrap();
        let outside = temp.path().join("keep.webm");
        fs::write(&outside, WEBM).unwrap();
        let linked = Path::new(&rec.output_dir).join("linked.webm");
        fs::hard_link(&outside, &linked).unwrap();
        assert!(delete(&store, &rec.id).is_err());
        assert_eq!(fs::read(&outside).unwrap(), WEBM);
        fs::remove_file(&linked).unwrap();
        let readonly = Path::new(&rec.output_dir).join("readonly");
        fs::write(&readonly, b"keep").unwrap();
        let original_permissions = fs::metadata(&readonly).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&readonly, permissions).unwrap();
        assert!(delete(&store, &rec.id).is_err());
        fs::set_permissions(&readonly, original_permissions).unwrap();
        {
            let mut state = store.lock().unwrap();
            state.recordings[0].output_dir = temp.path().to_string_lossy().into_owned();
        }
        assert!(store.prepare_delete(&rec.id).is_err());
        assert!(outside.exists());
    }
    #[test]
    fn deletion_cleans_only_exact_recording_cache_names() {
        let (temp, _store, rec) = fixture();
        let root = fs::canonicalize(temp.path()).unwrap();
        let name = format!("index-v2-{}-{}.sqlite", rec.id, "a".repeat(64));
        let journal = format!("{name}-journal");
        for file in [
            &name,
            &journal,
            "settings-v1.sqlite",
            "index-v2-other.sqlite",
        ] {
            fs::write(root.join(file), b"keep or delete").unwrap();
        }
        remove_replay_cache(&root, &rec.id).unwrap();
        assert!(!root.join(name).exists());
        assert!(!root.join(journal).exists());
        assert!(root.join("settings-v1.sqlite").exists());
        assert!(root.join("index-v2-other.sqlite").exists());
    }
}
