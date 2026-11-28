//! Catalog-only upload sources. Renderer input can select identities, never files.
use super::{CloudSource, CloudSourceRef, SourceResolveContext};
use crate::{
    platform_media::PlatformMediaService,
    streaming::{browser_store::{BrowserCaptureStore, BrowserReplaySource}, startup::DeferredBrowser},
};
use std::{fs::File, path::{Path, PathBuf}, sync::{Arc, atomic::Ordering}};

#[path = "source_plan.rs"]
mod plan;
pub use plan::{SourceCoverage, SourceDescription, SourcePart};

pub(crate) fn validate_reference(source: &CloudSourceRef) -> Result<(), String> {
    let id = &source.recording_id;
    let valid = match source.provider.as_str() {
        "chzzk" => id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            && uuid::Uuid::parse_str(id).is_ok(),
        "youtube" | "soop" => uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == *id)
            && source.part_index.is_none(),
        _ => false,
    };
    if !valid || !source.valid() { return Err("녹화 항목이 올바르지 않습니다.".into()); }
    Ok(())
}
fn source_bounds(duration: f64, offset: f64, bytes: u64) -> Result<(), String> {
    if !duration.is_finite() || duration <= 0.0 || !offset.is_finite() || offset < 0.0 || bytes == 0 {
        return Err("검증된 영상 구간 정보가 없습니다.".into());
    }
    Ok(())
}
fn chzzk_source(browser: &DeferredBrowser, reference: &CloudSourceRef) -> Result<(BrowserCaptureStore, BrowserReplaySource), String> {
    chzzk_source_cancellable(browser, reference, None)
}
fn chzzk_source_cancellable(browser: &DeferredBrowser, reference: &CloudSourceRef, cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<(BrowserCaptureStore, BrowserReplaySource), String> {
    let host = browser.get().map_err(|error| error.message)?;
    let store = host.capture_store();
    let store: BrowserCaptureStore = store.lock().map_err(|_| "녹화 기록을 확인하지 못했습니다.")?.clone();
    // The shared store is cloned before slower verification/opening; capture acknowledgements
    // never wait behind the host's store mutex.
    let held = if let Some(cancel) = cancel {
        store.upload_source_cancellable(&reference.recording_id, reference.part_index.is_some(), cancel)
    } else if reference.part_index.is_some() {
        store.upload_part_source(&reference.recording_id)
    } else {
        store.upload_source(&reference.recording_id)
    }.map_err(|error| error.message)?;
    Ok((store, held))
}
pub(crate) fn describe(
    browser: &DeferredBrowser, platform: &PlatformMediaService, data_dir: &Path, reference: &CloudSourceRef,
) -> Result<SourceDescription, String> {
    plan::describe(browser, platform, data_dir, reference)
}
pub(crate) fn get_plan(data_dir: &Path, plan_id: &str) -> Result<SourceDescription, String> {
    plan::get(data_dir, plan_id)
}
pub(crate) fn replay_plan_binding(data_dir: &Path, reference: &CloudSourceRef, sha256: &str,
    bytes: u64, duration: f64, offset: f64,
) -> Result<(SourceDescription, Vec<super::replay::CloudReplayInput>, Option<String>, bool), String> {
    plan::replay_binding(data_dir, reference, sha256, bytes, duration, offset)
}
pub(crate) fn prepare_plan(browser: &DeferredBrowser, platform: &PlatformMediaService, data_dir: &Path,
    reference: &CloudSourceRef, context: &SourceResolveContext) -> Result<Vec<CloudSourceRef>, String> {
    plan::prepare(browser, platform, data_dir, reference, context)
}
pub(crate) fn resolve(
    browser: &DeferredBrowser, platform: &PlatformMediaService, data_dir: &Path, reference: &CloudSourceRef,
    context: &SourceResolveContext,
) -> Result<CloudSource, String> {
    validate_reference(reference)?;
    if context.cancel.load(Ordering::Acquire) { return Err("업로드 준비를 일시정지했습니다.".into()); }
    if reference.plan_id.is_some() { return plan::resolve(browser, platform, data_dir, reference, context); }
    if reference.provider != "chzzk" {
        let held = platform.upload_source(&reference.provider, &reference.recording_id)?;
        source_bounds(held.item.duration, 0.0, held.item.bytes)?;
        return Ok(CloudSource {
            file: held.file, source: reference.clone(), title: held.item.title,
            duration_seconds: held.item.duration, media_offset_seconds: 0.0, lease: Box::new(held.lease),
        });
    }
    let (store, mut held) = chzzk_source_cancellable(browser, reference, Some(&context.cancel))?;
    let (file, duration, offset) = if held.parts.is_empty() {
        if reference.part_index.is_some() { return Err("이 녹화에는 선택한 구간이 없습니다.".into()); }
        (pin_file(&held.media)?, held.duration, 0.0)
    } else {
        let index = reference.part_index.ok_or("여러 구간으로 저장된 녹화입니다. 업로드할 구간을 선택해 주세요.")? as usize;
        if index >= held.parts.len() { return Err("선택한 녹화 구간이 없습니다.".into()); }
        let (file, part) = held.parts.swap_remove(index);
        (pin_file(&file)?, part.duration, part.offset)
    };
    source_bounds(duration, offset, file.metadata().map_err(|_| "영상을 확인하지 못했습니다.")?.len())?;
    // Only an explicitly queued valid source opts into durable retention. A
    // description is read-only. Pausing, restarting, or a remote 'ready' response
    // must not allow existing automatic cleanup to retire the local evidence
    // before the user has actually verified remote playback/chat synchronization.
    store.preserve_recording_sources(&reference.recording_id).map_err(|error| error.message)?;
    let lease: Arc<()> = held.storage_lease.ok_or("녹화 파일 보호 상태를 확인하지 못했습니다.")?;
    Ok(CloudSource { file, source: reference.clone(), title: held.recording.title,
        duration_seconds: duration, media_offset_seconds: offset, lease: Box::new(lease) })
}

/// Canonical path of the already held native file, never of a renderer argument.
fn held_path(file: &File) -> Result<PathBuf, String> {
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED}};
        let mut text = vec![0u16; 32768];
        let n = unsafe { GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut text, FILE_NAME_NORMALIZED) } as usize;
        if n == 0 || n >= text.len() { return Err("영상의 보관 위치를 확인하지 못했습니다.".into()); }
        Ok(PathBuf::from(String::from_utf16(&text[..n]).map_err(|_| "영상 경로가 올바르지 않습니다.")?))
    }
    #[cfg(not(windows))] {
        use std::os::fd::AsRawFd;
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|_| "영상 경로를 확인하지 못했습니다.".into())
    }
}

/// Reopen the exact owned handle, not a caller-provided/re-resolved path. On Windows
/// the held handle denies write/delete sharing until the upload worker drops it.
pub(crate) fn pin_file(file: &File) -> Result<File, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use windows::Win32::{Foundation::{HANDLE, GENERIC_READ}, Storage::FileSystem::{
            ReOpenFile, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ,
        }};
        let handle = unsafe { ReOpenFile(HANDLE(file.as_raw_handle()), GENERIC_READ.0, FILE_SHARE_READ, FILE_FLAGS_AND_ATTRIBUTES(0)) }
            .map_err(|_| "사용 중이거나 변경 중인 영상입니다. 잠시 후 다시 시도해 주세요.")?;
        let pinned = unsafe { File::from_raw_handle(handle.0) };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(pinned.as_raw_handle()), &mut info) }
            .map_err(|_| "영상 파일을 확인하지 못했습니다.")?;
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & 0x400 != 0 {
            return Err("연결된 파일은 업로드할 수 없습니다.".into());
        }
        if !pinned.metadata().map_err(|_| "영상을 확인하지 못했습니다.")?.is_file() {
            return Err("일반 영상 파일이 아닙니다.".into());
        }
        Ok(pinned)
    }
    #[cfg(not(windows))]
    {
        if !file.metadata().map_err(|_| "영상을 확인하지 못했습니다.")?.is_file() {
            return Err("일반 영상 파일이 아닙니다.".into());
        }
        file.try_clone().map_err(|_| "영상 파일을 열지 못했습니다.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_are_catalog_ids_and_platform_scoped() {
        let mut source = CloudSourceRef { provider: "chzzk".into(), recording_id: "a".repeat(32), part_index: None, plan_id: None, unit_id: None };
        assert!(validate_reference(&source).is_ok());
        source.recording_id = "../media.mp4".into();
        assert!(validate_reference(&source).is_err());
        source.recording_id = uuid::Uuid::new_v4().to_string();
        source.provider = "youtube".into();
        assert!(validate_reference(&source).is_ok());
        source.part_index = Some(0);
        assert!(validate_reference(&source).is_err());
        source.part_index = None;
        source.provider = "pixiv".into();
        assert!(validate_reference(&source).is_err());
    }
    #[test]
    fn part_clock_bounds_reject_nonfinite_or_empty_sources() {
        assert!(source_bounds(20.0, 60.0, 123).is_ok());
        for (duration, offset, bytes) in [(f64::NAN, 0.0, 1), (1.0, -1.0, 1), (1.0, f64::INFINITY, 1), (0.0, 0.0, 1), (1.0, 0.0, 0)] {
            assert!(source_bounds(duration, offset, bytes).is_err());
        }
    }
    #[test]
    fn pinned_handle_keeps_readable_identity() {
        use std::io::Read;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("media.mp4");
        std::fs::write(&path, b"source").unwrap();
        let file = File::open(&path).unwrap();
        let mut pinned = pin_file(&file).unwrap();
        let mut bytes = Vec::new(); pinned.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"source");
        #[cfg(windows)] {
            assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
            assert!(std::fs::remove_file(&path).is_err());
        }
    }
}

