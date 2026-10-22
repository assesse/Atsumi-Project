//! Read-only bounded replay of the job-owned chat archive. No account, network,
//! remote image or arbitrary path IPC. A receipt clock is explicitly approximate.
use super::{chat_assets, chat_parts, storage};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_LINE: usize = 512 * 1024;
const MAX_ROWS: usize = 120;
const MAX_TEXT: usize = 2048;
const PREPARE_BYTES: u64 = 8 * 1024 * 1024;
const PREPARE_UNITS: usize = 20_000;
const PREPARE_TIME: Duration = Duration::from_millis(75);
const CANCELLED: &str = "채팅 준비가 중단됐습니다. 다시 열어 주세요.";
const CHANGED: &str = "채팅 파일이 변경됐습니다. 다시 시도해 주세요.";
const LIMIT_ERROR: &str = "채팅 기록이 표시 한도를 넘었습니다. 원본은 보존됩니다.";
/// URLs are receive-only native data, never serialized into IPC or normalized archives.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatPart {
    Text {
        text: String,
    },
    Emoji {
        #[serde(rename = "assetId")]
        asset_id: String,
        alt: String,
        #[serde(skip)]
        source_url: Option<String>,
    },
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayChatItem {
    pub id: String,
    pub time_ms: Option<u64>,
    pub kind: String,
    pub author: String,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ChatPart>,
    pub amount: Option<String>,
    pub badges: Vec<String>,
    pub color: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NormalizedChatEvent {
    pub format: String,
    pub received_at_ms: u64,
    pub source_timestamp_ms: Option<u64>,
    pub message_id: Option<String>,
    pub kind: String,
    pub author: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ChatPart>,
    pub amount: Option<String>,
    pub badges: Vec<String>,
    pub color: Option<String>,
}
/// Normalize only public display fields. No cookie, endpoint, account ID or
/// raw server message ID crosses this contract; source IDs are one-way digests.
pub(super) fn normalize_public_line(
    provider: &str,
    bytes: &[u8],
) -> Result<Vec<NormalizedChatEvent>, String> {
    use sha2::{Digest, Sha256};
    if !matches!(provider, "youtube" | "soop") || bytes.len() > MAX_LINE {
        return Err("지원하지 않는 채팅 형식입니다.".into());
    }
    let parsed = parse_line(bytes, provider).map_err(|_| "채팅 기록을 읽지 못했습니다.")?;
    let received = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Ok(parsed
        .items
        .into_iter()
        .map(|row| NormalizedChatEvent {
            format: "atsumi-chat-v1".into(),
            received_at_ms: received,
            source_timestamp_ms: row.wall_ms.and_then(|v| v.try_into().ok()),
            message_id: row
                .source_id
                .map(|id| format!("{:x}", Sha256::digest(id.as_bytes()))),
            kind: row.item.kind,
            author: row.item.author,
            text: row.item.text,
            parts: row.item.parts,
            amount: row.item.amount,
            badges: row.item.badges,
            color: row.item.color,
        })
        .collect())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayChatPage {
    pub items: Vec<ReplayChatItem>,
    pub state: String,
    pub timeline_quality: String,
    pub next_at_ms: Option<u64>,
    pub cursor: Option<String>,
    pub previous_cursor: Option<String>,
    pub total: usize,
    pub malformed_lines: usize,
    pub unsupported_items: usize,
    pub reordered: usize,
    pub duplicates: usize,
    pub partial_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation: Option<ReplayPreparation>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayPreparation {
    pub scanned_bytes: u64,
    pub total_bytes: u64,
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: u128,
    identity: (u64, u64),
}
#[derive(Clone)]
struct Location {
    offset: u64,
    row: u16,
    at: Option<u64>,
}
struct Entry {
    path: PathBuf,
    stamp: Stamp,
    anchor_stamp: Option<Stamp>,
    provider: String,
    origin: Option<i64>,
    job_id: String,
    parts: Option<chat_parts::ReplaySource>,
    cancelled: AtomicBool,
    index: Mutex<Option<Index>>,
}
struct Index {
    database: Connection,
    progress: Progress,
    parts: PartProgress,
    // Both SQLite and retained read handles must close before their slot can be
    // reused. This includes evicted entries still held by an in-flight call.
    _permit: DatabasePermit,
}
#[derive(Default)]
struct PartProgress {
    files: Vec<HeldPart>,
    verified: usize,
    hash: sha2::Sha256,
}
struct HeldPart {
    reader: chat_parts::OwnedPart,
    stamp: Stamp,
}
#[derive(Clone, Default)]
struct Progress {
    offset: u64,
    previous: Option<u64>,
    any_offset: bool,
    any_wall: bool,
    missing: bool,
    ready: bool,
    total: usize,
    malformed: usize,
    unsupported: usize,
    reordered: usize,
    duplicates: usize,
}
impl Progress {
    fn quality(&self) -> &'static str {
        if self.missing {
            "missing-anchor"
        } else if self.any_wall {
            "receive-time-approximate"
        } else if self.any_offset {
            "native-offset"
        } else {
            "missing-anchor"
        }
    }
}
struct DatabasePermit(Arc<AtomicUsize>);
impl Drop for DatabasePermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Default)]
struct IndexCache {
    registry: Mutex<Registry>,
    databases: Arc<AtomicUsize>,
}
#[derive(Default)]
struct Registry {
    entries: VecDeque<Arc<Entry>>,
    tickets: Vec<Weak<ReadTicket>>,
}
struct ReadTicket {
    directory: PathBuf,
    cancelled: AtomicBool,
}
impl IndexCache {
    fn register(&self, directory: &Path) -> Result<Arc<ReadTicket>, String> {
        let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        registry.tickets.retain(|ticket| ticket.strong_count() > 0);
        // The IPC gate admits at most two calls. Also bound direct native callers
        // rather than retaining an unbounded historical directory/epoch map.
        if registry.tickets.len() >= 2 {
            return Err("다른 채팅 조회가 진행 중입니다. 잠시 후 다시 시도해 주세요.".into());
        }
        let ticket = Arc::new(ReadTicket {
            directory: directory.into(),
            cancelled: AtomicBool::new(false),
        });
        registry.tickets.push(Arc::downgrade(&ticket));
        Ok(ticket)
    }
    fn permit(&self) -> Option<DatabasePermit> {
        self.databases
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .ok()
            .map(|_| DatabasePermit(self.databases.clone()))
    }
    fn entry(
        &self,
        path: PathBuf,
        signature: Stamp,
        provider: &str,
        origin: Option<i64>,
        anchor_stamp: Option<Stamp>,
        ticket: &ReadTicket,
        job_id: &str,
        parts: Option<chat_parts::ReplaySource>,
    ) -> Result<Arc<Entry>, String> {
        let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        if ticket.cancelled.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        let cache = &mut registry.entries;
        if let Some(entry) = cache.iter().find(|entry| {
            entry.path == path
                && entry.provider == provider
                && entry.stamp == signature
                && entry.anchor_stamp == anchor_stamp
                && entry.job_id == job_id
                && entry.parts.as_ref().map(|source| &source.generation)
                    == parts.as_ref().map(|source| &source.generation)
                && !entry.cancelled.load(Ordering::Acquire)
        }) {
            return Ok(entry.clone());
        }
        let mut retired = Vec::new();
        let mut retained = VecDeque::new();
        while let Some(entry) = cache.pop_front() {
            if entry.path == path || entry.cancelled.load(Ordering::Acquire) {
                entry.cancelled.store(true, Ordering::Release);
                retired.push(entry);
            } else {
                retained.push_back(entry);
            }
        }
        while retained.len() >= 2 {
            if let Some(entry) = retained.pop_front() {
                entry.cancelled.store(true, Ordering::Release);
                retired.push(entry);
            }
        }
        let entry = Arc::new(Entry {
            path,
            stamp: signature,
            anchor_stamp,
            provider: provider.into(),
            origin,
            job_id: job_id.into(),
            parts,
            cancelled: AtomicBool::new(false),
            index: Mutex::new(None),
        });
        retained.push_back(entry.clone());
        *cache = retained;
        drop(registry);
        // SQLite destruction and in-flight work never run under the registry lock.
        drop(retired);
        Ok(entry)
    }
    fn cancel_matching(&self, matches: impl Fn(&Path) -> bool) {
        let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        // A ticket precedes disk IO. Closing A cancels its late registration,
        // without invalidating an unrelated B or waiting for either build lock.
        registry.tickets.retain(|ticket| {
            if let Some(ticket) = ticket.upgrade() {
                if matches(&ticket.directory) {
                    ticket.cancelled.store(true, Ordering::Release);
                }
                true
            } else {
                false
            }
        });
        let cache = &mut registry.entries;
        let mut retired = Vec::new();
        let mut retained = VecDeque::new();
        while let Some(entry) = cache.pop_front() {
            if entry.path.parent().is_some_and(&matches) {
                entry.cancelled.store(true, Ordering::Release);
                retired.push(entry);
            } else {
                retained.push_back(entry);
            }
        }
        *cache = retained;
        drop(registry);
        drop(retired);
    }
    fn discard(&self, entry: &Arc<Entry>) {
        entry.cancelled.store(true, Ordering::Release);
        let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        let retired = registry
            .entries
            .iter()
            .position(|candidate| Arc::ptr_eq(candidate, entry))
            .and_then(|position| registry.entries.remove(position));
        drop(registry);
        drop(retired);
    }
}
static CACHE: OnceLock<IndexCache> = OnceLock::new();
/// A closed player need not retain its temporary index. In-flight readers keep
/// their Arc until returning; SQLite then closes and deletes its own temp file.
pub(super) fn release_directory(directory: &Path) {
    if let Some(cache) = CACHE.get() {
        cache.cancel_matching(|candidate| candidate == directory);
    }
}
pub(super) fn release_all() {
    if let Some(cache) = CACHE.get() {
        cache.cancel_matching(|_| true);
    }
}
struct Parsed {
    item: ReplayChatItem,
    wall_ms: Option<i64>,
    offset_ms: Option<i64>,
    source_id: Option<String>,
}
#[derive(Default)]
struct ParsedLine {
    items: Vec<Parsed>,
    unsupported: usize,
}
fn number(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(limit)
        .collect()
}
fn rich(value: &Value, limit: usize) -> String {
    if let Some(text) = value["simpleText"].as_str() {
        return clean(text, limit);
    }
    let mut result = String::new();
    for run in value["runs"].as_array().into_iter().flatten().take(128) {
        if let Some(text) = run["text"].as_str() {
            result.push_str(text);
        } else if let Some(emoji) = run.get("emoji") {
            if let Some(text) = emoji["shortcuts"]
                .as_array()
                .and_then(|v| v.first())
                .and_then(Value::as_str)
                .or_else(|| emoji["image"]["accessibility"]["accessibilityData"]["label"].as_str())
            {
                result.push_str(text);
            } else {
                result.push_str("[이모티콘]");
            }
        }
        if result.len() > limit * 4 {
            break;
        }
    }
    clean(&result, limit)
}
fn literal_color(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    (s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| s.to_string())
}
fn youtube_parts(value: &Value) -> Vec<ChatPart> {
    let mut parts = Vec::new();
    let mut remaining = MAX_TEXT;
    for run in value["runs"].as_array().into_iter().flatten().take(128) {
        if remaining == 0 {
            break;
        }
        if let Some(text) = run["text"].as_str() {
            let text = clean(text, remaining);
            remaining = remaining.saturating_sub(text.chars().count());
            if !text.is_empty() {
                parts.push(ChatPart::Text { text });
            }
        } else if let Some(emoji) = run.get("emoji") {
            let alt = clean(
                emoji["shortcuts"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .or_else(|| {
                        emoji["image"]["accessibility"]["accessibilityData"]["label"].as_str()
                    })
                    .unwrap_or("[이모티콘]"),
                remaining.min(80),
            );
            remaining = remaining.saturating_sub(alt.chars().count());
            let source = emoji["image"]["thumbnails"]
                .as_array()
                .into_iter()
                .flatten()
                .take(8)
                .filter_map(|v| v["url"].as_str())
                .find_map(chat_assets::sanitize_url);
            if let Some(source) = source {
                parts.push(ChatPart::Emoji {
                    asset_id: chat_assets::asset_id(&source),
                    alt,
                    source_url: Some(source),
                });
            } else {
                parts.push(ChatPart::Text { text: alt });
            }
        }
    }
    parts
}
fn stored_parts(value: &Value) -> Vec<ChatPart> {
    let mut remaining = MAX_TEXT;
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|part| {
            if remaining == 0 {
                return None;
            }
            let result = match part["type"].as_str()? {
                "text" => ChatPart::Text {
                    text: clean(part["text"].as_str()?, remaining),
                },
                "emoji" => {
                    let id = part["assetId"].as_str()?;
                    if !chat_assets::valid_id(id) {
                        return None;
                    }
                    ChatPart::Emoji {
                        asset_id: id.into(),
                        alt: clean(
                            part["alt"].as_str().unwrap_or("[이모티콘]"),
                            remaining.min(80),
                        ),
                        source_url: None,
                    }
                }
                _ => return None,
            };
            remaining = remaining.saturating_sub(match &result {
                ChatPart::Text { text } => text.chars().count(),
                ChatPart::Emoji { alt, .. } => alt.chars().count(),
            });
            Some(result)
        })
        .collect()
}
fn blank(kind: &str) -> ReplayChatItem {
    ReplayChatItem {
        id: String::new(),
        time_ms: None,
        kind: kind.into(),
        author: String::new(),
        text: String::new(),
        parts: Vec::new(),
        amount: None,
        badges: Vec::new(),
        color: None,
    }
}
fn parse_soop(value: &Value) -> ParsedLine {
    let mut result = ParsedLine::default();
    let kind = match value["kind"].as_str() {
        Some("chat") => "chat",
        Some("gap") => "gap",
        Some("connected") => "connected",
        Some("ended") => "ended",
        _ => {
            result.unsupported = 1;
            return result;
        }
    };
    let mut item = blank(kind);
    item.text = clean(value["text"].as_str().unwrap_or(""), MAX_TEXT);
    if kind == "chat" {
        item.author = clean(value["nickname"].as_str().unwrap_or(""), 160);
        item.color = literal_color(&value["extra"]["color"]);
        if let Some(months) =
            number(&value["extra"]["subscriptionMonth"]).filter(|n| (1..=1200).contains(n))
        {
            item.badges.push(format!("구독 {months}개월"));
        }
    }
    let source_id = value["messageId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned);
    result.items.push(Parsed {
        item,
        wall_ms: number(&value["receivedAtMs"]),
        offset_ms: None,
        source_id,
    });
    result
}
fn parse_youtube(value: &Value) -> ParsedLine {
    let mut result = ParsedLine::default();
    let replay = value.get("replayChatItemAction");
    let offset = if value["isLive"].as_bool() == Some(true) {
        None
    } else {
        replay.and_then(|v| number(&v["videoOffsetTimeMsec"]))
    };
    let actions = replay
        .and_then(|v| v["actions"].as_array())
        .or_else(|| value["actions"].as_array());
    let single;
    let actions = if let Some(actions) = actions {
        actions.as_slice()
    } else {
        single = [value.clone()];
        &single
    };
    for action in actions.iter().take(256) {
        let Some(item) = action.get("addChatItemAction").and_then(|a| a.get("item")) else {
            continue;
        };
        let selected = [
            ("liveChatTextMessageRenderer", "chat"),
            ("liveChatPaidMessageRenderer", "donation"),
            ("liveChatPaidStickerRenderer", "donation"),
            ("liveChatMembershipItemRenderer", "membership"),
        ]
        .into_iter()
        .find_map(|(key, kind)| item.get(key).map(|v| (v, kind)));
        let Some((renderer, kind)) = selected else {
            result.unsupported += 1;
            continue;
        };
        let mut item = blank(kind);
        item.author = rich(&renderer["authorName"], 160);
        item.text = rich(&renderer["message"], MAX_TEXT);
        item.parts = youtube_parts(&renderer["message"]);
        if item.text.is_empty() {
            item.text = rich(&renderer["headerSubtext"], MAX_TEXT);
            item.parts = youtube_parts(&renderer["headerSubtext"]);
        }
        if item.text.is_empty() && renderer.get("sticker").is_some() {
            item.text = clean(
                renderer["sticker"]["accessibility"]["accessibilityData"]["label"]
                    .as_str()
                    .unwrap_or("[스티커]"),
                MAX_TEXT,
            );
        }
        let amount = rich(&renderer["purchaseAmountText"], 80);
        item.amount = (!amount.is_empty()).then_some(amount);
        for badge in renderer["authorBadges"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
        {
            if let Some(label) = badge["liveChatAuthorBadgeRenderer"]["tooltip"].as_str() {
                item.badges.push(clean(label, 80));
            }
        }
        // No color inference from a palette, role, amount or provider-specific integer.
        let source_id = renderer["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .map(str::to_owned);
        result.items.push(Parsed {
            item,
            wall_ms: number(&renderer["timestampUsec"]).map(|v| v / 1000),
            offset_ms: offset,
            source_id,
        });
    }
    result
}
fn parse_line(bytes: &[u8], provider: &str) -> Result<ParsedLine, ()> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    if value["format"] == "atsumi-chat-v1" {
        let kind = value["kind"].as_str().ok_or(())?;
        if !matches!(
            kind,
            "chat" | "donation" | "membership" | "gap" | "connected" | "ended"
        ) {
            return Ok(ParsedLine {
                items: Vec::new(),
                unsupported: 1,
            });
        }
        let mut item = blank(kind);
        item.author = clean(value["author"].as_str().unwrap_or(""), 160);
        item.text = clean(value["text"].as_str().unwrap_or(""), MAX_TEXT);
        item.parts = stored_parts(&value["parts"]);
        item.amount = value["amount"].as_str().map(|s| clean(s, 80));
        item.color = literal_color(&value["color"]);
        item.badges = value["badges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(8)
            .map(|v| clean(v, 80))
            .collect();
        return Ok(ParsedLine {
            items: vec![Parsed {
                item,
                wall_ms: number(&value["sourceTimestampMs"])
                    .or_else(|| number(&value["receivedAtMs"])),
                offset_ms: if provider == "soop" {
                    number(&value["videoOffsetMs"]).filter(|n| *n >= 0)
                } else {
                    None
                },
                source_id: value["messageId"]
                    .as_str()
                    .filter(|s| s.len() <= 128)
                    .map(str::to_owned),
            }],
            unsupported: 0,
        });
    }
    Ok(if provider == "soop" {
        parse_soop(&value)
    } else {
        parse_youtube(&value)
    })
}
fn stamp(file: &File) -> Result<Stamp, String> {
    let meta = file
        .metadata()
        .map_err(|_| "채팅 파일을 확인하지 못했습니다.")?;
    #[cfg(windows)]
    let identity = {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| "채팅 파일을 확인하지 못했습니다.")?;
        }
        (
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        )
    };
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(any(windows, unix)))]
    let identity = (0, 0);
    Ok(Stamp {
        length: meta.len(),
        identity,
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|v| v.as_nanos())
            .unwrap_or(0),
    })
}
fn open(path: &Path, limit: u64) -> Result<File, String> {
    let meta = storage::regular(path)?;
    if meta.len() > limit {
        return Err(LIMIT_ERROR.into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|_| "채팅 파일을 읽지 못했습니다.")?;
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
                .map_err(|_| "채팅 파일을 확인하지 못했습니다.")?;
        }
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & 0x400 != 0 {
            return Err("연결된 채팅 파일은 열 수 없습니다.".into());
        }
    }
    if file
        .metadata()
        .map_err(|_| "채팅 파일을 확인하지 못했습니다.")?
        .len()
        != meta.len()
    {
        return Err("채팅 파일이 변경됐습니다. 다시 시도해 주세요.".into());
    }
    Ok(file)
}
fn source(directory: &Path, provider: &str) -> Result<Option<PathBuf>, String> {
    storage::safe_path(directory)?;
    let names: &[&str] = match provider {
        "soop" => &["chat.events.jsonl", "chat.soop.jsonl"],
        "youtube" => &[
            "chat.events.jsonl",
            "chat.live_chat.json",
            "chat.live_chat.json.part",
        ],
        _ => return Err("채팅 서비스를 확인해 주세요.".into()),
    };
    for name in names {
        let path = directory.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Ok(Some(path)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("채팅 파일을 확인하지 못했습니다.".into()),
        }
    }
    Ok(None)
}
fn anchor(directory: &Path, provider: &str, job_id: &str) -> Result<(Option<i64>, Option<Stamp>), String> {
    let path = directory.join("chat-timeline.json");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(_) => return Err("채팅 시간 정보를 확인하지 못했습니다.".into()),
        Ok(_) => {}
    }
    let mut file = open(&path, 4096)?;
    let signature = stamp(&file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| "채팅 시간 정보를 읽지 못했습니다.")?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "채팅 시간 정보가 손상됐습니다.")?;
    if value.get("version").is_some_and(|version| version != &Value::from(1))
        || ["anchorMethod", "originMs", "evidence", "marker"].iter().any(|key| value.get(*key).is_some()) {
        if value["version"] != 2 { return Err("지원하지 않는 채팅 시간 정보입니다.".into()); }
        let origin = super::youtube_spool::raw_replay_origin(&bytes, directory, provider, job_id)
            .map_err(|_| "채팅 시간 정보가 손상됐습니다.")?;
        return Ok((Some(i64::try_from(origin).map_err(|_| "채팅 시간 정보가 손상됐습니다.")?), Some(signature)));
    }
    if value["timelineQuality"] != "receive-time-approximate" {
        return Ok((None, Some(signature)));
    }
    let start = number(&value["firstSegmentCommittedAtMs"])
        .zip(number(&value["firstSegmentDurationMs"]))
        .and_then(|(end, duration)| {
            if !(1..=120000).contains(&duration)
                || !(1_000_000_000_000..=9_000_000_000_000).contains(&end)
            {
                None
            } else {
                end.checked_sub(duration)
            }
        });
    Ok((start, Some(signature)))
}
fn line(reader: &mut BufReader<&mut File>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "채팅 기록을 읽지 못했습니다.")?;
    if bytes.len() > MAX_LINE {
        return Err(LIMIT_ERROR.into());
    }
    Ok(bytes)
}
#[derive(Clone, Copy)]
struct SliceBudget {
    bytes: u64,
    units: usize,
    time: Duration,
}
const SLICE: SliceBudget = SliceBudget {
    bytes: PREPARE_BYTES,
    units: PREPARE_UNITS,
    time: PREPARE_TIME,
};
fn check_cancelled(entry: &Entry) -> Result<(), String> {
    if entry.cancelled.load(Ordering::Acquire) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}
fn validate_source(entry: &Entry, file: &File) -> Result<(), String> {
    check_cancelled(entry)?;
    let directory = entry.path.parent().ok_or(CHANGED)?;
    if stamp(file)? != entry.stamp || anchor(directory, &entry.provider, &entry.job_id)?.1 != entry.anchor_stamp {
        return Err(CHANGED.into());
    }
    Ok(())
}
fn validate_parts(entry: &Entry) -> Result<(), String> {
    check_cancelled(entry)?;
    let directory = entry.path.parent().ok_or(CHANGED)?;
    let expected = entry.parts.as_ref().ok_or(CHANGED)?;
    let actual =
        chat_parts::replay_source(directory, &entry.provider, &entry.job_id)?.ok_or(CHANGED)?;
    if actual.generation != expected.generation || anchor(directory, &entry.provider, &entry.job_id)?.1 != entry.anchor_stamp {
        return Err(CHANGED.into());
    }
    Ok(())
}
fn partial(entry: &Entry) -> bool {
    entry.parts.as_ref().is_some_and(|source| !source.complete)
        || entry
            .path
            .extension()
            .is_some_and(|extension| extension == "part")
}
fn insert_line(
    entry: &Entry,
    progress: &mut Progress,
    insert: &mut rusqlite::Statement<'_>,
    bytes: &[u8],
    units: &mut usize,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    *units += 1;
    match parse_line(bytes, &entry.provider) {
        Err(()) => progress.malformed += 1,
        Ok(parsed) => {
            progress.unsupported += parsed.unsupported;
            *units += parsed.items.len();
            for (row, parsed) in parsed.items.into_iter().enumerate() {
                check_cancelled(entry)?;
                let at = if let Some(relative) = parsed.offset_ms {
                    progress.any_offset = true;
                    Some(relative)
                } else if let Some((wall, start)) = parsed.wall_ms.zip(entry.origin) {
                    progress.any_wall = true;
                    wall.checked_sub(start)
                } else {
                    progress.missing = true;
                    None
                };
                if at.is_some_and(|n| n < 0) {
                    continue;
                }
                let at = at.and_then(|n| u64::try_from(n).ok());
                if at.is_some_and(|n| n > 7 * 24 * 3600 * 1000) {
                    progress.missing = true;
                    continue;
                }
                let fingerprint = parsed
                    .source_id
                    .map(|id| Sha256::digest(id.as_bytes()).to_vec());
                let inserted = insert
                    .execute(params![
                        progress.total as i64,
                        progress.offset as i64,
                        row as i64,
                        at.map(|n| n as i64),
                        fingerprint
                    ])
                    .map_err(|_| LIMIT_ERROR.to_string())?;
                if inserted == 0 {
                    progress.duplicates += 1;
                    continue;
                }
                if at
                    .zip(progress.previous)
                    .is_some_and(|(next, prior)| next < prior)
                {
                    progress.reordered += 1;
                }
                if at.is_some() {
                    progress.previous = at;
                }
                progress.total += 1;
            }
        }
    }
    Ok(())
}
/// Transactions contain one bounded slice, not the entire archive. Bytes and
/// parsed-row/line units are hard bounds; time is cooperative between lines.
fn advance(
    entry: &Entry,
    index: &mut Index,
    file: &mut File,
    budget: SliceBudget,
) -> Result<(), String> {
    validate_source(entry, file)?;
    if index.progress.ready {
        return Ok(());
    }
    let started = Instant::now();
    let mut progress = index.progress.clone();
    let start_offset = progress.offset;
    let mut units = 0usize;
    file.seek(SeekFrom::Start(progress.offset))
        .map_err(|_| "채팅 위치를 읽지 못했습니다.")?;
    let mut reader = BufReader::new(file);
    let transaction = index
        .database
        .transaction()
        .map_err(|_| LIMIT_ERROR.to_string())?;
    let mut insert = transaction
        .prepare("INSERT OR IGNORE INTO rows(seq,off,item,at,fingerprint) VALUES(?1,?2,?3,?4,?5)")
        .map_err(|_| LIMIT_ERROR.to_string())?;
    loop {
        check_cancelled(entry)?;
        if progress.offset == entry.stamp.length {
            progress.ready = true;
            break;
        }
        // A source line is at most512KiB and parses at most256 public rows.
        // Reserve a full next line so no boundary can exceed either hard cap.
        if progress.offset > start_offset
            && (budget.bytes.saturating_sub(progress.offset - start_offset) < MAX_LINE as u64
                || budget.units.saturating_sub(units) < 257
                || started.elapsed() >= budget.time)
        {
            break;
        }
        let bytes = line(&mut reader)?;
        if bytes.is_empty() {
            return Err(CHANGED.into());
        }
        let length = bytes.len() as u64;
        if progress.offset.saturating_add(length) > entry.stamp.length {
            return Err(CHANGED.into());
        }
        insert_line(entry, &mut progress, &mut insert, &bytes, &mut units)?;
        progress.offset += length;
    }
    drop(insert);
    validate_source(entry, reader.get_ref())?;
    transaction.commit().map_err(|_| LIMIT_ERROR.to_string())?;
    check_cancelled(entry)?;
    index.progress = progress;
    Ok(())
}
/// V2 parts are a virtual concatenation. SQL offsets remain global and point at
/// a line wholly inside one sealed part; no concatenated copy is ever created.
fn advance_parts(entry: &Entry, index: &mut Index, budget: SliceBudget) -> Result<(), String> {
    use sha2::Digest;
    validate_parts(entry)?;
    if index.progress.ready {
        return Ok(());
    }
    let source = entry.parts.as_ref().ok_or(CHANGED)?;
    let started = Instant::now();
    let mut progress = index.progress.clone();
    let start_offset = progress.offset;
    let mut units = 0usize;
    let mut verified = index.parts.verified;
    let mut hash = index.parts.hash.clone();
    let mut part_start: u64 = source.events[..verified]
        .iter()
        .map(|part| part.bytes)
        .sum();
    let transaction = index
        .database
        .transaction()
        .map_err(|_| LIMIT_ERROR.to_string())?;
    let mut insert = transaction
        .prepare("INSERT OR IGNORE INTO rows(seq,off,item,at,fingerprint) VALUES(?1,?2,?3,?4,?5)")
        .map_err(|_| LIMIT_ERROR.to_string())?;
    while verified < source.events.len() {
        check_cancelled(entry)?;
        if units > 0
            && (budget.bytes.saturating_sub(progress.offset - start_offset) < MAX_LINE as u64
                || budget.units.saturating_sub(units) < 258
                || started.elapsed() >= budget.time)
        {
            break;
        }
        let part = &source.events[verified];
        if index.parts.files.len() == verified {
            // Acquired only after the entry owns one of the two DB/handle slots.
            let reader = chat_parts::open_replay_part(source, verified)?;
            let signature = stamp(&reader.file)?;
            if signature.length != part.bytes {
                return Err(CHANGED.into());
            }
            index.parts.files.push(HeldPart {
                reader,
                stamp: signature,
            });
        }
        let held = index.parts.files.get_mut(verified).ok_or(CHANGED)?;
        if stamp(&held.reader.file)? != held.stamp {
            return Err(CHANGED.into());
        }
        let mut local = progress.offset.checked_sub(part_start).ok_or(CHANGED)?;
        if local > part.bytes {
            return Err(CHANGED.into());
        }
        held.reader
            .file
            .seek(SeekFrom::Start(local))
            .map_err(|_| CHANGED)?;
        let mut reader = BufReader::new(&mut held.reader.file);
        while local < part.bytes {
            check_cancelled(entry)?;
            if units > 0
                && (budget.bytes.saturating_sub(progress.offset - start_offset) < MAX_LINE as u64
                    || budget.units.saturating_sub(units) < 258
                    || started.elapsed() >= budget.time)
            {
                break;
            }
            let bytes = line(&mut reader)?;
            if bytes.is_empty()
                || bytes.last() != Some(&b'\n')
                || local.saturating_add(bytes.len() as u64) > part.bytes
            {
                return Err("채팅 조각의 끝부분이 손상됐습니다. 원본은 보존됩니다.".into());
            }
            hash.update(&bytes);
            insert_line(entry, &mut progress, &mut insert, &bytes, &mut units)?;
            local += bytes.len() as u64;
            progress.offset += bytes.len() as u64;
        }
        if stamp(reader.get_ref())? != held.stamp {
            return Err(CHANGED.into());
        }
        if local != part.bytes {
            break;
        }
        if format!("{:x}", hash.clone().finalize()) != part.sha256 {
            return Err("채팅 조각 검증에 실패했습니다. 원본은 보존됩니다.".into());
        }
        units += 1;
        verified += 1;
        part_start += part.bytes;
        hash = sha2::Sha256::new();
    }
    progress.ready = verified == source.events.len() && progress.offset == source.total_event_bytes;
    drop(insert);
    validate_parts(entry)?;
    transaction.commit().map_err(|_| LIMIT_ERROR.to_string())?;
    check_cancelled(entry)?;
    index.progress = progress;
    index.parts.verified = verified;
    index.parts.hash = hash;
    Ok(())
}
fn index_database() -> Result<Connection, String> {
    let database =
        Connection::open("").map_err(|_| "채팅 탐색용 임시 공간을 만들지 못했습니다.")?;
    database.execute_batch("PRAGMA page_size=4096; PRAGMA max_page_count=131072; PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; PRAGMA mmap_size=0; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; CREATE TABLE rows(seq INTEGER PRIMARY KEY,off INTEGER NOT NULL,item INTEGER NOT NULL,at INTEGER,fingerprint BLOB UNIQUE); CREATE INDEX by_time ON rows(at,seq);").map_err(|_|LIMIT_ERROR.to_string())?;
    Ok(database)
}
fn empty(state: &str) -> ReplayChatPage {
    ReplayChatPage {
        items: Vec::new(),
        state: state.into(),
        timeline_quality: "missing-anchor".into(),
        next_at_ms: None,
        cursor: None,
        previous_cursor: None,
        total: 0,
        malformed_lines: 0,
        unsupported_items: 0,
        reordered: 0,
        duplicates: 0,
        partial_file: false,
        preparation: None,
    }
}
fn preparing(entry: &Entry, offset: u64) -> ReplayChatPage {
    let mut page = empty("indexing");
    page.preparation = Some(ReplayPreparation {
        scanned_bytes: offset,
        total_bytes: entry.stamp.length,
    });
    page.partial_file = partial(entry);
    page
}
/// The caller passes only a validated, completed owned job directory. Cursor is
/// an event ordinal for manual chronology; mapped video seeks pass no cursor.
pub(super) fn read_page(
    directory: &Path,
    provider: &str,
    position_ms: u64,
    cursor: Option<&str>,
    limit: usize,
) -> Result<ReplayChatPage, String> {
    read_page_for_job(directory, provider, "", position_ms, cursor, limit)
}
pub(super) fn read_page_for_job(
    directory: &Path,
    provider: &str,
    job_id: &str,
    position_ms: u64,
    cursor: Option<&str>,
    limit: usize,
) -> Result<ReplayChatPage, String> {
    read_page_for_job_using(
        CACHE.get_or_init(Default::default),
        directory,
        provider,
        job_id,
        position_ms,
        cursor,
        limit,
        SLICE,
    )
}
fn read_page_using(
    cache: &IndexCache,
    directory: &Path,
    provider: &str,
    position_ms: u64,
    cursor: Option<&str>,
    limit: usize,
    budget: SliceBudget,
) -> Result<ReplayChatPage, String> {
    read_page_for_job_using(
        cache,
        directory,
        provider,
        "",
        position_ms,
        cursor,
        limit,
        budget,
    )
}
fn read_page_for_job_using(
    cache: &IndexCache,
    directory: &Path,
    provider: &str,
    job_id: &str,
    position_ms: u64,
    cursor: Option<&str>,
    limit: usize,
    budget: SliceBudget,
) -> Result<ReplayChatPage, String> {
    // Reject malformed cursors before opening or preparing any derived index.
    let manual = cursor
        .map(|s| {
            if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err("채팅 목록 위치가 올바르지 않습니다.");
            }
            s.parse::<usize>()
                .map_err(|_| "채팅 목록 위치가 올바르지 않습니다.")
        })
        .transpose()?;
    let limit = limit.clamp(1, MAX_ROWS);
    let ticket = cache.register(directory)?;
    let opened = (|| {
        // Only the verified service supplies a job ID. The legacy wrapper passes
        // an empty ID, which a present V2 marker must reject rather than guess.
        if let Some(parts) = chat_parts::replay_source(directory, provider, job_id)? {
            let (origin, anchor_stamp) = anchor(directory, provider, job_id)?;
            let signature = Stamp {
                length: parts.total_event_bytes,
                modified: 0,
                identity: (0, 0),
            };
            return Ok(Some((
                directory.join("chat.events.jsonl"),
                None,
                signature,
                origin,
                anchor_stamp,
                Some(parts),
            )));
        }
        let Some(path) = source(directory, provider)? else {
            return Ok(None);
        };
        let file = open(&path, MAX_FILE)?;
        let signature = stamp(&file)?;
        let (origin, anchor_stamp) = anchor(directory, provider, job_id)?;
        Ok::<_, String>(Some((
            path,
            Some(file),
            signature,
            origin,
            anchor_stamp,
            None,
        )))
    })();
    let (path, mut file, signature, origin, anchor_stamp, parts) = match opened {
        Ok(Some(opened)) => opened,
        Ok(None) => {
            cache.cancel_matching(|candidate| candidate == directory);
            return Ok(empty("missing"));
        }
        Err(error) => {
            cache.cancel_matching(|candidate| candidate == directory);
            return Err(error);
        }
    };
    let entry = cache.entry(
        path,
        signature,
        provider,
        origin,
        anchor_stamp,
        &ticket,
        job_id,
        parts,
    )?;
    let result = read_entry(
        cache,
        &entry,
        file.as_mut(),
        position_ms,
        manual,
        limit,
        budget,
    );
    if result.is_err() {
        cache.discard(&entry);
    }
    result
}
fn read_entry(
    cache: &IndexCache,
    entry: &Entry,
    mut file: Option<&mut File>,
    position_ms: u64,
    manual: Option<usize>,
    limit: usize,
    budget: SliceBudget,
) -> Result<ReplayChatPage, String> {
    check_cancelled(entry)?;
    let mut state = entry
        .index
        .lock()
        .map_err(|_| "채팅 탐색 상태를 확인하지 못했습니다.")?;
    if entry.parts.is_some() {
        validate_parts(entry)?;
    } else {
        validate_source(entry, file.as_deref().ok_or(CHANGED)?)?;
    }
    if state.is_none() {
        let Some(permit) = cache.permit() else {
            // An evicted in-flight reader still owns its bounded DB slot. It
            // observes cancellation and releases it without growing the cache.
            return Ok(preparing(entry, 0));
        };
        *state = Some(Index {
            database: index_database()?,
            _permit: permit,
            progress: Progress::default(),
            parts: PartProgress::default(),
        });
    }
    let index = state.as_mut().ok_or(CANCELLED)?;
    if entry.parts.is_some() {
        advance_parts(entry, index, budget)?;
    } else {
        advance(entry, index, file.as_deref_mut().ok_or(CHANGED)?, budget)?;
    }
    if !index.progress.ready {
        check_cancelled(entry)?;
        return Ok(preparing(entry, index.progress.offset));
    }
    let progress = index.progress.clone();
    let unmapped = progress.quality() == "missing-anchor";
    let manual_page = unmapped || manual.is_some();
    let start = manual.unwrap_or(0).min(progress.total);
    let (locations, next_at_ms) = {
        let db = &index.database;
        let sql = if manual_page {
            "SELECT off,item,at FROM rows WHERE seq>=?1 ORDER BY seq LIMIT ?2"
        } else {
            "SELECT off,item,at FROM rows WHERE at<=?1 ORDER BY at DESC,seq DESC LIMIT ?2"
        };
        let bound = if manual_page {
            start as i64
        } else {
            position_ms.min(i64::MAX as u64) as i64
        };
        let mut statement = db
            .prepare(sql)
            .map_err(|_| "채팅 위치를 찾지 못했습니다.")?;
        let values = statement
            .query_map(params![bound, limit as i64], |row| {
                Ok(Location {
                    offset: row.get::<_, i64>(0)? as u64,
                    row: row.get::<_, i64>(1)? as u16,
                    at: if unmapped {
                        None
                    } else {
                        row.get::<_, Option<i64>>(2)?.map(|v| v as u64)
                    },
                })
            })
            .map_err(|_| "채팅 위치를 찾지 못했습니다.")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "채팅 위치를 읽지 못했습니다.")?;
        let next = if manual_page {
            None
        } else {
            db.query_row(
                "SELECT at FROM rows WHERE at>?1 ORDER BY at,seq LIMIT 1",
                [bound],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|_| "다음 채팅 위치를 찾지 못했습니다.")?
            .map(|v| v as u64)
        };
        (values, next)
    };
    let mut items = Vec::new();
    let mut output_bytes = 2048usize;
    for loc in &locations {
        check_cancelled(entry)?;
        let bytes = if let Some(source) = &entry.parts {
            let mut start = 0u64;
            let ordinal = source
                .events
                .iter()
                .position(|part| {
                    if loc.offset < start.saturating_add(part.bytes) {
                        true
                    } else {
                        start += part.bytes;
                        false
                    }
                })
                .ok_or(CHANGED)?;
            let held = index.parts.files.get_mut(ordinal).ok_or(CHANGED)?;
            if stamp(&held.reader.file)? != held.stamp {
                return Err(CHANGED.into());
            }
            held.reader
                .file
                .seek(SeekFrom::Start(loc.offset - start))
                .map_err(|_| "채팅 위치를 읽지 못했습니다.")?;
            let bytes = line(&mut BufReader::new(&mut held.reader.file))?;
            if stamp(&held.reader.file)? != held.stamp {
                return Err(CHANGED.into());
            }
            bytes
        } else {
            let file = file.as_deref_mut().ok_or(CHANGED)?;
            file.seek(SeekFrom::Start(loc.offset))
                .map_err(|_| "채팅 위치를 읽지 못했습니다.")?;
            line(&mut BufReader::new(file))?
        };
        if let Ok(parsed) = parse_line(&bytes, &entry.provider) {
            if let Some(mut row) = parsed.items.into_iter().nth(loc.row as usize) {
                row.item.id = format!("{}:{}", loc.offset, loc.row);
                row.item.time_ms = loc.at;
                output_bytes += serde_json::to_vec(&row.item)
                    .map_err(|_| "채팅 표시 정보를 만들지 못했습니다.")?
                    .len()
                    + 1;
                if output_bytes > 256 * 1024 {
                    break;
                }
                items.push(row.item);
            }
        }
    }
    if !manual_page {
        items.reverse();
    }
    let next = start + items.len();
    if entry.parts.is_some() {
        validate_parts(entry)?;
    } else {
        validate_source(entry, file.as_deref().ok_or(CHANGED)?)?;
    }
    Ok(ReplayChatPage {
        items,
        state: if progress.total == 0 {
            "empty"
        } else if unmapped {
            "unmapped"
        } else {
            "ready"
        }
        .into(),
        timeline_quality: progress.quality().into(),
        next_at_ms,
        cursor: (manual_page && next < progress.total).then(|| next.to_string()),
        previous_cursor: (manual_page && start > 0)
            .then(|| start.saturating_sub(limit).to_string()),
        total: progress.total,
        malformed_lines: progress.malformed,
        unsupported_items: progress.unsupported,
        reordered: progress.reordered,
        duplicates: progress.duplicates,
        partial_file: partial(entry),
        preparation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_soop_vod_offsets_are_explicit_without_changing_live_or_youtube() {
        let row=br#"{"format":"atsumi-chat-v1","kind":"chat","videoOffsetMs":12500,"receivedAtMs":0,"author":"Fixture","text":"fixture"}"#;
        let parsed=parse_line(row,"soop").unwrap();
        assert_eq!(parsed.items[0].offset_ms,Some(12500));
        assert_eq!(parse_line(row,"youtube").unwrap().items[0].offset_ms,None);
        let live=br#"{"format":"atsumi-chat-v1","kind":"chat","receivedAtMs":1700000000000,"author":"Fixture","text":"fixture"}"#;
        assert_eq!(parse_line(live,"soop").unwrap().items[0].offset_ms,None);
    }
    use std::fs;
    thread_local! {
        // Tests have independent caches, but exercise the same production step.
        // Parallel fixtures must not pretend to be >2 simultaneous app players.
        static TEST_CACHE: IndexCache = IndexCache::default();
    }
    fn read_page(
        directory: &Path,
        provider: &str,
        position_ms: u64,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ReplayChatPage, String> {
        TEST_CACHE.with(|cache| {
            read_page_using(
                cache,
                directory,
                provider,
                position_ms,
                cursor,
                limit,
                SLICE,
            )
        })
    }
    fn read_prepared(
        directory: &Path,
        provider: &str,
        position_ms: u64,
        cursor: Option<&str>,
        limit: usize,
    ) -> ReplayChatPage {
        for _ in 0..2048 {
            let page = read_page(directory, provider, position_ms, cursor, limit).unwrap();
            if page.state != "indexing" {
                return page;
            }
            assert!(page.items.is_empty());
            assert_eq!(page.total, 0);
        }
        panic!("bounded fixture preparation did not finish");
    }
    #[test]
    fn youtube_ordered_parts_preserve_run_boundaries_without_exporting_urls() {
        let value = serde_json::json!({"addChatItemAction":{"item":{"liveChatTextMessageRenderer":{
            "message":{"runs":[{"text":"before :typed:"},{"emoji":{"shortcuts":[":flower:"],"image":{"thumbnails":[{"url":"https://yt3.ggpht.com/flower=s32"}]}}},{"text":" after"}]}
        }}}});
        let events = normalize_public_line("youtube", value.to_string().as_bytes()).unwrap();
        let event = &events[0];
        assert_eq!(event.text, "before :typed::flower: after");
        assert_eq!(event.parts.len(), 3);
        assert!(matches!(&event.parts[0],ChatPart::Text{text} if text=="before :typed:"));
        assert!(
            matches!(&event.parts[1],ChatPart::Emoji{source_url:Some(_),alt,..} if alt==":flower:")
        );
        let serialized = serde_json::to_vec(event).unwrap();
        assert!(!String::from_utf8_lossy(&serialized).contains("https:"));
        let stored = parse_line(&serialized, "youtube").unwrap();
        assert!(matches!(
            &stored.items[0].item.parts[1],
            ChatPart::Emoji {
                source_url: None,
                ..
            }
        ));
    }
    #[test]
    fn legacy_normalized_text_and_unsafe_emoji_keep_safe_fallback() {
        let legacy =
            serde_json::json!({"format":"atsumi-chat-v1","kind":"chat","text":"old :emoji:"});
        assert!(parse_line(legacy.to_string().as_bytes(), "youtube")
            .unwrap()
            .items[0]
            .item
            .parts
            .is_empty());
        let parts = youtube_parts(
            &serde_json::json!({"runs":[{"emoji":{"shortcuts":[":bad:"],"image":{"thumbnails":[{"url":"https://localhost/private"}]}}}]}),
        );
        assert_eq!(
            parts,
            vec![ChatPart::Text {
                text: ":bad:".into()
            }]
        );
        assert!(stored_parts(
            &serde_json::json!([{"type":"emoji","assetId":"../file","alt":"bad"}])
        )
        .is_empty());
    }
    fn write(dir: &Path, name: &str, values: &[Value]) {
        fs::write(
            dir.join(name),
            values.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
    }
    fn soop(time: u64, text: &str) -> Value {
        serde_json::json!({"kind":"chat","receivedAtMs":time,"nickname":"Fixture","text":text,"extra":{"color":"#aabbcc"}})
    }
    fn anchor_file(dir: &Path) {
        fs::write(dir.join("chat-timeline.json"),r#"{"timelineQuality":"receive-time-approximate","firstSegmentCommittedAtMs":1700000004000,"firstSegmentDurationMs":4000}"#).unwrap();
    }
    #[test]
    fn raw_v2_invalid_or_missing_version_never_uses_legacy_fields() {
        let directory = tempfile::tempdir().unwrap();
        anchor_file(directory.path());
        assert_eq!(anchor(directory.path(), "youtube", "").unwrap().0, Some(1_700_000_000_000));
        assert_eq!(anchor(directory.path(), "soop", "").unwrap().0, Some(1_700_000_000_000));
        let legacy: Value = serde_json::from_slice(&fs::read(directory.path().join("chat-timeline.json")).unwrap()).unwrap();
        for version in [serde_json::json!(2), serde_json::json!(3), serde_json::json!("2"), serde_json::json!(null)] {
            let mut value = legacy.clone(); value["version"] = version;
            fs::write(directory.path().join("chat-timeline.json"), serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(anchor(directory.path(), "youtube", "").is_err());
        }
        for version in [None, Some(serde_json::json!(1))] {
            let mut value = legacy.clone(); value["anchorMethod"] = serde_json::json!("native-raw-boundary");
            if let Some(version) = version { value["version"] = version; }
            fs::write(directory.path().join("chat-timeline.json"), serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(anchor(directory.path(), "youtube", "").is_err());
        }
    }
    fn yt(time: &str, text: &str) -> Value {
        serde_json::json!({"isLive":true,"videoOffsetTimeMsec":"9999","replayChatItemAction":{"actions":[{"addChatItemAction":{"item":{"liveChatTextMessageRenderer":{"id":time,"timestampUsec":time,"authorName":{"simpleText":"Fixture"},"message":{"runs":[{"text":text}]},"authorExternalChannelId":"not-exported","contextMenuEndpoint":{"secret":"not-exported"}}}}}]}})
    }
    #[test]
    fn soop_maps_approximate_receipt_time_and_seeks_back() {
        let dir = tempfile::tempdir().unwrap();
        anchor_file(dir.path());
        write(
            dir.path(),
            "chat.soop.jsonl",
            &[soop(1700000001000, "one"), soop(1700000003000, "three")],
        );
        let later = read_page(dir.path(), "soop", 4000, None, 120).unwrap();
        assert_eq!(later.items.len(), 2);
        assert_eq!(later.timeline_quality, "receive-time-approximate");
        let earlier = read_page(dir.path(), "soop", 1500, None, 120).unwrap();
        assert_eq!(earlier.items.len(), 1);
        assert_eq!(earlier.next_at_ms, Some(3000));
    }
    #[test]
    fn missing_anchor_is_not_fabricated_as_zero_and_can_page() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "chat.soop.jsonl",
            &[soop(1, "one"), soop(2, "two")],
        );
        let page = read_page(dir.path(), "soop", 9999, None, 1).unwrap();
        assert_eq!(page.state, "unmapped");
        assert_eq!(page.items[0].time_ms, None);
        assert_eq!(page.cursor.as_deref(), Some("1"));
        let next = read_page(dir.path(), "soop", 0, page.cursor.as_deref(), 1).unwrap();
        assert_eq!(next.items[0].text, "two");
    }
    #[test]
    fn youtube_live_pseudo_offset_is_not_video_time() {
        let dir = tempfile::tempdir().unwrap();
        anchor_file(dir.path());
        write(
            dir.path(),
            "chat.live_chat.json.part",
            &[
                yt("1700000001000000", "<img onerror=bad>"),
                yt("1700000002000000", "later"),
            ],
        );
        let page = read_page(dir.path(), "youtube", 1500, None, 120).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].time_ms, Some(1000));
        let json = serde_json::to_string(&page).unwrap();
        assert!(!json.contains("not-exported"));
        assert!(!json.contains("onerror=bad>") || page.items[0].text.starts_with('<'));
        assert!(page.partial_file);
    }
    #[test]
    fn vod_offset_and_known_paid_fields_are_explicit() {
        let value = serde_json::json!({"replayChatItemAction":{"videoOffsetTimeMsec":"2500","actions":[{"addChatItemAction":{"item":{"liveChatPaidMessageRenderer":{"authorName":{"simpleText":"Fixture"},"purchaseAmountText":{"simpleText":"₩1,000"},"message":{"runs":[{"emoji":{"shortcuts":[":fixture:"]}}]}}}}}]}});
        let parsed = parse_youtube(&value);
        assert_eq!(parsed.items[0].offset_ms, Some(2500));
        assert_eq!(parsed.items[0].item.amount.as_deref(), Some("₩1,000"));
        assert_eq!(parsed.items[0].item.text, ":fixture:");
    }
    #[test]
    fn malformed_unknown_reordered_and_duplicates_preserve_safe_messages() {
        let dir = tempfile::tempdir().unwrap();
        anchor_file(dir.path());
        let mut late = yt("1700000003000000", "later");
        let first = yt("1700000001000000", "earlier");
        write(
            dir.path(),
            "chat.live_chat.json",
            &[late.clone(), first.clone(), late.clone()],
        );
        let mut file = OpenOptions::new()
            .append(true)
            .open(dir.path().join("chat.live_chat.json"))
            .unwrap();
        use std::io::Write;
        file.write_all(b"{truncated\n").unwrap();
        drop(file);
        let page = read_page(dir.path(), "youtube", 5000, None, 120).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.reordered, 1);
        assert_eq!(page.duplicates, 1);
        assert_eq!(page.malformed_lines, 1);
        assert_eq!(page.items[0].text, "earlier");
        late["replayChatItemAction"]["actions"][0]["addChatItemAction"]["item"] =
            serde_json::json!({"unknownRenderer":{"text":"private?"}});
        assert_eq!(parse_youtube(&late).unsupported, 1);
    }
    #[test]
    fn source_allowlist_limits_and_hardlinks_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_page(dir.path(), "../youtube", 0, None, 120).is_err());
        write(dir.path(), "chat.soop.jsonl", &[soop(1, "safe")]);
        fs::hard_link(
            dir.path().join("chat.soop.jsonl"),
            dir.path().join("extra.jsonl"),
        )
        .unwrap();
        #[cfg(windows)]
        assert!(read_page(dir.path(), "soop", 0, None, 120).is_err());
        let large = tempfile::tempdir().unwrap();
        fs::write(
            large.path().join("chat.soop.jsonl"),
            vec![b'x'; MAX_LINE + 1],
        )
        .unwrap();
        assert!(read_page(large.path(), "soop", 0, None, 1).is_err());
    }
    #[test]
    fn long_archive_is_disk_indexed_without_the_old_event_count_cutoff() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        anchor_file(dir.path());
        let mut file =
            std::io::BufWriter::new(File::create(dir.path().join("chat.soop.jsonl")).unwrap());
        for n in 0..100_005 {
            writeln!(file,"{{\"kind\":\"chat\",\"receivedAtMs\":{},\"nickname\":\"Fixture\",\"text\":\"fixture\"}}",1_700_000_000_000u64+n*100).unwrap();
        }
        file.flush().unwrap();
        drop(file);
        let page = read_prepared(dir.path(), "soop", 10_000_500, None, 120);
        assert_eq!(page.total, 100_005);
        assert_eq!(page.items.len(), 120);
        assert_eq!(page.items.last().unwrap().time_ms, Some(10_000_400));
        let db = index_database().unwrap();
        let cache_size: i64 = db.query_row("PRAGMA cache_size", [], |r| r.get(0)).unwrap();
        assert_eq!(cache_size, -2048);
        let max_pages: i64 = db
            .query_row("PRAGMA max_page_count", [], |r| r.get(0))
            .unwrap();
        assert_eq!(max_pages, 131072);
    }
    #[test]
    fn same_size_timestamp_replacement_invalidates_index_by_file_identity() {
        let dir = tempfile::tempdir().unwrap();
        anchor_file(dir.path());
        write(
            dir.path(),
            "chat.soop.jsonl",
            &[soop(1_700_000_001_000, "old")],
        );
        assert_eq!(
            read_page(dir.path(), "soop", 2000, None, 120)
                .unwrap()
                .items
                .len(),
            1
        );
        let path = dir.path().join("chat.soop.jsonl");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let prior = stamp(&File::open(&path).unwrap()).unwrap();
        write(
            dir.path(),
            "replacement.jsonl",
            &[soop(1_700_000_003_000, "new")],
        );
        let replacement = dir.path().join("replacement.jsonl");
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        fs::rename(&path, dir.path().join("old.jsonl")).unwrap();
        fs::rename(replacement, &path).unwrap();
        let current = stamp(&File::open(&path).unwrap()).unwrap();
        assert_eq!(prior.length, current.length);
        assert_eq!(prior.modified, current.modified);
        assert_ne!(prior.identity, current.identity);
        assert!(read_page(dir.path(), "soop", 2000, None, 120)
            .unwrap()
            .items
            .is_empty());
    }
    #[test]
    fn normalized_archive_preserves_only_public_fields_and_output_budget_can_continue() {
        let raw = serde_json::to_vec(&yt("1700000001000000", "fixture")).unwrap();
        let rows = normalize_public_line("youtube", &raw).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_timestamp_ms, Some(1_700_000_001_000));
        assert_eq!(rows[0].message_id.as_ref().unwrap().len(), 64);
        let encoded = serde_json::to_string(&rows[0]).unwrap();
        assert!(!encoded.contains("not-exported"));
        assert!(!encoded.contains("timestampUsec"));
        let dir = tempfile::tempdir().unwrap();
        let mut events = Vec::new();
        for n in 0..120 {
            let mut row = rows[0].clone();
            row.message_id = Some(format!("fixture{n}"));
            row.text = "가".repeat(MAX_TEXT);
            events.push(serde_json::to_value(row).unwrap());
        }
        write(dir.path(), "chat.events.jsonl", &events);
        let first = read_prepared(dir.path(), "youtube", 0, None, 120);
        assert!(first.items.len() < 120);
        assert!(serde_json::to_vec(&first).unwrap().len() < 256 * 1024);
        let cursor = first.cursor.clone().unwrap();
        assert_eq!(cursor.parse::<usize>().unwrap(), first.items.len());
        let next = read_page(dir.path(), "youtube", 0, Some(&cursor), 120).unwrap();
        assert_ne!(first.items.last().unwrap().id, next.items[0].id);
    }
    #[test]
    fn ancestor_link_is_rejected_when_platform_allows_link_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        write(&target, "chat.soop.jsonl", &[soop(1, "fixture")]);
        let link = dir.path().join("link");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&target, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link);
        #[cfg(any(windows, unix))]
        {
            if let Err(error) = made {
                assert!(
                    matches!(error.kind(), std::io::ErrorKind::PermissionDenied)
                        || error.raw_os_error() == Some(1314)
                );
                return;
            }
            assert!(read_page(&link, "soop", 0, None, 120).is_err());
        }
    }
    fn one_line_budget() -> SliceBudget {
        SliceBudget {
            bytes: MAX_LINE as u64,
            units: 257,
            time: PREPARE_TIME,
        }
    }
    fn cached_entry(cache: &IndexCache, directory: &Path) -> Arc<Entry> {
        cache
            .registry
            .lock()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.path.parent() == Some(directory))
            .unwrap()
            .clone()
    }
    fn one_step(cache: &IndexCache, directory: &Path) -> ReplayChatPage {
        read_page_using(
            cache,
            directory,
            "youtube",
            5000,
            None,
            120,
            one_line_budget(),
        )
        .unwrap()
    }
    fn multipart_fixture(
        parts: &[Vec<Value>],
        unsealed: Option<Vec<Value>>,
    ) -> (tempfile::TempDir, String) {
        use sha2::Digest;
        let directory = tempfile::tempdir().unwrap();
        let job_id = uuid::Uuid::new_v4().to_string();
        let marker =
            super::super::chat_status::begin_multipart(directory.path(), "youtube", &job_id)
                .unwrap();
        anchor_file(directory.path());
        let mut records = vec![serde_json::json!({"op":"header","marker":marker})];
        for (sequence, rows) in parts.iter().enumerate() {
            let name = chat_parts::part_name(chat_parts::Stream::Events, sequence as u32).unwrap();
            write(directory.path(), &name, rows);
            let bytes = fs::read(directory.path().join(name)).unwrap();
            records.push(serde_json::json!({"op":"open","stream":"events","sequence":sequence}));
            records.push(serde_json::json!({"op":"seal","stream":"events","sequence":sequence,
                "bytes":bytes.len(),"lines":rows.len(),"sha256":format!("{:x}",sha2::Sha256::digest(&bytes))}));
        }
        if let Some(rows) = unsealed {
            let sequence = parts.len();
            let name = chat_parts::part_name(chat_parts::Stream::Events, sequence as u32).unwrap();
            write(directory.path(), &name, &rows);
            records.push(serde_json::json!({"op":"open","stream":"events","sequence":sequence}));
        }
        write(directory.path(), "chat-parts.jsonl", &records);
        (directory, job_id)
    }
    fn multipart_step(
        cache: &IndexCache,
        directory: &Path,
        id: &str,
        budget: SliceBudget,
    ) -> Result<ReplayChatPage, String> {
        read_page_for_job_using(cache, directory, "youtube", id, 5000, None, 120, budget)
    }
    #[test]
    fn multipart_progress_cross_part_seek_and_manual_pages_match_legacy() {
        let late = yt("1700000003000000", "later");
        let early = yt("1700000001000000", "earlier");
        let last = yt("1700000004000000", "last");
        let (directory, id) = multipart_fixture(
            &[
                vec![late.clone()],
                vec![early.clone(), late.clone()],
                vec![last.clone()],
            ],
            None,
        );
        let cache = IndexCache::default();
        let mut previous = 0;
        let mut steps = 0;
        let ready = loop {
            let page = multipart_step(&cache, directory.path(), &id, one_line_budget()).unwrap();
            steps += 1;
            assert!(steps < 10);
            if page.state != "indexing" {
                break page;
            }
            assert!(page.items.is_empty() && page.cursor.is_none());
            assert_eq!(page.total, 0);
            let progress = page.preparation.unwrap();
            assert!(
                progress.scanned_bytes > previous && progress.scanned_bytes < progress.total_bytes
            );
            previous = progress.scanned_bytes;
        };
        assert_eq!(steps, 4);
        assert_eq!((ready.total, ready.duplicates, ready.reordered), (3, 1, 1));
        assert!(ready.partial_file); // No terminal proof is not a completed archive.
        let legacy = tempfile::tempdir().unwrap();
        anchor_file(legacy.path());
        write(
            legacy.path(),
            "chat.events.jsonl",
            &[late, early, yt("1700000003000000", "later"), last],
        );
        let mut expected = read_page_using(
            &IndexCache::default(),
            legacy.path(),
            "youtube",
            5000,
            None,
            120,
            SLICE,
        )
        .unwrap();
        expected.partial_file = true;
        assert_eq!(
            serde_json::to_value(&ready).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        let page = read_page_for_job_using(
            &cache,
            directory.path(),
            "youtube",
            &id,
            1500,
            None,
            120,
            SLICE,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].text, "earlier");
        let first = read_page_for_job_using(
            &cache,
            directory.path(),
            "youtube",
            &id,
            0,
            Some("0"),
            1,
            SLICE,
        )
        .unwrap();
        let second = read_page_for_job_using(
            &cache,
            directory.path(),
            "youtube",
            &id,
            0,
            first.cursor.as_deref(),
            1,
            SLICE,
        )
        .unwrap();
        assert_eq!(first.items[0].text, "later");
        assert_eq!(second.items[0].text, "earlier");
        assert_ne!(first.items[0].id, second.items[0].id);
        assert_eq!(second.previous_cursor.as_deref(), Some("0"));
        assert_eq!(
            cached_entry(&cache, directory.path())
                .index
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .parts
                .files
                .len(),
            3
        );
    }
    #[test]
    fn multipart_crash_ignores_unsealed_tail_and_empty_parts_still_verify() {
        let (directory, id) = multipart_fixture(
            &[vec![], vec![yt("1700000001000000", "sealed")], vec![]],
            Some(vec![yt("1700000002000000", "unsealed")]),
        );
        let before = fs::read(directory.path().join("chat.events.000003.jsonl")).unwrap();
        let cache = IndexCache::default();
        let page = multipart_step(&cache, directory.path(), &id, SLICE).unwrap();
        assert_eq!(
            (page.state.as_str(), page.total, page.partial_file),
            ("ready", 1, true)
        );
        assert_eq!(page.items[0].text, "sealed");
        assert_eq!(
            fs::read(directory.path().join("chat.events.000003.jsonl")).unwrap(),
            before
        );
        assert_eq!(
            cached_entry(&cache, directory.path())
                .index
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .parts
                .files
                .len(),
            3
        );
        let (empty, empty_id) =
            multipart_fixture(&[], Some(vec![yt("1700000001000000", "not sealed")]));
        let empty_page =
            multipart_step(&IndexCache::default(), empty.path(), &empty_id, SLICE).unwrap();
        assert_eq!(empty_page.state, "empty");
        assert!(empty_page.partial_file && empty_page.items.is_empty());
    }
    #[test]
    fn multipart_completed_writer_proof_reads_without_partial_warning() {
        let directory = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let marker =
            super::super::chat_status::begin_multipart(directory.path(), "youtube", &id).unwrap();
        anchor_file(directory.path());
        let mut writer = chat_parts::Writer::create(directory.path(), &marker).unwrap();
        writer
            .append(
                chat_parts::Stream::Events,
                yt("1700000001000000", "complete").to_string().as_bytes(),
            )
            .unwrap();
        super::super::chat_status::finish_multipart(
            directory.path(),
            &marker,
            writer.close(),
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        let page = multipart_step(&IndexCache::default(), directory.path(), &id, SLICE).unwrap();
        assert_eq!(page.total, 1);
        assert!(!page.partial_file);
    }
    #[test]
    fn multipart_rejects_wrong_owner_legacy_wrapper_and_corrupt_catalog_without_fallback() {
        let (directory, id) = multipart_fixture(&[vec![yt("1700000001000000", "one")]], None);
        assert!(read_page(directory.path(), "youtube", 5000, None, 120).is_err());
        assert!(multipart_step(
            &IndexCache::default(),
            directory.path(),
            &uuid::Uuid::new_v4().to_string(),
            SLICE
        )
        .is_err());
        fs::write(directory.path().join("chat-parts.jsonl"), b"{invalid}\n").unwrap();
        assert!(multipart_step(&IndexCache::default(), directory.path(), &id, SLICE).is_err());
        assert!(directory.path().join("chat.events.jsonl").exists());
    }
    #[test]
    fn multipart_hash_or_missing_later_part_never_exposes_prepared_rows() {
        for missing in [false, true] {
            let (directory, id) = multipart_fixture(
                &[
                    vec![yt("1700000001000000", "one")],
                    vec![yt("1700000002000000", "two")],
                ],
                None,
            );
            let later = directory.path().join("chat.events.000001.jsonl");
            if missing {
                fs::remove_file(&later).unwrap();
            } else {
                write(
                    directory.path(),
                    "chat.events.000001.jsonl",
                    &[yt("1700000002000000", "bad")],
                );
            }
            let cache = IndexCache::default();
            let first = multipart_step(&cache, directory.path(), &id, one_line_budget()).unwrap();
            assert_eq!(first.state, "indexing");
            assert!(first.items.is_empty() && first.total == 0);
            assert!(multipart_step(&cache, directory.path(), &id, one_line_budget()).is_err());
            assert!(cache.registry.lock().unwrap().entries.is_empty());
            assert_eq!(cache.databases.load(Ordering::Acquire), 0);
            assert!(directory.path().join("chat.events.jsonl").exists());
        }
    }
    #[test]
    fn multipart_generation_and_anchor_changes_discard_prepared_index() {
        use std::io::Write;
        let (directory, id) = multipart_fixture(
            &[vec![
                yt("1700000001000000", "one"),
                yt("1700000002000000", "two"),
            ]],
            None,
        );
        let cache = IndexCache::default();
        assert_eq!(
            multipart_step(&cache, directory.path(), &id, one_line_budget())
                .unwrap()
                .state,
            "indexing"
        );
        let prior = cached_entry(&cache, directory.path());
        OpenOptions::new()
            .append(true)
            .open(directory.path().join("chat-parts.jsonl"))
            .unwrap()
            .write_all(b"{unfinished")
            .unwrap();
        let page = multipart_step(&cache, directory.path(), &id, one_line_budget()).unwrap();
        assert_eq!(page.state, "indexing");
        assert!(prior.cancelled.load(Ordering::Acquire));
        assert!(matches!(validate_parts(&prior), Err(error) if error == CANCELLED));
        drop(prior);
        let prior = cached_entry(&cache, directory.path());
        fs::write(
            directory.path().join("chat-timeline.json"),
            b"{\"timelineQuality\":\"missing-anchor\"}",
        )
        .unwrap();
        assert_eq!(
            multipart_step(&cache, directory.path(), &id, one_line_budget())
                .unwrap()
                .state,
            "indexing"
        );
        assert!(prior.cancelled.load(Ordering::Acquire));
    }
    #[test]
    fn multipart_retained_handles_share_the_two_index_permit_and_release() {
        let fixtures: Vec<_> = (0..3)
            .map(|_| {
                multipart_fixture(
                    &[
                        vec![yt("1700000001000000", "one")],
                        vec![yt("1700000002000000", "two")],
                    ],
                    None,
                )
            })
            .collect();
        let cache = IndexCache::default();
        multipart_step(
            &cache,
            fixtures[0].0.path(),
            &fixtures[0].1,
            one_line_budget(),
        )
        .unwrap();
        let first = cached_entry(&cache, fixtures[0].0.path());
        multipart_step(
            &cache,
            fixtures[1].0.path(),
            &fixtures[1].1,
            one_line_budget(),
        )
        .unwrap();
        let waiting = multipart_step(
            &cache,
            fixtures[2].0.path(),
            &fixtures[2].1,
            one_line_budget(),
        )
        .unwrap();
        assert_eq!(waiting.preparation.unwrap().scanned_bytes, 0);
        let waiting_entry = cached_entry(&cache, fixtures[2].0.path());
        assert!(waiting_entry.index.lock().unwrap().is_none());
        assert_eq!(cache.databases.load(Ordering::Acquire), 2);
        assert!(first.cancelled.load(Ordering::Acquire));
        #[cfg(windows)]
        assert!(OpenOptions::new()
            .write(true)
            .open(fixtures[0].0.path().join("chat.events.jsonl"))
            .is_err());
        drop(first);
        #[cfg(windows)]
        assert!(OpenOptions::new()
            .write(true)
            .open(fixtures[0].0.path().join("chat.events.jsonl"))
            .is_ok());
        assert!(
            multipart_step(
                &cache,
                fixtures[2].0.path(),
                &fixtures[2].1,
                one_line_budget()
            )
            .unwrap()
            .preparation
            .unwrap()
            .scanned_bytes
                > 0
        );
        cache.cancel_matching(|_| true);
        assert!(waiting_entry.cancelled.load(Ordering::Acquire));
        assert!(read_entry(&cache, &waiting_entry, None, 0, None, 120, SLICE).is_err());
        drop(waiting_entry);
        assert_eq!(cache.databases.load(Ordering::Acquire), 0);
    }
    #[test]
    fn incremental_steps_hide_partial_rows_and_preserve_order_dedup_and_diagnostics() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        anchor_file(directory.path());
        let late = yt("1700000003000000", "later");
        let early = yt("1700000001000000", "earlier");
        write(
            directory.path(),
            "chat.live_chat.json",
            &[late.clone(), early, late],
        );
        let mut file = OpenOptions::new()
            .append(true)
            .open(directory.path().join("chat.live_chat.json"))
            .unwrap();
        file.write_all(b"{malformed\n{\"unknown\":true}\n").unwrap();
        drop(file);
        let cache = IndexCache::default();
        let mut previous = 0;
        let mut steps = 0;
        let complete = loop {
            let page = one_step(&cache, directory.path());
            steps += 1;
            assert!(steps <= 6);
            if page.state != "indexing" {
                break page;
            }
            assert!(page.items.is_empty());
            assert_eq!(page.total, 0);
            assert_eq!(
                (page.malformed_lines, page.reordered, page.duplicates),
                (0, 0, 0)
            );
            assert!(page.cursor.is_none() && page.previous_cursor.is_none());
            assert!(page.next_at_ms.is_none());
            let progress = page.preparation.unwrap();
            assert!(progress.scanned_bytes > previous);
            assert!(progress.scanned_bytes < progress.total_bytes);
            previous = progress.scanned_bytes;
        };
        assert_eq!(steps, 5);
        assert!(complete.preparation.is_none());
        assert_eq!(complete.total, 2);
        assert_eq!(complete.duplicates, 1);
        assert_eq!(complete.reordered, 1);
        assert_eq!(complete.malformed_lines, 1);
        let baseline = read_page_using(
            &IndexCache::default(),
            directory.path(),
            "youtube",
            5000,
            None,
            120,
            SLICE,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(complete).unwrap(),
            serde_json::to_value(baseline).unwrap()
        );
    }
    #[test]
    fn production_slice_limits_bytes_units_and_cooperative_time() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chat.soop.jsonl");
        let mut writer = std::io::BufWriter::new(File::create(&path).unwrap());
        for _ in 0..PREPARE_UNITS + 1 {
            writeln!(
                writer,
                "{{\"kind\":\"chat\",\"receivedAtMs\":1,\"text\":\"fixture\"}}"
            )
            .unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        let cache = IndexCache::default();
        let page = read_page_using(&cache, directory.path(), "soop", 0, None, 120, SLICE).unwrap();
        assert_eq!(page.state, "indexing");
        let progress = page.preparation.unwrap();
        assert!(progress.scanned_bytes > 0 && progress.scanned_bytes <= PREPARE_BYTES);
        let entry = cached_entry(&cache, directory.path());
        assert!(entry.index.lock().unwrap().as_ref().unwrap().progress.total * 2 <= PREPARE_UNITS);

        let timed = SliceBudget {
            time: Duration::ZERO,
            ..SLICE
        };
        let timed_cache = IndexCache::default();
        let timed_page =
            read_page_using(&timed_cache, directory.path(), "soop", 0, None, 120, timed).unwrap();
        assert_eq!(timed_page.state, "indexing");
        let timed_entry = cached_entry(&timed_cache, directory.path());
        assert_eq!(
            timed_entry
                .index
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .progress
                .total,
            1
        );

        // Large valid JSON lines hit the byte ceiling before the row ceiling.
        let bytes_directory = tempfile::tempdir().unwrap();
        let large_row = serde_json::json!({"format":"atsumi-chat-v1","kind":"chat","text":"x".repeat(MAX_LINE - 128)});
        write(
            bytes_directory.path(),
            "chat.events.jsonl",
            &vec![large_row; 20],
        );
        let byte_cache = IndexCache::default();
        let byte_page = read_page_using(
            &byte_cache,
            bytes_directory.path(),
            "youtube",
            0,
            None,
            120,
            SliceBudget {
                time: Duration::from_secs(5),
                ..SLICE
            },
        )
        .unwrap();
        assert_eq!(byte_page.state, "indexing");
        let progress = byte_page.preparation.unwrap();
        assert!(progress.scanned_bytes <= PREPARE_BYTES);
        assert!(progress.scanned_bytes > PREPARE_BYTES - 2 * MAX_LINE as u64);
    }
    #[test]
    fn release_cancels_without_waiting_for_build_and_cannot_republish_entry() {
        use std::sync::mpsc;
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "chat.live_chat.json",
            &[yt("1", "one"), yt("2", "two")],
        );
        let cache = Arc::new(IndexCache::default());
        assert_eq!(one_step(&cache, directory.path()).state, "indexing");
        let entry = cached_entry(&cache, directory.path());
        let held_build = entry.index.lock().unwrap();
        let worker_cache = cache.clone();
        let path = directory.path().to_path_buf();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            worker_cache.cancel_matching(|directory| directory == path);
            send.send(()).unwrap();
        });
        let completed = receive.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(held_build);
        worker.join().unwrap();
        assert!(completed, "release waited for the held build mutex");
        assert!(entry.cancelled.load(Ordering::Acquire));
        assert!(cache.registry.lock().unwrap().entries.is_empty());
        let mut file = open(&entry.path, MAX_FILE).unwrap();
        assert!(
            matches!(read_entry(&cache, &entry, Some(&mut file), 0, None, 120, SLICE), Err(error) if error == CANCELLED)
        );
        assert!(cache.registry.lock().unwrap().entries.is_empty());
        drop(entry);
        assert_eq!(cache.databases.load(Ordering::Acquire), 0);
        assert_eq!(one_step(&cache, directory.path()).state, "indexing");
    }
    #[test]
    fn close_before_registration_is_scoped_and_tickets_remain_bounded() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for directory in [a.path(), b.path()] {
            write(directory, "chat.live_chat.json", &[yt("1", "one")]);
        }
        let cache = IndexCache::default();
        let a_ticket = cache.register(a.path()).unwrap();
        let b_ticket = cache.register(b.path()).unwrap();
        assert!(cache.register(a.path()).is_err());
        cache.cancel_matching(|directory| directory == a.path());
        for (directory, ticket, cancelled) in [
            (a.path(), a_ticket.as_ref(), true),
            (b.path(), b_ticket.as_ref(), false),
        ] {
            let path = source(directory, "youtube").unwrap().unwrap();
            let mut file = open(&path, MAX_FILE).unwrap();
            let entry = cache.entry(
                path,
                stamp(&file).unwrap(),
                "youtube",
                None,
                None,
                ticket,
                "",
                None,
            );
            if cancelled {
                assert!(matches!(entry, Err(error) if error == CANCELLED));
            } else {
                let entry = entry.unwrap();
                let page =
                    read_entry(&cache, &entry, Some(&mut file), 0, None, 120, SLICE).unwrap();
                assert_eq!(page.state, "unmapped");
                assert_eq!(page.total, 1);
            }
        }
        drop(a_ticket);
        drop(b_ticket);
        for _ in 0..100 {
            drop(cache.register(a.path()).unwrap());
        }
        assert!(cache.registry.lock().unwrap().tickets.len() <= 2);
        let waiting = cache.register(b.path()).unwrap();
        cache.cancel_matching(|_| true);
        assert!(waiting.cancelled.load(Ordering::Acquire));
        assert!(cache.registry.lock().unwrap().entries.is_empty());
    }
    #[test]
    fn evicted_inflight_indexes_keep_the_two_database_cap_until_released() {
        let directories: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
        let cache = IndexCache::default();
        for directory in &directories {
            write(
                directory.path(),
                "chat.live_chat.json",
                &[yt("1", "one"), yt("2", "two")],
            );
        }
        one_step(&cache, directories[0].path());
        let first = cached_entry(&cache, directories[0].path());
        one_step(&cache, directories[1].path());
        assert_eq!(cache.databases.load(Ordering::Acquire), 2);
        let waiting = one_step(&cache, directories[2].path());
        assert_eq!(waiting.preparation.unwrap().scanned_bytes, 0);
        assert!(first.cancelled.load(Ordering::Acquire));
        assert_eq!(cache.databases.load(Ordering::Acquire), 2);
        assert_eq!(cache.registry.lock().unwrap().entries.len(), 2);
        drop(first);
        let advancing = one_step(&cache, directories[2].path());
        assert!(advancing.preparation.unwrap().scanned_bytes > 0);
        assert_eq!(cache.databases.load(Ordering::Acquire), 2);
    }
    #[test]
    fn partial_indexes_are_replaced_on_file_identity_or_anchor_change() {
        let directory = tempfile::tempdir().unwrap();
        anchor_file(directory.path());
        write(
            directory.path(),
            "chat.live_chat.json",
            &[yt("1700000001000000", "old"), yt("1700000002000000", "end")],
        );
        let cache = IndexCache::default();
        one_step(&cache, directory.path());
        let old = cached_entry(&cache, directory.path());
        let path = directory.path().join("chat.live_chat.json");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        write(
            directory.path(),
            "replacement.jsonl",
            &[yt("1700000001000000", "new"), yt("1700000002000000", "end")],
        );
        let replacement = directory.path().join("replacement.jsonl");
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        fs::rename(&path, directory.path().join("old.jsonl")).unwrap();
        fs::rename(replacement, &path).unwrap();
        one_step(&cache, directory.path());
        assert!(old.cancelled.load(Ordering::Acquire));
        let new = cached_entry(&cache, directory.path());
        assert_eq!(old.stamp.length, new.stamp.length);
        assert_eq!(old.stamp.modified, new.stamp.modified);
        assert_ne!(old.stamp.identity, new.stamp.identity);
        drop(old);
        let ready = one_step(&cache, directory.path());
        assert_eq!(ready.items[0].text, "new");
        fs::write(
            directory.path().join("chat-timeline.json"),
            "{\"timelineQuality\":\"missing\"}",
        )
        .unwrap();
        let preparing = one_step(&cache, directory.path());
        assert_eq!(preparing.state, "indexing");
        assert!(new.cancelled.load(Ordering::Acquire));
        drop(new);
        assert_eq!(one_step(&cache, directory.path()).state, "unmapped");
    }
    #[test]
    fn invalid_source_discards_partial_index_and_invalid_cursor_does_no_io() {
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "chat.live_chat.json",
            &[yt("1", "one"), yt("2", "two")],
        );
        let cache = IndexCache::default();
        one_step(&cache, directory.path());
        let entry = cached_entry(&cache, directory.path());
        fs::write(
            directory.path().join("chat.live_chat.json"),
            vec![b'x'; MAX_LINE + 1],
        )
        .unwrap();
        assert!(read_page_using(&cache, directory.path(), "youtube", 0, None, 120, SLICE).is_err());
        assert!(entry.cancelled.load(Ordering::Acquire));
        assert!(cache.registry.lock().unwrap().entries.is_empty());
        for cursor in ["", "-1", "x", "184467440737095516160000"] {
            assert!(read_page_using(
                &cache,
                directory.path(),
                "youtube",
                0,
                Some(cursor),
                120,
                SLICE
            )
            .is_err());
        }
        assert!(cache.registry.lock().unwrap().entries.is_empty());
    }
    #[test]
    #[ignore = "Explicit isolated archive fixture path; counts only, no raw chat output"]
    fn actual_saved_archive_counts_only() {
        let dir = PathBuf::from(
            std::env::var("ATSUMI_REPLAY_CHAT_FIXTURE").expect("isolated test archive required"),
        );
        let provider = std::env::var("ATSUMI_REPLAY_CHAT_PROVIDER").unwrap_or("youtube".into());
        let page = read_prepared(&dir, &provider, 0, Some("0"), 120);
        let messages = page
            .items
            .iter()
            .filter(|item| matches!(item.kind.as_str(), "chat" | "donation" | "membership"))
            .count();
        assert!(
            messages > 0,
            "No actual public message in the sampled archive page"
        );
        println!(
            "replay chat: rows={}, messages={}, total={}, quality={}, malformed={}, unsupported={}",
            page.items.len(),
            messages,
            page.total,
            page.timeline_quality,
            page.malformed_lines,
            page.unsupported_items
        );
    }
}
