//! Signed portable updates replace only application-owned files. User data and
//! optional fixed WebView2 runtimes are never part of the replacement set.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager};
use tauri_plugin_updater::UpdaterExt;

const TARGET: &str = "windows-x86_64-portable";
const OWNED: [&str; 5] = [
    "atsumi.exe",
    "media-tools",
    "atsumi-portable.json",
    "Start-Atsumi.cmd",
    "Readme.txt",
];
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Deserialize)]
struct Plan {
    parent_pid: u32,
    version: String,
    signature: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    downloaded: u64,
    total: Option<u64>,
    installing: bool,
}

fn io(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn durable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn portable_root(exe: &Path) -> Result<PathBuf> {
    let root = exe.parent().ok_or("실행 폴더를 확인하지 못했습니다.")?;
    if exe.file_name().and_then(|s| s.to_str()) != Some("atsumi.exe") {
        return Err("Atsumi 실행 파일 이름을 유지해 주세요.".into());
    }
    no_links(root)?;
    no_links(&root.join("atsumi-portable.json"))?;
    let marker: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("atsumi-portable.json")).map_err(io)?)
            .map_err(io)?;
    if marker["distribution"] != "portable" {
        return Err("무설치판 표시 파일이 올바르지 않습니다.".into());
    }
    Ok(root.to_owned())
}
fn no_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err("연결된 폴더에서는 자동 업데이트를 적용하지 않습니다.".into());
                    }
                }
                if meta.file_type().is_symlink() {
                    return Err("연결된 경로는 업데이트할 수 없습니다.".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(())
}
fn verify(bytes: &[u8], signature: &str) -> Result<()> {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).map_err(io)?;
    let key = STANDARD
        .decode(
            config["plugins"]["updater"]["pubkey"]
                .as_str()
                .ok_or("업데이트 키 없음")?,
        )
        .map_err(io)?;
    let signature = STANDARD.decode(signature).map_err(io)?;
    let key =
        minisign_verify::PublicKey::decode(std::str::from_utf8(&key).map_err(io)?).map_err(io)?;
    let signature =
        minisign_verify::Signature::decode(std::str::from_utf8(&signature).map_err(io)?)
            .map_err(io)?;
    key.verify(bytes, &signature, true)
        .map_err(|_| "업데이트 서명 검증에 실패했습니다.".into())
}
fn zip_path(name: &str) -> Result<PathBuf> {
    // Windows PowerShell 5 Compress-Archive emits backslashes; PowerShell 7
    // uses slashes. Normalize before, never after, traversal validation.
    let normalized = name.replace('\\', "/");
    let parts: Vec<_> = normalized.trim_end_matches('/').split('/').collect();
    if parts.first() != Some(&"Atsumi")
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.ends_with(['.', ' '])
                || p.contains(['\\', ':', '\0'])
                || p.chars().any(char::is_control)
                || [
                    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
                    "LPT8", "LPT9",
                ]
                .contains(&p.split('.').next().unwrap_or("").to_uppercase().as_str())
        })
    {
        return Err("업데이트 압축 파일에 안전하지 않은 경로가 있습니다.".into());
    }
    if parts.len() > 1 && !OWNED.contains(&parts[1]) {
        return Err("업데이트에 허용되지 않은 파일이 있습니다.".into());
    }
    Ok(parts.iter().skip(1).collect())
}
fn extract(bytes: &[u8], stage: &Path) -> Result<PathBuf> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("업데이트 파일이 너무 큽니다.".into());
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(io)?;
    if zip.len() > 4096 {
        return Err("업데이트 파일 수 제한을 초과했습니다.".into());
    }
    let payload = stage.join("payload");
    fs::create_dir(&payload).map_err(io)?;
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(io)?;
        let path = zip_path(file.name())?;
        if !seen.insert(path.to_string_lossy().to_lowercase())
            || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("업데이트에 중복 경로 또는 링크가 있습니다.".into());
        }
        total = total
            .checked_add(file.size())
            .filter(|size| *size <= MAX_EXPANDED)
            .ok_or("압축 해제 크기 제한을 초과했습니다.")?;
        let destination = payload.join(path);
        if file.is_dir() || file.name().ends_with('\\') {
            fs::create_dir_all(&destination).map_err(io)?;
            continue;
        }
        fs::create_dir_all(destination.parent().ok_or("파일 경로 오류")?).map_err(io)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(io)?;
        let size = file.size();
        if std::io::copy(&mut file.by_ref().take(size + 1), &mut output).map_err(io)? != size {
            return Err("압축 파일 크기가 일치하지 않습니다.".into());
        }
        output.sync_all().map_err(io)?;
    }
    for name in OWNED {
        if !payload.join(name).exists() {
            return Err(format!("업데이트 파일 누락: {name}"));
        }
    }
    portable_root(&payload.join("atsumi.exe"))?;
    Ok(payload)
}

/// Rename only the allowlisted roots. Roll back in reverse order without
/// deleting either version; retain backups for power-loss recovery as well.
fn replace(root: &Path, stage: &Path, payload: &Path) -> Result<()> {
    let backup = stage.join("backup");
    let rejected = stage.join("unapplied");
    fs::create_dir(&backup).map_err(io)?;
    fs::create_dir(&rejected).map_err(io)?;
    let mut moved: Vec<(&str, bool, bool)> = Vec::new();
    let result: Result<()> = (|| {
        for name in OWNED {
            no_links(&root.join(name))?;
            let existed = root.join(name).exists();
            if existed {
                fs::rename(root.join(name), backup.join(name)).map_err(io)?;
            }
            moved.push((name, existed, false));
            durable(&stage.join(format!("saved-{name}.step")), b"saved")?;
            fs::rename(payload.join(name), root.join(name)).map_err(io)?;
            moved.last_mut().unwrap().2 = true;
        }
        Ok(())
    })();
    if let Err(cause) = result {
        let mut rollback_error = None;
        for (name, old, new) in moved.into_iter().rev() {
            if new {
                if let Err(e) = fs::rename(root.join(name), rejected.join(name)) {
                    rollback_error = Some(io(e));
                    continue;
                }
            }
            if old {
                if let Err(e) = fs::rename(backup.join(name), root.join(name)) {
                    rollback_error = Some(io(e));
                }
            }
        }
        return Err(format!(
            "업데이트 적용 실패: {cause}. {}",
            rollback_error.map_or("이전 파일을 복구했습니다.".into(), |e| format!(
                "백업 폴더에서 복구가 필요합니다: {e}"
            ))
        ));
    }
    Ok(())
}
fn require_main(view: &tauri::Webview) -> Result<()> {
    if view.label() == "main" {
        Ok(())
    } else {
        Err("앱의 기본 창에서만 업데이트할 수 있습니다.".into())
    }
}
#[tauri::command]
pub fn app_update_mode(webview: tauri::Webview) -> Result<&'static str> {
    require_main(&webview)?;
    if cfg!(debug_assertions) {
        return Ok("development");
    }
    let exe = std::env::current_exe().map_err(io)?;
    if exe
        .parent()
        .is_some_and(|p| p.join("atsumi-portable.json").exists())
    {
        portable_root(&exe)?;
        Ok("portable")
    } else {
        Ok("installed")
    }
}
#[tauri::command]
pub async fn app_update_portable(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    version: String,
    progress: Channel<Progress>,
) -> Result<()> {
    require_main(&webview)?;
    if !app.state::<crate::interface::AppState>().update_reserved() {
        return Err("업데이트 작업 잠금이 필요합니다.".into());
    }
    let exe = std::env::current_exe().map_err(io)?;
    let root = portable_root(&exe)?;
    let update = app
        .updater_builder()
        .target(TARGET)
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(io)?
        .check()
        .await
        .map_err(io)?
        .ok_or("새 업데이트가 없습니다.")?;
    if update.version != version
        || update.download_url.scheme() != "https"
        || update.download_url.host_str() != Some("github.com")
    {
        return Err("업데이트 정보가 바뀌었습니다. 다시 확인해 주세요.".into());
    }
    let mut downloaded = 0;
    let bytes = update
        .download(
            |length, total| {
                downloaded += length as u64;
                let _ = progress.send(Progress {
                    downloaded,
                    total,
                    installing: false,
                });
            },
            || {},
        )
        .await
        .map_err(io)?;
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("업데이트 파일 크기 제한 초과".into());
    }
    let _ = progress.send(Progress {
        downloaded: bytes.len() as u64,
        total: Some(bytes.len() as u64),
        installing: true,
    });
    let stage = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf> {
        let updates = root.join(".atsumi-update");
        no_links(&updates)?;
        fs::create_dir_all(&updates).map_err(io)?;
        let stage = updates.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&stage).map_err(io)?;
        durable(&stage.join("update.zip"), &bytes)?;
        durable(
            &stage.join("plan.json"),
            &serde_json::to_vec(&Plan {
                parent_pid: std::process::id(),
                version,
                signature: update.signature,
            })
            .map_err(io)?,
        )?;
        fs::copy(exe, stage.join("update-helper.exe")).map_err(io)?;
        let mut command = std::process::Command::new(stage.join("update-helper.exe"));
        command.arg("--atsumi-portable-update");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(io)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(90) {
            if stage.join("ready").exists() {
                return Ok(stage);
            }
            if child.try_wait().map_err(io)?.is_some() {
                return Err(
                    "업데이트 준비 실패. .atsumi-update 폴더의 result.txt를 확인해 주세요.".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = durable(&stage.join("cancel"), b"cancel");
        Err("업데이트 준비 시간이 초과되었습니다. 현재 앱은 종료하지 않았습니다.".into())
    })
    .await
    .map_err(io)??;
    durable(&stage.join("apply"), b"apply")?;
    if let Err(e) = app
        .state::<crate::interface::AppState>()
        .finish_portable_update(app.clone())
    {
        let _ = durable(&stage.join("cancel"), b"cancel");
        return Err(e);
    }
    Ok(())
}

#[cfg(windows)]
fn helper(stage: &Path, root: &Path, plan: Plan) -> Result<()> {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            },
        },
    };
    // Pin the exact process before preparation; never forcibly terminate it.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            plan.parent_pid,
        )
    }
    .map_err(io)?;
    struct Handle(windows::Win32::Foundation::HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let handle = Handle(handle);
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(io)?;
    if fs::canonicalize(String::from_utf16_lossy(&buffer[..length as usize])).map_err(io)?
        != fs::canonicalize(root.join("atsumi.exe")).map_err(io)?
    {
        return Err("업데이트 대상 프로세스가 일치하지 않습니다.".into());
    }
    let archive = stage.join("update.zip");
    no_links(&archive)?;
    if fs::metadata(&archive).map_err(io)?.len() > MAX_ARCHIVE {
        return Err("업데이트 파일 크기 제한 초과".into());
    }
    let bytes = fs::read(&archive).map_err(io)?;
    verify(&bytes, &plan.signature)?;
    let payload = extract(&bytes, stage)?;
    drop(bytes);
    let marker: serde_json::Value =
        serde_json::from_slice(&fs::read(payload.join("atsumi-portable.json")).map_err(io)?)
            .map_err(io)?;
    if marker["version"].as_str() != Some(plan.version.as_str()) {
        return Err("업데이트 버전이 일치하지 않습니다.".into());
    }
    durable(&stage.join("ready"), b"verified")?;
    let started = Instant::now();
    loop {
        if stage.join("cancel").exists() {
            return Err("업데이트가 취소되었습니다.".into());
        }
        if unsafe { WaitForSingleObject(handle.0, 100) } == WAIT_OBJECT_0 {
            break;
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err("앱이 종료되지 않아 업데이트를 보류했습니다.".into());
        }
    }
    if !stage.join("apply").exists() || stage.join("cancel").exists() {
        return Err("업데이트 적용 승인이 없습니다.".into());
    }
    let result = replace(root, stage, &payload);
    // On replacement failure the rollback restores the old executable.
    if root.join("atsumi.exe").is_file() {
        std::process::Command::new(root.join("atsumi.exe"))
            .current_dir(root)
            .spawn()
            .map_err(io)?;
    }
    result
}

/// Called before Tauri, WebView, single-instance and user-data initialization.
pub fn run_helper() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref()
        != Some(std::ffi::OsStr::new("--atsumi-portable-update"))
    {
        return None;
    }
    let result = (|| -> Result<()> {
        let exe = std::env::current_exe().map_err(io)?;
        let stage = exe.parent().ok_or("helper path")?;
        if exe.file_name().and_then(|v| v.to_str()) != Some("update-helper.exe")
            || stage
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .is_none()
        {
            return Err("helper path".into());
        }
        let updates = stage.parent().ok_or("update path")?;
        if updates.file_name().and_then(|v| v.to_str()) != Some(".atsumi-update") {
            return Err("update path".into());
        }
        let root = portable_root(&updates.parent().ok_or("root path")?.join("atsumi.exe"))?;
        no_links(stage)?;
        no_links(&stage.join("plan.json"))?;
        no_links(&updates.join("update.lock"))?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(updates.join("update.lock"))
            .map_err(io)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "다른 업데이트가 진행 중입니다.")?;
        let plan: Plan =
            serde_json::from_slice(&fs::read(stage.join("plan.json")).map_err(io)?).map_err(io)?;
        #[cfg(windows)]
        let result = helper(stage, &root, plan);
        #[cfg(not(windows))]
        let result: Result<()> = Err("Windows only".into());
        let message = result
            .as_ref()
            .map(|_| "업데이트 완료".to_owned())
            .unwrap_or_else(Clone::clone);
        let _ = durable(&stage.join("result.txt"), message.as_bytes());
        result
    })();
    Some(if result.is_ok() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload(root: &Path) {
        fs::create_dir_all(root.join("media-tools")).unwrap();
        fs::write(root.join("media-tools/ffmpeg.exe"), b"fixture").unwrap();
        for file in ["atsumi.exe", "Start-Atsumi.cmd", "Readme.txt"] {
            fs::write(root.join(file), b"new").unwrap();
        }
        fs::write(
            root.join("atsumi-portable.json"),
            br#"{"distribution":"portable","format":1}"#,
        )
        .unwrap();
    }
    #[test]
    fn zip_paths_reject_traversal_and_user_data() {
        for path in [
            "../atsumi.exe",
            "Atsumi/../data",
            "Atsumi/media-tools/../../data",
            "Atsumi/C:/data",
            "Atsumi/atsumi.exe:stream",
            "Atsumi/NUL",
            "Atsumi/media-tools/CON.txt",
            "Atsumi/atsumi.exe.",
            "Atsumi/downloads/file",
            "Atsumi/\\other",
        ] {
            assert!(zip_path(path).is_err(), "{path}");
        }
        assert_eq!(
            zip_path("Atsumi/media-tools/avcodec.dll").unwrap(),
            PathBuf::from("media-tools/avcodec.dll")
        );
        assert_eq!(
            zip_path("Atsumi\\media-tools\\avcodec.dll").unwrap(),
            PathBuf::from("media-tools/avcodec.dll")
        );
        assert!(zip_path("Atsumi\\..\\outside").is_err());
    }
    #[test]
    fn unsigned_archive_cannot_be_applied() {
        assert!(verify(b"malicious", "AAAA").is_err());
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in entries {
            zip.start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn extraction_rejects_traversal_missing_files_and_case_collisions() {
        for entries in [
            vec![("Atsumi/../outside.txt", &b"bad"[..])],
            vec![("Atsumi/atsumi.exe", &b"new"[..])],
            vec![
                ("Atsumi/media-tools/a.dll", &b"one"[..]),
                ("Atsumi/media-tools/A.dll", &b"two"[..]),
            ],
        ] {
            let directory = tempfile::tempdir().unwrap();
            assert!(extract(&archive(&entries), directory.path()).is_err());
            assert!(!directory.path().join("outside.txt").exists());
        }
    }

    #[test]
    fn extraction_keeps_expected_nested_media_tools() {
        let directory = tempfile::tempdir().unwrap();
        let zip = archive(&[
            ("Atsumi/atsumi.exe", b"program"),
            ("Atsumi/media-tools/bin/ffmpeg.exe", b"tool"),
            (
                "Atsumi/atsumi-portable.json",
                br#"{"distribution":"portable"}"#,
            ),
            ("Atsumi/Start-Atsumi.cmd", b"launcher"),
            ("Atsumi/Readme.txt", b"readme"),
        ]);
        let payload = extract(&zip, directory.path()).unwrap();
        assert_eq!(
            fs::read(payload.join("media-tools/bin/ffmpeg.exe")).unwrap(),
            b"tool"
        );
    }

    #[cfg(windows)]
    #[test]
    fn locked_executable_does_not_modify_the_existing_installation() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("app");
        payload(&root);
        fs::write(root.join("atsumi.exe"), b"old").unwrap();
        let stage = directory.path().join("stage");
        let new = stage.join("payload");
        payload(&new);
        let _lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(root.join("atsumi.exe"))
            .unwrap();
        assert!(replace(&root, &stage, &new).is_err());
        assert_eq!(fs::read(new.join("atsumi.exe")).unwrap(), b"new");
        assert!(root.join("media-tools/ffmpeg.exe").exists());
    }

    #[test]
    #[ignore = "requires a locally built, signed release ZIP; no user app is opened"]
    fn production_signed_portable_archive_verifies_extracts_and_rejects_tampering() {
        let archive = PathBuf::from(
            std::env::var("ATSUMI_TEST_SIGNED_PORTABLE").expect("explicit signed ZIP fixture"),
        );
        let mut bytes = fs::read(&archive).unwrap();
        let signature = fs::read_to_string(format!("{}.sig", archive.display())).unwrap();
        verify(&bytes, signature.trim()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let payload = extract(&bytes, directory.path()).unwrap();
        assert!(fs::metadata(payload.join("atsumi.exe")).unwrap().len() > 1_000_000);
        bytes[100] ^= 1;
        assert!(verify(&bytes, signature.trim()).is_err());
    }
    #[test]
    fn replace_keeps_user_files_and_old_backup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app");
        payload(&root);
        fs::write(root.join("atsumi.exe"), b"old").unwrap();
        fs::write(root.join("my-download.txt"), b"keep").unwrap();
        let stage = dir.path().join("stage");
        let new = stage.join("payload");
        payload(&new);
        replace(&root, &stage, &new).unwrap();
        assert_eq!(fs::read(root.join("atsumi.exe")).unwrap(), b"new");
        assert_eq!(fs::read(root.join("my-download.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(stage.join("backup/atsumi.exe")).unwrap(), b"old");
    }
    #[test]
    fn failed_replacement_rolls_back_without_deleting_either_version() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("app");
        payload(&root);
        fs::write(root.join("atsumi.exe"), b"old").unwrap();
        let stage = dir.path().join("stage");
        let new = stage.join("payload");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("atsumi.exe"), b"new").unwrap();
        assert!(replace(&root, &stage, &new).is_err());
        assert_eq!(fs::read(root.join("atsumi.exe")).unwrap(), b"old");
        assert_eq!(
            fs::read(stage.join("unapplied/atsumi.exe")).unwrap(),
            b"new"
        );
        assert!(root.join("media-tools/ffmpeg.exe").exists());
    }
}
