//! Main-only archive commands and a closed, offline media protocol. HTML is not
//! served: the archive UI renders typed text, and only verified local media URLs.
use super::{
    archive::{self, ArchiveDraft, ArchiveFile, ArchiveRecord, ArchiveSummary, SaveRequest},
    surface, trusted, XSession,
};
use crate::interface::AppState;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::http::{Request, Response};
use tauri::{AppHandle, Emitter, Manager, State, Webview};
const ERROR: &str = "X 보관함을 열지 못했습니다.";
const CHUNK: u64 = 4 * 1024 * 1024;
const WHOLE: u64 = 32 * 1024 * 1024;
static IO_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static PENDING_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);
const IO_WAIT: Duration = Duration::from_secs(15);

struct IoAdmission<'a> {
    _pending: tokio::sync::SemaphorePermit<'a>,
    _io: tokio::sync::SemaphorePermit<'a>,
}

async fn admit_io<'a>(
    pending: &'a tokio::sync::Semaphore,
    io: &'a tokio::sync::Semaphore,
    wait: Duration,
) -> Result<IoAdmission<'a>, ()> {
    let pending = pending.try_acquire().map_err(|_| ())?;
    let io = tokio::time::timeout(wait, io.acquire())
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    Ok(IoAdmission {
        _pending: pending,
        _io: io,
    })
}
fn root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| ERROR)?
        .join("x-archives"))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn x_archive_inspect(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, XSession>,
    pane: String,
    generation: u64,
) -> Result<ArchiveDraft, String> {
    trusted(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    if !matches!(pane.as_str(), "feed" | "preview") {
        return Err(ERROR.into());
    }
    let app = window.app_handle().clone();
    let session = session.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        surface::inspect_archive(&session, &app, generation, &pane)
    })
    .await
    .map_err(|_| ERROR.to_string())?
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_archive_release(
    window: Webview,
    session: State<'_, XSession>,
    capture_id: String,
    account_id: String,
) -> Result<bool, String> {
    trusted(&window)?;
    session.archive.release_draft(&capture_id, &account_id)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_archive_save(
    window: Webview,
    state: State<'_, AppState>,
    session: State<'_, XSession>,
    request: SaveRequest,
) -> Result<ArchiveRecord, String> {
    trusted(&window)?;
    let path = root(window.app_handle())?;
    let store = session.archive.clone();
    // Join the same bounded FIFO as the native media buttons. The chooser's
    // exact frozen selection is owned before waiting, never recaptured later.
    let ticket = state.begin_platform_work(|| {
        let pending = store.reserve_queued(request.request_id.clone())?;
        store.prepare_queued(pending, request)
    })?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || store.save_reserved_with_progress(&path, ticket, move |progress| {
        // Main-only, immutable request/capture/account scope. The local chooser
        // also fences its generation and sequence before displaying a sample.
        if trusted(&window).is_ok() { let _ = app.emit_to("main", "x:archive-progress", progress); }
    }))
        .await
        .map_err(|_| ERROR.to_string())?
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_archive_cancel(
    window: Webview,
    session: State<'_, XSession>,
    request_id: String,
) -> Result<bool, String> {
    trusted(&window)?;
    if uuid::Uuid::parse_str(&request_id).is_err() {
        return Err(ERROR.into());
    }
    Ok(session.archive.cancel(&request_id))
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_archive_list(window: Webview) -> Result<Vec<ArchiveSummary>, String> {
    trusted(&window)?;
    let path = root(window.app_handle())?;
    tauri::async_runtime::spawn_blocking(move || archive::list(&path))
        .await
        .map_err(|_| ERROR.to_string())?
}
#[tauri::command(rename_all = "camelCase")]
pub async fn x_archive_read(window: Webview, archive_id: String) -> Result<ArchiveRecord, String> {
    trusted(&window)?;
    let path = root(window.app_handle())?;
    tauri::async_runtime::spawn_blocking(move || archive::read(&path, &archive_id))
        .await
        .map_err(|_| ERROR.to_string())?
}

fn source_request_valid(archive_id: &str, post_id: &str) -> bool {
    uuid::Uuid::parse_str(archive_id).is_ok_and(|id| id.to_string() == archive_id)
        && !post_id.is_empty()
        && post_id.len() <= 24
        && post_id.bytes().all(|b| b.is_ascii_digit())
}

/// Use only the exact source stored in a verified immutable capture. A local
/// archive must not depend on whichever posts happen to be in the live feed.
fn stored_post_source(records: &[archive::CapturePost], post_id: &str) -> Result<String, String> {
    let mut matching = records.iter().filter(|post| post.id == post_id);
    let post = matching.next().ok_or(ERROR)?;
    if matching.next().is_some() {
        return Err(ERROR.into());
    }
    let (handle, id) = post
        .url
        .strip_prefix("https://x.com/")
        .and_then(|path| path.split_once("/status/"))
        .ok_or(ERROR)?;
    if handle.is_empty()
        || handle.len() > 15
        || !handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || id != post_id
        || id.is_empty()
        || id.len() > 24
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(ERROR.into());
    }
    Ok(post.url.clone())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn x_archive_open_source(
    window: Webview,
    archive_id: String,
    post_id: String,
) -> Result<(), String> {
    trusted(&window)?;
    if !source_request_valid(&archive_id, &post_id) {
        return Err(ERROR.into());
    }
    let path = root(window.app_handle())?;
    let _admission = admit_io(&PENDING_SLOTS, &IO_SLOTS, IO_WAIT)
        .await
        .map_err(|_| ERROR)?;
    let source = tauri::async_runtime::spawn_blocking(move || {
        let record = archive::read(&path, &archive_id)?;
        stored_post_source(&record.document.records, &post_id)
    })
    .await
    .map_err(|_| ERROR.to_string())??;
    // A main webview may have navigated while disk integrity was checked.
    trusted(&window)?;
    #[cfg(windows)]
    {
        use windows::{
            core::{w, PCWSTR},
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let wide = source.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        if unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        }
        .0 as isize
            <= 32
        {
            return Err("외부 브라우저를 열지 못했습니다.".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = source;
        Err("이 환경에서는 외부 브라우저 열기를 지원하지 않습니다.".into())
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn x_archive_open_folder(window: Webview, archive_id: String) -> Result<(), String> {
    trusted(&window)?;
    let path = root(window.app_handle())?;
    tauri::async_runtime::spawn_blocking(move || {
        let folder = archive::verified_folder(&path, &archive_id)?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                core::{w, PCWSTR},
                Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            };
            let wide = folder
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if unsafe {
                ShellExecuteW(
                    None,
                    w!("open"),
                    PCWSTR(wide.as_ptr()),
                    None,
                    None,
                    SW_SHOWNORMAL,
                )
            }
            .0 as isize
                <= 32
            {
                return Err("보관 폴더를 열지 못했습니다.".into());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = folder;
            Err("이 환경에서는 폴더 열기를 지원하지 않습니다.".into())
        }
    })
    .await
    .map_err(|_| ERROR.to_string())?
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: u64,
    end: u64,
    partial: bool,
}
fn decimal(value: &str) -> Result<u64, ()> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    value.parse().map_err(|_| ())
}
fn span(header: Option<&str>, length: u64, whole: bool) -> Result<Span, ()> {
    if length == 0 {
        return Err(());
    }
    let Some(header) = header else {
        return Ok(Span {
            start: 0,
            end: if whole {
                length - 1
            } else {
                length.min(CHUNK) - 1
            },
            partial: !whole && length > CHUNK,
        });
    };
    if header.len() > 80 || header.contains(',') {
        return Err(());
    }
    let (start, end) = header
        .strip_prefix("bytes=")
        .and_then(|s| s.split_once('-'))
        .ok_or(())?;
    let (start, end) = if start.is_empty() {
        let suffix = decimal(end)?;
        if suffix == 0 || suffix > CHUNK {
            return Err(());
        }
        let start = length.saturating_sub(suffix);
        (start, length - 1)
    } else {
        let start = decimal(start)?;
        let end = if end.is_empty() {
            length
                .saturating_sub(1)
                .min(start.saturating_add(CHUNK - 1))
        } else {
            let end = decimal(end)?;
            if end
                .checked_sub(start)
                .and_then(|n| n.checked_add(1))
                .is_none_or(|n| n > CHUNK)
            {
                return Err(());
            }
            end
        };
        (start, end)
    };
    if start >= length || end < start {
        return Err(());
    }
    Ok(Span {
        start,
        end: end.min(length - 1),
        partial: true,
    })
}
fn media_path(path: &str) -> Option<(&str, &str)> {
    let (id, name) = path.strip_prefix('/')?.split_once('/')?;
    if uuid::Uuid::parse_str(id).ok()?.to_string() != id
        || name.contains('/')
        || name.contains('%')
        || name.contains("..")
        || !name.starts_with("asset-")
        || name.len() > 100
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
    {
        return None;
    }
    Some((id, name))
}
fn allowed_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg"
            | "image/png"
            | "image/webp"
            | "image/gif"
            | "video/mp4"
            | "video/mp2t"
            | "audio/aac"
            | "text/vtt"
            | "application/vnd.apple.mpegurl"
    )
}
struct Cached {
    file: Arc<Mutex<ArchiveFile>>,
    touched: Instant,
}
type CacheKey = (PathBuf, String, String);
fn cache() -> &'static Mutex<HashMap<CacheKey, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Cached>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn cached_file(
    root: &std::path::Path,
    id: &str,
    name: &str,
) -> Result<Arc<Mutex<ArchiveFile>>, String> {
    let key = (root.to_path_buf(), id.into(), name.into());
    {
        let mut entries = cache().lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|_, v| v.touched.elapsed() < Duration::from_secs(120));
        if let Some(value) = entries.get_mut(&key) {
            value.touched = Instant::now();
            return Ok(value.file.clone());
        }
    }
    // Verify all bytes only once per bounded immutable open handle; Windows
    // denies writers/deletion while it is open. Range seeks then stay O(chunk).
    let file = Arc::new(Mutex::new(archive::open_asset(root, id, name)?));
    let mut entries = cache().lock().unwrap_or_else(|p| p.into_inner());
    if entries.len() >= 16 {
        if let Some(old) = entries
            .iter()
            .min_by_key(|(_, v)| v.touched)
            .map(|(k, _)| k.clone())
        {
            entries.remove(&old);
        }
    }
    entries.insert(
        key,
        Cached {
            file: file.clone(),
            touched: Instant::now(),
        },
    );
    Ok(file)
}
pub fn clear_cache() {
    cache().lock().unwrap_or_else(|p| p.into_inner()).clear();
}
fn empty(status: u16) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(vec![])
        .expect("static archive response")
}
/// Queue ordinary simultaneous image/HLS requests without creating a blocking
/// worker per waiter. Both active and waiting requests count toward the cap.
pub async fn respond_async(
    app: AppHandle,
    label: String,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    if label != "main"
        || !matches!(request.method().as_str(), "GET" | "HEAD")
        || !request.body().is_empty()
        || request.uri().query().is_some()
        || media_path(request.uri().path()).is_none()
    {
        return empty(403);
    }
    let Ok(admission) = admit_io(&PENDING_SLOTS, &IO_SLOTS, IO_WAIT).await else {
        return empty(503);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _admission = admission;
        // Recheck the current main origin after waiting, not just at admission.
        respond_inner(&app, &label, request)
    })
    .await
    .unwrap_or_else(|_| empty(500))
}

fn respond_inner(app: &AppHandle, label: &str, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if label != "main"
        || !matches!(request.method().as_str(), "GET" | "HEAD")
        || !request.body().is_empty()
        || request.uri().query().is_some()
    {
        return empty(403);
    }
    let Some(main) = app.get_webview("main") else {
        return empty(403);
    };
    if trusted(&main).is_err() {
        return empty(403);
    }
    let origin = match request.headers().get("origin") {
        None => None,
        Some(value) => match value.to_str() {
            Ok(value)
                if crate::frontend_origin::trusted_origin(
                    value,
                    cfg!(debug_assertions),
                    app.config().build.dev_url.as_ref(),
                ) =>
            {
                Some(value.to_string())
            }
            _ => return empty(403),
        },
    };
    let Some((id, name)) = media_path(request.uri().path()) else {
        return empty(403);
    };
    let Ok(root) = root(app) else {
        return empty(404);
    };
    let Ok(handle) = cached_file(&root, id, name) else {
        return empty(404);
    };
    let mut file = handle.lock().unwrap_or_else(|p| p.into_inner());
    if !allowed_mime(&file.mime) {
        return empty(403);
    }
    let mut ranges = request.headers().get_all("range").iter();
    let range = match ranges.next() {
        Some(v) => match v.to_str() {
            Ok(v) => Some(v),
            Err(_) => return empty(416),
        },
        None => None,
    };
    if ranges.next().is_some() {
        return empty(416);
    }
    // HLS segments and image decoders need a complete ordinary response. MP4
    // root videos use bounded ranges; m4s/init fragments are complete <=32MiB.
    let whole = file.mime.starts_with("image/")
        || matches!(
            file.kind.as_str(),
            "video_segment" | "video_playlist" | "video_hls"
        )
        || file.bytes <= CHUNK;
    if whole && file.bytes > WHOLE {
        return empty(413);
    }
    let head = request.method() == "HEAD";
    let selected = if head && range.is_none() {
        Ok(Span {
            start: 0,
            end: file.bytes.saturating_sub(1),
            partial: false,
        })
    } else {
        span(range, file.bytes, whole)
    };
    let Ok(selected) = selected else {
        return Response::builder()
            .status(416)
            .header("Content-Range", format!("bytes */{}", file.bytes))
            .header("Cache-Control", "no-store")
            .body(vec![])
            .expect("static range error");
    };
    let count = selected.end - selected.start + 1;
    let bytes = if head {
        vec![]
    } else if count <= CHUNK {
        match file.read_range(selected.start, count as usize) {
            Ok(bytes) => bytes,
            Err(_) => return empty(500),
        }
    } else {
        let mut bytes = Vec::with_capacity(count as usize);
        let mut at = selected.start;
        while at <= selected.end {
            let size = (selected.end - at + 1).min(CHUNK) as usize;
            match file.read_range(at, size) {
                Ok(part) => bytes.extend_from_slice(&part),
                Err(_) => return empty(500),
            }
            at += size as u64;
        }
        bytes
    };
    let mut response = Response::builder()
        .status(if selected.partial { 206 } else { 200 })
        .header("Content-Type", &file.mime)
        .header("Content-Length", count.to_string())
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "sandbox; default-src 'none'");
    if selected.partial {
        response = response.header(
            "Content-Range",
            format!("bytes {}-{}/{}", selected.start, selected.end, file.bytes),
        );
    }
    if let Some(origin) = origin {
        response = response
            .header("Access-Control-Allow-Origin", origin)
            .header(
                "Access-Control-Expose-Headers",
                "Content-Length, Content-Range, Accept-Ranges",
            )
            .header("Vary", "Origin");
    }
    response.body(bytes).unwrap_or_else(|_| empty(500))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn source_post(id: &str, url: &str) -> archive::CapturePost {
        archive::CapturePost {
            id: id.into(),
            url: url.into(),
            author: String::new(),
            body: String::new(),
            timestamp: String::new(),
            counts: String::new(),
            context: String::new(),
            quotes: vec![],
            media: vec![],
            truncated: false,
            photo_only: false,
            presentation: None,
        }
    }

    #[test]
    fn source_open_requires_canonical_bounded_identifiers() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(source_request_valid(&id, "123456789"));
        for bad in [
            "",
            "../record",
            "{00000000-0000-0000-0000-000000000000}",
            "not-a-uuid",
        ] {
            assert!(!source_request_valid(bad, "123"));
        }
        for bad in [
            "",
            "1/2",
            "１２３",
            "-123",
            "123?x=1",
            "1234567890123456789012345",
        ] {
            assert!(!source_request_valid(&id, bad));
        }
    }

    #[test]
    fn source_open_uses_exact_unique_saved_post_without_live_cache() {
        let post = source_post("123", "https://x.com/Test_1/status/123");
        assert_eq!(
            stored_post_source(&[post.clone()], "123").unwrap(),
            post.url
        );
        assert!(stored_post_source(&[post.clone()], "456").is_err());
        assert!(stored_post_source(&[post.clone(), post], "123").is_err());
    }

    #[test]
    fn source_open_rejects_arbitrary_or_ambiguous_destinations() {
        for url in [
            "http://x.com/test/status/123",
            "https://x.com.evil.test/test/status/123",
            "https://test@x.com/test/status/123",
            "https://x.com:443/test/status/123",
            "https://x.com/test/status/456",
            "https://x.com/test/status/123?token=private",
            "https://x.com/test/status/123#reply",
            "https://x.com/test/status/123/photo/1",
            "https://x.com/test/status/123/",
            "https://x.com/%74est/status/123",
            "https://x.com/../test/status/123",
            "https://x.com//status/123",
            "https://x.com/abcdefghijklmnop/status/123",
            "javascript:alert(1)",
            "file:///C:/private",
        ] {
            assert!(
                stored_post_source(&[source_post("123", url)], "123").is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn third_request_waits_and_bounded_queue_recovers_every_permit() {
        tauri::async_runtime::block_on(async {
            let pending = tokio::sync::Semaphore::new(3);
            let io = tokio::sync::Semaphore::new(2);
            let first = admit_io(&pending, &io, IO_WAIT).await.unwrap();
            let second = admit_io(&pending, &io, IO_WAIT).await.unwrap();
            let mut third = Box::pin(admit_io(&pending, &io, IO_WAIT));
            assert!(
                tokio::time::timeout(Duration::from_millis(5), third.as_mut())
                    .await
                    .is_err()
            );
            assert_eq!(pending.available_permits(), 0);
            assert_eq!(io.available_permits(), 0);
            assert!(admit_io(&pending, &io, IO_WAIT).await.is_err());
            drop(first);
            let third = third.await.unwrap();
            assert_eq!(io.available_permits(), 0);
            drop(second);
            drop(third);
            assert_eq!(pending.available_permits(), 3);
            assert_eq!(io.available_permits(), 2);
        });
    }

    #[test]
    fn waiting_timeout_and_cancellation_release_pending_admission() {
        tauri::async_runtime::block_on(async {
            let pending = tokio::sync::Semaphore::new(3);
            let io = tokio::sync::Semaphore::new(1);
            let active = admit_io(&pending, &io, IO_WAIT).await.unwrap();
            assert!(admit_io(&pending, &io, Duration::from_millis(5))
                .await
                .is_err());
            assert_eq!(pending.available_permits(), 2);
            let mut waiting = Box::pin(admit_io(&pending, &io, IO_WAIT));
            assert!(
                tokio::time::timeout(Duration::from_millis(5), waiting.as_mut())
                    .await
                    .is_err()
            );
            assert_eq!(pending.available_permits(), 1);
            drop(waiting);
            assert_eq!(pending.available_permits(), 2);
            drop(active);
            assert_eq!(pending.available_permits(), 3);
            assert_eq!(io.available_permits(), 1);
        });
    }

    #[test]
    fn closed_paths_never_expose_html_or_other_files() {
        let id = uuid::Uuid::new_v4().to_string();
        assert!(media_path(&format!("/{id}/asset-abc.mp4")).is_some());
        for tail in [
            "index.html",
            "archive.json",
            "../secret",
            "asset-../secret",
            "asset-%2e%2e.mp4",
            "asset-a/x.mp4",
        ] {
            assert!(media_path(&format!("/{id}/{tail}")).is_none(), "{tail}");
        }
    }
    #[test]
    fn byte_ranges_are_bounded_and_suffix_seeks_are_correct() {
        let length = 10 * CHUNK;
        assert_eq!(
            span(Some("bytes=10-"), length, false).unwrap(),
            Span {
                start: 10,
                end: 10 + CHUNK - 1,
                partial: true
            }
        );
        assert_eq!(
            span(Some("bytes=-100"), length, false).unwrap(),
            Span {
                start: length - 100,
                end: length - 1,
                partial: true
            }
        );
        assert_eq!(
            span(None, 100, true).unwrap(),
            Span {
                start: 0,
                end: 99,
                partial: false
            }
        );
        for range in [
            "bytes=0-1,2-3",
            "bytes=-0",
            "bytes=9-8",
            "bytes=+1-2",
            "bytes=99999999999999999999-",
        ] {
            assert!(span(Some(range), 100, false).is_err());
        }
        assert!(span(Some("bytes=100-"), 100, false).is_err());
        assert!(span(Some("bytes=-8388608"), length, false).is_err());
        assert!(span(Some("bytes=0-8388607"), length, false).is_err());
        assert_eq!(
            span(Some("bytes=10-19"), length, false).unwrap(),
            Span {
                start: 10,
                end: 19,
                partial: true
            }
        );
    }
    #[test]
    fn only_inert_media_types_are_served() {
        for mime in [
            "text/html",
            "application/javascript",
            "image/svg+xml",
            "text/xml",
        ] {
            assert!(!allowed_mime(mime));
        }
        assert!(allowed_mime("video/mp4"));
        assert!(allowed_mime("text/vtt"));
        assert!(allowed_mime("application/vnd.apple.mpegurl"));
    }
}
