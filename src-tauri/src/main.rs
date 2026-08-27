#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fmt::Display,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const STARTUP_FAILURE_EXIT_CODE: i32 = 1;

fn main() {
    if let Err(error) = atsumi_next_lib::run() {
        let diagnostic = redact_startup_diagnostic(&error.to_string());
        eprintln!("Atsumi Next could not be started: {diagnostic}");
        let log_path = persist_startup_failure(&diagnostic)
            .unwrap_or_else(|| PathBuf::from(".runtime").join("app-launch.log"));
        show_startup_failure(&startup_failure_message(&log_path));
        std::process::exit(STARTUP_FAILURE_EXIT_CODE);
    }
}

fn redact_startup_diagnostic(value: &str) -> String {
    let profile = std::env::var("USERPROFILE").ok();
    redact_startup_diagnostic_with_profile(value, profile.as_deref())
}

fn redact_startup_diagnostic_with_profile(value: &str, profile: Option<&str>) -> String {
    let mut redacted = profile.filter(|profile| !profile.is_empty()).map_or_else(
        || value.to_owned(),
        |profile| value.replace(profile, "%USERPROFILE%"),
    );
    for scheme in ["https://", "http://", "file:///"] {
        while let Some(start) = redacted.find(scheme) {
            let end = redacted[start..]
                .char_indices()
                .find_map(|(offset, character)| {
                    (offset > 0
                        && (character.is_whitespace()
                            || matches!(character, '\'' | '"' | ')' | ']' | '}' | ',' | ';')))
                    .then_some(start + offset)
                })
                .unwrap_or(redacted.len());
            redacted.replace_range(start..end, "<redacted-url>");
        }
    }
    redacted
}

fn persist_startup_failure(error: &impl Display) -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let log_directory = PathBuf::from(local_app_data)
        .join("Atsumi Next")
        .join("Logs");
    std::fs::create_dir_all(&log_directory).ok()?;
    let log_path = log_directory.join("startup-error.log");
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    writeln!(log, "{timestamp} Atsumi Next startup failure: {error}").ok()?;
    Some(log_path)
}

fn startup_failure_message(log_path: &Path) -> String {
    format!(
        "Atsumi Next를 시작하지 못했습니다.\n\n다른 버전에서 만든 데이터이거나 마이그레이션 백업에 실패했을 수 있습니다. 원본 데이터는 변경하지 않았습니다.\n\n자세한 기록:\n{}",
        log_path.display()
    )
}

#[cfg(windows)]
fn show_startup_failure(message: &str) {
    use std::{ffi::c_void, iter, ptr};

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            window: *mut c_void,
            text: *const u16,
            caption: *const u16,
            message_type: u32,
        ) -> i32;
    }

    const MB_OK: u32 = 0;
    const MB_ICONERROR: u32 = 0x10;
    let text = message
        .encode_utf16()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let caption = "Atsumi Next"
        .encode_utf16()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both strings are NUL-terminated and remain alive for the duration
    // of this synchronous Win32 call. A null owner is intentional at startup.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_startup_failure(_message: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failure_is_nonzero_and_points_to_a_log() {
        assert_ne!(STARTUP_FAILURE_EXIT_CODE, 0);
        let message = startup_failure_message(Path::new("Logs/startup-error.log"));
        assert!(message.contains("startup-error.log"));
        assert!(message.contains("원본 데이터는 변경하지 않았습니다"));
    }

    #[test]
    fn startup_diagnostic_hides_user_profile_and_complete_urls() {
        let redacted = redact_startup_diagnostic_with_profile(
            "failed at C:\\Users\\private\\Atsumi and https://example.invalid/path?token=secret",
            Some("C:\\Users\\private"),
        );
        assert_eq!(
            redacted,
            "failed at %USERPROFILE%\\Atsumi and <redacted-url>"
        );
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("private"));
    }
}
