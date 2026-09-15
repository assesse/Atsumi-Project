//! Best-effort, immutable local copies of public chat decorations. Playback never
//! fetches a URL. Failure to cache a badge must not interrupt chat/video storage.
use super::{
    chat_assets,
    model::{ChatRich, StreamError},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_ASSET: u64 = 1_500_000;
const MAX_DISK: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const QUEUE: usize = 128;

#[path = "replay_channel_profile.rs"]
pub mod channel_profile;
type ProfileFetcher =
    Arc<dyn Fn(&str) -> Result<(String, Option<String>), StreamError> + Send + Sync>;
enum AssetJob {
    Image(String, String, Option<PathBuf>),
    Channel(String, PathBuf),
}

#[derive(Serialize, Deserialize)]
struct StoredAsset {
    version: u8,
    mime: String,
    body: String,
    sha256: String,
}
pub struct CachedAsset {
    pub bytes: Vec<u8>,
    pub mime: String,
}

pub fn asset_id(url: &str) -> Option<String> {
    Some(format!(
        "{:x}",
        Sha256::digest(chat_assets::sanitize_chat_asset_url(url)?.as_bytes())
    ))
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn unavailable() -> StreamError {
    StreamError::new(
        "REPLAY_ASSET_UNAVAILABLE",
        "저장된 꾸밈 이미지를 사용할 수 없습니다.",
        false,
    )
}
fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("streaming").join("replay-assets")
}
fn plain_path(path: &Path) -> Result<(), StreamError> {
    if !path.is_absolute() {
        return Err(unavailable());
    }
    for ancestor in path.ancestors() {
        let Ok(metadata) = fs::symlink_metadata(ancestor) else {
            continue;
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(unavailable());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err(unavailable());
        }
    }
    Ok(())
}

pub fn read_cached(data_dir: &Path, id: &str) -> Result<CachedAsset, StreamError> {
    read_from(&directory(data_dir), id)
}
pub fn read_recording(root: &Path, id: &str) -> Result<CachedAsset, StreamError> {
    read_from(&root.join("replay-assets"), id)
}
fn read_from(root: &Path, id: &str) -> Result<CachedAsset, StreamError> {
    if !valid_id(id) {
        return Err(unavailable());
    }
    let path = root.join(format!("{id}.json"));
    plain_path(&path)?;
    let meta = fs::symlink_metadata(&path).map_err(|_| unavailable())?;
    if !meta.is_file() || meta.len() > MAX_ASSET {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| unavailable())?
        .take(MAX_ASSET + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > MAX_ASSET {
        return Err(unavailable());
    }
    let stored: StoredAsset = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if stored.version != 1 {
        return Err(unavailable());
    }
    let bytes = STANDARD.decode(&stored.body).map_err(|_| unavailable())?;
    if format!("{:x}", Sha256::digest(&bytes)) != stored.sha256 {
        return Err(unavailable());
    }
    // Recheck disk content, not just a trusted-looking extension/data URL. This
    // enforces raster dimensions and animation limits on hand-edited archives.
    let mime = chat_assets::validate_raster(&bytes, Some(&stored.mime))?;
    Ok(CachedAsset {
        bytes,
        mime: mime.into(),
    })
}

struct State {
    root: PathBuf,
    cancel: AtomicBool,
    seen: Mutex<HashSet<(String, Option<PathBuf>)>>,
}
#[derive(Clone)]
pub struct ReplayAssetCache {
    sender: Option<SyncSender<AssetJob>>,
    state: Arc<State>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}
type AssetFetcher = Arc<dyn Fn(&str) -> Result<String, StreamError> + Send + Sync>;

impl ReplayAssetCache {
    pub fn disabled() -> Self {
        Self {
            sender: None,
            state: Arc::new(State {
                root: PathBuf::new(),
                cancel: AtomicBool::new(true),
                seen: Mutex::new(HashSet::new()),
            }),
            handle: Arc::new(Mutex::new(None)),
        }
    }
    pub fn new(data_dir: &Path, enabled: bool) -> Result<Self, StreamError> {
        Self::with_fetch(
            data_dir,
            enabled,
            Arc::new(|url| chat_assets::fetch_chat_asset(url).map(|asset| asset.data_url)),
        )
    }
    fn with_fetch(
        data_dir: &Path,
        enabled: bool,
        fetch: AssetFetcher,
    ) -> Result<Self, StreamError> {
        Self::with_fetchers(
            data_dir,
            enabled,
            fetch,
            Arc::new(|channel| super::provider::ChzzkProvider::new()?.channel_profile(channel)),
        )
    }
    fn with_fetchers(
        data_dir: &Path,
        enabled: bool,
        fetch: AssetFetcher,
        profile: ProfileFetcher,
    ) -> Result<Self, StreamError> {
        if !enabled {
            return Ok(Self::disabled());
        }
        let root = directory(data_dir);
        plain_path(&root)?;
        fs::create_dir_all(&root).map_err(|_| unavailable())?;
        let mut used = 0u64;
        let mut count = 0usize;
        // Bounded metadata inventory only. No image decode/network on app start.
        for entry in fs::read_dir(&root)
            .map_err(|_| unavailable())?
            .take(MAX_FILES + QUEUE + 1)
        {
            let entry = entry.map_err(|_| unavailable())?;
            plain_path(&entry.path())?;
            let meta = entry.metadata().map_err(|_| unavailable())?;
            if !meta.is_file() {
                return Err(unavailable());
            }
            used = used.saturating_add(meta.len());
            count += 1;
            if used > MAX_DISK || count > MAX_FILES {
                return Err(unavailable());
            }
        }
        let state = Arc::new(State {
            root,
            cancel: AtomicBool::new(false),
            seen: Mutex::new(HashSet::new()),
        });
        let (sender, receiver) = mpsc::sync_channel::<AssetJob>(QUEUE);
        let run = state.clone();
        let handle = thread::Builder::new()
            .name("replay-assets".into())
            .spawn(move || {
                let mut recording_budgets = HashMap::new();
                while !run.cancel.load(Ordering::Acquire) {
                    let job = match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    let (id, url, recording) = match job {
                        AssetJob::Image(id, url, root) => (id, url, root),
                        AssetJob::Channel(channel, root) => {
                            // A single bounded worker, not one thread/request per UI
                            // update. Capture works even when chat saving is disabled.
                            if root.join("channel-profile.json").exists() {
                                continue;
                            }
                            let Ok((name, image)) = profile(&channel) else {
                                continue;
                            };
                            if run.cancel.load(Ordering::Acquire) {
                                break;
                            }
                            let id = image.as_deref().and_then(asset_id);
                            if channel_profile::save(&root, &channel, &name, id.as_deref()).is_err()
                            {
                                continue;
                            }
                            let (Some(id), Some(url)) = (id, image) else {
                                continue;
                            };
                            (id, url, Some(root))
                        }
                    };
                    let data = if run.root.join(format!("{id}.json")).exists() {
                        let Ok(asset) = read_from(&run.root, &id) else {
                            continue;
                        };
                        format!(
                            "data:{};base64,{}",
                            asset.mime,
                            STANDARD.encode(asset.bytes)
                        )
                    } else {
                        if count >= MAX_FILES || used.saturating_add(MAX_ASSET) > MAX_DISK {
                            continue;
                        }
                        let Ok(data) = fetch(&url) else {
                            continue;
                        };
                        if let Ok(size) = store_data(&run.root, &id, &data) {
                            used = used.saturating_add(size);
                            count += 1;
                        }
                        data
                    };
                    if run.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    if let Some(root) = recording {
                        let _ = mirror_recording(&root, &id, &data, &mut recording_budgets);
                    }
                }
            })
            .map_err(|_| unavailable())?;
        Ok(Self {
            sender: Some(sender),
            state,
            handle: Arc::new(Mutex::new(Some(handle))),
        })
    }
    /// Nonblocking: one bounded public image job per distinct URL. No account
    /// cookies or raw profiles enter this queue. Recording roots come only from
    /// the native ChatStore, never from page-supplied decoration metadata.
    pub fn submit(&self, rich: Option<&ChatRich>) {
        self.submit_target(None, rich);
    }
    pub fn submit_recording(&self, root: &Path, rich: Option<&ChatRich>) {
        self.submit_target(Some(root.to_owned()), rich);
    }
    /// Native-owned recording root and validated channel only. No browser URL,
    /// cookie or account data enters the queue. Never called while opening replay.
    pub fn submit_channel(&self, root: &Path, channel: &str) {
        let Some(sender) = &self.sender else {
            return;
        };
        if self.state.cancel.load(Ordering::Acquire)
            || !root.is_absolute()
            || channel.len() != 32
            || !channel.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return;
        }
        let Ok(mut seen) = self.state.seen.try_lock() else {
            return;
        };
        let key = (format!("channel:{channel}"), Some(root.to_owned()));
        if seen.len() >= MAX_FILES || seen.contains(&key) {
            return;
        }
        if sender
            .try_send(AssetJob::Channel(channel.into(), root.to_owned()))
            .is_ok()
        {
            seen.insert(key);
        }
    }
    fn submit_target(&self, root: Option<PathBuf>, rich: Option<&ChatRich>) {
        let (Some(sender), Some(rich)) = (&self.sender, rich) else {
            return;
        };
        if self.state.cancel.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut seen) = self.state.seen.try_lock() else {
            return;
        };
        for url in rich
            .badges
            .iter()
            .map(|badge| &badge.image_url)
            .chain(rich.emojis.iter().map(|emoji| &emoji.image_url))
            .take(24)
        {
            if seen.len() >= MAX_FILES {
                break;
            }
            let Some(id) = asset_id(url) else {
                continue;
            };
            let key = (id.clone(), root.clone());
            if seen.contains(&key) {
                continue;
            }
            if sender
                .try_send(AssetJob::Image(id, url.clone(), root.clone()))
                .is_ok()
            {
                seen.insert(key);
            }
        }
    }
    pub fn shutdown_and_wait(&self) {
        self.state.cancel.store(true, Ordering::Release);
        if let Some(handle) = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = handle.join();
        }
    }
}
fn mirror_recording(
    root: &Path,
    id: &str,
    data: &str,
    budgets: &mut HashMap<PathBuf, (u64, usize)>,
) -> Result<(), StreamError> {
    // Root comes only from ChatStore, not a remote message. No overwrite,
    // symlink/reparse traversal, unbounded per-recording files or directories.
    plain_path(root)?;
    if !root.is_dir() || fs::canonicalize(root).map_err(|_| unavailable())? != root {
        return Err(unavailable());
    }
    let directory = root.join("replay-assets");
    plain_path(&directory)?;
    if !budgets.contains_key(root) {
        if budgets.len() >= 64 {
            return Err(unavailable());
        }
        if !directory.exists() {
            fs::create_dir(&directory).map_err(|_| unavailable())?;
        }
        let mut bytes = 0u64;
        let mut files = 0usize;
        for entry in fs::read_dir(&directory)
            .map_err(|_| unavailable())?
            .take(MAX_FILES + 1)
        {
            let entry = entry.map_err(|_| unavailable())?;
            plain_path(&entry.path())?;
            let meta = entry.metadata().map_err(|_| unavailable())?;
            if !meta.is_file() {
                return Err(unavailable());
            }
            bytes = bytes.saturating_add(meta.len());
            files += 1;
        }
        if files > MAX_FILES || bytes > MAX_DISK {
            return Err(unavailable());
        }
        budgets.insert(root.to_owned(), (bytes, files));
    }
    if directory.join(format!("{id}.json")).exists() {
        return Ok(());
    }
    let (bytes, files) = budgets.get_mut(root).ok_or_else(unavailable)?;
    if *files >= MAX_FILES || bytes.saturating_add(MAX_ASSET) > MAX_DISK {
        return Err(unavailable());
    }
    let added = store_data(&directory, id, data)?;
    *bytes = bytes.saturating_add(added);
    *files += 1;
    Ok(())
}
impl Drop for ReplayAssetCache {
    fn drop(&mut self) {
        if Arc::strong_count(&self.handle) == 1 {
            self.shutdown_and_wait();
        }
    }
}
fn store_data(root: &Path, id: &str, data: &str) -> Result<u64, StreamError> {
    if !valid_id(id) || data.len() as u64 > MAX_ASSET {
        return Err(unavailable());
    }
    let (mime, body) = data
        .strip_prefix("data:")
        .and_then(|s| s.split_once(";base64,"))
        .ok_or_else(unavailable)?;
    let bytes = STANDARD.decode(body).map_err(|_| unavailable())?;
    let actual = chat_assets::validate_raster(&bytes, Some(mime))?;
    let stored = StoredAsset {
        version: 1,
        mime: actual.into(),
        body: body.into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let payload = serde_json::to_vec(&stored).map_err(|_| unavailable())?;
    if payload.len() as u64 > MAX_ASSET {
        return Err(unavailable());
    }
    let path = root.join(format!("{id}.json"));
    plain_path(&path)?;
    if path.exists() {
        return Err(unavailable());
    }
    let temp = root.join(format!("{}.partial", uuid::Uuid::new_v4().simple()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| unavailable())?;
    file.write_all(&payload)
        .and_then(|_| file.sync_all())
        .map_err(|_| unavailable())?;
    drop(file);
    // A single app-owned worker writes this cache. Never replace another file.
    if path.exists() {
        return Err(unavailable());
    }
    fs::rename(temp, path).map_err(|_| unavailable())?;
    Ok(payload.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    fn data() -> String {
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([10, 20, 30, 255])))
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        format!(
            "data:image/png;base64,{}",
            STANDARD.encode(out.into_inner())
        )
    }
    #[test]
    fn recording_copy_is_portable_bounded_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive");
        fs::create_dir(&archive).unwrap();
        let archive = fs::canonicalize(archive).unwrap();
        let id = asset_id("https://ssl.pstatic.net/portable.png").unwrap();
        let mut budgets = HashMap::new();
        mirror_recording(&archive, &id, &data(), &mut budgets).unwrap();
        let stored = archive.join("replay-assets").join(format!("{id}.json"));
        let before = fs::read(&stored).unwrap();
        assert_eq!(read_recording(&archive, &id).unwrap().mime, "image/png");
        mirror_recording(&archive, &id, "data:bad;base64,AA==", &mut budgets).unwrap();
        assert_eq!(fs::read(&stored).unwrap(), before);
        budgets.insert(archive.clone(), (MAX_DISK, MAX_FILES));
        assert!(mirror_recording(&archive, &"b".repeat(64), &data(), &mut budgets).is_err());
        assert!(mirror_recording(Path::new("relative"), &id, &data(), &mut budgets).is_err());
    }
    #[test]
    fn only_known_public_urls_have_opaque_ids() {
        assert_eq!(asset_id("https://ssl.pstatic.net/a.png").unwrap().len(), 64);
        for url in [
            "file:///private",
            "http://ssl.pstatic.net/a.png",
            "https://localhost/a",
            "https://ssl.pstatic.net/a?token=secret",
        ] {
            assert!(asset_id(url).is_none());
        }
    }
    #[test]
    fn offline_copy_round_trips_without_replacing_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = directory(dir.path());
        fs::create_dir_all(&root).unwrap();
        let id = asset_id("https://ssl.pstatic.net/a.png").unwrap();
        store_data(&root, &id, &data()).unwrap();
        let prior = fs::read(root.join(format!("{id}.json"))).unwrap();
        assert_eq!(read_cached(dir.path(), &id).unwrap().mime, "image/png");
        assert!(store_data(&root, &id, &data()).is_err());
        assert_eq!(fs::read(root.join(format!("{id}.json"))).unwrap(), prior);
        assert!(read_cached(dir.path(), "../../secret").is_err());
    }
    #[test]
    fn edited_assets_and_active_markup_are_not_rendered() {
        let dir = tempfile::tempdir().unwrap();
        let root = directory(dir.path());
        fs::create_dir_all(&root).unwrap();
        let id = "a".repeat(64);
        assert!(store_data(&root, &id, "data:image/svg+xml;base64,PHN2Zz4=").is_err());
        let bad = StoredAsset {
            version: 1,
            mime: "image/png".into(),
            body: STANDARD.encode(b"not raster"),
            sha256: format!("{:x}", Sha256::digest(b"not raster")),
        };
        fs::write(
            root.join(format!("{id}.json")),
            serde_json::to_vec(&bad).unwrap(),
        )
        .unwrap();
        assert!(read_cached(dir.path(), &id).is_err());
    }
    #[test]
    fn duplicate_jobs_are_bounded_and_no_network_runs_in_disabled_mode() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let cache = ReplayAssetCache::with_fetch(
            dir.path(),
            true,
            Arc::new(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(data())
            }),
        )
        .unwrap();
        let rich = ChatRich {
            emojis: vec![super::super::model::ChatEmoji {
                id: "e".into(),
                image_url: "https://ssl.pstatic.net/a.png".into(),
            }],
            ..Default::default()
        };
        for _ in 0..100 {
            cache.submit(Some(&rich));
        }
        let id = asset_id(&rich.emojis[0].image_url).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while read_cached(dir.path(), &id).is_err() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(read_cached(dir.path(), &id).is_ok());
        cache.shutdown_and_wait();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let disabled = ReplayAssetCache::new(dir.path(), false).unwrap();
        disabled.submit(Some(&rich));
        disabled.shutdown_and_wait();
    }

    #[test]
    fn channel_profile_is_saved_once_per_recording_without_chat_or_replay_network() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let first = fs::canonicalize(first).unwrap();
        let second = fs::canonicalize(second).unwrap();
        let channel = "a".repeat(32);
        let expected_channel = channel.clone();
        let image_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let profile_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted_images = image_calls.clone();
        let counted_profiles = profile_calls.clone();
        let cache = ReplayAssetCache::with_fetchers(
            dir.path(),
            true,
            Arc::new(move |_| {
                counted_images.fetch_add(1, Ordering::SeqCst);
                Ok(data())
            }),
            Arc::new(move |id| {
                assert_eq!(id, expected_channel);
                counted_profiles.fetch_add(1, Ordering::SeqCst);
                Ok((
                    "합성 채널".into(),
                    Some("https://ssl.pstatic.net/channel-profile.png".into()),
                ))
            }),
        )
        .unwrap();
        for _ in 0..100 {
            cache.submit_channel(&first, &channel);
            cache.submit_channel(&second, &channel);
        }
        cache.submit_channel(&first, "../bad");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while channel_profile::read(&second, &channel).1.is_none()
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        cache.shutdown_and_wait();
        assert_eq!(profile_calls.load(Ordering::SeqCst), 2);
        assert_eq!(image_calls.load(Ordering::SeqCst), 1);
        for root in [&first, &second] {
            let (name, image) = channel_profile::read(root, &channel);
            assert_eq!(name.as_deref(), Some("합성 채널"));
            assert_eq!(image, Some(data()));
            let metadata = fs::read_to_string(root.join("channel-profile.json")).unwrap();
            assert!(!metadata.contains("https:") && !metadata.contains("Cookie"));
            assert!(!root.join("chat.jsonl").exists());
        }
        // The archive remains portable without the shared cache and after restart.
        let portable = dir.path().join("portable");
        fs::rename(&second, &portable).unwrap();
        let calls_before = image_calls.load(Ordering::SeqCst);
        assert_eq!(channel_profile::read(&portable, &channel).1, Some(data()));
        assert_eq!(image_calls.load(Ordering::SeqCst), calls_before);
    }

    #[test]
    fn unavailable_channel_profile_is_best_effort_and_does_not_stop_asset_worker() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let cache = ReplayAssetCache::with_fetchers(
            dir.path(),
            true,
            Arc::new(|_| Ok(data())),
            Arc::new(|_| Err(unavailable())),
        )
        .unwrap();
        cache.submit_channel(&root, &"a".repeat(32));
        let rich = ChatRich {
            emojis: vec![super::super::model::ChatEmoji {
                id: "ok".into(),
                image_url: "https://ssl.pstatic.net/ok.png".into(),
            }],
            ..Default::default()
        };
        cache.submit_recording(&root, Some(&rich));
        let id = asset_id(&rich.emojis[0].image_url).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while read_recording(&root, &id).is_err() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        cache.shutdown_and_wait();
        assert!(read_recording(&root, &id).is_ok());
        assert_eq!(channel_profile::read(&root, &"a".repeat(32)), (None, None));
    }
}
