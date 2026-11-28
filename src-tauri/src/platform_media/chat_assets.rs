//! Capture-owned public chat images. No account cookies, caller URLs, redirects or replay network.
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
// Same bounded target budget as relay chat: 64 queued + one active 5s fetch.
const ARCHIVE_DRAIN: Duration = Duration::from_secs(330);
const ERROR: &str = "이모티콘 이미지를 읽지 못했습니다. 문자로 표시합니다.";
#[cfg(test)]
#[path = "chat_assets_custom_tests.rs"]
mod custom_tests;
#[cfg(test)]
#[path="chat_assets_ogq_tests.rs"]
mod ogq_tests;
#[cfg(test)]
#[path="chat_assets_subscription_tests.rs"]
mod subscription_tests;
#[cfg(test)]
#[path = "chat_assets_archive_drain_tests.rs"]
mod archive_drain_tests;
#[cfg(test)]
#[path = "chat_assets_standard_gift_tests.rs"]
mod standard_gift_tests;
use crate::soop::chat::gifts::{assets::Request as SoopGiftRequest, SoopGiftKind, SoopGiftMetadata};
use crate::soop::chat::ogq::{Request as SoopOgqRequest, Metadata as SoopOgqMetadata};
use crate::soop::chat::subscriptions::{Request as SoopSubscriptionRequest, Metadata as SoopSubscriptionMetadata};
#[derive(Clone)]
enum AssetSource {
    YoutubeEmoji(String),
    SoopGift(SoopGiftRequest),
    SoopMission(crate::soop::chat::missions::Request),
    SoopOgq(SoopOgqRequest),
    SoopSubscription(SoopSubscriptionRequest),
    SoopPersonalcon(crate::soop::chat::personalcon::Request),
    SoopAvatar(crate::soop::chat::portrait::Request),
}
impl AssetSource {
    fn url(&self)->&str { match self { Self::YoutubeEmoji(url)=>url, Self::SoopGift(request)=>request.url(), Self::SoopMission(request)=>request.url(), Self::SoopOgq(request)=>request.url(), Self::SoopSubscription(request)=>request.url(),Self::SoopPersonalcon(request)=>request.url(),Self::SoopAvatar(request)=>request.url() } }
    fn id(&self)->String { match self { Self::YoutubeEmoji(url)=>asset_id(url), Self::SoopGift(request)=>request.id().into(), Self::SoopMission(request)=>request.id().into(),Self::SoopOgq(request)=>request.id().into(), Self::SoopSubscription(request)=>request.id().into(),Self::SoopPersonalcon(request)=>request.id().into(),Self::SoopAvatar(request)=>request.id().into() } }
    fn valid(&self)->bool { match self {
        Self::YoutubeEmoji(url)=>sanitize_url(url).as_deref()==Some(url.as_str()),
        Self::SoopGift(request)=>request.valid(),
        Self::SoopMission(request)=>request.valid(),
        Self::SoopOgq(request)=>crate::soop::chat::ogq::canonical_url(request.url())==Some(request.url()),
        Self::SoopSubscription(request)=>crate::soop::chat::subscriptions::canonical_url(request.url())==Some(request.url()),
        Self::SoopPersonalcon(request)=>crate::soop::chat::personalcon::canonical_url(request.url())==Some(request.url()),
        Self::SoopAvatar(request)=>crate::soop::chat::portrait::canonical_url(request.url())==Some(request.url()),
    } }
}
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
    fetch_source(&AssetSource::YoutubeEmoji(source.into()))
}
fn fetch_source(source: &AssetSource) -> Result<Blob, String> {
    if !source.valid() { return Err(ERROR.into()); }
    let deadline = Instant::now() + Duration::from_secs(5);
    let url = Url::parse(source.url()).map_err(|_| ERROR)?;
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
    settled: HashSet<String>,
    bytes: usize,
    failed: bool,
    protected: bool,
    admitted: HashSet<String>,
    drain_deadline: Option<Instant>,
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
                settled: HashSet::new(),
                bytes: 0,
                failed: false,
                protected: false,
                admitted: HashSet::new(),
                drain_deadline: None,
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
    fn missing(&self) {
        let mut state=self.state.lock().unwrap_or_else(|p|p.into_inner());
        if !state.closed {state.failed=true;}
    }
    fn missing_id(&self, id: &str) {
        let mut state=self.state.lock().unwrap_or_else(|p|p.into_inner());
        if !state.closed && state.expected.contains(id) && state.settled.insert(id.into()) {
            state.failed=true;
        }
    }
    fn admitted(&self, id: &str) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.protected && !s.closed && s.expected.contains(id) {
            s.admitted.insert(id.to_owned());
        }
    }
    fn stopped_observation(&self, id: &str) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !s.protected {
            // Preserve the existing YouTube/unprotected stop warning contract.
            if !s.closed {
                if s.expected.len() < ITEMS { s.expected.insert(id.into()); } else { s.failed = true; }
                if s.expected.contains(id) && s.settled.insert(id.into()) { s.failed = true; }
            }
            return true;
        }
        if s.closed || (s.expected.contains(id) && s.settled.contains(id)) { return false; }
        // A duplicate in-flight receive must not settle an accepted image early.
        if s.protected && s.admitted.contains(id) && !s.settled.contains(id)
            && s.drain_deadline.is_none_or(|end| Instant::now() < end) { return false; }
        if s.expected.len() < ITEMS || s.expected.contains(id) {
            s.expected.insert(id.into());
            s.settled.insert(id.into());
        }
        s.failed = true;
        true
    }
    fn begin_drain(&self, now: Instant, budget: Duration) -> Instant {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        *s.drain_deadline.get_or_insert(now + budget)
    }
    fn retain_on_stop(&self, id: &str, now: Instant) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !s.protected || s.closed || !s.admitted.contains(id) { return false; }
        let end = *s.drain_deadline.get_or_insert(now + ARCHIVE_DRAIN);
        now < end && !s.settled.contains(id)
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
        s.settled.insert(id.into());
        result
    }
    /// The same lock covers the final write and close: a delayed worker can never
    /// append after detach. Missing assets remain text; video finalization proceeds.
    pub(super) fn close(&self) -> bool {
        self.close_with_sync(File::sync_all)
    }
    /// A relay may wait on its own target without stopping the shared fetcher
    /// or waiting for another recording's assets. Caller supplies the deadline.
    pub(super) fn pending(&self) -> bool {
        let state=self.state.lock().unwrap_or_else(|p|p.into_inner());
        !state.closed && !state.expected.is_subset(&state.settled)
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

/// SOOP-only ownership, acquired at attach before receiver EOF can stop assets.
/// The guard outlives asset settlement until the caller persists its chat proof.
pub(super) struct ArchiveAssetDrain {
    target: Arc<ArchiveAssets>,
    core: Arc<Core>,
    settled: bool,
}
impl ArchiveAssetDrain {
    pub(super) fn begin_drain(&self) {
        self.target.begin_drain(Instant::now(), ARCHIVE_DRAIN);
    }
    pub(super) fn settle_cancellable(&mut self, cancel: &AtomicBool) -> bool {
        self.settle_with_budget(cancel, ARCHIVE_DRAIN)
    }
    fn settle_with_budget(&mut self, cancel: &AtomicBool, budget: Duration) -> bool {
        let end = self.target.begin_drain(Instant::now(), budget);
        while self.target.pending() && !cancel.load(Ordering::Acquire) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.abort()
    }
    pub(super) fn abort(&mut self) -> bool {
        if self.target.pending() { self.target.missing(); }
        let incomplete = self.target.close();
        self.settled = true;
        incomplete
    }
}
impl Drop for ArchiveAssetDrain {
    fn drop(&mut self) {
        if !self.settled { self.abort(); }
        self.core.archive_owners.fetch_sub(1, Ordering::AcqRel);
        self.core.wake.notify_all();
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
    source: AssetSource,
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
    archive_owners: AtomicUsize,
}
type Fetcher = Arc<dyn Fn(&AssetSource) -> Result<Blob, String> + Send + Sync>;
pub(super) struct SessionAssets {
    core: Arc<Core>,
    started: AtomicBool,
    fetcher: Fetcher,
}
impl SessionAssets {
    #[cfg(test)]
    pub(super) fn with_test_fetcher(fetcher: impl Fn(&str)->Result<Vec<u8>,String> + Send + Sync + 'static)->Self {
        let mut session=Self::new();
        session.fetcher=Arc::new(move |source|blob(fetcher(source.url())?,None));
        session
    }
    pub(super) fn new() -> Self {
        Self {
            core: Arc::new(Core {
                state: Mutex::new(CacheState::default()),
                wake: Condvar::new(),
                done: AtomicBool::new(true),
                failed: AtomicBool::new(false),
                archive_owners: AtomicUsize::new(0),
            }),
            started: AtomicBool::new(false),
            fetcher: Arc::new(fetch_source),
        }
    }
    pub(super) fn protect_archive(&self, target: Arc<ArchiveAssets>) -> ArchiveAssetDrain {
        target.state.lock().unwrap_or_else(|p| p.into_inner()).protected = true;
        self.core.archive_owners.fetch_add(1, Ordering::AcqRel);
        ArchiveAssetDrain { target, core: self.core.clone(), settled: false }
    }
    pub(super) fn protected_archive_drains_pending(&self) -> bool {
        self.core.archive_owners.load(Ordering::Acquire) != 0
    }
    pub(super) fn observe(&self, parts: &[ChatPart], archive: Option<&Arc<ArchiveAssets>>) {
        let sources=parts.iter().filter_map(|part| {
            let ChatPart::Emoji {asset_id:id, source_url:Some(url),..}=part else {return None;};
            if sanitize_url(url).as_deref()!=Some(url.as_str()) || asset_id(url)!=*id {return None;}
            Some(AssetSource::YoutubeEmoji(url.clone()))
        });
        self.observe_sources(sources,archive);
    }
    pub(super) fn observe_mission(&self, meta:&crate::soop::chat::missions::Metadata, request:Option<&crate::soop::chat::missions::Request>, archive:Option<&Arc<ArchiveAssets>>) {
        if !meta.has_image() {return;}
        let source=request.filter(|r|r.matches(meta)&&meta.image_asset_id.as_deref()==Some(r.id())).cloned().map(AssetSource::SoopMission)
            .or_else(|| {
                let id=meta.image_asset_id.as_deref().filter(|id|valid_id(id))?;
                let state=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
                state.entries.get(id).filter(|entry|matches!(&entry.source,AssetSource::SoopMission(r) if r.matches(meta))).map(|entry|entry.source.clone())
            });
        if let Some(source)=source {self.observe_sources([source],archive);} else {
            self.core.failed.store(true,Ordering::Release);
            if let Some(target)=archive {target.missing();if let Some(id)=meta.image_asset_id.as_deref().filter(|id|valid_id(id)){target.expect(id);target.missing_id(id);}}
        }
    }
    pub(super) fn observe_gift(&self, gift:&SoopGiftMetadata, request:Option<&SoopGiftRequest>, archive:Option<&Arc<ArchiveAssets>>) {
        // Old standard/video rows have no captured identity. Never backfill them
        // from a current URL or convert the local fallback into capture evidence.
        if (gift.is_default || gift.kind==SoopGiftKind::VideoBalloon) && gift.image_asset_id.is_none() && request.is_none() {return;}
        let source=request.filter(|request|request.matches(gift) && gift.image_asset_id.as_deref()==Some(request.id())).cloned().map(AssetSource::SoopGift)
            .or_else(|| {
                let id=gift.image_asset_id.as_deref().filter(|id|valid_id(id))?;
                let state=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
                state.entries.get(id).filter(|entry|matches!(&entry.source,AssetSource::SoopGift(request) if request.matches(gift))).map(|entry|entry.source.clone())
            });
        if let Some(source)=source {self.observe_sources([source],archive);} else {
            self.core.failed.store(true,Ordering::Release);
            if let Some(archive)=archive {archive.missing();if let Some(id)=gift.image_asset_id.as_deref().filter(|id|valid_id(id)){archive.expect(id);archive.missing_id(id);}}
        }
    }
    pub(super) fn observe_ogq(&self, emote:&SoopOgqMetadata, requests:&[SoopOgqRequest], archive:Option<&Arc<ArchiveAssets>>) {
        let ids=emote.ids().into_iter().collect::<HashSet<_>>();
        let mut missing=ids.len()!=emote.expected_count();
        let mut sources=Vec::new();
        for id in ids {
            if let Some(target)=archive {target.expect(id);}
            let source=requests.iter().find(|request|request.id()==id).cloned().map(AssetSource::SoopOgq)
                .or_else(|| {
                    let state=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
                    state.entries.get(id).filter(|entry|matches!(&entry.source,AssetSource::SoopOgq(_))).map(|entry|entry.source.clone())
                });
            if let Some(source)=source {sources.push(source);} else {missing=true;if let Some(target)=archive{target.missing_id(id);}}
        }
        if missing {self.core.failed.store(true,Ordering::Release);if let Some(target)=archive {target.missing();}}
        self.observe_sources(sources,archive);
    }
    pub(super) fn observe_subscription(&self, meta:&SoopSubscriptionMetadata, requests:&[SoopSubscriptionRequest], archive:Option<&Arc<ArchiveAssets>>) {
        let ids=meta.ids().into_iter().collect::<HashSet<_>>();
        // Primary and fallback may intentionally refer to the same gudok_1 image.
        let mut missing=meta.image_asset_id.is_none() || meta.fallback_asset_id.is_none();
        let mut sources=Vec::new();
        for id in ids {
            if let Some(target)=archive {target.expect(id);}
            let source=requests.iter().find(|request|request.id()==id).cloned().map(AssetSource::SoopSubscription)
                .or_else(|| {
                    let state=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
                    state.entries.get(id).filter(|entry|matches!(&entry.source,AssetSource::SoopSubscription(_))).map(|entry|entry.source.clone())
                });
            if let Some(source)=source {sources.push(source);} else {missing=true;if let Some(target)=archive{target.missing_id(id);}}
        }
        if missing {self.core.failed.store(true,Ordering::Release);if let Some(target)=archive {target.missing();}}
        self.observe_sources(sources,archive);
    }
    pub(super) fn observe_personalcon(&self, meta:&crate::soop::chat::personalcon::Metadata, request:Option<&crate::soop::chat::personalcon::Request>,archive:Option<&Arc<ArchiveAssets>>) {
        let id=&meta.image_asset_id;
        if let Some(target)=archive{target.expect(id);}
        let source=request.filter(|r|r.matches(meta)).cloned().map(AssetSource::SoopPersonalcon).or_else(||{
            let state=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
            state.entries.get(id).filter(|e|matches!(&e.source,AssetSource::SoopPersonalcon(_))).map(|e|e.source.clone())
        });
        if let Some(source)=source{self.observe_sources([source],archive);}else{self.core.failed.store(true,Ordering::Release);if let Some(target)=archive{target.missing_id(id);}}
    }
    pub(super) fn observe_avatar(&self,meta:&crate::soop::chat::portrait::Metadata,request:Option<&crate::soop::chat::portrait::Request>,archive:Option<&Arc<ArchiveAssets>>){
        let id=&meta.image_asset_id;if let Some(target)=archive{target.expect(id);}
        let source=request.filter(|r|r.matches(meta)).cloned().map(AssetSource::SoopAvatar).or_else(||{let state=self.core.state.lock().unwrap_or_else(|p|p.into_inner());state.entries.get(id).filter(|e|matches!(&e.source,AssetSource::SoopAvatar(_))).map(|e|e.source.clone())});
        if let Some(source)=source{self.observe_sources([source],archive);}else{self.core.failed.store(true,Ordering::Release);if let Some(target)=archive{target.missing_id(id);}}
    }
    fn observe_sources(&self, sources:impl IntoIterator<Item=AssetSource>, archive:Option<&Arc<ArchiveAssets>>) {
        let mut s = self.core.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut sources=sources.into_iter();
        if s.stopped {
            // A frame already read by the receiver may be archived after stop.
            // Preserve that event and honest asset coverage without new work.
            for source in sources {
                if archive.is_none_or(|archive| archive.stopped_observation(&source.id())) {
                    self.core.failed.store(true,Ordering::Release);
                }
            }
            return;
        }
        for source in sources {
            if !source.valid() {continue;}
            let id=source.id();
            if let Some(target) = archive {
                target.expect(&id);
            }
            if !s.entries.contains_key(&id) {
                if s.entries.len() >= ITEMS {
                    self.core.failed.store(true, Ordering::Release);
                    if let Some(target)=archive {target.missing_id(&id);}
                    continue;
                }
                s.entries.insert(
                    id.clone(),
                    Entry {
                        source,
                        image: None,
                        attempted: false,
                        queued: false,
                        targets: Vec::new(),
                    },
                );
            }
            let queue_room = s.queue.len() < 64;
            let entry = s.entries.get_mut(&id).unwrap();
            entry.targets.retain(|t| t.strong_count() > 0);
            if let Some(target) = archive {
                let already_registered = entry
                    .targets
                    .iter()
                    .any(|t| t.ptr_eq(&Arc::downgrade(target)));
                if !already_registered && entry.targets.len() < 8 {
                    entry.targets.push(Arc::downgrade(target));
                } else if !already_registered {
                    target.missing_id(&id);
                    self.core.failed.store(true,Ordering::Release);
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
                    if let Some(target)=archive {target.missing_id(&id);}
                }
            } else if !entry.queued && entry.attempted && entry.image.is_none() {
                // Failed or evicted immutable capture bytes cannot be silently
                // replaced by a later fetch of the same mutable URL.
                if let Some(target)=archive {target.missing_id(&id);}
            }
            if let Some(target) = archive {
                if s.entries.get(&id).is_some_and(|entry| entry.queued
                    && entry.targets.iter().any(|t| t.ptr_eq(&Arc::downgrade(target)))) {
                    target.admitted(&id);
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
                settle_failed_targets(&s);
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
        let now = Instant::now();
        for (id, entry) in &mut s.entries { retain_stopped_targets(id, entry, now); }
        let queue = std::mem::take(&mut s.queue);
        s.queue = queue.into_iter().filter(|id| s.entries.get(id).is_some_and(|e| !e.targets.is_empty())).collect();
        self.core.wake.notify_all();
    }
    pub(super) fn is_done(&self) -> bool {
        self.core.done.load(Ordering::Acquire)
    }
    pub(super) fn incomplete(&self) -> bool {
        self.core.failed.load(Ordering::Acquire)
    }
}
fn retain_stopped_targets(id: &str, entry: &mut Entry, now: Instant) {
    entry.targets.retain(|weak| {
        let Some(target) = weak.upgrade() else { return false; };
        if target.retain_on_stop(id, now) { true } else { target.missing_id(id); false }
    });
    if entry.targets.is_empty() { entry.queued = false; }
}
fn settle_failed_targets(state:&CacheState) {
    for (id,entry) in &state.entries {
        for target in entry.targets.iter().filter_map(Weak::upgrade) {target.missing_id(id);}
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
                let mut state=self.0.state.lock().unwrap_or_else(|p|p.into_inner());
                state.stopped=true;
                settle_failed_targets(&state);
            }
            self.0.done.store(true, Ordering::Release);
        }
    }
    let _done = Done(core.clone());
    loop {
        let (id, source, cached, attempted) = {
            let mut s = core.state.lock().unwrap_or_else(|p| p.into_inner());
            while s.queue.is_empty() && !s.stopped {
                s = core.wake.wait(s).unwrap_or_else(|p| p.into_inner());
            }
            if s.queue.is_empty() {
                return;
            }
            let id = s.queue.pop_front().unwrap();
            if s.stopped {
                let entry = s.entries.get_mut(&id).unwrap();
                retain_stopped_targets(&id, entry, Instant::now());
                if entry.targets.is_empty() { continue; }
            }
            let entry = s.entries.get(&id).unwrap();
            (
                id.clone(),
                entry.source.clone(),
                entry.image.clone(),
                entry.attempted,
            )
        };
        let fetched = cached.or_else(|| {
            (!attempted)
                .then(|| fetcher(&source).ok().map(Arc::new))
                .flatten()
        });
        let (image, targets) = {
            let mut s = core.state.lock().unwrap_or_else(|p| p.into_inner());
            if s.stopped {
                let entry = s.entries.get_mut(&id).unwrap();
                retain_stopped_targets(&id, entry, Instant::now());
                if entry.targets.is_empty() { continue; }
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
        // Targets stay unsettled while bytes are transferred out of the queue
        // and actually written. A failed sibling never settles these early.
        struct SettleTargets {id:String,targets:Vec<Weak<ArchiveAssets>>}
        impl Drop for SettleTargets {fn drop(&mut self){for target in self.targets.iter().filter_map(Weak::upgrade){target.missing_id(&self.id);}}}
        let targets=SettleTargets{id:id.clone(),targets};
        if let Some(image) = image {
            for target in targets.targets.iter().filter_map(|t| t.upgrade()) {
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

#[cfg(test)]
#[path="chat_assets_personalcon_tests.rs"]
mod personalcon_tests;

#[cfg(test)]
mod settlement_tests {
    use super::*;
    fn png() -> Blob {
        let mut bytes=std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2,2).write_to(&mut bytes,image::ImageFormat::Png).unwrap();
        blob(bytes.into_inner(),None).unwrap()
    }
    fn until(done:impl Fn()->bool) {
        let end=Instant::now()+Duration::from_secs(3);
        while !done(){assert!(Instant::now()<end);std::thread::sleep(Duration::from_millis(2));}
    }
    fn source(n:usize)->AssetSource {AssetSource::YoutubeEmoji(format!("https://yt3.ggpht.com/settle{n}"))}
    #[test]
    fn one_failed_id_does_not_settle_a_different_pending_image() {
        let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();
        let a=source(0).id();let b=source(1).id();target.expect(&a);target.expect(&b);
        target.missing_id(&a);assert!(target.pending());
        target.store(&b,&png()).unwrap();assert!(!target.pending());assert!(target.close());
        assert_eq!(target.state.lock().unwrap().stored.len(),1);
    }
    #[test]
    fn worker_failed_then_pending_target_drains_without_stopping_shared_receiver() {
        let release=Arc::new(AtomicBool::new(false));let gate=release.clone();
        let mut session=SessionAssets::new();session.fetcher=Arc::new(move|source|{
            if source.id()==self::source(0).id(){return Err("fixture".into());}
            while !gate.load(Ordering::Acquire){std::thread::sleep(Duration::from_millis(2));}
            Ok(png())
        });
        let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();
        session.observe_sources([source(0),source(1)],Some(&target));
        until(||target.state.lock().unwrap().settled.contains(&source(0).id()));
        assert!(target.pending());release.store(true,Ordering::Release);until(||!target.pending());
        assert_eq!(target.state.lock().unwrap().stored.len(),1);assert!(target.close());
        assert!(!session.core.state.lock().unwrap().stopped);session.stop();until(||session.is_done());
    }
    #[test]
    fn prior_failed_fetch_settles_new_target_without_retry_or_long_wait() {
        let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();
        let mut session=SessionAssets::new();session.fetcher=Arc::new(move|_|{count.fetch_add(1,Ordering::AcqRel);Err("fixture".into())});
        session.observe_sources([source(0)],None);
        until(||session.core.state.lock().unwrap().entries.get(&source(0).id()).is_some_and(|e|e.attempted));
        let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();
        session.observe_sources([source(0)],Some(&target));assert!(!target.pending());assert!(target.close());
        assert_eq!(calls.load(Ordering::Acquire),1);session.stop();until(||session.is_done());
    }
    #[test]
    fn stop_settles_queued_targets_as_failed_without_claiming_stored() {
        let release=Arc::new(AtomicBool::new(false));let gate=release.clone();
        let mut session=SessionAssets::new();session.fetcher=Arc::new(move|_|{while !gate.load(Ordering::Acquire){std::thread::sleep(Duration::from_millis(2));}Ok(png())});
        let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();
        session.observe_sources([source(0),source(1)],Some(&target));assert!(target.pending());
        session.stop();assert!(!target.pending());assert!(target.close());release.store(true,Ordering::Release);until(||session.is_done());
        assert_eq!(target.state.lock().unwrap().stored.len(),0);
    }
    #[test]
    fn queue_and_target_caps_settle_only_rejected_ids() {
        let session=SessionAssets::new();session.started.store(true,Ordering::Release);
        let root=tempfile::tempdir().unwrap();let target=ArchiveAssets::new(root.path()).unwrap();
        session.observe_sources((0..65).map(source),Some(&target));
        assert!(target.state.lock().unwrap().settled.contains(&source(64).id()));
        assert!(target.pending());session.stop();assert!(!target.pending());assert!(target.close());

        let session=SessionAssets::new();session.started.store(true,Ordering::Release);
        let roots=(0..9).map(|_|tempfile::tempdir().unwrap()).collect::<Vec<_>>();
        let targets=roots.iter().map(|r|ArchiveAssets::new(r.path()).unwrap()).collect::<Vec<_>>();
        for target in &targets{session.observe_sources([source(0)],Some(target));}
        assert!(!targets[8].pending());assert!(targets[0].pending());assert!(targets[8].close());
        session.stop();for target in targets.iter().take(8){assert!(!target.pending());assert!(target.close());}
    }
}

#[cfg(test)]
#[path="chat_assets_mission_tests.rs"]
mod mission_tests;
