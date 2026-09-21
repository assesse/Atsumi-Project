//! Explicit, per-user login startup registration. Never called automatically at launch.
//! Windows owns startup approval; we inspect its override but never rewrite it.

use serde::Serialize;
use std::path::{Path, PathBuf};

const LINK_NAME: &str = "Atsumi.Autostart.lnk";
const OWNER_DESCRIPTION: &str = "Atsumi login startup [local.atsumi.next.autostart.v1]";
// A stable UI-startup scenario, separate from the old unqualified launch trace.
// This selects a Windows ALPF bucket; it does not disable caching or prefetch.
const RELEASE_PREFETCH_ARGUMENT: &str = "/prefetch:1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchMode {
    Development,
    Installed,
    #[cfg_attr(windows, allow(dead_code))]
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    pub supported: bool,
    /// An owned registration exists. Windows can independently suppress it.
    pub enabled: bool,
    pub launch_mode: LaunchMode,
    pub needs_repair: bool,
    pub disabled_by_windows: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchSpec {
    executable: PathBuf,
    arguments: String,
    working_directory: PathBuf,
    hidden: bool,
}

fn trusted_main(label: &str, url: &tauri::Url, development: bool) -> bool {
    label == "main"
        && url.username().is_empty()
        && url.password().is_none()
        && (matches!(
            (url.scheme(), url.host_str(), url.port()),
            ("tauri", Some("localhost"), None) | ("http" | "https", Some("tauri.localhost"), None)
        ) || development
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(1420))
}

fn require_main(window: &tauri::Webview) -> Result<(), String> {
    let url = window
        .url()
        .map_err(|_| "앱 창의 주소를 확인하지 못했습니다.")?;
    if !trusted_main(window.label(), &url, cfg!(debug_assertions)) {
        return Err("자동 실행 설정은 Atsumi 기본 창에서만 변경할 수 있습니다.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn autostart_status_get(window: tauri::Webview) -> Result<AutostartStatus, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(|| platform::run(None))
        .await
        .map_err(|_| "자동 실행 상태 확인 작업을 완료하지 못했습니다.".to_string())?
}

#[tauri::command]
pub async fn autostart_enabled_set(
    window: tauri::Webview,
    enabled: bool,
) -> Result<AutostartStatus, String> {
    require_main(&window)?;
    tauri::async_runtime::spawn_blocking(move || platform::run(Some(enabled)))
        .await
        .map_err(|_| "자동 실행 설정 작업을 완료하지 못했습니다.".to_string())?
}

/// Quote an argument for Windows' argv rules; no shell expression is constructed.
fn quote_argument(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        if character == '"' {
            result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            result.extend(std::iter::repeat_n('\\', slashes));
        }
        slashes = 0;
        result.push(character);
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

fn launch_spec(
    mode: LaunchMode,
    executable: &Path,
    workspace: &Path,
    system_directory: &Path,
) -> Result<LaunchSpec, String> {
    match mode {
        LaunchMode::Development => {
            // The compile-time workspace is intentional: a relocated debug binary must
            // not discover and launch some other checkout through its ancestors.
            let launcher = workspace.join("tools/start_debug_app_hidden.ps1");
            let powershell = system_directory.join("WindowsPowerShell/v1.0/powershell.exe");
            if !workspace.is_absolute() || !launcher.is_file() || !powershell.is_file() {
                return Err(
                    "현재 개발 앱의 실행 스크립트 또는 Windows PowerShell을 찾지 못했습니다."
                        .into(),
                );
            }
            let launcher_text = launcher
                .to_str()
                .ok_or("개발 앱 실행 경로를 올바른 문자로 변환하지 못했습니다.")?;
            Ok(LaunchSpec {
                executable: powershell,
                arguments: format!(
                    "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File {}",
                    quote_argument(launcher_text)
                ),
                working_directory: workspace.to_path_buf(),
                hidden: true,
            })
        }
        LaunchMode::Installed => {
            if !executable.is_absolute() || !executable.is_file() {
                return Err("현재 설치 앱의 실행 파일을 찾지 못했습니다.".into());
            }
            let directory = executable
                .parent()
                .ok_or("앱 설치 경로를 찾지 못했습니다.")?;
            Ok(LaunchSpec {
                executable: executable.to_path_buf(),
                arguments: RELEASE_PREFETCH_ARGUMENT.into(),
                working_directory: directory.to_path_buf(),
                hidden: false,
            })
        }
        LaunchMode::Unsupported => Err("자동 실행은 Windows에서 지원합니다.".into()),
    }
}

/// StartupApproved is an OS implementation detail, not an app-owned setting.
/// Accept only recognized enabled records; unknown/malformed data is a warning.
fn approval_blocks_startup(value: Option<&[u8]>) -> bool {
    let Some(value) = value else { return false };
    value.len() != 12 || !matches!(u32::from_le_bytes(value[..4].try_into().unwrap()), 2 | 6)
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn run(change: Option<bool>) -> Result<AutostartStatus, String> {
        if change.is_some() {
            return Err("자동 실행은 Windows에서 지원합니다.".into());
        }
        Ok(AutostartStatus {
            supported: false,
            enabled: false,
            launch_mode: LaunchMode::Unsupported,
            needs_repair: false,
            disabled_by_windows: false,
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{fs, os::windows::ffi::OsStringExt, sync::Mutex};
    use windows::{
        core::{Interface, HSTRING},
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND},
            Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
                },
                Registry::{
                    RegGetValueW, HKEY_CURRENT_USER, REG_BINARY, REG_VALUE_TYPE, RRF_RT_ANY,
                },
                SystemInformation::GetSystemDirectoryW,
            },
            UI::{
                Shell::{
                    FOLDERID_Startup, IShellLinkW, SHGetKnownFolderPath, ShellLink,
                    KF_FLAG_DONT_VERIFY, SLGP_RAWPATH,
                },
                WindowsAndMessaging::{SW_SHOWMINNOACTIVE, SW_SHOWNORMAL},
            },
        },
    };

    static REGISTRATION_LOCK: Mutex<()> = Mutex::new(());

    fn native_error(context: &str, error: impl std::fmt::Display) -> String {
        format!("{context}: {error}")
    }

    // spawn_blocking workers can already be MTA. A short-lived dedicated STA avoids
    // RPC_E_CHANGED_MODE and ensures all ShellLink objects die before CoUninitialize.
    pub(super) fn run(change: Option<bool>) -> Result<AutostartStatus, String> {
        std::thread::spawn(move || {
            let _lock = REGISTRATION_LOCK
                .lock()
                .map_err(|_| "자동 실행 설정 잠금을 획득하지 못했습니다.".to_string())?;
            with_com(|| {
                let link_path = startup_directory()?.join(LINK_NAME);
                let mode = if cfg!(debug_assertions) {
                    LaunchMode::Development
                } else {
                    LaunchMode::Installed
                };
                // Failure to find a moved/deleted launch target must not block OFF.
                let expected = expected_launch(mode);
                // A fallible approval read must precede ON, otherwise a successful
                // registration could be reported as failed. OFF never needs it.
                let blocked = match change {
                    Some(false) => false,
                    Some(true) => windows_disabled()?,
                    None if read_registration(&link_path)?.is_some() => windows_disabled()?,
                    None => false,
                };
                if let Some(enabled) = change {
                    set_registration(&link_path, enabled, expected.as_ref())?;
                }
                registration_status(&link_path, mode, expected.as_ref(), blocked)
            })
        })
        .join()
        .map_err(|_| "Windows 자동 실행 작업이 중단되었습니다.".to_string())?
    }

    fn with_com<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| {
                native_error("Windows 시작 바로가기를 초기화하지 못했습니다", error)
            })?;
        let _apartment = Apartment;
        operation()
    }

    fn expected_launch(mode: LaunchMode) -> Result<LaunchSpec, String> {
        let executable = std::env::current_exe()
            .map_err(|error| native_error("현재 앱 경로를 확인하지 못했습니다", error))?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("현재 개발 작업 폴더를 확인하지 못했습니다.")?;
        let system_directory = if mode == LaunchMode::Development {
            let mut buffer = vec![0u16; 32_768];
            let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
            if length == 0 || length >= buffer.len() {
                return Err("Windows 시스템 폴더를 확인하지 못했습니다.".into());
            }
            PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]))
        } else {
            PathBuf::new()
        };
        launch_spec(mode, &executable, workspace, &system_directory)
    }

    fn startup_directory() -> Result<PathBuf, String> {
        let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DONT_VERIFY, None) }
            .map_err(|error| native_error("사용자 시작 프로그램 폴더를 찾지 못했습니다", error))?;
        let result = unsafe { raw.to_string() }
            .map(PathBuf::from)
            .map_err(|error| native_error("시작 프로그램 폴더 경로를 읽지 못했습니다", error));
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        result
    }

    fn windows_disabled() -> Result<bool, String> {
        let key = HSTRING::from(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\StartupFolder",
        );
        let mut value = [0u8; 64];
        let mut length = value.len() as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                &HSTRING::from(LINK_NAME),
                RRF_RT_ANY,
                Some(&mut kind),
                Some(value.as_mut_ptr().cast()),
                Some(&mut length),
            )
        };
        if result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND {
            return Ok(false);
        }
        if result == ERROR_MORE_DATA {
            return Ok(true);
        }
        result.ok().map_err(|error| {
            native_error(
                "Windows 시작 프로그램 허용 상태를 확인하지 못했습니다",
                error,
            )
        })?;
        Ok(kind != REG_BINARY || approval_blocks_startup(Some(&value[..length as usize])))
    }

    fn create_link() -> Result<IShellLinkW, String> {
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| native_error("Windows 시작 바로가기를 만들지 못했습니다", error))
    }

    fn wide_path(path: &Path) -> HSTRING {
        HSTRING::from(path.as_os_str())
    }

    fn read_text(
        read: impl FnOnce(&mut [u16]) -> windows::core::Result<()>,
    ) -> Result<String, String> {
        let mut buffer = vec![0u16; 32_768];
        read(&mut buffer)
            .map_err(|error| native_error("시작 바로가기 정보를 읽지 못했습니다", error))?;
        let length = buffer
            .iter()
            .position(|value| *value == 0)
            .ok_or("시작 바로가기 정보가 너무 깁니다.")?;
        String::from_utf16(&buffer[..length])
            .map_err(|_| "시작 바로가기 경로를 올바른 문자로 읽지 못했습니다.".into())
    }

    fn read_registration(path: &Path) -> Result<Option<LaunchSpec>, String> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(native_error("시작 바로가기를 확인하지 못했습니다", error)),
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err("같은 이름의 다른 시작 항목이 있어 변경하지 않았습니다.".into());
            }
            Ok(_) => {}
        }
        let link = create_link()?;
        let file: IPersistFile = link
            .cast()
            .map_err(|error| native_error("시작 바로가기를 열지 못했습니다", error))?;
        unsafe { file.Load(&wide_path(path), STGM_READ) }.map_err(|error| {
            native_error("기존 시작 바로가기를 읽지 못해 변경하지 않았습니다", error)
        })?;
        let description = read_text(|buffer| unsafe { link.GetDescription(buffer) })?;
        if description != OWNER_DESCRIPTION {
            return Err("같은 이름의 다른 시작 바로가기가 있어 변경하지 않았습니다.".into());
        }
        let executable = read_text(|buffer| unsafe {
            link.GetPath(buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
        })?;
        let arguments = read_text(|buffer| unsafe { link.GetArguments(buffer) })?;
        let directory = read_text(|buffer| unsafe { link.GetWorkingDirectory(buffer) })?;
        let hidden = unsafe { link.GetShowCmd() }
            .map_err(|error| native_error("시작 바로가기 표시 상태를 읽지 못했습니다", error))?
            == SW_SHOWMINNOACTIVE;
        Ok(Some(LaunchSpec {
            executable: executable.into(),
            arguments,
            working_directory: directory.into(),
            hidden,
        }))
    }

    fn registration_status(
        path: &Path,
        mode: LaunchMode,
        expected: Result<&LaunchSpec, &String>,
        blocked: bool,
    ) -> Result<AutostartStatus, String> {
        let registered = read_registration(path)?;
        let needs_repair = registered
            .as_ref()
            .is_some_and(|actual| expected != Ok(actual));
        Ok(AutostartStatus {
            supported: true,
            enabled: registered.is_some(),
            launch_mode: mode,
            needs_repair,
            disabled_by_windows: registered.is_some() && blocked,
        })
    }

    fn write_link(path: &Path, spec: &LaunchSpec) -> Result<(), String> {
        let link = create_link()?;
        unsafe {
            link.SetPath(&wide_path(&spec.executable))
                .and_then(|_| link.SetArguments(&HSTRING::from(&spec.arguments)))
                .and_then(|_| link.SetWorkingDirectory(&wide_path(&spec.working_directory)))
                .and_then(|_| link.SetDescription(&HSTRING::from(OWNER_DESCRIPTION)))
                .and_then(|_| {
                    link.SetShowCmd(if spec.hidden {
                        // ShellLink normalizes unsupported SW_HIDE to normal; the
                        // launcher suppresses its window with -WindowStyle Hidden.
                        SW_SHOWMINNOACTIVE
                    } else {
                        SW_SHOWNORMAL
                    })
                })
        }
        .map_err(|error| native_error("시작 바로가기를 구성하지 못했습니다", error))?;
        let file: IPersistFile = link
            .cast()
            .map_err(|error| native_error("시작 바로가기를 저장하지 못했습니다", error))?;
        unsafe { file.Save(&wide_path(path), true) }
            .map_err(|error| native_error("시작 바로가기를 저장하지 못했습니다", error))
    }

    fn set_registration(
        path: &Path,
        enabled: bool,
        expected: Result<&LaunchSpec, &String>,
    ) -> Result<(), String> {
        let existing = read_registration(path)?;
        if !enabled {
            if existing.is_some() {
                fs::remove_file(path).map_err(|error| {
                    native_error("Atsumi 시작 바로가기를 제거하지 못했습니다", error)
                })?;
            }
            return Ok(());
        }
        let expected = expected.map_err(Clone::clone)?;
        if existing.as_ref() == Some(expected) {
            return Ok(());
        }
        let directory = path
            .parent()
            .ok_or("시작 프로그램 폴더가 올바르지 않습니다.")?;
        fs::create_dir_all(directory)
            .map_err(|error| native_error("시작 프로그램 폴더를 준비하지 못했습니다", error))?;
        let temporary = directory.join(format!(".atsumi-startup-{}.tmp", uuid::Uuid::new_v4()));
        // Reserve only our unique temporary file. A failed Save/replace leaves the
        // previous registration intact; no entire folder is ever removed.
        let reserved = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                native_error("시작 바로가기 임시 파일을 준비하지 못했습니다", error)
            })?;
        drop(reserved);
        let result = (|| {
            write_link(&temporary, expected)?;
            if read_registration(&temporary)?.as_ref() != Some(expected) {
                return Err(
                    "저장한 시작 바로가기의 대상이 일치하지 않아 변경하지 않았습니다.".into(),
                );
            }
            // Recheck ownership immediately before replacing an old registration.
            if read_registration(path)? != existing {
                return Err(
                    "시작 바로가기가 다른 곳에서 변경되었습니다. 다시 시도해 주세요.".into(),
                );
            }
            unsafe {
                MoveFileExW(
                    &wide_path(&temporary),
                    &wide_path(path),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(|error| native_error("시작 바로가기를 교체하지 못했습니다", error))
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn fixture(root: &Path) -> LaunchSpec {
            LaunchSpec {
                executable: root.join("한글 폴더/app file.exe"),
                arguments: "--title \"한글 & space\"".into(),
                working_directory: root.to_path_buf(),
                hidden: true,
            }
        }

        #[test]
        fn real_development_command_and_long_arguments_roundtrip_only_in_tempdir() {
            use std::os::windows::ffi::OsStrExt;
            with_com(|| {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join(LINK_NAME);
                let expected = expected_launch(LaunchMode::Development)?;
                assert!(expected.executable.is_file());
                eprintln!(
                    "development startup command length: {} UTF-16 characters",
                    expected.executable.as_os_str().encode_wide().count()
                        + expected.arguments.encode_utf16().count()
                        + 3
                );
                set_registration(&path, true, Ok(&expected))?;
                assert_eq!(read_registration(&path)?, Some(expected.clone()));
                // CI checkout paths can be shorter. Independently guarantee coverage
                // beyond Run's 260-character command limit without executing anything.
                let mut long = expected;
                long.arguments.push_str(&format!(" {}", "x".repeat(300)));
                assert!(long.arguments.encode_utf16().count() > 260);
                set_registration(&path, true, Ok(&long))?;
                assert_eq!(read_registration(&path)?, Some(long));
                Ok(())
            })
            .unwrap();
        }

        #[test]
        fn temporary_shortcut_roundtrip_idempotence_and_off_do_not_touch_startup() {
            with_com(|| {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join(LINK_NAME);
                let spec = fixture(directory.path());
                let missing = "deleted developer launcher".to_string();
                set_registration(&path, false, Err(&missing))?;
                assert!(
                    !registration_status(&path, LaunchMode::Development, Err(&missing), true)?
                        .enabled
                );
                set_registration(&path, true, Ok(&spec))?;
                assert_eq!(read_registration(&path)?, Some(spec.clone()));
                let original = fs::read(&path).unwrap();
                set_registration(&path, true, Ok(&spec))?;
                assert_eq!(fs::read(&path).unwrap(), original);
                let status = registration_status(&path, LaunchMode::Development, Ok(&spec), true)?;
                assert!(status.enabled && status.disabled_by_windows && !status.needs_repair);
                assert!(
                    registration_status(&path, LaunchMode::Development, Err(&missing), false)?
                        .needs_repair
                );
                set_registration(&path, false, Err(&missing))?;
                assert!(!path.exists());
                assert!(directory.path().exists());
                Ok(())
            })
            .unwrap();
        }

        #[test]
        fn stale_registration_repairs_and_failed_enable_preserves_existing() {
            with_com(|| {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join(LINK_NAME);
                let old = fixture(directory.path());
                set_registration(&path, true, Ok(&old))?;
                let mut current = old.clone();
                current.arguments = "--new-workspace".into();
                assert!(
                    registration_status(&path, LaunchMode::Development, Ok(&current), false)?
                        .needs_repair
                );
                let unavailable = "target missing".to_string();
                assert!(set_registration(&path, true, Err(&unavailable)).is_err());
                assert_eq!(read_registration(&path)?, Some(old));
                set_registration(&path, true, Ok(&current))?;
                assert_eq!(read_registration(&path)?, Some(current));
                assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
                Ok(())
            })
            .unwrap();
        }

        #[test]
        fn failed_atomic_replacement_preserves_previous_shortcut_and_cleans_temp() {
            with_com(|| {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join(LINK_NAME);
                let original = fixture(directory.path());
                set_registration(&path, true, Ok(&original))?;
                let previous_bytes = fs::read(&path).unwrap();
                let mut changed = original.clone();
                changed.arguments = "--updated".into();
                let original_permissions = fs::metadata(&path).unwrap().permissions();
                let mut permissions = original_permissions.clone();
                permissions.set_readonly(true);
                fs::set_permissions(&path, permissions).unwrap();
                let result = set_registration(&path, true, Ok(&changed));
                fs::set_permissions(&path, original_permissions).unwrap();
                assert!(result.is_err());
                assert_eq!(fs::read(&path).unwrap(), previous_bytes);
                assert_eq!(read_registration(&path)?, Some(original));
                assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
                Ok(())
            })
            .unwrap();
        }

        #[test]
        fn foreign_link_and_unreadable_collision_are_never_overwritten_or_deleted() {
            with_com(|| {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join(LINK_NAME);
                let spec = fixture(directory.path());
                write_link(&path, &spec)?;
                let link = create_link()?;
                let file: IPersistFile = link.cast().unwrap();
                unsafe {
                    file.Load(&wide_path(&path), STGM_READ).unwrap();
                    link.SetDescription(&HSTRING::from("someone else's shortcut"))
                        .unwrap();
                    file.Save(&wide_path(&path), true).unwrap();
                }
                let original = fs::read(&path).unwrap();
                for enabled in [true, false] {
                    assert!(set_registration(&path, enabled, Ok(&spec)).is_err());
                    assert_eq!(fs::read(&path).unwrap(), original);
                }
                fs::write(&path, b"not a ShellLink").unwrap();
                assert!(set_registration(&path, false, Ok(&spec)).is_err());
                assert_eq!(fs::read(&path).unwrap(), b"not a ShellLink");
                Ok(())
            })
            .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_missing_known_disabled_and_malformed_records() {
        assert!(!approval_blocks_startup(None));
        for state in [2u32, 6] {
            let mut value = [0u8; 12];
            value[..4].copy_from_slice(&state.to_le_bytes());
            assert!(!approval_blocks_startup(Some(&value)));
        }
        for state in [0u32, 1, 3, 7, 99] {
            let mut value = [0u8; 12];
            value[..4].copy_from_slice(&state.to_le_bytes());
            assert!(approval_blocks_startup(Some(&value)));
        }
        for value in [&[][..], &[2][..], &[2, 0, 0, 0][..], &[0; 32][..]] {
            assert!(approval_blocks_startup(Some(value)));
        }
    }

    #[test]
    fn argument_quoting_preserves_unicode_spaces_quotes_and_trailing_slashes() {
        assert_eq!(
            quote_argument("C:\\한글 폴더\\start.ps1"),
            "\"C:\\한글 폴더\\start.ps1\""
        );
        assert_eq!(quote_argument("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_argument("C:\\folder\\"), "\"C:\\folder\\\\\"");
    }

    #[test]
    fn development_uses_its_compiled_workspace_and_installed_uses_current_exe() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("한글 workspace");
        let system = directory.path().join("System32");
        let launcher = workspace.join("tools/start_debug_app_hidden.ps1");
        let powershell = system.join("WindowsPowerShell/v1.0/powershell.exe");
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::create_dir_all(powershell.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "fixture only").unwrap();
        std::fs::write(&powershell, "fixture only").unwrap();
        let executable = workspace.join("atsumi.exe");
        std::fs::write(&executable, "fixture only").unwrap();
        let development =
            launch_spec(LaunchMode::Development, &executable, &workspace, &system).unwrap();
        assert_eq!(development.executable, powershell);
        assert!(development.hidden);
        assert!(development
            .arguments
            .ends_with(&quote_argument(launcher.to_str().unwrap())));
        assert_eq!(development.working_directory, workspace);
        let installed = launch_spec(
            LaunchMode::Installed,
            &executable,
            Path::new("missing"),
            Path::new("missing"),
        )
        .unwrap();
        assert_eq!(installed.executable, executable);
        assert_eq!(installed.arguments, "/prefetch:1");
        assert!(!installed.hidden);
        std::fs::remove_file(&launcher).unwrap();
        assert!(launch_spec(LaunchMode::Development, &executable, &workspace, &system).is_err());
    }

    #[test]
    fn status_serializes_native_frontend_contract_without_db_setting() {
        let status = AutostartStatus {
            supported: true,
            enabled: true,
            launch_mode: LaunchMode::Development,
            needs_repair: false,
            disabled_by_windows: true,
        };
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({
                "supported": true,
                "enabled": true,
                "launchMode": "development",
                "needsRepair": false,
                "disabledByWindows": true
            })
        );
        assert_eq!(
            serde_json::to_value(LaunchMode::Unsupported).unwrap(),
            "unsupported"
        );
    }

    #[test]
    fn only_main_app_origins_can_read_or_change_startup() {
        for url in [
            "tauri://localhost/",
            "http://tauri.localhost/",
            "https://tauri.localhost/path",
        ] {
            let url = url.parse().unwrap();
            assert!(trusted_main("main", &url, false));
            assert!(!trusted_main("official-browser", &url, true));
        }
        let development = "http://127.0.0.1:1420/".parse().unwrap();
        assert!(trusted_main("main", &development, true));
        assert!(!trusted_main("main", &development, false));
        for url in [
            "https://chzzk.naver.com/",
            "http://tauri.localhost.evil.example/",
            "http://localhost:1420/",
            "http://127.0.0.1:9999/",
            "http://tauri.localhost:1420/",
            "http://user@tauri.localhost/",
        ] {
            assert!(!trusted_main("main", &url.parse().unwrap(), true), "{url}");
        }
    }
}
