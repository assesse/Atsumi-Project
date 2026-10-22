use super::{LibraryItem, MediaJob};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("저장 경로가 올바르지 않습니다.".into());
    }
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("연결된 폴더에는 저장할 수 없습니다.".into());
                }
            }
            if meta.file_type().is_symlink() {
                return Err("연결된 경로에는 접근할 수 없습니다.".into());
            }
        }
    }
    Ok(())
}
pub(crate) fn regular(path: &Path) -> Result<fs::Metadata, String> {
    safe_path(path)?;
    let meta = fs::symlink_metadata(path).map_err(|_| "파일을 찾을 수 없습니다.")?;
    if !meta.is_file() {
        return Err("일반 파일이 아닙니다.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|_| "파일을 잠글 수 없습니다.")?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| "파일 정보를 확인하지 못했습니다.")?;
        }
        if info.nNumberOfLinks != 1 {
            return Err("연결된 파일에는 접근할 수 없습니다.".into());
        }
    }
    Ok(meta)
}
pub(super) fn save_job(root: &Path, job: &MediaJob) -> Result<(), String> {
    safe_path(root)?;
    fs::create_dir_all(root).map_err(|_| "작업 기록을 만들지 못했습니다.")?;
    let path = root.join(format!("{}.json", job.job_id));
    let temp = root.join(format!("{}.{}.tmp", job.job_id, uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(job).map_err(|_| "작업 기록을 만들지 못했습니다.")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "작업 기록을 저장하지 못했습니다.")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "작업 기록을 저장하지 못했습니다.")?;
    drop(file);
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            },
        };
        let from: Vec<_> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(|_| "작업 기록을 교체하지 못했습니다.")?;
        }
    }
    #[cfg(not(windows))]
    fs::rename(temp, path).map_err(|_| "작업 기록을 교체하지 못했습니다.")?;
    Ok(())
}
pub(super) fn save_batch(root: &Path, jobs: &[MediaJob]) -> Result<(), String> {
    if jobs.is_empty() {
        return Ok(());
    }
    safe_path(root)?;
    let bytes = serde_json::to_vec(jobs).map_err(|_| "작업 묶음을 기록하지 못했습니다.")?;
    if bytes.len() > 1024 * 1024 {
        return Err("작업 묶음이 너무 큽니다.".into());
    }
    let id = uuid::Uuid::new_v4();
    let temporary = root.join(format!("{id}.batch.tmp"));
    let committed = root.join(format!("{id}.batch.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "작업 묶음을 기록하지 못했습니다.")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "작업 묶음을 기록하지 못했습니다.")?;
    drop(file);
    fs::rename(temporary, committed).map_err(|_| "작업 묶음을 확정하지 못했습니다.".to_string())
}
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
pub(super) fn load_jobs(root: &Path) -> Vec<MediaJob> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut jobs = std::collections::HashMap::<String, MediaJob>::new();
    for e in entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|e| e == "json"))
    {
        let path = e.path();
        let Ok(meta) = regular(&path) else {
            continue;
        };
        if meta.len() > 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let records = if path
            .file_name()
            .is_some_and(|v| v.to_string_lossy().ends_with(".batch.json"))
        {
            serde_json::from_slice::<Vec<MediaJob>>(&bytes).unwrap_or_default()
        } else {
            serde_json::from_slice::<MediaJob>(&bytes)
                .map(|v| vec![v])
                .unwrap_or_default()
        };
        for mut job in records.into_iter().take(100) {
            if uuid::Uuid::parse_str(&job.job_id).is_err() || job.reference.canonical_url().is_err()
            {
                continue;
            }
            super::broadcast::sanitize(&mut job);
            if jobs
                .get(&job.job_id)
                .is_none_or(|old| old.revision < job.revision)
            {
                jobs.insert(job.job_id.clone(), job);
            }
        }
    }
    jobs.into_values().collect()
}
pub(super) fn job_dir(root: &Path, provider: &str, id: &str) -> Result<PathBuf, String> {
    if !matches!(provider, "youtube" | "soop") || uuid::Uuid::parse_str(id).is_err() {
        return Err("작업 경로가 올바르지 않습니다.".into());
    }
    safe_path(root)?;
    let path = root
        .join(if provider == "youtube" {
            "YouTube"
        } else {
            "SOOP"
        })
        .join(id);
    safe_path(&path)?;
    Ok(path)
}
pub(super) fn hash(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<String, String> {
    regular(path)?;
    let mut file = File::open(path).map_err(|_| "검증 파일을 읽지 못했습니다.")?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1024 * 1024];
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let n = file
            .read(&mut buf)
            .map_err(|_| "검증 파일을 읽지 못했습니다.")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub(super) fn owned_file(job: &MediaJob, item: &LibraryItem) -> Result<PathBuf, String> {
    let path = job_dir(&job.download_root, &job.reference.provider, &job.job_id)?.join("media.mp4");
    let meta = regular(&path)?;
    if meta.len() != item.bytes {
        return Err("저장 파일이 변경됐습니다. 다시 검증해야 합니다.".into());
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0);
    if item.modified_millis != 0 && item.modified_millis != modified {
        return Err("저장 파일이 변경됐습니다. 다시 검증해야 합니다.".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(id: &str, root: &Path) -> MediaJob {
        MediaJob {
            broadcast_id: None,
            job_id: id.into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            reference: super::super::MediaRef {
                provider: "youtube".into(),
                kind: "video".into(),
                external_id: "abcdefghijk".into(),
            },
            revision: 1,
            state: "queued".into(),
            title: String::new(),
            channel_name: String::new(),
            created_at: 0,
            received_bytes: 0,
            total_bytes: None,
            error: None,
            library: None,
            download_root: root.to_path_buf(),
        }
    }
    #[test]
    fn broadcast_identity_roundtrips_without_guessing_legacy_or_invalid_fields() {
        let root = tempfile::tempdir().unwrap();
        let mut recording = job(&uuid::Uuid::new_v4().to_string(), root.path());
        recording.reference = super::super::MediaRef {
            provider: "soop".into(),
            kind: "live".into(),
            external_id: "fixture".into(),
        };
        recording.broadcast_id = Some("123".into());
        recording.library = Some(LibraryItem {
            broadcast_id: Some("123".into()),
            chat_archive: None,
            capture_outcome: None,
            library_id: recording.job_id.clone(),
            revision: 2,
            reference: recording.reference.clone(),
            title: String::new(),
            channel_name: String::new(),
            bytes: 1,
            duration: 4.0,
            sha256: "0".repeat(64),
            completed_at: 1,
            modified_millis: 0,
        });
        save_job(root.path(), &recording).unwrap();
        let loaded = load_jobs(root.path()).pop().unwrap();
        assert_eq!(loaded.broadcast_id.as_deref(), Some("123"));
        assert_eq!(loaded.library.unwrap().broadcast_id.as_deref(), Some("123"));
        let path = root.path().join(format!("{}.json", recording.job_id));
        for bad in [
            serde_json::Value::Null,
            serde_json::json!(123),
            serde_json::json!({"id":"123"}),
            serde_json::json!("0123"),
            serde_json::json!("123\n"),
        ] {
            let mut value = serde_json::to_value(&recording).unwrap();
            value["broadcastId"] = bad;
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            let loaded = load_jobs(root.path())
                .pop()
                .expect("bad optional field must not discard recording");
            assert!(loaded.broadcast_id.is_none());
            assert!(loaded.library.unwrap().broadcast_id.is_none());
        }
        let mut legacy = serde_json::to_value(&recording).unwrap();
        legacy.as_object_mut().unwrap().remove("broadcastId");
        legacy["library"]
            .as_object_mut()
            .unwrap()
            .remove("broadcastId");
        fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(load_jobs(root.path())[0].broadcast_id.is_none());
        let mut second = recording.clone();
        second.job_id = uuid::Uuid::new_v4().to_string();
        second.broadcast_id = Some("124".into());
        second.library.as_mut().unwrap().broadcast_id = Some("124".into());
        save_job(root.path(), &recording).unwrap();
        save_job(root.path(), &second).unwrap();
        let ids: std::collections::HashSet<_> = load_jobs(root.path())
            .into_iter()
            .map(|j| j.broadcast_id.unwrap())
            .collect();
        assert_eq!(ids.len(), 2);
    }
    #[test]
    fn restart_loads_every_atomic_batch_job_and_prefers_newest_revision() {
        let root = tempfile::tempdir().unwrap();
        let a = job(&uuid::Uuid::new_v4().to_string(), root.path());
        let mut b = job(&uuid::Uuid::new_v4().to_string(), root.path());
        save_batch(root.path(), &[a.clone(), b.clone()]).unwrap();
        b.state = "cancelled".into();
        b.revision += 1;
        save_job(root.path(), &b).unwrap();
        let loaded = load_jobs(root.path());
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded
                .iter()
                .find(|job| job.job_id == b.job_id)
                .unwrap()
                .state,
            "cancelled"
        );
        assert_eq!(
            loaded
                .iter()
                .find(|job| job.job_id == a.job_id)
                .unwrap()
                .state,
            "queued"
        );
    }
    #[test]
    fn incomplete_batch_does_not_register_a_partial_request() {
        let root = tempfile::tempdir().unwrap();
        let a = job(&uuid::Uuid::new_v4().to_string(), root.path());
        fs::write(
            root.path().join("unfinished.batch.tmp"),
            serde_json::to_vec(&vec![a]).unwrap(),
        )
        .unwrap();
        assert!(load_jobs(root.path()).is_empty());
    }
}
