//! Trusted-window screenshot catalog. Remote pages never supply filesystem paths.
use super::super::{host, open_local_record_file, require_main, unavailable};
use super::{
    child_directory, directory, error, invalid, regular, storage, SavedScreenshot, StreamError,
    MAX_BYTES, MAX_SIDE,
};
use crate::interface::{ApiResult, AppState};
use crate::streaming::browser_store::atomic_write;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, Webview};

static FILE_GATE: Mutex<()> = Mutex::new(());
static DECODE_GATE: Mutex<()> = Mutex::new(());
const REGISTRY: &str = "chzzk-screenshot-roots.json";
const MAX_ROOTS: usize = 64;

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read_small(path: &Path, limit: u64) -> Result<Vec<u8>, StreamError> {
    let meta = fs::symlink_metadata(path).map_err(|_| storage())?;
    if !meta.is_file() || !regular(&meta) || meta.len() > limit {
        return Err(storage());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| storage())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| storage())?;
    if bytes.len() as u64 > limit {
        return Err(storage());
    }
    Ok(bytes)
}
fn registered(data: &Path) -> Result<Vec<PathBuf>, StreamError> {
    let path = directory(data)?.join(REGISTRY);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(_) => Err(storage()),
        Ok(_) => {
            let roots: Vec<PathBuf> =
                serde_json::from_slice(&read_small(&path, 65536)?).map_err(|_| storage())?;
            if roots.len() > MAX_ROOTS {
                return Err(storage());
            }
            Ok(roots)
        }
    }
}
pub fn register_root(data: &Path, download: &Path) -> Result<PathBuf, StreamError> {
    let _gate = FILE_GATE.lock().map_err(|_| storage())?;
    let root = directory(download)?;
    let root = child_directory(&child_directory(&root, "CHZZK")?, "Screenshots")?;
    let mut roots = registered(data)?;
    if !roots.contains(&root) {
        if roots.len() >= MAX_ROOTS {
            return Err(storage());
        }
        roots.push(root.clone());
        atomic_write(
            data,
            REGISTRY,
            &serde_json::to_vec(&roots).map_err(|_| storage())?,
        )?;
    }
    Ok(root)
}
fn roots(data: &Path, download: &Path) -> Result<Vec<PathBuf>, StreamError> {
    let mut roots = registered(data)?;
    if let Ok(current) = directory(&download.join("CHZZK/Screenshots")) {
        if !roots.contains(&current) {
            roots.push(current);
        }
    }
    // Disconnected drives remain registered for their next mount, but are not read.
    Ok(roots
        .into_iter()
        .filter_map(|path| directory(&path).ok())
        .collect())
}
pub fn save_metadata(root: &Path, item: &SavedScreenshot) -> Result<(), StreamError> {
    if !valid_id(&item.id) {
        return Err(invalid());
    }
    atomic_write(
        root,
        &format!("{}.json", item.id),
        &serde_json::to_vec(item).map_err(|_| storage())?,
    )
}
pub fn remove_metadata(root: &Path, id: &str) -> Result<(), StreamError> {
    if !valid_id(id) {
        return Err(invalid());
    }
    let path = directory(root)?.join(format!("{id}.json"));
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.is_file() && regular(&meta) => {
            fs::remove_file(path).map_err(|_| storage())
        }
        _ => Err(storage()),
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotItem {
    #[serde(flatten)]
    pub saved: SavedScreenshot,
    bytes: u64,
    width: u32,
    height: u32,
}
fn inspect(path: &Path, id: &str) -> Result<ScreenshotItem, StreamError> {
    let meta = fs::symlink_metadata(path).map_err(|_| storage())?;
    if !meta.is_file() || !regular(&meta) || meta.len() > MAX_BYTES as u64 {
        return Err(invalid());
    }
    let mut header = [0u8; 24];
    fs::File::open(path)
        .map_err(|_| storage())?
        .read_exact(&mut header)
        .map_err(|_| invalid())?;
    let width = u32::from_be_bytes(header[16..20].try_into().map_err(|_| invalid())?);
    let height = u32::from_be_bytes(header[20..24].try_into().map_err(|_| invalid())?);
    if &header[..8] != b"\x89PNG\r\n\x1a\n"
        || &header[12..16] != b"IHDR"
        || !super::dimensions(width, height)
    {
        return Err(invalid());
    }
    let saved = read_small(&path.with_extension("json"), 4096)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SavedScreenshot>(&bytes).ok())
        .filter(|saved| {
            saved.id == id
                && saved.file_name == format!("{id}.png")
                && saved.channel_id.len() == 32
                && saved.channel_id.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| SavedScreenshot {
            id: id.into(),
            channel_id: String::new(),
            file_name: format!("{id}.png"),
            created_at: meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_millis() as u64)
                .unwrap_or(0),
        });
    Ok(ScreenshotItem {
        saved,
        bytes: meta.len(),
        width,
        height,
    })
}
fn list(data: &Path, download: &Path) -> Result<Vec<ScreenshotItem>, StreamError> {
    // Remember legacy captures on first browse, before a future folder change.
    if directory(&download.join("CHZZK/Screenshots")).is_ok() {
        register_root(data, download)?;
    }
    let directories = {
        let _gate = FILE_GATE.lock().map_err(|_| storage())?;
        roots(data, download)?
    };
    let mut items = Vec::new();
    // A large catalog scan must not hold the capture/registry write gate.
    for root in directories {
        for entry in fs::read_dir(root).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_suffix(".png"))
                .filter(|id| valid_id(id))
            else {
                continue;
            };
            if let Ok(item) = inspect(&entry.path(), id) {
                items.push(item);
            }
        }
    }
    items.sort_by(|a, b| {
        b.saved
            .created_at
            .cmp(&a.saved.created_at)
            .then_with(|| a.saved.id.cmp(&b.saved.id))
    });
    // A duplicate ID in two locations is ambiguous; never choose one for deletion.
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.saved.id.clone()));
    Ok(items)
}
fn resolve(data: &Path, download: &Path, id: &str) -> Result<PathBuf, StreamError> {
    if !valid_id(id) {
        return Err(invalid());
    }
    let mut found = None;
    for root in roots(data, download)? {
        let path = root.join(format!("{id}.png"));
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.is_file() || !regular(&meta) || found.is_some() {
                return Err(storage());
            }
            found = Some(path);
        }
    }
    found.ok_or_else(|| {
        error(
            "SCREENSHOT_MISSING",
            "스크린샷 파일을 찾을 수 없습니다. 목록을 새로고침해 주세요.",
        )
    })
}
fn picture(data: &Path, download: &Path, id: &str, full: bool) -> Result<String, StreamError> {
    let _decode = DECODE_GATE.lock().map_err(|_| storage())?;
    let bytes = {
        let _gate = FILE_GATE.lock().map_err(|_| storage())?;
        read_small(&resolve(data, download, id)?, MAX_BYTES as u64)?
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| invalid())?;
    if full {
        return Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
    }
    let thumb = image.thumbnail(480, 300).to_rgb8();
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(thumb)
        .write_to(&mut output, image::ImageFormat::Jpeg)
        .map_err(|_| invalid())?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(output.into_inner())
    ))
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReport {
    deleted: Vec<String>,
    failures: Vec<DeleteFailure>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFailure {
    id: String,
    message: String,
}
fn delete(data: &Path, download: &Path, ids: &[String]) -> Result<DeleteReport, StreamError> {
    if ids.is_empty() || ids.len() > 500 || !ids.iter().all(|id| valid_id(id)) {
        return Err(invalid());
    }
    let _gate = FILE_GATE.lock().map_err(|_| storage())?;
    let mut report = DeleteReport::default();
    for id in ids {
        let result = (|| {
            let path = resolve(data, download, id)?;
            fs::remove_file(&path).map_err(|_| storage())?;
            // The image is already deleted; a stale sidecar cannot bring it back.
            if remove_metadata(path.parent().ok_or_else(storage)?, id).is_err() {
                tracing::warn!("screenshot sidecar cleanup failed");
            }
            Ok::<_, StreamError>(())
        })();
        match result {
            Ok(()) => report.deleted.push(id.clone()),
            Err(cause) => report.failures.push(DeleteFailure {
                id: id.clone(),
                message: cause.message,
            }),
        }
    }
    Ok(report)
}
fn context(app: &AppHandle, window: &Webview) -> Result<(PathBuf, PathBuf), StreamError> {
    require_main(window)?;
    Ok((
        host(app)?.inner.data_dir.clone(),
        PathBuf::from(
            app.state::<AppState>()
                .settings_snapshot()
                .map_err(|_| unavailable())?
                .download_root,
        ),
    ))
}
#[tauri::command]
pub async fn chzzk_screenshots_list(
    app: AppHandle,
    window: Webview,
) -> ApiResult<Vec<ScreenshotItem>> {
    let result = async {
        let (data, download) = context(&app, &window)?;
        tauri::async_runtime::spawn_blocking(move || list(&data, &download))
            .await
            .map_err(|_| unavailable())?
    }
    .await;
    result.into()
}
#[tauri::command]
pub async fn chzzk_screenshot_image(
    app: AppHandle,
    window: Webview,
    id: String,
    full: bool,
) -> ApiResult<String> {
    let result = async {
        let (data, download) = context(&app, &window)?;
        tauri::async_runtime::spawn_blocking(move || picture(&data, &download, &id, full))
            .await
            .map_err(|_| unavailable())?
    }
    .await;
    result.into()
}
#[tauri::command]
pub async fn chzzk_screenshots_delete(
    app: AppHandle,
    window: Webview,
    ids: Vec<String>,
) -> ApiResult<DeleteReport> {
    let result = async {
        let (data, download) = context(&app, &window)?;
        tauri::async_runtime::spawn_blocking(move || delete(&data, &download, &ids))
            .await
            .map_err(|_| unavailable())?
    }
    .await;
    result.into()
}
#[tauri::command]
pub async fn chzzk_screenshots_folder(
    app: AppHandle,
    window: Webview,
    id: Option<String>,
) -> ApiResult<()> {
    let result = async {
        let (data, download) = context(&app, &window)?;
        let path = tauri::async_runtime::spawn_blocking(move || match id {
            Some(id) => Ok(resolve(&data, &download, &id)?
                .parent()
                .ok_or_else(storage)?
                .to_path_buf()),
            None => register_root(&data, &download),
        })
        .await
        .map_err(|_| unavailable())??;
        open_local_record_file(path)
    }
    .await;
    result.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "0123456789abcdef0123456789abcdef";
    fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
        let data = tempfile::tempdir().unwrap();
        let download = tempfile::tempdir().unwrap();
        let root = register_root(data.path(), download.path()).unwrap();
        let image = image::DynamicImage::new_rgb8(4, 3);
        image.save(root.join(format!("{ID}.png"))).unwrap();
        (data, download, root)
    }
    #[test]
    fn legacy_photos_and_metadata_survive_restart_and_folder_change() {
        let (data, download, root) = setup();
        let legacy = list(data.path(), download.path()).unwrap();
        assert_eq!(legacy.len(), 1);
        assert!(legacy[0].saved.channel_id.is_empty());
        let saved = SavedScreenshot {
            id: ID.into(),
            channel_id: ID.into(),
            file_name: format!("{ID}.png"),
            created_at: 123,
        };
        save_metadata(&root, &saved).unwrap();
        let another = tempfile::tempdir().unwrap();
        register_root(data.path(), another.path()).unwrap();
        let items = list(data.path(), another.path()).unwrap();
        assert_eq!(items[0].saved.created_at, 123);
        assert_eq!(items[0].saved.channel_id, ID);
        assert!(picture(data.path(), another.path(), ID, false)
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        assert!(picture(data.path(), another.path(), ID, true)
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }
    #[test]
    fn delete_is_exact_and_does_not_remove_other_files() {
        let (data, download, root) = setup();
        fs::write(root.join("unrelated.png"), b"keep").unwrap();
        fs::write(root.join(format!("{ID}.json")), b"{}").unwrap();
        assert!(delete(data.path(), download.path(), &["../recording".into()]).is_err());
        let result = delete(data.path(), download.path(), &[ID.into()]).unwrap();
        assert_eq!(result.deleted, [ID]);
        assert!(result.failures.is_empty());
        assert!(!root.join(format!("{ID}.png")).exists());
        assert!(!root.join(format!("{ID}.json")).exists());
        assert!(root.join("unrelated.png").exists());
        assert_eq!(
            delete(data.path(), download.path(), &[ID.into()])
                .unwrap()
                .failures
                .len(),
            1
        );
    }
    #[test]
    fn ambiguous_ids_are_never_deleted() {
        let (data, download, root) = setup();
        let other = tempfile::tempdir().unwrap();
        let target = register_root(data.path(), other.path()).unwrap();
        fs::copy(
            root.join(format!("{ID}.png")),
            target.join(format!("{ID}.png")),
        )
        .unwrap();
        assert_eq!(
            delete(data.path(), download.path(), &[ID.into()])
                .unwrap()
                .failures
                .len(),
            1
        );
        assert!(root.join(format!("{ID}.png")).exists());
        assert!(target.join(format!("{ID}.png")).exists());
    }
}
