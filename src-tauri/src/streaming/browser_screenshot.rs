//! One approved, memory-bounded PNG transfer. Never accepts a remote file path.
use super::{error, now_ms, StreamError};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub const CHUNK: usize = 128 * 1024;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(20);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedScreenshot {
    pub id: String,
    pub channel_id: String,
    pub file_name: String,
    pub created_at: u64,
}
struct Transfer {
    size: usize,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}
struct Job {
    id: String,
    channel: String,
    generation: u64,
    root: PathBuf,
    created: Instant,
    transfer: Option<Transfer>,
}
#[derive(Default)]
pub struct ScreenshotCapture {
    job: Option<Job>,
    pub last: Option<SavedScreenshot>,
}
fn invalid() -> StreamError {
    error(
        "SCREENSHOT_INVALID",
        "스크린샷 데이터 또는 전송 순서가 올바르지 않습니다.",
    )
}
fn stale() -> StreamError {
    error(
        "SCREENSHOT_STALE",
        "스크린샷 승인이 만료됐거나 화면이 변경됐습니다. 다시 요청해 주세요.",
    )
}
fn storage() -> StreamError {
    error(
        "SCREENSHOT_STORAGE",
        "스크린샷을 저장하지 못했습니다. 다운로드 폴더와 남은 공간을 확인해 주세요.",
    )
}
fn dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && width as u64 * height as u64 <= MAX_PIXELS
}
impl ScreenshotCapture {
    pub fn expire(&mut self) {
        if self
            .job
            .as_ref()
            .is_some_and(|job| job.created.elapsed() > TTL)
        {
            self.job = None;
        }
    }
    pub fn cancel(&mut self) {
        self.job = None;
    }
    pub fn active(&mut self) -> bool {
        self.expire();
        self.job.is_some()
    }
    pub fn arm(
        &mut self,
        root: &Path,
        channel: &str,
        generation: u64,
    ) -> Result<String, StreamError> {
        self.expire();
        if self.job.is_some() {
            return Err(error(
                "SCREENSHOT_BUSY",
                "현재 스크린샷 저장이 끝난 뒤 다시 요청해 주세요.",
            ));
        }
        if channel.len() != 32 || !channel.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let root = directory(root)?;
        let id = uuid::Uuid::new_v4().to_string();
        self.job = Some(Job {
            id: id.clone(),
            channel: channel.into(),
            generation,
            root,
            created: Instant::now(),
            transfer: None,
        });
        Ok(id)
    }
    fn checked(
        &mut self,
        id: &str,
        channel: &str,
        generation: u64,
    ) -> Result<&mut Job, StreamError> {
        self.expire();
        self.job
            .as_mut()
            .filter(|job| job.id == id && job.channel == channel && job.generation == generation)
            .ok_or_else(stale)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Capture identity, page generation and dimensions must be checked together"
    )]
    pub fn begin(
        &mut self,
        id: &str,
        channel: &str,
        generation: u64,
        mime: &str,
        size: usize,
        width: u32,
        height: u32,
    ) -> Result<(), StreamError> {
        let job = self.checked(id, channel, generation)?;
        if mime != "image/png" || size == 0 || size > MAX_BYTES || !dimensions(width, height) {
            return Err(invalid());
        }
        if let Some(old) = &job.transfer {
            return if old.size == size && old.width == width && old.height == height {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        job.transfer = Some(Transfer {
            size,
            width,
            height,
            bytes: Vec::with_capacity(size),
        });
        Ok(())
    }
    pub fn append(
        &mut self,
        id: &str,
        channel: &str,
        generation: u64,
        index: u64,
        bytes: &[u8],
    ) -> Result<(), StreamError> {
        let transfer = self
            .checked(id, channel, generation)?
            .transfer
            .as_mut()
            .ok_or_else(invalid)?;
        let offset = usize::try_from(index)
            .ok()
            .and_then(|n| n.checked_mul(CHUNK))
            .filter(|offset| *offset < transfer.size)
            .ok_or_else(invalid)?;
        let expected = CHUNK.min(transfer.size - offset);
        if bytes.len() != expected {
            return Err(invalid());
        }
        if offset < transfer.bytes.len() {
            return if transfer.bytes.get(offset..offset + expected) == Some(bytes) {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if offset != transfer.bytes.len() {
            return Err(invalid());
        }
        transfer.bytes.extend_from_slice(bytes);
        Ok(())
    }
    pub fn abort(&mut self, id: &str) {
        if self.job.as_ref().is_some_and(|job| job.id == id) {
            self.job = None;
        }
    }
    #[cfg(test)]
    pub fn finish(
        &mut self,
        id: &str,
        channel: &str,
        generation: u64,
    ) -> Result<SavedScreenshot, StreamError> {
        self.finish_checked(id, channel, generation, || true)
    }
    pub fn finish_checked(
        &mut self,
        id: &str,
        channel: &str,
        generation: u64,
        current: impl Fn() -> bool,
    ) -> Result<SavedScreenshot, StreamError> {
        self.checked(id, channel, generation)?;
        // Consume approval before validation/storage, including failure paths.
        let job = self.job.take().ok_or_else(stale)?;
        let transfer = job.transfer.ok_or_else(invalid)?;
        if transfer.bytes.len() != transfer.size {
            return Err(invalid());
        }
        validate_png(&transfer.bytes, transfer.width, transfer.height)?;
        if job.created.elapsed() > TTL || !current() {
            return Err(stale());
        }
        let root = directory(&job.root)?;
        let root = child_directory(&child_directory(&root, "CHZZK")?, "Screenshots")?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let file_name = format!("{id}.png");
        let path = root.join(&file_name);
        if job.created.elapsed() > TTL || !current() {
            return Err(stale());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| storage())?;
        if file
            .write_all(&transfer.bytes)
            .and_then(|_| file.sync_all())
            .is_err()
        {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(storage());
        }
        let saved = SavedScreenshot {
            id,
            channel_id: channel.into(),
            file_name,
            created_at: now_ms(),
        };
        self.last = Some(saved.clone());
        Ok(saved)
    }
}
fn validate_png(bytes: &[u8], width: u32, height: u32) -> Result<(), StreamError> {
    if bytes.len() > MAX_BYTES
        || !dimensions(width, height)
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(invalid());
    }
    let mut offset = 8usize;
    let mut data = false;
    let mut ended = false;
    while offset < bytes.len() {
        let length = bytes
            .get(offset..offset + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
            .ok_or_else(invalid)? as usize;
        let end = offset
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .filter(|n| *n <= bytes.len())
            .ok_or_else(invalid)?;
        let kind = &bytes[offset + 4..offset + 8];
        if offset == 8 && (kind != b"IHDR" || length != 13) {
            return Err(invalid());
        }
        if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") || (offset != 8 && kind == b"IHDR") {
            return Err(invalid());
        }
        if kind == b"IDAT" {
            data = true;
        }
        if kind == b"IEND" {
            if length != 0 || end != bytes.len() || !data {
                return Err(invalid());
            }
            ended = true;
        }
        offset = end;
    }
    if !ended {
        return Err(invalid());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| invalid())?;
    if decoded.width() != width || decoded.height() != height {
        return Err(invalid());
    }
    Ok(())
}
fn regular(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}
fn directory(path: &Path) -> Result<PathBuf, StreamError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(storage());
    }
    for ancestor in path.ancestors() {
        let m = fs::symlink_metadata(ancestor).map_err(|_| storage())?;
        if !m.is_dir() || !regular(&m) {
            return Err(storage());
        }
    }
    fs::canonicalize(path).map_err(|_| storage())
}
fn child_directory(root: &Path, name: &str) -> Result<PathBuf, StreamError> {
    let path = root.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(storage()),
    }
    let path = directory(&path)?;
    if path.parent() != Some(root) {
        return Err(storage());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
    #[test]
    #[ignore = "Explicit local screenshot timing probe; synthetic image and temporary storage only"]
    fn screenshot_storage_timing_probe() {
        let root = tempfile::tempdir().unwrap();
        let mut random = 0x12345678u32;
        let image = image::RgbaImage::from_fn(1920, 1080, |_, _| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            image::Rgba([random as u8, (random >> 8) as u8, (random >> 16) as u8, 255])
        });
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let bytes = png.into_inner();
        let mut capture = ScreenshotCapture::default();
        let id = capture.arm(root.path(), CHANNEL, 1).unwrap();
        let before = Instant::now();
        capture
            .begin(&id, CHANNEL, 1, "image/png", bytes.len(), 1920, 1080)
            .unwrap();
        for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
            capture
                .append(&id, CHANNEL, 1, index as u64, chunk)
                .unwrap();
        }
        let copy_ms = before.elapsed().as_millis();
        let before = Instant::now();
        capture.finish(&id, CHANNEL, 1).unwrap();
        println!(
            "SCREENSHOT_STORAGE_OK bytes={} chunks={} copyMs={} validateAndSaveMs={}",
            bytes.len(),
            bytes.len().div_ceil(CHUNK),
            copy_ms,
            before.elapsed().as_millis()
        );
    }
    fn png() -> Vec<u8> {
        let mut out = Cursor::new(vec![]);
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }
    #[test]
    fn one_use_png_save_is_durable_and_remote_result_contains_no_path() {
        let root = tempfile::tempdir().unwrap();
        let mut capture = ScreenshotCapture::default();
        let id = capture.arm(root.path(), CHANNEL, 3).unwrap();
        let bytes = png();
        capture
            .begin(&id, CHANNEL, 3, "image/png", bytes.len(), 2, 2)
            .unwrap();
        capture.append(&id, CHANNEL, 3, 0, &bytes).unwrap();
        capture.append(&id, CHANNEL, 3, 0, &bytes).unwrap();
        let saved = capture.finish(&id, CHANNEL, 3).unwrap();
        assert_eq!(
            fs::read(root.path().join("CHZZK/Screenshots").join(&saved.file_name)).unwrap(),
            bytes
        );
        assert!(capture.finish(&id, CHANNEL, 3).is_err());
        let value = serde_json::to_string(&saved).unwrap();
        assert!(
            !value.contains("path") && !value.contains(&root.path().to_string_lossy().to_string())
        );
    }
    #[test]
    fn invalid_order_binding_and_conflicting_retries_do_not_write() {
        let root = tempfile::tempdir().unwrap();
        let mut c = ScreenshotCapture::default();
        let id = c.arm(root.path(), CHANNEL, 2).unwrap();
        let bytes = png();
        assert!(c.arm(root.path(), CHANNEL, 2).is_err());
        assert!(c
            .begin(&id, CHANNEL, 3, "image/png", bytes.len(), 2, 2)
            .is_err());
        c.begin(&id, CHANNEL, 2, "image/png", bytes.len(), 2, 2)
            .unwrap();
        assert!(c.append(&id, CHANNEL, 2, 1, &bytes).is_err());
        c.append(&id, CHANNEL, 2, 0, &bytes).unwrap();
        let mut changed = bytes;
        changed[0] = 0;
        assert!(c.append(&id, CHANNEL, 2, 0, &changed).is_err());
        c.abort("unrelated");
        assert!(c.active());
        c.abort(&id);
        assert!(!c.active());
        assert!(!root.path().join("CHZZK").exists());
    }
    #[test]
    fn png_validation_rejects_truncation_trailing_bytes_dimensions_and_non_png() {
        let bytes = png();
        assert!(validate_png(&bytes, 2, 2).is_ok());
        assert!(validate_png(&bytes, 3, 2).is_err());
        assert!(validate_png(&bytes[..bytes.len() - 1], 2, 2).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(validate_png(&trailing, 2, 2).is_err());
        assert!(validate_png(b"<svg/>", 2, 2).is_err());
        assert!(!dimensions(4097, 1));
        assert!(!dimensions(0, 1));
    }
    #[test]
    fn expiration_and_size_limits_release_without_creating_files() {
        let root = tempfile::tempdir().unwrap();
        let mut c = ScreenshotCapture::default();
        let id = c.arm(root.path(), CHANNEL, 1).unwrap();
        assert!(c
            .begin(&id, CHANNEL, 1, "image/png", MAX_BYTES + 1, 2, 2)
            .is_err());
        c.job.as_mut().unwrap().created = Instant::now() - Duration::from_secs(21);
        assert!(!c.active());
        assert!(c.begin(&id, CHANNEL, 1, "image/png", 1, 2, 2).is_err());
        assert!(fs::read_dir(root.path()).unwrap().next().is_none());
    }
    #[test]
    fn navigation_during_validation_cancels_before_file_creation() {
        let root = tempfile::tempdir().unwrap();
        let mut c = ScreenshotCapture::default();
        let id = c.arm(root.path(), CHANNEL, 1).unwrap();
        let bytes = png();
        c.begin(&id, CHANNEL, 1, "image/png", bytes.len(), 2, 2)
            .unwrap();
        c.append(&id, CHANNEL, 1, 0, &bytes).unwrap();
        assert!(c.finish_checked(&id, CHANNEL, 1, || false).is_err());
        assert!(!c.active());
        assert!(!root.path().join("CHZZK").exists());
    }
    #[test]
    fn path_traversal_and_non_directory_destination_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut c = ScreenshotCapture::default();
        assert!(c.arm(&root.path().join("../escape"), CHANNEL, 0).is_err());
        fs::write(root.path().join("CHZZK"), b"existing-user-file").unwrap();
        let id = c.arm(root.path(), CHANNEL, 0).unwrap();
        let bytes = png();
        c.begin(&id, CHANNEL, 0, "image/png", bytes.len(), 2, 2)
            .unwrap();
        c.append(&id, CHANNEL, 0, 0, &bytes).unwrap();
        assert!(c.finish(&id, CHANNEL, 0).is_err());
        assert_eq!(
            fs::read(root.path().join("CHZZK")).unwrap(),
            b"existing-user-file"
        );
    }
}
