//! Public Pinterest image pins and public profile/board discovery. No account cookies
//! or remote URL from IPC is trusted; a login shell is never an empty-results success.
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine};
use image::codecs::jpeg::JpegEncoder;
use image::{ImageReader, Limits};
use reqwest::blocking::{Client, Response};
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{Manager, State};

use crate::interface::AppState;

const MAX_HTML: u64 = 5 * 1024 * 1024;
const MAX_IMAGE: u64 = 32 * 1024 * 1024;
const MAX_PIN_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST: u64 = 256 * 1024;
const MAX_PAGES: usize = 50;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const CANCELLED: &str = "Pinterest 요청을 취소했습니다.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestMedia {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestPin {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_path: Option<String>,
    #[serde(default)]
    pub summary_only: bool,
    pub source_url: String,
    pub media: Vec<PinterestMedia>,
    pub unsupported_video: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestDownload {
    pub pin: PinterestPin,
    /// Relative file names inside downloadRoot/Pinterest/<pin ID>.
    pub files: Vec<String>,
    pub bytes: u64,
    pub saved_at: u64,
    #[serde(default)]
    pub sha256: Vec<String>,
    #[serde(default)]
    pub missing_files: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestDownloadOutcome {
    pub pin_id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<PinterestDownload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestRemoved {
    pub token: String,
    pub record: PinterestDownload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestBoard {
    pub title: String,
    pub path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinterestDiscovery {
    pub title: String,
    pub path: String,
    pub pins: Vec<PinterestPin>,
    pub boards: Vec<PinterestBoard>,
    pub limited: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub auth_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generation: Option<u64>,
}

pub(crate) fn clear_remote_cache() {
    for cache in [PIN_CACHE.get(), DISCOVERY_CACHE.get()]
        .into_iter()
        .flatten()
    {
        cache.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
}
pub(crate) fn session_pin_id(input: &str) -> Result<String, String> {
    normalize_input(input)
}
pub(crate) fn session_discovery_path(input: &str, kind: &str) -> Result<String, String> {
    match kind {
        "home" if input.trim().is_empty() => Ok("/".into()),
        "keyword"
            if !input.trim().is_empty()
                && input.chars().count() <= 120
                && !input.chars().any(char::is_control) =>
        {
            let mut url = Url::parse("https://www.pinterest.com/search/pins/").unwrap();
            url.query_pairs_mut().append_pair("q", input.trim());
            Ok(format!(
                "{}?{}",
                url.path(),
                url.query().unwrap_or_default()
            ))
        }
        "profile" => profile_path(input),
        _ => Err("Pinterest 탐색 조건이 올바르지 않습니다.".into()),
    }
}
pub(crate) fn session_pins(
    value: &Value,
    exact: Option<&str>,
) -> Result<Vec<PinterestPin>, String> {
    let items = value
        .as_array()
        .filter(|items| items.len() <= 60)
        .ok_or("Pinterest 목록 응답이 올바르지 않습니다.")?;
    if items.is_empty() && exact.is_none() {
        return Ok(Vec::new());
    }
    let mut pins = Vec::new();
    for item in items {
        let Ok(pin) = serde_json::from_value::<PinterestPin>(item.clone()) else {
            continue;
        };
        if !valid_id(&pin.id)
            || exact.is_some_and(|id| id != pin.id || pin.summary_only)
            || pin.title.len() > 1200
            || pin.description.len() > 12000
            || pin
                .creator_name
                .as_ref()
                .is_some_and(|name| name.len() > 480)
            || pin
                .creator_path
                .as_ref()
                .is_some_and(|path| profile_path(path).is_err())
            || normalize_input(&pin.source_url).ok().as_deref() != Some(&pin.id)
            || pin.media.is_empty()
            || pin.media.len() > MAX_PAGES
            || pin.media.iter().any(|media| {
                safe_url(&media.url, true).is_err()
                    || media.width.is_some_and(|n| n == 0 || n > 16384)
                    || media.height.is_some_and(|n| n == 0 || n > 16384)
            })
        {
            continue;
        }
        if !pins.iter().any(|p: &PinterestPin| p.id == pin.id) {
            pins.push(pin);
        }
    }
    if pins.is_empty() {
        return Err("Pinterest에서 접근 가능한 이미지 정보를 확인하지 못했습니다.".into());
    }
    Ok(pins)
}
pub(crate) fn session_boards(value: &Value) -> Vec<PinterestBoard> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(40)
        .filter_map(|value| {
            let board = serde_json::from_value::<PinterestBoard>(value.clone()).ok()?;
            if board.title.len() > 800 {
                return None;
            }
            Some(PinterestBoard {
                title: board.title,
                path: profile_path(&board.path).ok()?,
            })
        })
        .collect()
}
pub(crate) fn remember_session_discovery(pins: &[PinterestPin]) {
    let mut cache = DISCOVERY_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for pin in pins {
        cache.insert(pin.id.clone(), (Instant::now(), pin.clone()));
    }
    while cache.len() > 160 {
        if let Some(id) = cache
            .iter()
            .min_by_key(|(_, (time, _))| *time)
            .map(|(id, _)| id.clone())
        {
            cache.remove(&id);
        } else {
            break;
        }
    }
}

struct Operation {
    cancel: Arc<AtomicBool>,
    download: bool,
}
type OperationRegistry = Mutex<HashMap<String, Operation>>;
static OPERATIONS: OnceLock<Arc<OperationRegistry>> = OnceLock::new();
type PinCache = Mutex<HashMap<String, (Instant, PinterestPin)>>;
static PIN_CACHE: OnceLock<PinCache> = OnceLock::new();
static DISCOVERY_CACHE: OnceLock<PinCache> = OnceLock::new();
static PIN_WRITES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
struct PinWriteGuard(PathBuf);
impl PinWriteGuard {
    fn begin(path: PathBuf) -> Result<Self, String> {
        let mut paths = PIN_WRITES
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !paths.insert(path.clone()) {
            return Err("이 핀을 처리 중입니다. 작업이 끝난 뒤 다시 시도해 주세요.".into());
        }
        Ok(Self(path))
    }
}
impl Drop for PinWriteGuard {
    fn drop(&mut self) {
        if let Some(lock) = PIN_WRITES.get() {
            lock.lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.0);
        }
    }
}
fn operations() -> &'static Arc<OperationRegistry> {
    OPERATIONS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn lock_operations(registry: &OperationRegistry) -> MutexGuard<'_, HashMap<String, Operation>> {
    // The map contains only independently owned jobs; no multi-field transaction or user
    // callback runs under this lock. Recover the actual map so cancellation and guard Drop
    // still drain it after a panic instead of leaving a permanent shutdown sentinel.
    registry.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Parent app shutdown/update guards must include this count. Counts queued I/O as active too.
pub fn active_download_count() -> usize {
    active_download_count_in(operations())
}
fn active_download_count_in(registry: &OperationRegistry) -> usize {
    lock_operations(registry)
        .values()
        .filter(|item| item.download)
        .count()
}

/// Stable operation IDs for the app's active-work fingerprint, including after mutex poison.
pub fn active_download_ids() -> Vec<String> {
    active_download_ids_in(operations())
}
fn active_download_ids_in(registry: &OperationRegistry) -> Vec<String> {
    let items = lock_operations(registry);
    let mut ids: Vec<_> = items
        .iter()
        .filter(|(_, item)| item.download)
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    ids
}

/// Cooperative cancellation: streams check this between every read; connect/read timeout bounds waits.
pub fn cancel_all_downloads() {
    cancel_all_downloads_in(operations());
}
fn cancel_all_downloads_in(registry: &OperationRegistry) {
    for item in lock_operations(registry)
        .values()
        .filter(|item| item.download)
    {
        item.cancel.store(true, Ordering::Relaxed);
    }
}

struct OperationGuard {
    id: String,
    cancel: Arc<AtomicBool>,
    registry: Arc<OperationRegistry>,
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        lock_operations(&self.registry).remove(&self.id);
    }
}
impl OperationGuard {
    fn begin(id: String, download: bool) -> Result<Self, String> {
        Self::begin_in(Arc::clone(operations()), id, download)
    }
    fn begin_in(
        registry: Arc<OperationRegistry>,
        id: String,
        download: bool,
    ) -> Result<Self, String> {
        if id.len() < 8
            || id.len() > 100
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("Pinterest 요청 ID가 올바르지 않습니다.".into());
        }
        let mut items = lock_operations(&registry);
        if items.len() >= 6
            || items.contains_key(&id)
            || (download && items.values().filter(|item| item.download).count() >= 2)
        {
            return Err("진행 중인 Pinterest 요청이 많습니다. 잠시 후 다시 시도해 주세요.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        items.insert(
            id.clone(),
            Operation {
                cancel: cancel.clone(),
                download,
            },
        );
        drop(items);
        Ok(Self {
            id,
            cancel,
            registry,
        })
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    (5..=25).contains(&id.len())
        && !id.starts_with('0')
        && id.bytes().all(|byte| byte.is_ascii_digit())
}
fn pinterest_host(host: &str) -> bool {
    host == "pinterest.com"
        || host == "www.pinterest.com"
        || host.strip_suffix(".pinterest.com").is_some_and(|prefix| {
            prefix.len() == 2 && prefix.bytes().all(|byte| byte.is_ascii_lowercase())
        })
}
fn safe_url(raw: &str, media: bool) -> Result<Url, String> {
    if raw.len() > 2048 {
        return Err("Pinterest 주소가 너무 깁니다.".into());
    }
    let url = Url::parse(raw).map_err(|_| "Pinterest 주소가 올바르지 않습니다.")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !url.host_str().is_some_and(|host| {
            if media {
                host == "i.pinimg.com"
            } else {
                pinterest_host(host)
            }
        })
    {
        return Err("허용되지 않은 Pinterest 주소입니다.".into());
    }
    Ok(url)
}
fn normalize_input(input: &str) -> Result<String, String> {
    let input = input.trim();
    if valid_id(input) {
        return Ok(input.into());
    }
    let url = safe_url(input, false)?;
    let path = url.path().trim_end_matches('/');
    let segment = path
        .strip_prefix("/pin/")
        .filter(|value| !value.contains('/'))
        .ok_or("공개 핀의 전체 링크 또는 ID를 입력해 주세요.")?;
    let id = segment.rsplit("--").next().unwrap_or(segment);
    if !valid_id(id) {
        return Err("공개 핀의 전체 링크 또는 ID를 입력해 주세요.".into());
    }
    Ok(id.into())
}

fn profile_path(input: &str) -> Result<String, String> {
    let input = input.trim().trim_start_matches('@');
    let path = if input.starts_with("https://") {
        safe_url(input, false)?.path().to_owned()
    } else {
        input.to_owned()
    };
    let parts = path
        .trim_matches('/')
        .split('/')
        .map(profile_segment)
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty()
        || parts.len() > 2
        || matches!(
            parts[0].to_ascii_lowercase().as_str(),
            "pin" | "search" | "resource" | "login" | "settings" | "ideas" | "_" | "today"
        )
    {
        return Err("프로필 이름 또는 공개 프로필·보드 주소를 입력해 주세요.".into());
    }
    Ok(safe_url(
        &format!("https://www.pinterest.com/{}/", parts.join("/")),
        false,
    )?
    .path()
    .to_owned())
}
fn profile_segment(raw: &str) -> Result<String, String> {
    let invalid = "프로필 이름 또는 공개 프로필·보드 주소를 입력해 주세요.";
    if raw.is_empty() || raw.len() > 900 {
        return Err(invalid.into());
    }
    let bytes = raw.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = bytes
                .get(i + 1)
                .and_then(|b| (*b as char).to_digit(16))
                .ok_or(invalid)?;
            let low = bytes
                .get(i + 2)
                .and_then(|b| (*b as char).to_digit(16))
                .ok_or(invalid)?;
            decoded.push((high * 16 + low) as u8);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let value = String::from_utf8(decoded).map_err(|_| invalid)?;
    if value.chars().count() > 100
        || !value
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(invalid.into());
    }
    Ok(value)
}

fn script_json(html: &str, id: &str) -> Option<Value> {
    let marker = format!("id=\"{id}\"");
    let start = html.find(&marker)?;
    let body = start + html[start..].find('>')? + 1;
    let end = body + html[body..].find("</script>")?;
    serde_json::from_str(&html[body..end]).ok()
}

fn resource_data<'a>(state: &'a Value, name: &str) -> Vec<&'a Value> {
    state
        .pointer(&format!("/resources/{name}"))
        .and_then(Value::as_object)
        .map(|items| items.values().collect())
        .unwrap_or_default()
}

fn discovery_pin(value: &Value, creator: &str, creator_path: &str) -> Option<PinterestPin> {
    let id = value.get("id")?.as_str()?;
    if !valid_id(id) || value.get("type").and_then(Value::as_str) != Some("pin") {
        return None;
    }
    let image = value
        .pointer("/images/orig")
        .or_else(|| value.pointer("/images/736x"))?;
    let mut media = Vec::new();
    append_media(image, &mut media, 0).ok()?;
    let title = text(value.get("title").or_else(|| value.get("grid_title")), 300);
    Some(PinterestPin {
        id: id.into(),
        title: if title.trim().is_empty() {
            format!("핀 #{id}")
        } else {
            title
        },
        description: text(value.get("description"), 3000),
        creator_name: Some(creator.into()),
        creator_path: Some(creator_path.into()),
        summary_only: true,
        source_url: format!("https://www.pinterest.com/pin/{id}/"),
        media,
        unsupported_video: value.get("videos").is_some_and(|v| !v.is_null())
            || value.get("story_pin_data").is_some_and(|v| !v.is_null()),
    })
}

fn parse_discovery(html: &str, path: &str, keyword: bool) -> Result<PinterestDiscovery, String> {
    let payload = script_json(html, "__PWS_INITIAL_PROPS__")
        .ok_or("Pinterest 목록 정보를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.")?;
    let state = payload
        .get("initialReduxState")
        .ok_or("Pinterest 목록 응답이 올바르지 않습니다.")?;
    let profile = resource_data(state, "UserResource")
        .first()
        .and_then(|r| r.get("data"))
        .cloned();
    let board = resource_data(state, "BoardResource")
        .first()
        .and_then(|r| r.get("data"))
        .cloned();
    let creator_path = profile_path(if keyword {
        "pinterest"
    } else {
        path.split('/')
            .find(|part| !part.is_empty())
            .unwrap_or("pinterest")
    })?;
    let name = text(profile.as_ref().and_then(|p| p.get("full_name")), 120);
    let name = if name.is_empty() {
        creator_path.trim_matches('/').to_owned()
    } else {
        name
    };
    let title = text(board.as_ref().and_then(|b| b.get("name")), 200);
    let mut pins = Vec::new();
    let mut limited = false;
    for resource_name in if keyword {
        vec!["BaseSearchResource"]
    } else if board.is_some() {
        vec!["BoardFeedResource"]
    } else {
        vec!["UserPinsResource"]
    } {
        for resource in resource_data(state, resource_name).into_iter().take(2) {
            limited |= resource
                .get("nextBookmark")
                .and_then(Value::as_str)
                .is_some_and(|b| !b.is_empty() && b != "-end-");
            let values = resource.get("data").and_then(|v| {
                v.as_array()
                    .or_else(|| v.get("results").and_then(Value::as_array))
            });
            if let Some(values) = values {
                for value in values.iter().take(60) {
                    if let Some(pin) = discovery_pin(value, &name, &creator_path) {
                        if !pins
                            .iter()
                            .any(|existing: &PinterestPin| existing.id == pin.id)
                        {
                            pins.push(pin);
                        }
                    }
                }
            }
        }
    }
    let mut boards = Vec::new();
    for resource in resource_data(state, "BoardsFeedResource")
        .into_iter()
        .take(1)
    {
        if let Some(values) = resource.get("data").and_then(Value::as_array) {
            for value in values.iter().take(40) {
                if let Some(url) = value.get("url").and_then(Value::as_str) {
                    if let Ok(path) = profile_path(url) {
                        boards.push(PinterestBoard {
                            title: text(value.get("name"), 200),
                            path,
                        });
                    }
                }
            }
        }
    }
    if pins.is_empty() && boards.is_empty() {
        return Err(if keyword {
            "Pinterest 키워드 검색은 현재 로그인이 필요합니다. 공개 프로필·보드 탐색과 저장한 핀 검색은 앱 안에서 사용할 수 있습니다."
        } else { "공개 핀을 확인하지 못했습니다. 비공개·빈 보드 또는 로그인이 필요한 목록일 수 있습니다." }.into());
    }
    let cache = DISCOVERY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut items) = cache.lock() {
        for pin in &pins {
            items.insert(pin.id.clone(), (Instant::now(), pin.clone()));
        }
        while items.len() > 160 {
            let oldest = items
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                items.remove(&key);
            } else {
                break;
            }
        }
    }
    Ok(PinterestDiscovery {
        title: if title.is_empty() { name } else { title },
        path: path.into(),
        pins,
        boards,
        limited,
        cursor: None,
        auth_status: None,
        generation: None,
    })
}

fn discover(input: &str, kind: &str, cancel: &AtomicBool) -> Result<PinterestDiscovery, String> {
    let keyword = kind == "keyword";
    let path = if keyword {
        let query = input.trim();
        if query.is_empty() || query.chars().count() > 120 || query.chars().any(char::is_control) {
            return Err("검색어를 1~120자로 입력해 주세요.".into());
        }
        let mut url = Url::parse("https://www.pinterest.com/search/pins/").unwrap();
        url.query_pairs_mut().append_pair("q", query);
        format!("{}?{}", url.path(), url.query().unwrap_or_default())
    } else if kind == "profile" {
        profile_path(input)?
    } else {
        return Err("탐색 종류가 올바르지 않습니다.".into());
    };
    let response = get(
        &client()?,
        safe_url(&format!("https://www.pinterest.com{path}"), false)?,
        false,
        cancel,
    )?;
    let bytes = read_bounded(response, MAX_HTML, cancel)?;
    let html =
        std::str::from_utf8(&bytes).map_err(|_| "Pinterest 목록 인코딩을 읽지 못했습니다.")?;
    check_cancel(cancel)?;
    parse_discovery(html, &path, keyword)
}
fn client() -> Result<Client, String> {
    #[cfg(test)]
    tests::check_network_client_entry()?;
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Pinterest 연결을 준비하지 못했습니다.".into())
}
fn get(
    client: &Client,
    mut url: Url,
    media: bool,
    cancel: &AtomicBool,
) -> Result<Response, String> {
    for _ in 0..5 {
        check_cancel(cancel)?;
        safe_url(url.as_str(), media)?;
        let response = client
            .get(url.clone())
            .header(
                "Accept",
                if media {
                    "image/jpeg,image/png,image/webp"
                } else {
                    "text/html"
                },
            )
            .send()
            .map_err(|_| {
                "Pinterest에 연결하지 못했습니다. 네트워크를 확인한 뒤 다시 시도해 주세요."
            })?;
        if response.status().is_redirection() {
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("Pinterest 이동 주소가 올바르지 않습니다.")?;
            url = url
                .join(target)
                .map_err(|_| "Pinterest 이동 주소가 올바르지 않습니다.")?;
            continue;
        }
        return match response.status().as_u16() {
            200 => Ok(response),
            401 | 403 => Err("공개 접근이 제한된 핀입니다. Pinterest에서 공개 여부와 로그인 필요 여부를 확인해 주세요.".into()),
            404 | 410 => Err("삭제되었거나 찾을 수 없는 핀입니다.".into()),
            429 => Err("Pinterest 요청 제한에 도달했습니다. 잠시 후 다시 시도해 주세요.".into()),
            _ => Err("Pinterest 응답을 불러오지 못했습니다. 잠시 후 다시 시도해 주세요.".into()),
        };
    }
    Err("Pinterest 이동 횟수가 너무 많습니다.".into())
}
fn read_bounded(
    mut response: Response,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err("Pinterest 응답이 안전한 크기 제한을 초과했습니다.".into());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 32 * 1024];
    loop {
        check_cancel(cancel)?;
        let length = response
            .read(&mut chunk)
            .map_err(|_| "Pinterest 다운로드가 중단되었습니다.")?;
        if length == 0 {
            break;
        }
        if bytes.len() as u64 + length as u64 > limit {
            return Err("Pinterest 응답이 안전한 크기 제한을 초과했습니다.".into());
        }
        bytes.extend_from_slice(&chunk[..length]);
    }
    check_cancel(cancel)?;
    Ok(bytes)
}

fn text(value: Option<&Value>, maximum: usize) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(maximum)
        .collect()
}
fn schema_nodes<'a>(value: &'a Value, nodes: &mut Vec<&'a Value>, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Some(array) = value.as_array() {
        for item in array.iter().take(100) {
            schema_nodes(item, nodes, depth + 1);
        }
    } else if value.is_object() {
        nodes.push(value);
        if let Some(graph) = value.get("@graph") {
            schema_nodes(graph, nodes, depth + 1);
        }
    }
}
fn append_media(value: &Value, media: &mut Vec<PinterestMedia>, depth: usize) -> Result<(), ()> {
    if depth > 4 {
        return Err(());
    }
    if let Some(array) = value.as_array() {
        if array.len() > MAX_PAGES {
            return Err(());
        }
        for item in array {
            append_media(item, media, depth + 1)?;
        }
        return Ok(());
    }
    let url = value
        .as_str()
        .or_else(|| value.get("contentUrl").and_then(Value::as_str))
        .or_else(|| value.get("url").and_then(Value::as_str));
    let url = safe_url(url.ok_or(())?, true).map_err(|_| ())?;
    // Page order matters even if two pages intentionally reuse the same image.
    if media.len() >= MAX_PAGES {
        return Err(());
    }
    media.push(PinterestMedia {
        url: url.to_string(),
        width: value
            .get("width")
            .and_then(Value::as_u64)
            .and_then(|number| number.try_into().ok()),
        height: value
            .get("height")
            .and_then(Value::as_u64)
            .and_then(|number| number.try_into().ok()),
    });
    Ok(())
}
fn schema_pin(value: &Value, requested: &str) -> Option<PinterestPin> {
    let kind = value.get("@type").and_then(Value::as_str)?;
    if kind != "SocialMediaPosting" && kind != "CreativeWork" && kind != "ImageObject" {
        return None;
    }
    let main = value
        .pointer("/mainEntityOfPage/@id")
        .or_else(|| value.get("mainEntityOfPage"))
        .or_else(|| value.get("url"))
        .and_then(Value::as_str);
    let id = main
        .and_then(|url| normalize_input(url).ok())
        .unwrap_or_else(|| requested.into());
    let mut media = Vec::new();
    if let Some(image) = value.get("image") {
        append_media(image, &mut media, 0).ok()?;
    }
    if kind == "ImageObject" && media.is_empty() {
        append_media(value, &mut media, 0).ok()?;
    }
    if media.is_empty() {
        return None;
    }
    let unsupported_video = value.get("video").is_some()
        || value
            .pointer("/sharedContent/@type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "VideoObject");
    let mut title = text(value.get("headline").or_else(|| value.get("name")), 300);
    if title.trim().is_empty() {
        title = format!("Pinterest #{id}");
    }
    let author = text(value.pointer("/author/name"), 120);
    Some(PinterestPin {
        source_url: format!("https://www.pinterest.com/pin/{id}/"),
        id,
        title,
        description: text(
            value
                .get("articleBody")
                .or_else(|| value.get("description")),
            3000,
        ),
        creator_name: (!author.is_empty()).then_some(author),
        creator_path: value
            .pointer("/author/url")
            .and_then(Value::as_str)
            .and_then(|url| profile_path(url).ok()),
        summary_only: false,
        media,
        unsupported_video,
    })
}
fn parse_pin(html: &str, requested: &str) -> Result<PinterestPin, String> {
    // Read only the page's JSON-LD. Hydration data may contain unrelated recommendation pins.
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find("<script") {
        let start = cursor + offset;
        let Some(header_end_offset) = lower[start..].find('>') else {
            break;
        };
        let header_end = start + header_end_offset;
        let Some(end_offset) = lower[header_end + 1..].find("</script>") else {
            break;
        };
        let end = header_end + 1 + end_offset;
        cursor = end + "</script>".len();
        let header = &lower[start..header_end];
        if !header.contains("application/ld+json") {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&html[header_end + 1..end]) {
            let mut nodes = Vec::new();
            schema_nodes(&value, &mut nodes, 0);
            for node in nodes {
                if let Some(pin) = schema_pin(node, requested) {
                    return Ok(pin);
                }
            }
        }
    }
    Err("공개 이미지 정보를 확인하지 못했습니다. 삭제·비공개·로그인 필요 또는 지원하지 않는 핀일 수 있습니다.".into())
}
fn pin_get(input: &str, cancel: &AtomicBool) -> Result<PinterestPin, String> {
    check_cancel(cancel)?;
    let id = normalize_input(input)?;
    let cache = PIN_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(items) = cache.lock() {
        if let Some((time, pin)) = items.get(&id) {
            if time.elapsed() < Duration::from_secs(300) {
                return Ok(pin.clone());
            }
        }
    }
    let client = client()?;
    let response = get(
        &client,
        safe_url(&format!("https://www.pinterest.com/pin/{id}/"), false)?,
        false,
        cancel,
    )?;
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return Err("Pinterest 페이지 응답 형식이 올바르지 않습니다.".into());
    }
    let bytes = read_bounded(response, MAX_HTML, cancel)?;
    let html =
        std::str::from_utf8(&bytes).map_err(|_| "Pinterest 페이지 인코딩을 읽지 못했습니다.")?;
    let pin = parse_pin(html, &id)?;
    if let Ok(mut items) = cache.lock() {
        if items.len() >= 64 {
            if let Some(oldest) = items
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(key, _)| key.clone())
            {
                items.remove(&oldest);
            }
        }
        items.insert(pin.id.clone(), (Instant::now(), pin.clone()));
    }
    Ok(pin)
}

fn pin_get_session(
    app: &tauri::AppHandle,
    input: &str,
    cancel: &AtomicBool,
) -> Result<PinterestPin, String> {
    let session = app.state::<crate::pinterest_session::PinterestSession>();
    session.pin(app, input, cancel)
}

fn image_bytes(
    client: &Client,
    media: &PinterestMedia,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let response = get(client, safe_url(&media.url, true)?, true, cancel)?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default();
    if !matches!(mime, "image/jpeg" | "image/png" | "image/webp") {
        return Err("JPEG·PNG·WebP 정적 이미지 핀만 지원합니다.".into());
    }
    read_bounded(response, MAX_IMAGE, cancel)
}
fn decode_image(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "이미지 형식을 확인하지 못했습니다.")?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| {
        "이미지를 검증하지 못했습니다. 손상되었거나 너무 큰 이미지일 수 있습니다.".into()
    })
}
fn preview_get(
    pin_id: &str,
    page: usize,
    thumbnail: bool,
    root: Option<&str>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let id = normalize_input(pin_id)?;
    // A completed download remains viewable offline; never load an arbitrary IPC file path.
    let local_folder = root
        .and_then(|root| download_root(root).ok())
        .map(|root| root.join(&id));
    let local = if let Some(folder) = local_folder.filter(|folder| folder.exists()) {
        let record = read_record(&folder)?;
        Some((folder, record))
    } else {
        None
    };
    let bytes = if let Some((folder, record)) = local {
        original_bytes(&folder, &record, page)?
    } else {
        let summary = if thumbnail {
            DISCOVERY_CACHE
                .get()
                .and_then(|cache| cache.lock().ok())
                .and_then(|items| {
                    items
                        .get(&id)
                        .filter(|(time, _)| time.elapsed() < Duration::from_secs(600))
                        .map(|(_, pin)| pin.clone())
                })
        } else {
            None
        };
        let pin = match summary {
            Some(pin) => pin,
            None => pin_get(pin_id, cancel)?,
        };
        let media = pin
            .media
            .get(page)
            .ok_or("페이지 번호가 올바르지 않습니다.")?;
        image_bytes(&client()?, media, cancel)?
    };
    image_data_url(bytes, thumbnail, cancel)
}

fn image_data_url(bytes: Vec<u8>, thumbnail: bool, cancel: &AtomicBool) -> Result<String, String> {
    check_cancel(cancel)?;
    let image = decode_image(&bytes)?;
    if !thumbnail {
        let mime = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Jpeg) => "image/jpeg",
            Ok(image::ImageFormat::Png) => "image/png",
            Ok(image::ImageFormat::WebP) => "image/webp",
            _ => return Err("지원하지 않는 원본 형식입니다.".into()),
        };
        check_cancel(cancel)?;
        return Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
    }
    let edge = 480;
    let reduced = if image.width() > edge || image.height() > edge {
        image.thumbnail(edge, edge).to_rgb8()
    } else {
        image.to_rgb8()
    };
    drop(image);
    drop(bytes);
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, if thumbnail { 78 } else { 88 })
        .encode_image(&reduced)
        .map_err(|_| "미리보기를 만들지 못했습니다.")?;
    if jpeg.len()
        > if thumbnail {
            256 * 1024
        } else {
            5 * 1024 * 1024
        }
    {
        return Err("미리보기 이미지가 너무 큽니다.".into());
    }
    check_cancel(cancel)?;
    Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)))
}

fn preview_get_session(
    app: &tauri::AppHandle,
    pin_id: &str,
    page: usize,
    thumbnail: bool,
    root: Option<&str>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let id = normalize_input(pin_id)?;
    if root
        .and_then(|r| download_root(r).ok())
        .is_some_and(|r| r.join(&id).exists())
    {
        return preview_get(pin_id, page, thumbnail, root, cancel);
    }
    let session = app.state::<crate::pinterest_session::PinterestSession>();
    let generation = session.generation();
    let summary = if thumbnail {
        DISCOVERY_CACHE
            .get()
            .and_then(|cache| cache.lock().ok())
            .and_then(|cache| {
                cache
                    .get(&id)
                    .filter(|(time, _)| time.elapsed() < Duration::from_secs(300))
                    .map(|(_, pin)| pin.clone())
            })
    } else {
        None
    };
    let pin = match summary {
        Some(pin) => pin,
        None => pin_get_session(app, &id, cancel)?,
    };
    let media = pin
        .media
        .get(page)
        .ok_or("페이지 번호가 올바르지 않습니다.")?;
    let bytes = image_bytes(&client()?, media, cancel)?;
    if generation != session.generation() {
        return Err("Pinterest 로그인 상태가 변경되었습니다. 다시 불러와 주세요.".into());
    }
    image_data_url(bytes, thumbnail, cancel)
}
fn download_root(root: &str) -> Result<PathBuf, String> {
    let base = PathBuf::from(root.trim());
    if root.trim().is_empty() || !base.is_absolute() {
        return Err("설정에서 다운로드 폴더를 먼저 지정해 주세요.".into());
    }
    let path = base.join("Pinterest");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if redirected(&meta) || !meta.is_dir() {
            return Err("Pinterest 저장 폴더를 확인해 주세요.".into());
        }
    }
    Ok(path)
}
fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn read_regular_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(path).map_err(|_| "저장된 파일을 확인하지 못했습니다.")?;
    if redirected(&before) || !before.is_file() || before.len() > limit {
        return Err("저장된 파일 경로 또는 크기가 올바르지 않습니다.".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options
        .open(path)
        .map_err(|_| "저장된 파일을 열지 못했습니다.")?;
    if redirected(
        &file
            .metadata()
            .map_err(|_| "저장된 파일을 확인하지 못했습니다.")?,
    ) {
        return Err("연결된 파일은 지원하지 않습니다.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| "저장 파일의 연결 상태를 확인하지 못했습니다.")?;
        if info.nNumberOfLinks != 1 {
            return Err("연결된 파일은 지원하지 않습니다.".into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|_| "파일 확인 실패")?.nlink() != 1 {
            return Err("연결된 파일은 지원하지 않습니다.".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "저장된 파일을 읽지 못했습니다.")?;
    if bytes.len() as u64 > limit {
        return Err("저장된 파일 크기가 너무 큽니다.".into());
    }
    Ok(bytes)
}

fn original_bytes(
    folder: &Path,
    record: &PinterestDownload,
    page: usize,
) -> Result<Vec<u8>, String> {
    let name = record
        .files
        .get(page)
        .ok_or("페이지 번호가 올바르지 않습니다.")?;
    let path = folder.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(|_| "저장된 원본 파일이 없습니다.")?;
    if redirected(&metadata) || !metadata.is_file() || metadata.len() > MAX_IMAGE {
        return Err("저장된 원본 경로가 올바르지 않습니다.".into());
    }
    let bytes = read_regular_file(&path, MAX_IMAGE)?;
    if bytes.len() as u64 > MAX_IMAGE || bytes.is_empty() {
        return Err("저장된 이미지 크기가 올바르지 않습니다.".into());
    }
    if let Some(expected) = record.sha256.get(page) {
        if &format!("{:x}", Sha256::digest(&bytes)) != expected {
            return Err("저장된 원본이 변경되었거나 손상되었습니다.".into());
        }
    }
    Ok(bytes)
}
fn read_record(folder: &Path) -> Result<PinterestDownload, String> {
    let expected_id = folder
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("저장 폴더 이름이 올바르지 않습니다.")?;
    read_record_for(folder, expected_id)
}
fn read_record_for(folder: &Path, expected_id: &str) -> Result<PinterestDownload, String> {
    read_record_metadata(folder, expected_id, false)
}
fn read_record_metadata(
    folder: &Path,
    expected_id: &str,
    allow_missing: bool,
) -> Result<PinterestDownload, String> {
    if redirected(&fs::symlink_metadata(folder).map_err(|_| "저장된 핀을 찾지 못했습니다.")?)
    {
        return Err("저장된 핀 경로가 올바르지 않습니다.".into());
    }
    let path = folder.join("pin.json");
    let metadata = fs::symlink_metadata(&path).map_err(|_| "저장 정보가 없습니다.")?;
    if !metadata.is_file() || redirected(&metadata) || metadata.len() > MAX_MANIFEST {
        return Err("저장 정보가 올바르지 않습니다.".into());
    }
    let mut record: PinterestDownload =
        serde_json::from_slice(&read_regular_file(&path, MAX_MANIFEST)?)
            .map_err(|_| "저장 정보를 읽지 못했습니다.")?;
    if !valid_id(&record.pin.id)
        || expected_id != record.pin.id
        || record.files.is_empty()
        || record.files.len() > MAX_PAGES
        || record.files.len() != record.pin.media.len()
        || record.pin.unsupported_video
        || (!record.sha256.is_empty()
            && (record.sha256.len() != record.files.len()
                || record
                    .sha256
                    .iter()
                    .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))))
    {
        return Err("저장 정보가 올바르지 않습니다.".into());
    }
    let mut bytes = 0_u64;
    record.missing_files = false;
    for (index, name) in record.files.iter().enumerate() {
        let expected = format!("{:03}.", index + 1);
        if !name.starts_with(&expected)
            || !matches!(name.strip_prefix(&expected), Some("jpg" | "png" | "webp"))
        {
            return Err("저장된 파일 이름이 올바르지 않습니다.".into());
        }
        let metadata = match fs::symlink_metadata(folder.join(name)) {
            Ok(metadata) => metadata,
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                record.missing_files = true;
                continue;
            }
            Err(_) => return Err("저장된 원본 파일이 없습니다.".into()),
        };
        if !metadata.is_file()
            || redirected(&metadata)
            || metadata.len() == 0
            || metadata.len() > MAX_IMAGE
        {
            return Err("저장된 원본 파일을 확인해 주세요.".into());
        }
        bytes = bytes.saturating_add(metadata.len());
    }
    if (!record.missing_files && bytes != record.bytes)
        || normalize_input(&record.pin.source_url).ok().as_deref() != Some(&record.pin.id)
        || record
            .pin
            .media
            .iter()
            .any(|media| safe_url(&media.url, true).is_err())
    {
        return Err("저장된 원본 크기 또는 출처가 올바르지 않습니다.".into());
    }
    Ok(record)
}
struct Staging {
    path: PathBuf,
    committed: bool,
    files: Vec<String>,
}
impl Drop for Staging {
    fn drop(&mut self) {
        if !self.committed {
            if fs::symlink_metadata(&self.path).is_ok_and(|metadata| !redirected(&metadata)) {
                for name in &self.files {
                    let path = self.path.join(name);
                    if fs::symlink_metadata(&path)
                        .is_ok_and(|metadata| metadata.is_file() && !redirected(&metadata))
                    {
                        let _ = fs::remove_file(path);
                    }
                }
                let _ = fs::remove_dir(&self.path);
            }
        }
    }
}
fn store_pin(
    root: &Path,
    pin: PinterestPin,
    cancel: &AtomicBool,
    fetch: impl FnMut(&PinterestMedia) -> Result<Vec<u8>, String>,
) -> Result<PinterestDownload, String> {
    store_pin_verified(root, pin, cancel, fetch, || Ok(()), publish_pin)
}
fn publish_pin(stage: &Path, target: &Path) -> Result<(), String> {
    fs::rename(stage, target)
        .map_err(|_| "완성된 핀을 보관함에 옮기지 못했습니다. 기존 파일은 유지했습니다.".into())
}
fn store_pin_verified(
    root: &Path,
    pin: PinterestPin,
    cancel: &AtomicBool,
    mut fetch: impl FnMut(&PinterestMedia) -> Result<Vec<u8>, String>,
    verify: impl Fn() -> Result<(), String>,
    publish: impl Fn(&Path, &Path) -> Result<(), String>,
) -> Result<PinterestDownload, String> {
    verify()?;
    if !valid_id(&pin.id)
        || pin.media.is_empty()
        || pin.media.len() > MAX_PAGES
        || pin.unsupported_video
    {
        return Err("정적 이미지 핀만 원본 저장할 수 있습니다.".into());
    }
    check_cancel(cancel)?;
    fs::create_dir_all(root).map_err(|_| "Pinterest 저장 폴더를 만들지 못했습니다.")?;
    let final_path = root.join(&pin.id);
    let _ownership = PinWriteGuard::begin(final_path.clone())?;
    if final_path.exists() {
        let existing = read_record(&final_path)
            .map_err(|_| "같은 핀의 저장 폴더가 이미 있습니다. 기존 파일을 덮어쓰지 않았습니다.")?;
        for page in 0..existing.files.len() {
            check_cancel(cancel)?;
            decode_image(&original_bytes(&final_path, &existing, page)?)?;
        }
        return Ok(existing);
    }
    let mut stage = Staging {
        path: root.join(format!(".partial-{}-{}", pin.id, uuid::Uuid::new_v4())),
        committed: false,
        files: Vec::new(),
    };
    fs::create_dir(&stage.path).map_err(|_| "임시 저장 폴더를 만들지 못했습니다.")?;
    let mut files = Vec::new();
    let mut hashes = Vec::new();
    let mut total = 0_u64;
    for (index, media) in pin.media.iter().enumerate() {
        check_cancel(cancel)?;
        verify()?;
        let bytes = fetch(media)?;
        total = total.saturating_add(bytes.len() as u64);
        if bytes.is_empty() || bytes.len() as u64 > MAX_IMAGE || total > MAX_PIN_BYTES {
            return Err("핀의 전체 이미지 크기가 안전한 저장 한도를 초과했습니다.".into());
        }
        decode_image(&bytes)?;
        let extension = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Jpeg) => "jpg",
            Ok(image::ImageFormat::Png) => "png",
            Ok(image::ImageFormat::WebP) => "webp",
            _ => return Err("지원하지 않는 원본 이미지 형식입니다.".into()),
        };
        let name = format!("{:03}.{extension}", index + 1);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.path.join(&name))
            .map_err(|_| "원본 파일을 만들지 못했습니다.")?;
        stage.files.push(name.clone());
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "원본 파일을 저장하지 못했습니다. 저장 공간을 확인해 주세요.")?;
        drop(file);
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let readback =
            fs::read(stage.path.join(&name)).map_err(|_| "저장한 원본을 검증하지 못했습니다.")?;
        if digest != format!("{:x}", Sha256::digest(&readback)) {
            return Err("저장한 원본 검증에 실패했습니다.".into());
        }
        hashes.push(digest);
        files.push(name);
    }
    check_cancel(cancel)?;
    let record = PinterestDownload {
        pin,
        files,
        bytes: total,
        sha256: hashes,
        missing_files: false,
        saved_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    };
    let json = serde_json::to_vec(&record).map_err(|_| "저장 정보를 만들지 못했습니다.")?;
    if json.len() as u64 > MAX_MANIFEST {
        return Err("저장 정보가 너무 큽니다.".into());
    }
    let mut manifest = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(stage.path.join("pin.json"))
        .map_err(|_| "저장 정보를 만들지 못했습니다.")?;
    stage.files.push("pin.json".into());
    manifest
        .write_all(&json)
        .and_then(|_| manifest.sync_all())
        .map_err(|_| "저장 정보를 기록하지 못했습니다.")?;
    drop(manifest);
    check_cancel(cancel)?;
    verify()?;
    // Directory rename publishes the manifest and every validated original at once.
    // Windows refuses a destination directory that already exists. Do not overwrite it.
    if final_path.exists() {
        return Err("같은 핀이 이미 저장되었습니다. 기존 파일을 덮어쓰지 않았습니다.".into());
    }
    publish(&stage.path, &final_path)?;
    stage.committed = true;
    Ok(record)
}
fn download(pin_id: &str, root: &str, cancel: &AtomicBool) -> Result<PinterestDownload, String> {
    let pin = pin_get(pin_id, cancel)?;
    let root = download_root(root)?;
    let client = client()?;
    store_pin(&root, pin, cancel, |media| {
        image_bytes(&client, media, cancel)
    })
}
fn download_session(
    app: &tauri::AppHandle,
    pin_id: &str,
    root: &str,
    cancel: &AtomicBool,
    generation: u64,
) -> Result<PinterestDownload, String> {
    let session = app.state::<crate::pinterest_session::PinterestSession>();
    session.with_generation(generation, || Ok(()))?;
    let pin = pin_get_session(app, pin_id, cancel)?;
    let root = download_root(root)?;
    let client = client()?;
    store_pin_verified(
        &root,
        pin,
        cancel,
        |media| image_bytes(&client, media, cancel),
        || session.with_generation(generation, || Ok(())),
        |stage, target| {
            session.with_generation(generation, || {
                check_cancel(cancel)?;
                publish_pin(stage, target)
            })
        },
    )
}
fn downloads(root: &str) -> Result<Vec<PinterestDownload>, String> {
    let root = download_root(root)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(root).map_err(|_| "Pinterest 다운로드 목록을 읽지 못했습니다.")?;
    let mut records = Vec::new();
    for entry in entries.take(5000).flatten() {
        if let Some(id) = entry.file_name().to_str().filter(|id| valid_id(id)) {
            if let Ok(record) = read_record_metadata(&entry.path(), id, true) {
                records.push(record);
            }
        }
    }
    records.sort_by_key(|record| std::cmp::Reverse(record.saved_at));
    Ok(records)
}

fn archive_pin(root: &str, id: &str) -> Result<String, String> {
    let id = normalize_input(id)?;
    let root = download_root(root)?;
    let source = root.join(&id);
    let _ownership = PinWriteGuard::begin(source.clone())?;
    let record = read_record_metadata(&source, &id, true)?;
    owned_folder_contents(&source, &record)?;
    let archive = root.join(".removed");
    if archive.exists()
        && redirected(
            &fs::symlink_metadata(&archive).map_err(|_| "보관 경로를 확인하지 못했습니다.")?,
        )
    {
        return Err("보관 경로가 올바르지 않습니다.".into());
    }
    fs::create_dir_all(&archive).map_err(|_| "제외 보관 폴더를 만들지 못했습니다.")?;
    let token = format!("{}-{}", id, uuid::Uuid::new_v4());
    fs::rename(&source, archive.join(&token))
        .map_err(|_| "핀을 보관함에서 제외하지 못했습니다. 원본은 유지했습니다.")?;
    Ok(token)
}

fn owned_folder_contents(source: &Path, record: &PinterestDownload) -> Result<(), String> {
    let names: HashSet<_> = record
        .files
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("pin.json"))
        .collect();
    for entry in fs::read_dir(source).map_err(|_| "저장 폴더를 확인하지 못했습니다.")?
    {
        let entry = entry.map_err(|_| "저장 폴더를 확인하지 못했습니다.")?;
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|name| names.contains(name))
        {
            return Err("핀 폴더에 별도 파일이 있어 이동하지 않았습니다.".into());
        }
    }
    Ok(())
}

fn removed_id(token: &str) -> Result<&str, String> {
    let (id, nonce) = token
        .split_once('-')
        .ok_or("복원 정보가 올바르지 않습니다.")?;
    if !valid_id(id)
        || uuid::Uuid::parse_str(nonce).is_err()
        || nonce.len() != 36
        || token.contains(['/', '\\'])
    {
        return Err("복원 정보가 올바르지 않습니다.".into());
    }
    Ok(id)
}

fn removed_record(root: &Path, token: &str) -> Result<(PathBuf, PinterestDownload), String> {
    let id = removed_id(token)?;
    let parent = root.join(".removed");
    let source = parent.join(token);
    if redirected(&fs::symlink_metadata(&parent).map_err(|_| "복원할 파일이 없습니다.")?)
        || redirected(&fs::symlink_metadata(&source).map_err(|_| "복원할 파일이 없습니다.")?)
    {
        return Err("복원 경로가 올바르지 않습니다.".into());
    }
    let record = read_record_metadata(&source, id, true)?;
    owned_folder_contents(&source, &record)?;
    Ok((source, record))
}

fn removed_records(root: &str, cancel: &AtomicBool) -> Result<Vec<PinterestRemoved>, String> {
    let root = download_root(root)?;
    let parent = root.join(".removed");
    if !parent.exists() {
        return Ok(Vec::new());
    }
    if redirected(&fs::symlink_metadata(&parent).map_err(|_| "제외 목록을 읽지 못했습니다.")?)
    {
        return Err("제외 보관 경로가 올바르지 않습니다.".into());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&parent)
        .map_err(|_| "제외 목록을 읽지 못했습니다.")?
        .take(5000)
        .flatten()
    {
        check_cancel(cancel)?;
        let Some(token) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Ok((_, record)) = removed_record(&root, &token) {
            records.push(PinterestRemoved { token, record });
        }
    }
    records.sort_by_key(|item| std::cmp::Reverse(item.record.saved_at));
    Ok(records)
}

fn removed_preview(
    root: &str,
    token: &str,
    pin_id: &str,
    page: usize,
    thumbnail: bool,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let id = normalize_input(pin_id)?;
    if removed_id(token)? != id {
        return Err("제외된 핀 정보가 일치하지 않습니다.".into());
    }
    let (folder, record) = removed_record(&download_root(root)?, token)?;
    image_data_url(original_bytes(&folder, &record, page)?, thumbnail, cancel)
}

fn restore_pin(root: &str, token: &str) -> Result<(), String> {
    let id = removed_id(token)?;
    let root = download_root(root)?;
    let target = root.join(id);
    let _ownership = PinWriteGuard::begin(target.clone())?;
    if target.exists() {
        return Err("같은 핀이 이미 있어 덮어쓰지 않았습니다.".into());
    }
    let (source, _) = removed_record(&root, token)?;
    fs::rename(&source, &target).map_err(|_| "핀을 복원하지 못했습니다.")?;
    Ok(())
}

async fn run<T: Send + 'static>(
    request_id: String,
    is_download: bool,
    task: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let guard = OperationGuard::begin(request_id, is_download)?;
    run_guarded(guard, task).await
}
async fn run_guarded<T: Send + 'static>(
    guard: OperationGuard,
    task: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || run_guarded_sync(guard, task))
        .await
        .map_err(|_| "Pinterest 작업을 완료하지 못했습니다.")?
}
fn run_guarded_sync<T>(
    guard: OperationGuard,
    task: impl FnOnce(&AtomicBool) -> Result<T, String>,
) -> Result<T, String> {
    let started = Instant::now();
    let result = task(&guard.cancel);
    tracing::info!(
        provider = "pinterest",
        diag_stage = "finished",
        success = result.is_ok(),
        cancelled = guard.cancel.load(Ordering::Relaxed),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "Pinterest operation finished"
    );
    drop(guard);
    result
}

fn download_batch_plan(ids: Vec<String>) -> Result<Vec<String>, String> {
    if ids.is_empty() || ids.len() > 128 || ids.iter().any(|id| !valid_id(id)) {
        return Err("다운로드할 핀 1~128개를 선택해 주세요.".into());
    }
    let mut seen = HashSet::new();
    Ok(ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect())
}

/// The caller owns one guarded operation for the complete accepted plan. In
/// particular, a failed item never releases the guard before the next item starts.
fn download_batch_with(
    ids: &[String],
    cancel: &AtomicBool,
    mut download_one: impl FnMut(&str, &AtomicBool) -> Result<PinterestDownload, String>,
    mut settled: impl FnMut(&PinterestDownloadOutcome),
) -> Vec<PinterestDownloadOutcome> {
    let mut outcomes = Vec::with_capacity(ids.len());
    for id in ids {
        let result = check_cancel(cancel).and_then(|()| download_one(id, cancel));
        let (status, record, error) = match result {
            Ok(record) => ("completed", Some(record), None),
            Err(error) if error == CANCELLED || cancel.load(Ordering::Relaxed) => {
                ("cancelled", None, None)
            }
            Err(error) => ("failed", None, Some(error)),
        };
        let outcome = PinterestDownloadOutcome {
            pin_id: id.clone(),
            status,
            record,
            error,
        };
        settled(&outcome);
        outcomes.push(outcome);
    }
    outcomes
}
fn require_main(window: &tauri::Webview) -> Result<(), String> {
    let url = window
        .url()
        .map_err(|_| "앱 창의 주소를 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("Pinterest 기능은 Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_pin_get(
    window: tauri::Webview,
    state: State<'_, AppState>,
    input: String,
    request_id: String,
) -> Result<PinterestPin, String> {
    require_main(&window)?;
    let root = state.settings_snapshot().ok().map(|s| s.download_root);
    let app = window.app_handle().clone();
    run(request_id, false, move |cancel| {
        let id = normalize_input(&input)?;
        if let Some(root) = root.as_deref().and_then(|r| download_root(r).ok()) {
            if root.join(&id).exists() {
                return read_record(&root.join(&id)).map(|r| r.pin);
            }
        }
        pin_get_session(&app, &input, cancel)
    })
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_discover(
    window: tauri::Webview,
    state: State<'_, AppState>,
    input: String,
    kind: String,
    request_id: String,
    cursor: Option<String>,
) -> Result<PinterestDiscovery, String> {
    require_main(&window)?;
    state.begin_platform_work(|| Ok(()))?;
    let app = window.app_handle().clone();
    run(request_id, false, move |cancel| {
        let session = app.state::<crate::pinterest_session::PinterestSession>();
        let page = session.discover(&app, &input, &kind, cursor.as_deref(), cancel)?;
        let mut result = page.discovery;
        result.cursor = page.cursor;
        result.auth_status = Some(page.auth_status.into());
        result.generation = Some(page.generation);
        Ok(result)
    })
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_preview_get(
    window: tauri::Webview,
    state: State<'_, AppState>,
    pin_id: String,
    page: usize,
    request_id: String,
    thumbnail: bool,
    removed_token: Option<String>,
) -> Result<String, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .ok()
        .map(|settings| settings.download_root);
    let app = window.app_handle().clone();
    run(request_id, false, move |cancel| {
        if let Some(token) = removed_token {
            return removed_preview(
                root.as_deref().ok_or("다운로드 설정을 읽지 못했습니다.")?,
                &token,
                &pin_id,
                page,
                thumbnail,
                cancel,
            );
        }
        preview_get_session(&app, &pin_id, page, thumbnail, root.as_deref(), cancel)
    })
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_download(
    window: tauri::Webview,
    state: State<'_, AppState>,
    pin_id: String,
    request_id: String,
) -> Result<PinterestDownload, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    let app = window.app_handle().clone();
    let (guard, generation) = state.begin_platform_work(|| {
        Ok((
            OperationGuard::begin(request_id, true)?,
            app.state::<crate::pinterest_session::PinterestSession>()
                .generation(),
        ))
    })?;
    run_guarded(guard, move |cancel| {
        download_session(&app, &pin_id, &root, cancel, generation)
    })
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_download_batch(
    window: tauri::Webview,
    state: State<'_, AppState>,
    pin_ids: Vec<String>,
    request_id: String,
    on_progress: tauri::ipc::Channel<PinterestDownloadOutcome>,
) -> Result<Vec<PinterestDownloadOutcome>, String> {
    require_main(&window)?;
    let ids = download_batch_plan(pin_ids)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    // Registration is atomic with app quit/update reservation and remains live
    // until every accepted pin, including cancelled queued pins, is settled.
    let app = window.app_handle().clone();
    let (guard, generation) = state.begin_platform_work(|| {
        Ok((
            OperationGuard::begin(request_id, true)?,
            app.state::<crate::pinterest_session::PinterestSession>()
                .generation(),
        ))
    })?;
    run_guarded(guard, move |cancel| {
        Ok(download_batch_with(
            &ids,
            cancel,
            |id, cancel| download_session(&app, id, &root, cancel, generation),
            |outcome| {
                // A closed renderer must not leave a detached unobserved queue running.
                if on_progress.send(outcome.clone()).is_err() {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        ))
    })
    .await
}
#[tauri::command]
pub async fn pinterest_downloads_list(
    window: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Vec<PinterestDownload>, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    tauri::async_runtime::spawn_blocking(move || downloads(&root))
        .await
        .map_err(|_| "Pinterest 목록을 읽지 못했습니다.")?
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_removed_list(
    window: tauri::Webview,
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Vec<PinterestRemoved>, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    run(request_id, false, move |cancel| {
        removed_records(&root, cancel)
    })
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_library_action(
    window: tauri::Webview,
    state: State<'_, AppState>,
    action: String,
    id: String,
    request_id: String,
) -> Result<String, String> {
    require_main(&window)?;
    if !matches!(action.as_str(), "archive" | "restore" | "folder") {
        return Err("보관함 동작이 올바르지 않습니다.".into());
    }
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    let guard = state.begin_platform_work(|| OperationGuard::begin(request_id, true))?;
    run_guarded(guard, move |cancel| {
        check_cancel(cancel)?;
        match action.as_str() {
            "archive" => archive_pin(&root, &id),
            "restore" => {
                restore_pin(&root, &id)?;
                Ok(String::new())
            }
            _ => {
                let normalized = normalize_input(&id)?;
                let path = download_root(&root)?.join(&normalized);
                read_record_metadata(&path, &normalized, true)?;
                open_browser(path.to_str().ok_or("저장 경로를 열지 못했습니다.")?)?;
                Ok(String::new())
            }
        }
    })
    .await
}
#[tauri::command(rename_all = "camelCase")]
pub fn pinterest_cancel(window: tauri::Webview, request_id: String) -> Result<(), String> {
    require_main(&window)?;
    cancel_request(&request_id);
    Ok(())
}
fn cancel_request(request_id: &str) {
    cancel_request_in(operations(), request_id);
}
fn cancel_request_in(registry: &OperationRegistry, request_id: &str) {
    if let Some(item) = lock_operations(registry).get(request_id) {
        item.cancel.store(true, Ordering::Relaxed);
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_open_external(
    window: tauri::Webview,
    pin_id: Option<String>,
) -> Result<(), String> {
    require_main(&window)?;
    let url = match pin_id {
        Some(id) => format!("https://www.pinterest.com/pin/{}/", normalize_input(&id)?),
        None => "https://www.pinterest.com/search/pins/".into(),
    };
    tauri::async_runtime::spawn_blocking(move || open_browser(&url))
        .await
        .map_err(|_| "브라우저를 열지 못했습니다.")?
}
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let wide = url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err("기본 브라우저를 열지 못했습니다.".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("이 환경에서는 외부 브라우저 열기를 지원하지 않습니다.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    std::thread_local! {
        static OFFLINE_NETWORK: std::cell::Cell<(bool, usize)> = const { std::cell::Cell::new((false, 0)) };
    }
    const OFFLINE_NETWORK_ERROR: &str = "offline verification forbids network client entry";
    pub(super) fn check_network_client_entry() -> Result<(), String> {
        OFFLINE_NETWORK.with(|state| {
            let (denied, attempts) = state.get();
            if denied {
                state.set((true, attempts + 1));
                Err(OFFLINE_NETWORK_ERROR.into())
            } else {
                Ok(())
            }
        })
    }
    struct OfflineNetworkGuard((bool, usize));
    impl OfflineNetworkGuard {
        fn new() -> Self {
            Self(OFFLINE_NETWORK.with(|state| state.replace((true, 0))))
        }
        fn attempts(&self) -> usize {
            OFFLINE_NETWORK.with(|state| state.get().1)
        }
    }
    impl Drop for OfflineNetworkGuard {
        fn drop(&mut self) {
            OFFLINE_NETWORK.with(|state| state.set(self.0));
        }
    }
    #[test]
    fn offline_client_guard_denies_entry_and_is_thread_local() {
        let prior = OFFLINE_NETWORK.with(std::cell::Cell::get);
        {
            let guard = OfflineNetworkGuard::new();
            assert!(matches!(client(), Err(error) if error == OFFLINE_NETWORK_ERROR));
            assert_eq!(guard.attempts(), 1);
            assert!(std::thread::spawn(|| check_network_client_entry().is_ok())
                .join()
                .unwrap());
            assert_eq!(guard.attempts(), 1);
        }
        assert_eq!(OFFLINE_NETWORK.with(std::cell::Cell::get), prior);
    }
    #[test]
    fn offline_client_guard_restores_state_after_unwind() {
        let prior = OFFLINE_NETWORK.with(std::cell::Cell::get);
        let result = std::panic::catch_unwind(|| {
            let _guard = OfflineNetworkGuard::new();
            panic!("offline guard unwind fixture");
        });
        assert!(result.is_err());
        assert_eq!(OFFLINE_NETWORK.with(std::cell::Cell::get), prior);
    }
    fn qa_media_root() -> PathBuf {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut path = repository.to_path_buf();
        for component in [
            ".runtime",
            "platform-app-qa",
            "local.atsumi.platformqa.fa0e0692d85b478ba8f3b1ddbe41431b",
            "media",
            "Pinterest",
            "1076430748452447388",
        ] {
            path.push(component);
            let metadata = fs::symlink_metadata(&path).expect("missing approved offline fixture");
            assert!(
                metadata.is_dir() && !redirected(&metadata),
                "unsafe approved fixture directory"
            );
        }
        let provider = path.parent().unwrap();
        let entries = fs::read_dir(provider)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            entries.len() == 1 && entries[0].file_name() == "1076430748452447388",
            "unexpected fixture provider entry"
        );
        provider.parent().unwrap().to_path_buf()
    }
    fn qa_snapshot(folder: &Path) -> Vec<(String, u64, String)> {
        let mut names = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        assert!(
            names
                == [
                    std::ffi::OsString::from("001.jpg"),
                    std::ffi::OsString::from("pin.json")
                ],
            "unexpected approved fixture files"
        );
        names
            .into_iter()
            .map(|name| {
                let bytes = read_regular_file(&folder.join(&name), MAX_IMAGE)
                    .expect("approved fixture read failed");
                (
                    name.to_string_lossy().into_owned(),
                    bytes.len() as u64,
                    format!("{:x}", Sha256::digest(&bytes)),
                )
            })
            .collect()
    }
    #[test]
    #[ignore = "Opt-in read-only exact GUI-saved QA fixture; run alone in a fresh native test process"]
    fn qa_saved_original_reopens_offline_without_mutation() {
        let guard = OfflineNetworkGuard::new();
        let root = qa_media_root();
        let folder = root.join("Pinterest/1076430748452447388");
        let before = qa_snapshot(&folder);
        assert!(
            before[0].1 == 1_256_242
                && before[0].2
                    == "916a1cf1b6e0c574682e6cafd74662f3b60005ba07b1d56490f34c814d6e976e",
            "original fixture baseline mismatch"
        );
        assert!(
            before[1].1 == 750
                && before[1].2
                    == "4b49418ec0dfd6c2270fe86bf7fb51ce2168bec817c19d468b6b05e33a97bace",
            "manifest fixture baseline mismatch"
        );
        let records = downloads(root.to_str().unwrap()).expect("offline list failed");
        assert!(
            records.len() == 1 && records[0].pin.id == "1076430748452447388",
            "offline list mismatch"
        );
        let record = read_record(&folder).expect("offline detail failed");
        assert_eq!(record.files.len(), 1);
        let cancel = AtomicBool::new(false);
        let original = preview_get(&record.pin.id, 0, false, root.to_str(), &cancel)
            .expect("offline original reader failed");
        let bytes = STANDARD
            .decode(
                original
                    .strip_prefix("data:image/jpeg;base64,")
                    .expect("original MIME mismatch"),
            )
            .expect("original payload invalid");
        assert!(
            bytes == original_bytes(&folder, &record, 0).expect("saved original read failed"),
            "original reader changed stored bytes"
        );
        let decoded = decode_image(&bytes).expect("original decode failed");
        assert_eq!((decoded.width(), decoded.height()), (1440, 2560));
        let thumbnail = preview_get(&record.pin.id, 0, true, root.to_str(), &cancel)
            .expect("offline thumbnail reader failed");
        let thumb = STANDARD
            .decode(
                thumbnail
                    .strip_prefix("data:image/jpeg;base64,")
                    .expect("thumbnail MIME mismatch"),
            )
            .expect("thumbnail payload invalid");
        let decoded_thumb = decode_image(&thumb).expect("thumbnail decode failed");
        assert!(
            thumb.len() <= 256 * 1024
                && decoded_thumb.width() <= 480
                && decoded_thumb.height() <= 480,
            "thumbnail exceeds display bounds"
        );
        assert_eq!(guard.attempts(), 0, "offline reader entered HTTP client");
        assert!(
            before == qa_snapshot(&folder),
            "approved originals or manifest changed during read"
        );
    }
    fn pin() -> PinterestPin {
        PinterestPin {
            id: "621004236103700020".into(),
            title: "Landscape".into(),
            description: String::new(),
            creator_name: None,
            creator_path: None,
            summary_only: false,
            source_url: "https://www.pinterest.com/pin/621004236103700020/".into(),
            media: vec![PinterestMedia {
                url: "https://i.pinimg.com/originals/a/b.jpg".into(),
                width: None,
                height: None,
            }],
            unsupported_video: false,
        }
    }
    fn jpeg() -> Vec<u8> {
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::new(4, 4))
            .unwrap();
        bytes
    }
    #[test]
    fn validates_pin_urls_and_string_ids() {
        assert_eq!(
            normalize_input("https://se.pinterest.com/pin/mountain--621004236103700020/?x=1")
                .unwrap(),
            "621004236103700020"
        );
        for url in [
            "http://www.pinterest.com/pin/621004236103700020/",
            "https://www.pinterest.com.evil.test/pin/621004236103700020/",
            "https://user@www.pinterest.com/pin/621004236103700020/",
            "https://127.0.0.1/pin/621004236103700020/",
            "https://www.pinterest.com:8080/pin/621004236103700020/",
            "https://www.pinterest.com/pin/abc",
            "https://www.pinterest.com/pin/12345/extra",
        ] {
            assert!(normalize_input(url).is_err(), "{url}");
        }
    }
    #[test]
    fn media_allowlist_is_exact_and_https_only() {
        assert!(safe_url("https://i.pinimg.com/originals/a.jpg", true).is_ok());
        for url in [
            "http://i.pinimg.com/a.jpg",
            "https://i.pinimg.com.evil.test/a.jpg",
            "https://localhost/a.jpg",
            "file:///image.png",
            "https://user:pass@i.pinimg.com/a.jpg",
        ] {
            assert!(safe_url(url, true).is_err());
        }
    }
    #[test]
    fn reads_only_schema_images_not_recommendations() {
        let html = r#"<script type="application/json">{"recommendation":"https://i.pinimg.com/originals/wrong.jpg"}</script><script type="application/ld+json">{"@type":"SocialMediaPosting","headline":"Mountains","image":["https://i.pinimg.com/originals/a.jpg",{"contentUrl":"https://i.pinimg.com/originals/b.png","width":300}],"author":{"name":"Public author"},"mainEntityOfPage":{"@id":"https://www.pinterest.com/pin/621004236103700020/"}}</script>"#;
        let result = parse_pin(html, "621004236103700020").unwrap();
        assert_eq!(result.media.len(), 2);
        assert_eq!(result.title, "Mountains");
        assert_eq!(result.media[1].width, Some(300));
        assert!(!result.unsupported_video);
        assert!(parse_pin("<html>Log in</html>", "621004236103700020").is_err());
    }
    #[test]
    fn marks_video_pin_without_download() {
        let html = r#"<script type="application/ld+json">{"@type":"SocialMediaPosting","image":"https://i.pinimg.com/originals/a.jpg","sharedContent":{"@type":"VideoObject"}}</script>"#;
        assert!(
            parse_pin(html, "621004236103700020")
                .unwrap()
                .unsupported_video
        );
    }
    #[test]
    fn never_silently_drops_unsafe_or_over_limit_pages() {
        let mixed = serde_json::json!([
            "https://i.pinimg.com/originals/a.jpg",
            "https://evil.example/image.jpg"
        ]);
        assert!(append_media(&mixed, &mut Vec::new(), 0).is_err());
        let many = Value::Array(
            (0..51)
                .map(|index| Value::String(format!("https://i.pinimg.com/originals/{index}.jpg")))
                .collect(),
        );
        assert!(append_media(&many, &mut Vec::new(), 0).is_err());
    }
    #[test]
    fn commits_all_pages_and_keeps_existing_originals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let mut two = pin();
        two.media.push(PinterestMedia {
            url: "https://i.pinimg.com/originals/b.png".into(),
            width: None,
            height: None,
        });
        let record = store_pin(&root, two.clone(), &cancel, |_| Ok(jpeg())).unwrap();
        assert_eq!(record.files, ["001.jpg", "002.jpg"]);
        assert_eq!(
            read_record(&root.join(&two.id)).unwrap().bytes,
            record.bytes
        );
        assert!(store_pin(&root, two, &cancel, |_| panic!("must not download twice")).is_ok());
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }
    #[test]
    fn failure_or_cancel_never_publishes_incomplete_record() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let mut two = pin();
        two.media.push(two.media[0].clone());
        let mut count = 0;
        assert!(store_pin(&root, two, &cancel, |_| {
            count += 1;
            if count == 2 {
                Err("network failure".into())
            } else {
                Ok(jpeg())
            }
        })
        .is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        assert!(store_pin(&root, pin(), &cancel, |_| {
            cancel.store(true, Ordering::Relaxed);
            Ok(jpeg())
        })
        .is_err());
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }
    #[test]
    fn ignores_partial_staging_and_missing_media_in_download_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let record = store_pin(&root, pin(), &cancel, |_| Ok(jpeg())).unwrap();
        fs::create_dir(root.join(".partial-interrupted")).unwrap();
        assert_eq!(downloads(dir.path().to_str().unwrap()).unwrap().len(), 1);
        fs::remove_file(root.join(record.pin.id).join("001.jpg")).unwrap();
        let remaining = downloads(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].missing_files);
    }
    #[test]
    fn batch_plan_is_bounded_validated_and_deduplicated_before_work() {
        assert!(download_batch_plan(Vec::new()).is_err());
        assert!(download_batch_plan(vec!["12345".into(); 129]).is_err());
        assert!(download_batch_plan(vec!["12345".into(), "../12346".into()]).is_err());
        assert_eq!(
            download_batch_plan(vec!["12345".into(), "12346".into(), "12345".into()]).unwrap(),
            ["12345", "12346"]
        );
    }

    fn batch_record(id: &str) -> PinterestDownload {
        let mut item = pin();
        item.id = id.into();
        PinterestDownload {
            pin: item,
            files: vec!["001.jpg".into()],
            bytes: 100,
            saved_at: 1,
            sha256: Vec::new(),
            missing_files: false,
        }
    }

    #[test]
    fn whole_batch_stays_guarded_between_success_and_failure_boundaries() {
        let registry = Arc::new(Mutex::new(HashMap::new()));
        let id = "pinterest-test-whole-batch";
        let guard = OperationGuard::begin_in(registry.clone(), id.into(), true).unwrap();
        let ids =
            download_batch_plan(vec!["12345".into(), "12346".into(), "12347".into()]).unwrap();
        let mut emitted = Vec::new();
        let outcomes = run_guarded_sync(guard, |cancel| {
            Ok(download_batch_with(
                &ids,
                cancel,
                |pin_id, _| {
                    assert_eq!(active_download_ids_in(&registry), [id]);
                    if pin_id == "12346" {
                        Err("fixture failure".into())
                    } else {
                        Ok(batch_record(pin_id))
                    }
                },
                |outcome| {
                    // This is the old renderer reply/next-invoke gap. The complete plan
                    // remains registered even after a pin has fully completed or failed.
                    assert_eq!(active_download_count_in(&registry), 1);
                    emitted.push(outcome.status);
                },
            ))
        })
        .unwrap();
        assert_eq!(emitted, ["completed", "failed", "completed"]);
        assert_eq!(outcomes[1].error.as_deref(), Some("fixture failure"));
        assert!(active_download_ids_in(&registry).is_empty());
    }

    #[test]
    fn batch_shutdown_cancels_every_queued_pin_without_starting_it() {
        let registry = Arc::new(Mutex::new(HashMap::new()));
        let guard = OperationGuard::begin_in(
            registry.clone(),
            "pinterest-test-batch-shutdown".into(),
            true,
        )
        .unwrap();
        let ids =
            download_batch_plan(vec!["12345".into(), "12346".into(), "12347".into()]).unwrap();
        let mut started = Vec::new();
        let outcomes = run_guarded_sync(guard, |cancel| {
            Ok(download_batch_with(
                &ids,
                cancel,
                |id, _| {
                    started.push(id.to_owned());
                    Ok(batch_record(id))
                },
                |_| {
                    assert_eq!(active_download_count_in(&registry), 1);
                    cancel_all_downloads_in(&registry);
                },
            ))
        })
        .unwrap();
        assert_eq!(started, ["12345"]);
        assert_eq!(
            outcomes.iter().map(|item| item.status).collect::<Vec<_>>(),
            ["completed", "cancelled", "cancelled"]
        );
        assert_eq!(active_download_count_in(&registry), 0);
    }

    #[test]
    fn batch_cancel_before_first_item_and_panic_both_drain_guard() {
        let registry = Arc::new(Mutex::new(HashMap::new()));
        let id = "pinterest-test-batch-cancel";
        let guard = OperationGuard::begin_in(registry.clone(), id.into(), true).unwrap();
        cancel_request_in(&registry, id);
        let ids = download_batch_plan(vec!["12345".into(), "12346".into()]).unwrap();
        let outcomes = run_guarded_sync(guard, |cancel| {
            Ok(download_batch_with(
                &ids,
                cancel,
                |_, _| panic!("cancelled plan must not start I/O"),
                |_| {},
            ))
        })
        .unwrap();
        assert!(outcomes
            .iter()
            .all(|item| item.status == "cancelled" && item.error.is_none()));
        assert_eq!(active_download_count_in(&registry), 0);
        let guard = OperationGuard::begin_in(registry.clone(), id.into(), true).unwrap();
        let result = std::panic::catch_unwind(|| {
            run_guarded_sync(guard, |cancel| {
                Ok(download_batch_with(
                    &ids,
                    cancel,
                    |_, _| panic!("fixture worker panic"),
                    |_| {},
                ))
            })
        });
        assert!(result.is_err());
        assert!(active_download_ids_in(&registry).is_empty());
    }

    #[test]
    fn checks_cancellation_and_counts_only_download_work() {
        let registry = Arc::new(Mutex::new(HashMap::new()));
        let guard = OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-cancel-123".into(),
            true,
        )
        .unwrap();
        let metadata = OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-metadata-123".into(),
            false,
        )
        .unwrap();
        assert_eq!(active_download_count_in(&registry), 1);
        cancel_request_in(&registry, "pinterest-test-cancel-123");
        assert_eq!(check_cancel(&guard.cancel).unwrap_err(), CANCELLED);
        assert!(check_cancel(&metadata.cancel).is_ok());
        drop(guard);
        assert_eq!(active_download_count_in(&registry), 0);
    }
    #[test]
    fn poisoned_local_registry_still_cancels_and_drains_real_jobs() {
        let registry = Arc::new(Mutex::new(HashMap::new()));
        let first = OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-download-a".into(),
            true,
        )
        .unwrap();
        let second = OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-download-b".into(),
            true,
        )
        .unwrap();
        let metadata = OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-metadata".into(),
            false,
        )
        .unwrap();
        let thread_registry = Arc::clone(&registry);
        assert!(std::thread::spawn(move || {
            let _lock = thread_registry.lock().unwrap();
            panic!("poison only the isolated test registry");
        })
        .join()
        .is_err());
        assert!(registry.is_poisoned());
        assert_eq!(active_download_count_in(&registry), 2);
        assert_eq!(
            active_download_ids_in(&registry),
            ["pinterest-test-download-a", "pinterest-test-download-b"]
        );
        assert!(OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-over-limit".into(),
            true,
        )
        .is_err());
        cancel_request_in(&registry, "pinterest-test-download-a");
        assert!(first.cancel.load(Ordering::Relaxed));
        assert!(!second.cancel.load(Ordering::Relaxed));
        cancel_all_downloads_in(&registry);
        assert!(second.cancel.load(Ordering::Relaxed));
        assert!(!metadata.cancel.load(Ordering::Relaxed));
        drop(first);
        drop(second);
        assert_eq!(active_download_count_in(&registry), 0);
        assert!(active_download_ids_in(&registry).is_empty());
        let next = OperationGuard::begin_in(
            Arc::clone(&registry),
            "pinterest-test-next-download".into(),
            true,
        )
        .unwrap();
        assert_eq!(active_download_count_in(&registry), 1);
        drop(next);
        drop(metadata);
        assert!(lock_operations(&registry).is_empty());
    }
    #[test]
    fn rejects_invalid_image_payloads_before_publish() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        assert!(store_pin(&root, pin(), &AtomicBool::new(false), |_| Ok(
            b"<html>Login</html>".to_vec()
        ))
        .is_err());
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }
    #[test]
    fn session_projection_revalidates_ids_origins_dimensions_and_board_paths() {
        let mut candidate = serde_json::to_value(pin()).unwrap();
        assert_eq!(
            session_pins(&serde_json::json!([candidate]), None)
                .unwrap()
                .len(),
            1
        );
        candidate["media"][0]["url"] = serde_json::json!("https://evil.test/original.jpg");
        assert!(session_pins(&serde_json::json!([candidate]), None).is_err());
        let mut candidate = serde_json::to_value(pin()).unwrap();
        candidate["media"][0]["width"] = serde_json::json!(20000);
        assert!(session_pins(&serde_json::json!([candidate]), None).is_err());
        assert!(session_pins(&serde_json::json!([]), None)
            .unwrap()
            .is_empty());
        assert!(session_pins(&serde_json::json!([]), Some("1234567")).is_err());
        let boards = session_boards(&serde_json::json!([
            {"title":"한글", "path":"/fixture/한글/"},
            {"title":"foreign", "path":"https://evil.test/fixture/board/"},
            {"title":"separator", "path":"/fixture/%2fetc/"}
        ]));
        assert_eq!(boards.len(), 1);
        assert_eq!(session_discovery_path("", "home").unwrap(), "/");
        assert!(session_discovery_path("unexpected", "home").is_err());
    }
    #[test]
    fn a_session_change_at_publish_discards_only_the_new_partial() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let outcome = store_pin_verified(
            &root,
            pin(),
            &AtomicBool::new(false),
            |_| Ok(jpeg()),
            || Ok(()),
            |_, _| Err("stale generation fixture".into()),
        );
        assert!(matches!(outcome, Err(error) if error == "stale generation fixture"));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
    #[test]
    fn local_previews_work_without_remote_requests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        store_pin(&root, pin(), &cancel, |_| Ok(jpeg())).unwrap();
        let preview =
            preview_get("621004236103700020", 0, true, dir.path().to_str(), &cancel).unwrap();
        assert!(preview.starts_with("data:image/jpeg;base64,"));
        assert!(preview.len() < 8_000);
    }
    #[test]
    fn public_profile_and_board_keep_order_and_do_not_collect_unrelated_pins() {
        let value = serde_json::json!({"initialReduxState": {
            "pins": {"99999": {"id":"99999","type":"pin"}},
            "resources": {
                "UserResource": {"key":{"data":{"username":"creator","full_name":"Creator"}}},
                "UserPinsResource": {"key":{"data":[
                    {"id":"123456","type":"pin","images":{"orig":{"url":"https://i.pinimg.com/originals/a.jpg"}},"videos":null},
                    {"id":"123457","type":"pin","images":{"orig":{"url":"https://i.pinimg.com/originals/b.jpg"}},"videos":null}
                ],"nextBookmark":"more"}},
                "BoardsFeedResource":{"key":{"data":[{"name":"Art","url":"/creator/art/"},{"name":"Invalid","url":"https://evil.example/a/"}]}}
            }
        }});
        let html = format!(
            "<script id=\"__PWS_INITIAL_PROPS__\" type=\"application/json\">{value}</script>"
        );
        let result = parse_discovery(&html, "/creator/", false).unwrap();
        assert_eq!(
            result
                .pins
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["123456", "123457"]
        );
        assert_eq!(result.boards.len(), 1);
        assert!(result.limited && result.pins.iter().all(|pin| pin.summary_only));
        assert_eq!(result.pins[0].creator_path.as_deref(), Some("/creator/"));
    }
    #[test]
    fn login_shell_is_not_empty_search_success_and_profile_paths_are_constrained() {
        let html = r#"<script id="__PWS_INITIAL_PROPS__">{"initialReduxState":{"resources":{},"pins":{}}}</script>"#;
        assert!(parse_discovery(html, "/search/pins/?q=landscape", true)
            .unwrap_err()
            .contains("로그인"));
        assert_eq!(profile_path("@pinterest").unwrap(), "/pinterest/");
        assert_eq!(
            profile_path("https://www.pinterest.com/pinterest/girls-night-in/").unwrap(),
            "/pinterest/girls-night-in/"
        );
        for invalid in [
            "../secrets",
            "https://localhost/pinterest/",
            "/login/",
            "/resource/thing/",
            "/one/two/three/",
        ] {
            assert!(profile_path(invalid).is_err());
        }
    }
    #[test]
    fn verified_original_bytes_are_preserved_and_corruption_is_not_remote_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let bytes = jpeg();
        let record = store_pin(&root, pin(), &cancel, |_| Ok(bytes.clone())).unwrap();
        assert_eq!(record.sha256, [format!("{:x}", Sha256::digest(&bytes))]);
        let full = preview_get(&record.pin.id, 0, false, dir.path().to_str(), &cancel).unwrap();
        assert_eq!(
            full,
            format!("data:image/jpeg;base64,{}", STANDARD.encode(&bytes))
        );
        let mut corrupted = bytes;
        corrupted[10] ^= 1;
        fs::write(root.join(&record.pin.id).join("001.jpg"), corrupted).unwrap();
        assert!(
            preview_get(&record.pin.id, 0, false, dir.path().to_str(), &cancel)
                .unwrap_err()
                .contains("손상")
        );
    }
    #[test]
    fn archive_is_recoverable_and_missing_originals_do_not_trap_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let record = store_pin(&root, pin(), &AtomicBool::new(false), |_| Ok(jpeg())).unwrap();
        fs::remove_file(root.join(&record.pin.id).join("001.jpg")).unwrap();
        assert!(downloads(dir.path().to_str().unwrap()).unwrap()[0].missing_files);
        let token = archive_pin(dir.path().to_str().unwrap(), &record.pin.id).unwrap();
        assert!(downloads(dir.path().to_str().unwrap()).unwrap().is_empty());
        assert!(root
            .join(".removed")
            .join(&token)
            .join("pin.json")
            .is_file());
        restore_pin(dir.path().to_str().unwrap(), &token).unwrap();
        assert!(downloads(dir.path().to_str().unwrap()).unwrap()[0].missing_files);
        assert!(restore_pin(dir.path().to_str().unwrap(), "../../../wrong").is_err());
    }
    #[test]
    fn archive_rejects_unowned_extra_files_and_busy_pin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let record = store_pin(&root, pin(), &AtomicBool::new(false), |_| Ok(jpeg())).unwrap();
        let folder = root.join(&record.pin.id);
        let ownership = PinWriteGuard::begin(folder.clone()).unwrap();
        assert!(archive_pin(dir.path().to_str().unwrap(), &record.pin.id).is_err());
        drop(ownership);
        fs::write(folder.join("user-notes.txt"), "keep").unwrap();
        assert!(archive_pin(dir.path().to_str().unwrap(), &record.pin.id).is_err());
        assert!(folder.join("user-notes.txt").is_file());
    }
    #[test]
    fn removed_records_are_rediscovered_and_previewed_without_network_or_session_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let bytes = jpeg();
        let record = store_pin(&root, pin(), &cancel, |_| Ok(bytes.clone())).unwrap();
        let token = archive_pin(dir.path().to_str().unwrap(), &record.pin.id).unwrap();
        let records = removed_records(dir.path().to_str().unwrap(), &cancel).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].token, token);
        assert_eq!(records[0].record.pin.id, record.pin.id);
        let preview = removed_preview(
            dir.path().to_str().unwrap(),
            &records[0].token,
            &record.pin.id,
            0,
            false,
            &cancel,
        )
        .unwrap();
        assert_eq!(
            preview,
            format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
        );
        assert!(removed_preview(
            dir.path().to_str().unwrap(),
            &token,
            "999999",
            0,
            false,
            &cancel
        )
        .is_err());
        // A newly saved copy must not be overwritten by an older excluded copy.
        store_pin(&root, record.pin.clone(), &cancel, |_| Ok(jpeg())).unwrap();
        assert!(restore_pin(dir.path().to_str().unwrap(), &token)
            .unwrap_err()
            .contains("덮어쓰지"));
        assert_eq!(
            removed_records(dir.path().to_str().unwrap(), &cancel)
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn removed_records_skip_unowned_folders_but_retain_missing_file_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let record = store_pin(&root, pin(), &cancel, |_| Ok(jpeg())).unwrap();
        let token = archive_pin(dir.path().to_str().unwrap(), &record.pin.id).unwrap();
        let folder = root.join(".removed").join(&token);
        fs::remove_file(folder.join("001.jpg")).unwrap();
        assert!(
            removed_records(dir.path().to_str().unwrap(), &cancel).unwrap()[0]
                .record
                .missing_files
        );
        assert!(removed_preview(
            dir.path().to_str().unwrap(),
            &token,
            &record.pin.id,
            0,
            false,
            &cancel
        )
        .is_err());
        fs::create_dir(root.join(".removed").join("user-documents")).unwrap();
        assert_eq!(
            removed_records(dir.path().to_str().unwrap(), &cancel)
                .unwrap()
                .len(),
            1
        );
        fs::write(folder.join("user-notes.txt"), "keep").unwrap();
        assert!(removed_records(dir.path().to_str().unwrap(), &cancel)
            .unwrap()
            .is_empty());
        assert!(restore_pin(dir.path().to_str().unwrap(), &token).is_err());
        assert!(folder.join("user-notes.txt").exists());
        assert!(removed_records(dir.path().to_str().unwrap(), &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn hardlinked_originals_are_not_served() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pinterest");
        let record = store_pin(&root, pin(), &AtomicBool::new(false), |_| Ok(jpeg())).unwrap();
        fs::hard_link(
            root.join(&record.pin.id).join("001.jpg"),
            dir.path().join("other.jpg"),
        )
        .unwrap();
        assert!(preview_get(
            &record.pin.id,
            0,
            false,
            dir.path().to_str(),
            &AtomicBool::new(false)
        )
        .is_err());
    }
    #[test]
    fn unicode_board_paths_are_canonical_but_encoded_separators_are_rejected() {
        assert_eq!(
            profile_path("/artist/풍경/").unwrap(),
            "/artist/%ED%92%8D%EA%B2%BD/"
        );
        assert_eq!(
            profile_path("/picturerna/fr%c3%a5n-fl%c3%b6det/").unwrap(),
            "/picturerna/fr%C3%A5n-fl%C3%B6det/"
        );
        for input in [
            "/artist/%2fprivate/",
            "/artist/%2e%2e/",
            "/artist/%5cprivate/",
            "/%73earch/test/",
            "/artist/%00/",
            "/artist/%zz/",
        ] {
            assert!(profile_path(input).is_err(), "{input}");
        }
    }
    #[test]
    #[ignore = "explicit anonymous discovery and isolated download verification"]
    fn live_public_discovery_roundtrip() {
        let cancel = AtomicBool::new(false);
        let profile = discover("pinterest", "profile", &cancel).unwrap();
        assert!(!profile.pins.is_empty() && !profile.boards.is_empty());
        let board = discover("/sheetal_takhar/_quick_creates/", "profile", &cancel).unwrap();
        assert!(!board.pins.is_empty());
        let fixture = tempfile::Builder::new()
            .prefix("atsumi-pinterest-discovery-")
            .tempdir()
            .unwrap();
        let candidate = board
            .pins
            .iter()
            .find(|pin| pin.id == "1076430748452447388" && !pin.unsupported_video)
            .expect("verified public tree/sky fixture must still be present; never substitute another pin");
        let record = download(&candidate.id, fixture.path().to_str().unwrap(), &cancel).unwrap();
        assert!(!record.files.is_empty() && record.sha256.len() == record.files.len());
        assert_eq!(
            downloads(fixture.path().to_str().unwrap()).unwrap().len(),
            1
        );
        let preview =
            preview_get(&record.pin.id, 0, false, fixture.path().to_str(), &cancel).unwrap();
        assert!(preview.starts_with("data:image/"));
        let keyword = discover("landscape", "keyword", &cancel);
        println!("PINTEREST_PUBLIC_PROOF profile={} boards={} board_pins={} saved_pages={} bytes={} keyword={}", profile.pins.len(), profile.boards.len(), board.pins.len(), record.files.len(), record.bytes, if keyword.is_ok() { "available" } else { "restricted" });
    }
    #[test]
    #[ignore = "explicit public network fixture verification; never uses user download root"]
    fn live_public_pin_roundtrip_in_isolated_fixture() {
        let base = PathBuf::from(
            std::env::var("ATSUMI_PINTEREST_FIXTURE_ROOT")
                .expect("set a workspace .runtime fixture directory"),
        );
        let workspace = std::env::current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let runtime = workspace.join(".runtime").canonicalize().unwrap();
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        assert!(
            base.starts_with(&runtime) && base != runtime,
            "only an isolated workspace .runtime subdirectory is allowed"
        );
        let fixture = tempfile::Builder::new()
            .prefix("public-pins-")
            .tempdir_in(&base)
            .unwrap();
        let root = fixture.path().join("Pinterest");
        let cancel = AtomicBool::new(false);
        let mut results = Vec::new();
        for id in ["621004236103700020", "611504455628959257"] {
            let pin = pin_get(id, &cancel).unwrap();
            assert!(!pin.media.is_empty());
            assert!(!pin.unsupported_video);
            let record = download(&pin.id, fixture.path().to_str().unwrap(), &cancel).unwrap();
            assert_eq!(
                read_record(&root.join(&pin.id)).unwrap().bytes,
                record.bytes
            );
            let thumb = preview_get(&pin.id, 0, true, fixture.path().to_str(), &cancel).unwrap();
            let preview = preview_get(&pin.id, 0, false, fixture.path().to_str(), &cancel).unwrap();
            assert!(thumb.len() < 350_000);
            assert!(preview.len() < 8_000_000);
            results.push(serde_json::json!({ "requestedId": id, "pin": pin, "bytes": record.bytes, "pages": record.files.len(), "thumbnail": thumb, "preview": preview }));
        }
        fs::write(
            fixture.path().join("verified-pins.json"),
            serde_json::to_vec(&results).unwrap(),
        )
        .unwrap();
        assert_eq!(
            downloads(fixture.path().to_str().unwrap()).unwrap().len(),
            2
        );
        println!("PINTEREST_FIXTURE={}", fixture.keep().display());
    }
}
