//! YouTube public emoji assets. No account cookies, caller URLs, redirects or replay network.
//! A session owns one bounded receiver worker; each recording owns an immutable local manifest.
use super::{replay_chat::ChatPart, storage};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex, Weak,
    },
    time::{Duration, Instant},
};
const BODY: usize = 1024 * 1024;
const ITEMS: usize = 2048;
const TOTAL: usize = 128 * 1024 * 1024;
const RAM: usize = 8 * 1024 * 1024;
const ERROR: &str = "이모티콘 이미지를 읽지 못했습니다. 문자로 표시합니다.";
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAssetRead {
    pub state: &'static str,
    pub data_url: Option<String>,
}
impl ChatAssetRead {
    pub(super) fn local(image: Option<String>) -> Self {
        Self {
            state: if image.is_some() { "ready" } else { "missing" },
            data_url: image,
        }
    }
}
static DNS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(super) fn asset_id(url: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("youtube-emoji:{url}").as_bytes())
    )
}
pub(super) fn sanitize_url(input: &str) -> Option<String> {
    if input.len() > 1024
        || input
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
        || input.contains('\\')
    {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("yt3.ggpht.com" | "fonts.gstatic.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
    {
        return None;
    }
    Some(url.into())
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            let p = ip.segments();
            (p[0] & 0xe000) == 0x2000
                && p[0] != 0x2002
                && !(p[0] == 0x2001 && p[1] < 0x0200)
                && !(p[0] == 0x2001 && p[1] == 0x0db8)
                && !(p[0] == 0x3fff && p[1] < 0x1000)
        }
    }
}
pub(super) fn addresses(host: &str) -> Result<Vec<SocketAddr>, String> {
    DNS_ACTIVE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 4).then_some(n + 1)
        })
        .map_err(|_| ERROR)?;
    struct DnsPermit;
    impl Drop for DnsPermit {
        fn drop(&mut self) {
            DNS_ACTIVE.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let permit = DnsPermit;
    let host = host.to_owned();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("platform-emoji-dns".into())
        .spawn(move || {
            let _permit = permit; // OS DNS may outlive timeout, but global concurrency stays bounded.
            let _ = tx.send(
                (host.as_str(), 443)
                    .to_socket_addrs()
                    .map(|a| a.take(16).collect::<Vec<_>>()),
            );
        })
        .map_err(|_| ERROR)?;
    let result = rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| ERROR)?
        .map_err(|_| ERROR)?;
    if result.is_empty() || !result.iter().all(|a| public_ip(a.ip())) {
        return Err(ERROR.into());
    }
    Ok(result)
}
#[derive(Clone)]
struct Blob {
    bytes: Vec<u8>,
    mime: String,
}
fn blob(bytes: Vec<u8>, declared: Option<&str>) -> Result<Blob, String> {
    if bytes.len() > BODY {
        return Err(ERROR.into());
    }
    let mime = crate::streaming::chat_assets::validate_raster(&bytes, declared)
        .map_err(|_| ERROR)?
        .to_owned();
    Ok(Blob { bytes, mime })
}
fn fetch(source: &str) -> Result<Blob, String> {
    let source = sanitize_url(source).ok_or(ERROR)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let url = Url::parse(&source).map_err(|_| ERROR)?;
    let host = url.host_str().ok_or(ERROR)?;
    let addresses = addresses(host)?;
    let timeout = deadline
        .checked_duration_since(Instant::now())
        .ok_or(ERROR)?;
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(timeout.min(Duration::from_secs(3)))
        .timeout(timeout)
        .build()
        .map_err(|_| ERROR)?;
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "image/png,image/jpeg,image/gif,image/webp",
        )
        .send()
        .map_err(|_| ERROR)?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > BODY as u64)
    {
        return Err(ERROR.into());
    }
    let declared = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .map(|v| v.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| ERROR)?;
    let mut bytes = Vec::new();
    response
        .take((BODY + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    blob(bytes, declared.as_deref())
}
fn data_url(blob: &Blob) -> String {
    format!("data:{};base64,{}", blob.mime, STANDARD.encode(&blob.bytes))
}

#[derive(Serialize, Deserialize)]
struct ManifestEntry {
    version: u8,
    id: String,
    sha256: String,
    bytes: usize,
    mime: String,
}
struct ArchiveState {
    closed: bool,
    manifest: File,
    expected: HashSet<String>,
    stored: HashSet<String>,
    bytes: usize,
    failed: bool,
}
pub(super) struct ArchiveAssets {
    directory: PathBuf,
    state: Mutex<ArchiveState>,
    _parents: Vec<File>,
}
impl ArchiveAssets {
    pub(super) fn new(directory: &Path) -> Result<Arc<Self>, String> {
        let directory = directory.join("chat-assets");
        let _base = pin_parents(&directory)?;
        fs::create_dir(&directory).map_err(|_| ERROR)?;
        let parents = pin_parents(&directory.join("manifest.jsonl"))?;
        let manifest = create(&directory.join("manifest.jsonl"))?;
        Ok(Arc::new(Self {
            directory,
            _parents: parents,
            state: Mutex::new(ArchiveState {
                closed: false,
                manifest,
                expected: HashSet::new(),
                stored: HashSet::new(),
                bytes: 0,
                failed: false,
            }),
        }))
    }
    fn expect(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !s.closed {
            if s.expected.len() < ITEMS {
                s.expected.insert(id.into());
            } else {
                s.failed = true;
            }
        }
    }
    fn store(&self, id: &str, asset: &Blob) -> Result<(), String> {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.closed || !s.expected.contains(id) || s.stored.contains(id) {
            return Ok(());
        }
        let result = (|| {
            if s.stored.len() >= ITEMS || s.bytes + asset.bytes.len() > TOTAL {
                return Err(ERROR.into());
            }
            storage::safe_path(&self.directory)?;
            let mut file = create(&self.directory.join(id))?;
            file.write_all(&asset.bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| ERROR)?;
            let row = ManifestEntry {
                version: 1,
                id: id.into(),
                sha256: format!("{:x}", Sha256::digest(&asset.bytes)),
                bytes: asset.bytes.len(),
                mime: asset.mime.clone(),
            };
            let bytes = serde_json::to_vec(&row).map_err(|_| ERROR)?;
            s.manifest
                .write_all(&bytes)
                .and_then(|_| s.manifest.write_all(b"\n"))
                .and_then(|_| s.manifest.sync_data())
                .map_err(|_| ERROR)?;
            s.bytes += asset.bytes.len();
            s.stored.insert(id.into());
            Ok(())
        })();
        if result.is_err() {
            s.failed = true;
        }
        result
    }
    /// The same lock covers the final write and close: a delayed worker can never
    /// append after detach. Missing assets remain text; video finalization proceeds.
    pub(super) fn close(&self) -> bool {
        self.close_with_sync(File::sync_all)
    }
    fn close_with_sync(&self, sync: impl FnOnce(&File) -> std::io::Result<()>) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.closed {
            return s.failed || s.expected.len() != s.stored.len();
        }
        s.closed = true;
        // Sync even when some assets are missing; preserve the successfully stored
        // subset. Compute the status only after this durable-manifest attempt.
        if sync(&s.manifest).is_err() {
            s.failed = true;
        }
        let incomplete = s.failed || s.expected.len() != s.stored.len();
        let status = serde_json::json!({"version":1,"expected":s.expected.len(),"stored":s.stored.len(),"bytes":s.bytes,"incomplete":incomplete});
        let result = create(&self.directory.join("status.json")).and_then(|mut file| {
            file.write_all(status.to_string().as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|_| ERROR.to_string())
        });
        if result.is_err() {
            s.failed = true;
        }
        incomplete || s.failed
    }
}
fn pin_parents(path: &Path) -> Result<Vec<File>, String> {
    storage::safe_path(path)?;
    let mut guards = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        for parent in path
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(parent)
                .map_err(|_| ERROR)?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                    .map_err(|_| ERROR)?;
            }
            if info.dwFileAttributes & 0x400 != 0 || info.dwFileAttributes & 0x10 == 0 {
                return Err(ERROR.into());
            }
            guards.push(file);
        }
    }
    Ok(guards)
}
fn create(path: &Path) -> Result<File, String> {
    storage::safe_path(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    options.open(path).map_err(|_| ERROR.into())
}
/// Hold a no-write/no-delete handle while checking link count and reading bytes.
fn read_owned(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let _parents = pin_parents(path)?;
    storage::regular(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| ERROR)?;
        }
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & 0x400 != 0 {
            return Err(ERROR.into());
        }
    }
    if !file.metadata().map_err(|_| ERROR)?.is_file() {
        return Err(ERROR.into());
    }
    let mut bytes = Vec::new();
    file.take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() > max {
        return Err(ERROR.into());
    }
    Ok(bytes)
}
pub(super) fn replay_asset(directory: &Path, id: &str) -> Result<Option<String>, String> {
    if !valid_id(id) {
        return Err(ERROR.into());
    }
    let directory = directory.join("chat-assets");
    storage::safe_path(&directory)?;
    if !directory.exists() {
        return Ok(None);
    }
    let manifest = read_owned(&directory.join("manifest.jsonl"), ITEMS * 512)?;
    let mut found = None;
    let mut seen = HashSet::new();
    let mut total = 0usize;
    for line in manifest
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .take(ITEMS + 1)
    {
        if line.len() > 512 {
            return Err(ERROR.into());
        }
        let entry: ManifestEntry = serde_json::from_slice(line).map_err(|_| ERROR)?;
        if entry.version != 1
            || !valid_id(&entry.id)
            || !valid_id(&entry.sha256)
            || entry.bytes > BODY
        {
            return Err(ERROR.into());
        }
        total += entry.bytes;
        if !seen.insert(entry.id.clone()) || seen.len() > ITEMS || total > TOTAL {
            return Err(ERROR.into());
        }
        if entry.id == id {
            if found.is_some() {
                return Err(ERROR.into());
            }
            found = Some(entry);
        }
    }
    let Some(entry) = found else {
        return Ok(None);
    };
    let bytes = read_owned(&directory.join(id), BODY)?;
    if bytes.len() != entry.bytes || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256 {
        return Err(ERROR.into());
    }
    Ok(Some(data_url(&blob(bytes, Some(&entry.mime))?)))
}

struct Entry {
    url: String,
    image: Option<Arc<Blob>>,
    attempted: bool,
    queued: bool,
    targets: Vec<Weak<ArchiveAssets>>,
}
#[derive(Default)]
struct CacheState {
    entries: HashMap<String, Entry>,
    queue: VecDeque<String>,
    lru: VecDeque<String>,
    ram: usize,
    downloaded: usize,
    stopped: bool,
}
struct Core {
    state: Mutex<CacheState>,
    wake: Condvar,
    done: AtomicBool,
    failed: AtomicBool,
}
type Fetcher = Arc<dyn Fn(&str) -> Result<Blob, String> + Send + Sync>;
pub(super) struct SessionAssets {
    core: Arc<Core>,
    started: AtomicBool,
    fetcher: Fetcher,
}
impl SessionAssets {
    pub(super) fn new() -> Self {
        Self {
            core: Arc::new(Core {
                state: Mutex::new(CacheState::default()),
                wake: Condvar::new(),
                done: AtomicBool::new(true),
                failed: AtomicBool::new(false),
            }),
            started: AtomicBool::new(false),
            fetcher: Arc::new(fetch),
        }
    }
    pub(super) fn observe(&self, parts: &[ChatPart], archive: Option<&Arc<ArchiveAssets>>) {
        let mut s = self.core.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.stopped {
            return;
        }
        for part in parts {
            let ChatPart::Emoji {
                asset_id: id,
                source_url: Some(url),
                ..
            } = part
            else {
                continue;
            };
            if sanitize_url(url).as_deref() != Some(url.as_str()) || asset_id(url) != *id {
                continue;
            }
            if let Some(target) = archive {
                target.expect(id);
            }
            if !s.entries.contains_key(id) {
                if s.entries.len() >= ITEMS {
                    self.core.failed.store(true, Ordering::Release);
                    continue;
                }
                s.entries.insert(
                    id.clone(),
                    Entry {
                        url: url.clone(),
                        image: None,
                        attempted: false,
                        queued: false,
                        targets: Vec::new(),
                    },
                );
            }
            let queue_room = s.queue.len() < 64;
            let entry = s.entries.get_mut(id).unwrap();
            entry.targets.retain(|t| t.strong_count() > 0);
            if let Some(target) = archive {
                if !entry
                    .targets
                    .iter()
                    .any(|t| t.ptr_eq(&Arc::downgrade(target)))
                    && entry.targets.len() < 8
                {
                    entry.targets.push(Arc::downgrade(target));
                }
            }
            if !entry.queued
                && (!entry.attempted || (entry.image.is_some() && !entry.targets.is_empty()))
            {
                if queue_room {
                    entry.queued = true;
                    s.queue.push_back(id.clone());
                } else {
                    self.core.failed.store(true, Ordering::Release);
                }
            }
        }
        if !s.queue.is_empty() && !self.started.swap(true, Ordering::AcqRel) {
            self.core.done.store(false, Ordering::Release);
            let core = self.core.clone();
            let fetcher = self.fetcher.clone();
            if std::thread::Builder::new()
                .name("platform-emoji-assets".into())
                .spawn(move || worker(core, fetcher))
                .is_err()
            {
                s.stopped = true;
                self.core.failed.store(true, Ordering::Release);
                self.core.done.store(true, Ordering::Release);
            }
        }
        self.core.wake.notify_one();
    }
    pub(super) fn read(&self, id: &str) -> Result<Option<String>, String> {
        if !valid_id(id) {
            return Err(ERROR.into());
        }
        let image = self
            .core
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entries
            .get(id)
            .and_then(|e| e.image.clone());
        Ok(image.as_ref().map(|b| data_url(b)))
    }
    pub(super) fn reply(&self, id: &str) -> Result<ChatAssetRead, String> {
        if !valid_id(id) {
            return Err(ERROR.into());
        }
        let s = self.core.state.lock().unwrap_or_else(|p| p.into_inner());
        let entry = s.entries.get(id);
        let image = entry.and_then(|e| e.image.clone());
        let pending = !s.stopped
            && !self.core.done.load(Ordering::Acquire)
            && entry.is_some_and(|e| e.queued);
        drop(s);
        Ok(ChatAssetRead {
            state: if image.is_some() {
                "ready"
            } else if pending {
                "pending"
            } else {
                "missing"
            },
            data_url: image.as_ref().map(|b| data_url(b)),
        })
    }
    pub(super) fn stop(&self) {
        let mut s = self.core.state.lock().unwrap_or_else(|p| p.into_inner());
        s.stopped = true;
        s.queue.clear();
        self.core.wake.notify_all();
    }
    pub(super) fn is_done(&self) -> bool {
        self.core.done.load(Ordering::Acquire)
    }
    pub(super) fn incomplete(&self) -> bool {
        self.core.failed.load(Ordering::Acquire)
    }
}
impl Drop for SessionAssets {
    fn drop(&mut self) {
        self.stop();
    }
}
fn worker(core: Arc<Core>, fetcher: Fetcher) {
    struct Done(Arc<Core>);
    impl Drop for Done {
        fn drop(&mut self) {
            if std::thread::panicking() {
                self.0.failed.store(true, Ordering::Release);
            }
            self.0.done.store(true, Ordering::Release);
        }
    }
    let _done = Done(core.clone());
    loop {
        let (id, url, cached, attempted) = {
            let mut s = core.state.lock().unwrap_or_else(|p| p.into_inner());
            while s.queue.is_empty() && !s.stopped {
                s = core.wake.wait(s).unwrap_or_else(|p| p.into_inner());
            }
            if s.stopped {
                return;
            }
            let id = s.queue.pop_front().unwrap();
            let entry = s.entries.get(&id).unwrap();
            (
                id.clone(),
                entry.url.clone(),
                entry.image.clone(),
                entry.attempted,
            )
        };
        let fetched = cached.or_else(|| {
            (!attempted)
                .then(|| fetcher(&url).ok().map(Arc::new))
                .flatten()
        });
        let (image, targets) = {
            let mut s = core.state.lock().unwrap_or_else(|p| p.into_inner());
            if s.stopped {
                return;
            }
            let fresh = !attempted;
            let image = fetched.filter(|b| !fresh || s.downloaded + b.bytes.len() <= TOTAL);
            if fresh {
                if let Some(b) = &image {
                    s.downloaded += b.bytes.len();
                    s.ram += b.bytes.len();
                    s.lru.push_back(id.clone());
                }
            }
            let entry = s.entries.get_mut(&id).unwrap();
            entry.queued = false;
            entry.attempted = true;
            entry.image = image.clone();
            let targets = std::mem::take(&mut entry.targets);
            while s.ram > RAM || s.lru.len() > 256 {
                let Some(old) = s.lru.pop_front() else {
                    break;
                };
                if let Some(old) = s.entries.get_mut(&old).and_then(|e| e.image.take()) {
                    s.ram = s.ram.saturating_sub(old.bytes.len());
                }
            }
            (image, targets)
        };
        if let Some(image) = image {
            for target in targets.into_iter().filter_map(|t| t.upgrade()) {
                if target.store(&id, &image).is_err() {
                    core.failed.store(true, Ordering::Release);
                }
            }
        } else {
            core.failed.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Blob {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        blob(out.into_inner(), Some("image/png")).unwrap()
    }
    fn part(n: usize) -> ChatPart {
        let url = format!("https://yt3.ggpht.com/fixture-{n}");
        ChatPart::Emoji {
            asset_id: asset_id(&url),
            alt: "fixture".into(),
            source_url: Some(url),
        }
    }
    fn until(check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !check() {
            assert!(Instant::now() < deadline, "bounded fixture deadline");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn exact_public_sources_and_ip_boundaries() {
        assert!(sanitize_url("https://yt3.ggpht.com/a=s32-c").is_some());
        assert!(
            sanitize_url("https://fonts.gstatic.com/s/e/notoemoji/latest/1f600/32.png").is_some()
        );
        for url in [
            "http://yt3.ggpht.com/a",
            "https://yt3.ggpht.com.evil.test/a",
            "https://user@yt3.ggpht.com/a",
            "https://fonts.gstatic.com:8443/a",
            "https://fonts.gstatic.com/a?token=x",
            "https://fonts.gstatic.com/a#x",
            "https://127.0.0.1/a",
            "https://ssl.pstatic.net/a",
        ] {
            assert!(sanitize_url(url).is_none());
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "::1",
            "::ffff:8.8.8.8",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()));
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
        assert!(blob(b"<svg/>".to_vec(), Some("image/svg+xml")).is_err());
        assert!(blob(vec![0; BODY + 1], None).is_err());
    }
    #[test]
    fn incomplete_archive_still_syncs_manifest_before_status() {
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        target.expect(&asset_id("https://yt3.ggpht.com/missing"));
        let calls = std::cell::Cell::new(0);
        assert!(target.close_with_sync(|file| {
            calls.set(calls.get() + 1);
            file.sync_all()
        }));
        assert_eq!(calls.get(), 1);
        let status: serde_json::Value =
            serde_json::from_slice(&fs::read(root.path().join("chat-assets/status.json")).unwrap())
                .unwrap();
        assert_eq!(status["incomplete"], true);
        assert_eq!(status["expected"], 1);
        assert_eq!(status["stored"], 0);
    }
    #[test]
    fn manifest_sync_failure_is_reflected_in_status_and_repeated_close() {
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        assert!(target
            .close_with_sync(|_| Err(std::io::Error::other("synthetic manifest sync failure"))));
        let status: serde_json::Value =
            serde_json::from_slice(&fs::read(root.path().join("chat-assets/status.json")).unwrap())
                .unwrap();
        assert_eq!(status["incomplete"], true);
        assert!(target.close());
    }
    #[test]
    fn status_publication_failure_stays_incomplete_and_preserves_unknown_file() {
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        let path = root.path().join("chat-assets/status.json");
        fs::write(&path, b"unowned fixture").unwrap();
        assert!(target.close());
        assert!(target.close());
        assert_eq!(fs::read(&path).unwrap(), b"unowned fixture");
    }
    #[test]
    fn immutable_archive_is_local_verified_and_closed_against_late_writes() {
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        let a = png();
        let id = asset_id("https://yt3.ggpht.com/a");
        target.expect(&id);
        target.store(&id, &a).unwrap();
        let before = fs::read(root.path().join("chat-assets").join(&id)).unwrap();
        target
            .store(
                &id,
                &Blob {
                    bytes: vec![1],
                    mime: "image/png".into(),
                },
            )
            .unwrap();
        assert!(!target.close());
        drop(target);
        assert_eq!(
            replay_asset(root.path(), &id).unwrap().unwrap(),
            data_url(&a)
        );
        assert_eq!(
            before,
            fs::read(root.path().join("chat-assets").join(&id)).unwrap()
        );
        assert!(replay_asset(root.path(), "../other").is_err());
        fs::write(root.path().join("chat-assets").join(&id), b"changed").unwrap();
        assert!(replay_asset(root.path(), &id).is_err());
        let second = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(second.path()).unwrap();
        target.expect(&id);
        assert!(target.close());
        target.store(&id, &a).unwrap();
        drop(target);
        assert!(!second.path().join("chat-assets").join(&id).exists());
        assert!(replay_asset(second.path(), &id).unwrap().is_none());
    }
    #[test]
    fn assets_and_manifests_reject_hardlinks_and_oversized_input() {
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        let id = asset_id("https://yt3.ggpht.com/a");
        target.expect(&id);
        target.store(&id, &png()).unwrap();
        target.close();
        drop(target);
        fs::hard_link(
            root.path().join("chat-assets").join(&id),
            root.path().join("linked"),
        )
        .unwrap();
        assert!(replay_asset(root.path(), &id).is_err());
        assert!(read_owned(&root.path().join("chat-assets/manifest.jsonl"), 1).is_err());
    }
    #[test]
    fn deduplicated_worker_mirrors_only_referenced_recording_and_drains() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(move |_| {
            count.fetch_add(1, Ordering::AcqRel);
            Ok(png())
        });
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        let p = part(1);
        for _ in 0..20 {
            session.observe(&[p.clone()], Some(&target));
        }
        let ChatPart::Emoji { asset_id: id, .. } = p else {
            unreachable!()
        };
        until(|| target.state.lock().unwrap().stored.contains(&id));
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert!(session.read(&id).unwrap().is_some());
        assert!(session
            .read(&asset_id("https://yt3.ggpht.com/unobserved"))
            .unwrap()
            .is_none());
        assert!(!target.close());
        session.stop();
        until(|| session.is_done());
        drop(target);
        assert!(replay_asset(root.path(), &id).unwrap().is_some());
    }
    #[test]
    fn queue_and_registry_are_bounded_without_network() {
        let session = SessionAssets::new();
        session.started.store(true, Ordering::Release);
        for n in 0..ITEMS + 100 {
            session.observe(&[part(n)], None);
        }
        let state = session.core.state.lock().unwrap();
        assert_eq!(state.entries.len(), ITEMS);
        assert_eq!(state.queue.len(), 64);
        drop(state);
        assert!(session.incomplete());
        session.stop();
        assert!(session.is_done());
    }
    #[test]
    fn cancellation_waits_for_actual_worker_and_late_asset_never_enters_archive() {
        let entered = Arc::new(AtomicBool::new(false));
        let permit = Arc::new(AtomicBool::new(false));
        let e = entered.clone();
        let p = permit.clone();
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(move |_| {
            e.store(true, Ordering::Release);
            until(|| p.load(Ordering::Acquire));
            Ok(png())
        });
        let root = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(root.path()).unwrap();
        session.observe(&[part(0)], Some(&target));
        until(|| entered.load(Ordering::Acquire));
        let ChatPart::Emoji { asset_id: id, .. } = part(0) else {
            unreachable!()
        };
        assert_eq!(session.reply(&id).unwrap().state, "pending");
        assert!(target.close());
        session.stop();
        assert!(!session.is_done());
        assert_eq!(session.reply(&id).unwrap().state, "missing");
        permit.store(true, Ordering::Release);
        until(|| session.is_done());
        assert_eq!(target.state.lock().unwrap().stored.len(), 0);
    }
    #[test]
    fn acquisition_failure_is_text_fallback_not_receiver_failure() {
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(|_| Err(ERROR.into()));
        session.observe(&[part(0)], None);
        until(|| session.incomplete());
        session.stop();
        until(|| session.is_done());
        let ChatPart::Emoji { asset_id: id, .. } = part(0) else {
            unreachable!()
        };
        assert!(session.read(&id).unwrap().is_none());
    }
    #[test]
    #[ignore = "Explicit isolated public raw fixture; at most one image per exact allowed host, temp archive only"]
    fn public_youtube_emoji_smoke() {
        let fixture = PathBuf::from(
            std::env::var("ATSUMI_CHAT_ASSET_SMOKE_SOURCE")
                .expect("explicit isolated fixture required"),
        );
        let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".runtime/platform-media-live-tests")
            .canonicalize()
            .unwrap();
        assert!(fixture.canonicalize().unwrap().starts_with(allowed));
        let bytes = read_owned(&fixture, 1024 * 1024).unwrap();
        let before = Sha256::digest(&bytes);
        let mut candidates = HashMap::new();
        for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            if let Ok(events) = super::super::replay_chat::normalize_public_line("youtube", line) {
                for event in events {
                    for part in event.parts {
                        if let ChatPart::Emoji {
                            source_url: Some(url),
                            ..
                        } = part
                        {
                            let host = Url::parse(&url).unwrap().host_str().unwrap().to_owned();
                            candidates.entry(host).or_insert(url);
                        }
                    }
                }
            }
        }
        assert!(
            !candidates.is_empty() && candidates.len() <= 2,
            "only exact permitted hosts from the captured public source"
        );
        let temp = tempfile::tempdir().unwrap();
        let target = ArchiveAssets::new(temp.path()).unwrap();
        let mut acquired = Vec::new();
        let mut total = 0usize;
        for url in candidates.values() {
            let image = fetch(url).unwrap();
            total += image.bytes.len();
            let id = asset_id(url);
            target.expect(&id);
            target.store(&id, &image).unwrap();
            acquired.push((id, data_url(&image)));
        }
        assert!(!target.close());
        drop(target);
        // No fetch/session/URL is reachable from replay_asset; read twice locally.
        for (id, expected) in &acquired {
            assert_eq!(
                replay_asset(temp.path(), id).unwrap().as_ref(),
                Some(expected)
            );
            assert_eq!(
                replay_asset(temp.path(), id).unwrap().as_ref(),
                Some(expected)
            );
        }
        assert_eq!(
            before,
            Sha256::digest(read_owned(&fixture, 1024 * 1024).unwrap())
        );
        eprintln!(
            "public emoji assets={},bytes={},localReplays={},sourceUnchanged=true",
            acquired.len(),
            total,
            acquired.len() * 2
        );
    }
}
