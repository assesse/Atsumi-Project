//! Pixiv browsing with isolated optional official sessions. Local bookmarks stay local.
pub mod account;
mod display;
mod discovery;
mod home;
mod localization;
mod session;
use crate::interface::commands::AppState;
use base64::{engine::general_purpose::STANDARD, Engine};
use display::DisplayPolicy;
use image::{codecs::jpeg::JpegEncoder, ImageReader, Limits};
use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
    Url,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State};

const SITE: &str = "https://www.pixiv.net";
const JSON_LIMIT: u64 = 8 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 16 * 1024 * 1024;
const PAGE_LIMIT: u64 = 256 * 1024 * 1024;
const ALBUM_LIMIT: u64 = 1024 * 1024 * 1024;
const RECORD_FILE: &str = ".atsumi-pixiv.json";
const RECORD_LIMIT: u64 = 128 * 1024;
const DISPLAY_ORIGINAL_LIMIT: u64 = 32 * 1024 * 1024;
const BODY_TOO_LARGE: &str = "이미지 또는 응답이 안전한 처리 범위를 초과합니다.";
const DISPLAY_TOO_LARGE: &str = "PIXIV_DISPLAY_LIMIT: 원본이 안전한 표시 범위(32 MB, 2,400만 화소)를 초과합니다.";
const RESTRICTED: &str = "로그인 또는 열람 설정이 필요한 작품입니다. Pixiv 원문에서 확인해 주세요.";

// Fixed stage names only: never record IDs, URLs, content, local paths or error
// strings. The shared bounded recorder filters these allowlisted fields.
fn observed<T>(
    stage: &'static str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let started = Instant::now();
    let result = operation();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if result.is_ok() {
        tracing::info!(diag_stage = stage, success = true, elapsed_ms);
    } else {
        tracing::warn!(diag_stage = stage, success = false, elapsed_ms);
    }
    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PixivWork {
    pub id: String,
    pub title: String,
    pub artist_id: String,
    pub artist: String,
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    pub bookmark_count: u64,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_translations: Vec<localization::TagTranslation>,
    pub created_at: String,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PixivPage {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub preview_url: String,
    pub original_url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PixivDetail {
    pub work: PixivWork,
    pub pages: Vec<PixivPage>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PixivSearchRequest {
    pub query: String,
    pub page: u32,
    pub mode: String,
    #[serde(default)]
    pub home_cursor: Option<String>,
    #[serde(default)]
    pub safe_only: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PixivSearchPage {
    pub items: Vec<PixivWork>,
    pub page: u32,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_suggestions: Option<home::HomeSuggestions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PixivDownload {
    pub work: PixivWork,
    pub saved_at: String,
    pub page_count: u32,
    pub bytes: u64,
    pub folder: String,
    #[serde(default)]
    pub pages: Vec<PixivLocalPage>,
    // Derived from the current filesystem, never trusted from the manifest.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub removed_token: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PixivLocalPage {
    pub index: u32,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PixivProgress {
    pub id: String,
    pub completed_pages: u32,
    pub total_pages: u32,
    pub bytes: u64,
    pub cancelled: bool,
    pub queued: bool,
}
#[derive(Clone, Serialize)]
pub struct PixivDownloadOutcome {
    pub id: String,
    pub saved: Option<PixivDownload>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct PixivLibraryResult {
    token: Option<String>,
    record: Option<PixivDownload>,
}
struct Job {
    cancel: Arc<AtomicBool>,
    progress: PixivProgress,
}
#[derive(Default)]
struct JobRegistry {
    entries: Mutex<HashMap<String, Job>>,
}
impl JobRegistry {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Job>> {
        // The registry contains owned Strings, atomic flags and plain progress
        // values; no user callbacks or I/O execute while its lock is held. Rust
        // preserves HashMap memory invariants during unwinding, so recover the
        // actual entries instead of inventing an empty/busy sentinel. All read,
        // cancellation, progress, registration and RAII removal paths use this.
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
    fn count(&self) -> usize {
        self.lock().len()
    }
    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.lock().keys().cloned().collect();
        ids.sort();
        ids
    }
    fn snapshot(&self) -> Vec<PixivProgress> {
        let mut result: Vec<_> = self
            .lock()
            .values()
            .map(|job| {
                let mut value = job.progress.clone();
                value.cancelled = job.cancel.load(Ordering::Relaxed);
                value
            })
            .collect();
        result.sort_by(|left, right| left.id.cmp(&right.id));
        result
    }
    fn cancel_all(&self) {
        for job in self.lock().values() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
static JOBS: OnceLock<Arc<JobRegistry>> = OnceLock::new();
fn jobs() -> &'static Arc<JobRegistry> {
    JOBS.get_or_init(|| Arc::new(JobRegistry::default()))
}

/// The app exit/update guard should treat a nonzero count as unfinished work.
/// Cancellation drains current I/O (30s timeout) before deleting only its own
/// staging directory. Never terminate the process while this count is nonzero.
pub fn active_download_count() -> usize {
    jobs().count()
}
pub fn active_download_ids() -> Vec<String> {
    jobs().ids()
}
pub fn download_snapshot() -> Vec<PixivProgress> {
    jobs().snapshot()
}
pub fn cancel_all_downloads() {
    jobs().cancel_all();
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 12
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
}
fn artwork_id(input: &str) -> Result<String, String> {
    if input.len() > 2048 {
        return Err("작품 링크가 너무 깁니다.".into());
    }
    let value = input.trim();
    if valid_id(value) {
        return Ok(value.to_owned());
    }
    let url = Url::parse(value).map_err(|_| "Pixiv 작품 링크 또는 번호를 입력해 주세요.")?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("pixiv.net" | "www.pixiv.net"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err("올바른 Pixiv 작품 링크가 아닙니다.".into());
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let id = match parts.as_slice() {
        ["artworks", id] => *id,
        [lang, "artworks", id]
            if lang.len() == 2 && lang.bytes().all(|byte| byte.is_ascii_lowercase()) =>
        {
            *id
        }
        _ => return Err("작품의 artworks 링크를 입력해 주세요.".into()),
    };
    if !valid_id(id) {
        return Err("올바른 작품 번호가 아닙니다.".into());
    }
    Ok(id.to_owned())
}
fn allowed_url(raw: &str, image: bool) -> Result<Url, String> {
    if raw.len() > 2048 {
        return Err("Pixiv 주소가 너무 깁니다.".into());
    }
    let url = Url::parse(raw).map_err(|_| "잘못된 Pixiv 주소입니다.")?;
    let host = if image {
        "i.pximg.net"
    } else {
        "www.pixiv.net"
    };
    if url.scheme() != "https"
        || url.host_str() != Some(host)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || url.fragment().is_some()
    {
        return Err("허용되지 않은 Pixiv 주소입니다.".into());
    }
    if image
        && !(url.path().starts_with("/c/")
            || url.path().starts_with("/img-master/")
            || url.path().starts_with("/user-profile/")
            || url.path().starts_with("/img-original/"))
    {
        return Err("허용되지 않은 이미지 경로입니다.".into());
    }
    Ok(url)
}
fn client() -> Result<&'static Client, String> {
    #[cfg(test)]
    tests::check_network_client_entry()?;
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent("Mozilla/5.0 Atsumi/PixivPublicReader")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirect(Policy::custom(|attempt| {
                    if attempt.previous().len() >= 3
                        || attempt.url().scheme() != "https"
                        || !matches!(
                            attempt.url().host_str(),
                            Some("www.pixiv.net" | "i.pximg.net")
                        )
                        || attempt.url().port().is_some_and(|port| port != 443)
                        || !attempt.url().username().is_empty()
                        || attempt.url().password().is_some()
                    {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }))
                .build()
                .map_err(|_| "Pixiv 연결을 준비하지 못했습니다.".to_owned())
        })
        .as_ref()
        .map_err(Clone::clone)
}
struct MediaPermit;
static MEDIA_GATE: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
impl MediaPermit {
    fn acquire() -> Result<Self, String> {
        let mut count = MEDIA_GATE
            .0
            .lock()
            .map_err(|_| "이미지 작업을 준비하지 못했습니다.")?;
        while *count >= 4 {
            count = MEDIA_GATE
                .1
                .wait(count)
                .map_err(|_| "이미지 작업을 준비하지 못했습니다.")?;
        }
        *count += 1;
        Ok(Self)
    }
}
impl Drop for MediaPermit {
    fn drop(&mut self) {
        if let Ok(mut count) = MEDIA_GATE.0.lock() {
            *count = count.saturating_sub(1);
            MEDIA_GATE.1.notify_one();
        }
    }
}
fn response(url: &str, image: bool) -> Result<Response, String> {
    response_with_session(url, image, &session::Lease::default())
}
fn response_with_session(
    url: &str,
    image: bool,
    lease: &session::Lease,
) -> Result<Response, String> {
    lease.check()?;
    let url = allowed_url(url, image)?;
    if !image {
        static LAST: Mutex<Option<Instant>> = Mutex::new(None);
        let mut last = LAST
            .lock()
            .map_err(|_| "Pixiv 요청을 준비하지 못했습니다.")?;
        if let Some(previous) = *last {
            if previous.elapsed() < Duration::from_millis(180) {
                std::thread::sleep(Duration::from_millis(180).saturating_sub(previous.elapsed()));
            }
        }
        *last = Some(Instant::now());
    }
    let authenticated_client;
    let selected_client = if lease.authenticated() && !image {
        authenticated_client = Client::builder()
            .user_agent("Mozilla/5.0 Atsumi/PixivPublicReader")
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "Pixiv 연결을 준비하지 못했습니다.")?;
        &authenticated_client
    } else {
        client()?
    };
    let mut request = selected_client
        .get(url.clone())
        .header("Referer", "https://www.pixiv.net/");
    if !image {
        if let Some(cookie) = lease.cookie_for(&url)? {
            request = request.header("Cookie", cookie);
        }
    }
    let result = request.send().map_err(|error| {
        crate::diagnostics::http_error(&error);
        tracing::warn!(diag_stage = "pixiv_http_failed", status = 0u64);
        "Pixiv 연결에 실패했습니다. 네트워크를 확인하고 다시 시도해 주세요.".to_owned()
    })?;
    lease.check()?;
    allowed_url(result.url().as_str(), image)?;
    if result.status().as_u16() != 200 {
        tracing::warn!(
            diag_stage = "pixiv_http_failed",
            status = result.status().as_u16() as u64
        );
    }
    match result.status().as_u16() {
        200 => Ok(result),
        401 | 403 => Err(RESTRICTED.into()),
        404 => Err("삭제되었거나 공개되지 않은 작품입니다.".into()),
        429 => Err("Pixiv 요청이 잠시 제한되었습니다. 잠시 후 다시 시도해 주세요.".into()),
        _ => Err(format!(
            "Pixiv 응답 오류 (HTTP {}). 잠시 후 다시 시도해 주세요.",
            result.status().as_u16()
        )),
    }
}
fn limited_body(mut response: Response, limit: u64) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(BODY_TOO_LARGE.into());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Pixiv 응답을 읽지 못했습니다.")?;
    if bytes.len() as u64 > limit {
        return Err(BODY_TOO_LARGE.into());
    }
    Ok(bytes)
}
fn json(url: &str) -> Result<Value, String> {
    json_with_session(url, &session::Lease::default())
}
fn json_with_session(url: &str, lease: &session::Lease) -> Result<Value, String> {
    let _permit = MediaPermit::acquire()?;
    let body = limited_body(response_with_session(url, false, lease)?, JSON_LIMIT)?;
    lease.check()?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| "Pixiv 응답 형식이 변경되었거나 연결이 제한되었습니다.")?;
    if value.get("error").and_then(Value::as_bool) == Some(true) {
        return Err(
            "작품을 불러올 수 없습니다. 삭제·비공개 또는 로그인 필요 여부를 원문에서 확인해 주세요.".into(),
        );
    }
    Ok(value)
}
fn number(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or(0)
}
fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
        .unwrap_or_default()
}
fn take_text(value: &Value, limit: usize) -> String {
    text(value).chars().take(limit).collect()
}
fn is_restricted(value: &Value) -> bool {
    is_restricted_for_session(value, false)
}
fn is_restricted_for_session(value: &Value, authenticated: bool) -> bool {
    DisplayPolicy::new(authenticated, false).excludes(value)
}
fn work(value: &Value) -> Option<PixivWork> {
    work_with_session(value, false)
}
fn work_with_session(value: &Value, authenticated: bool) -> Option<PixivWork> {
    work_with_display(value, DisplayPolicy::new(authenticated, false))
}
fn work_with_display(value: &Value, display: DisplayPolicy) -> Option<PixivWork> {
    if display.excludes(value) {
        return None;
    }
    let rank = value.get("illust_id").is_some();
    let id = text(if rank {
        &value["illust_id"]
    } else {
        value.get("id").unwrap_or(&value["illustId"])
    });
    if !valid_id(&id) {
        return None;
    }
    let kind = number(if rank {
        &value["illust_type"]
    } else {
        &value["illustType"]
    });
    let image = if rank {
        &value["url"]
    } else {
        value
            .pointer("/urls/small")
            .filter(|value| value.is_string())
            .or_else(|| value.pointer("/urls/thumb"))
            .unwrap_or(&value["url"])
    };
    let thumbnail_url = image
        .as_str()
        .filter(|url| allowed_url(url, true).is_ok())
        .map(str::to_owned);
    let tags_value = value.pointer("/tags/tags").unwrap_or(&value["tags"]);
    let tags: Vec<String> = tags_value
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str().or_else(|| row["tag"].as_str()))
                .take(50)
                .map(|tag| tag.chars().take(100).collect())
                .collect()
        })
        .unwrap_or_default();
    Some(PixivWork {
        id,
        title: take_text(&value["title"], 300),
        artist_id: text(if rank {
            &value["user_id"]
        } else {
            &value["userId"]
        }),
        artist: take_text(
            if rank {
                &value["user_name"]
            } else {
                &value["userName"]
            },
            120,
        ),
        page_count: number(if rank {
            &value["illust_page_count"]
        } else {
            &value["pageCount"]
        })
        .clamp(1, 10000) as u32,
        width: number(&value["width"]).min(u32::MAX as u64) as u32,
        height: number(&value["height"]).min(u32::MAX as u64) as u32,
        thumbnail_url,
        bookmark_count: number(&value["bookmarkCount"]),
        tag_translations: localization::from_work(value, &tags),
        tags,
        created_at: take_text(value.get("createDate").unwrap_or(&value["date"]), 40),
        kind: match kind {
            1 => "manga",
            2 => "ugoira",
            _ => "illustration",
        }
        .into(),
    })
}
fn search_sync(request: PixivSearchRequest) -> Result<PixivSearchPage, String> {
    search_with_session(request, &session::Lease::default())
}
fn keyword_search_url(query: &str, page: u32, display: DisplayPolicy) -> Result<Url, String> {
    let mut url = Url::parse(&format!("{SITE}/ajax/search/artworks/")).unwrap();
    url.path_segments_mut()
        .map_err(|_| "검색 주소를 준비하지 못했습니다.")?
        .pop_if_empty()
        .push(query);
    url.query_pairs_mut()
        .append_pair("word", query)
        .append_pair("order", "date_d")
        .append_pair("mode", display.query_mode())
        .append_pair("p", &page.to_string())
        .append_pair("s_mode", "s_tag")
        .append_pair("type", "all")
        .append_pair("lang", "ko");
    Ok(url)
}
fn search_with_session(
    request: PixivSearchRequest,
    lease: &session::Lease,
) -> Result<PixivSearchPage, String> {
    lease.check()?;
    if request.page == 0 || request.page > 100 || request.query.chars().count() > 120 {
        return Err("검색어는 120자 이내, 페이지는 1~100 범위로 입력해 주세요.".into());
    }
    if request.mode == "home" {
        if !request.query.trim().is_empty() {
            return Err("검색어로 찾으려면 태그 검색을 선택해 주세요.".into());
        }
        return home::read(
            lease,
            request.page,
            request.home_cursor.as_deref(),
            request.safe_only,
        );
    }
    if request.home_cursor.is_some() {
        return Err("추천 작품의 추가 로딩 정보는 일반 검색에 사용할 수 없습니다.".into());
    }
    if request.mode == "artist" {
        return artist_with_display(&request.query, request.page, lease, request.safe_only);
    }
    let display = DisplayPolicy::new(lease.authenticated(), request.safe_only);
    let mut url;
    let ranking = request.mode != "latest";
    if ranking {
        if !matches!(request.mode.as_str(), "daily" | "weekly") {
            return Err("지원하지 않는 Pixiv 정렬입니다.".into());
        }
        if !request.query.trim().is_empty() {
            return Err(
                "랭킹은 전체 작품 기준입니다. 태그 검색은 최신 작품을 선택해 주세요.".into(),
            );
        }
        if request.page > 10 {
            return Ok(PixivSearchPage {
                items: vec![],
                page: request.page,
                has_more: false,
                home_suggestions: None,
                home_cursor: None,
            });
        }
        url = Url::parse(&format!("{SITE}/ranking.php")).unwrap();
        url.query_pairs_mut()
            .append_pair("mode", &request.mode)
            .append_pair("format", "json")
            .append_pair("p", &request.page.to_string());
    } else {
        let query = request.query.trim();
        if query.is_empty() {
            return Err("태그를 입력하거나 일간·주간 랭킹을 선택해 주세요.".into());
        }
        url = keyword_search_url(query, request.page, display)?;
    }
    let result = json_with_session(url.as_str(), lease)?;
    let rows = if ranking {
        result["contents"].as_array()
    } else {
        result
            .pointer("/body/illustManga/data")
            .and_then(Value::as_array)
    }
    .ok_or("Pixiv 검색 응답 형식이 변경되었습니다.")?;
    let items = rows
        .iter()
        .filter_map(|row| work_with_display(row, display))
        .map(|mut work| { localization::enrich(&mut work, &result["body"]["tagTranslation"]); work })
        .take(60)
        .collect();
    let has_more = if ranking {
        request.page < 10 && result["next"].as_bool() != Some(false) && rows.len() >= 50
    } else {
        (request.page as u64 * 60) < number(&result["body"]["illustManga"]["total"])
            && request.page < 100
    };
    Ok(PixivSearchPage {
        items,
        page: request.page,
        has_more,
        home_suggestions: None,
        home_cursor: None,
    })
}
fn artist_sync(id: &str, page: u32) -> Result<PixivSearchPage, String> {
    artist_with_session(id, page, &session::Lease::default())
}
fn artist_with_session(
    id: &str,
    page: u32,
    lease: &session::Lease,
) -> Result<PixivSearchPage, String> {
    artist_with_display(id, page, lease, false)
}
fn artist_with_display(
    id: &str,
    page: u32,
    lease: &session::Lease,
    safe_only: bool,
) -> Result<PixivSearchPage, String> {
    if !valid_id(id) || !(1..=100).contains(&page) {
        return Err("올바른 작가 번호 또는 페이지가 아닙니다.".into());
    }
    let profile = json_with_session(&format!("{SITE}/ajax/user/{id}/profile/all?lang=ko"), lease)?;
    let mut ids: Vec<_> = ["illusts", "manga"]
        .iter()
        .flat_map(|kind| {
            profile["body"][kind]
                .as_object()
                .into_iter()
                .flat_map(|works| works.keys())
        })
        .filter(|id| valid_id(id))
        .cloned()
        .collect();
    ids.sort_unstable_by_key(|value| std::cmp::Reverse(value.parse::<u64>().unwrap_or(0)));
    ids.dedup();
    let offset = (page as usize - 1) * 60;
    let mut url = Url::parse(&format!("{SITE}/ajax/user/{id}/profile/illusts")).unwrap();
    let selected: Vec<_> = ids.iter().skip(offset).take(60).collect();
    if selected.is_empty() {
        return Ok(PixivSearchPage {
            items: vec![],
            page,
            has_more: false,
            home_suggestions: None,
            home_cursor: None,
        });
    }
    for id in &selected {
        url.query_pairs_mut().append_pair("ids[]", id);
    }
    url.query_pairs_mut()
        .append_pair("work_category", "illustManga")
        .append_pair("is_first_page", if page == 1 { "1" } else { "0" })
        .append_pair("lang", "ko");
    let result = json_with_session(url.as_str(), lease)?;
    let works = result
        .pointer("/body/works")
        .and_then(Value::as_object)
        .ok_or("작가 작품 응답 형식이 변경되었습니다.")?;
    let items = selected
        .iter()
        .filter_map(|id| works.get(id.as_str()))
        .filter_map(|row| {
            work_with_display(row, DisplayPolicy::new(lease.authenticated(), safe_only))
        })
        .map(|mut work| { localization::enrich(&mut work, &result["body"]["tagTranslation"]); work })
        .collect();
    Ok(PixivSearchPage {
        items,
        page,
        has_more: offset + 60 < ids.len() && page < 100,
        home_suggestions: None,
        home_cursor: None,
    })
}
fn detail_sync(input: &str) -> Result<PixivDetail, String> {
    detail_with_session(input, &session::Lease::default())
}
fn detail_with_session(input: &str, lease: &session::Lease) -> Result<PixivDetail, String> {
    lease.check()?;
    let id = artwork_id(input)?;
    static DETAILS: OnceLock<Mutex<VecDeque<(String, Instant, PixivDetail)>>> = OnceLock::new();
    let cache = DETAILS.get_or_init(|| Mutex::new(VecDeque::new()));
    // Account-bound metadata never enters the anonymous process cache.
    if !lease.authenticated() {
        if let Ok(mut rows) = cache.lock() {
            rows.retain(|(_, created, _)| created.elapsed() < Duration::from_secs(300));
            if let Some(index) = rows.iter().position(|(key, _, _)| key == &id) {
                if let Some(row) = rows.remove(index) {
                    let result = row.2.clone();
                    rows.push_back(row);
                    return Ok(result);
                }
            }
        }
    }
    let response = json_with_session(&format!("{SITE}/ajax/illust/{id}?lang=ko"), lease)?;
    let value = &response["body"];
    let mut work = work_with_session(value, lease.authenticated()).ok_or(RESTRICTED)?;
    localization::enrich(&mut work, &response["body"]["tagTranslation"]);
    if work.id != id {
        return Err("Pixiv 작품 번호가 일치하지 않습니다.".into());
    }
    if work.kind == "ugoira" {
        return Ok(PixivDetail {
            work,
            pages: vec![],
        });
    }
    if work.page_count > 100 {
        return Err("100쪽을 넘는 작품은 Pixiv 원문에서 확인해 주세요.".into());
    }
    let response = json_with_session(&format!("{SITE}/ajax/illust/{id}/pages"), lease)?;
    let rows = response["body"]
        .as_array()
        .ok_or("작품 페이지를 불러오지 못했습니다.")?;
    if rows.is_empty() || rows.len() != work.page_count as usize {
        return Err("작품의 전체 페이지를 확인하지 못했습니다. 다시 시도해 주세요.".into());
    }
    let pages = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let original_url = text(&row["urls"]["original"]);
            let preview_url = text(&row["urls"]["regular"]);
            allowed_url(&original_url, true)?;
            allowed_url(&preview_url, true)?;
            Ok(PixivPage {
                index: index as u32,
                width: number(&row["width"]).min(u32::MAX as u64) as u32,
                height: number(&row["height"]).min(u32::MAX as u64) as u32,
                preview_url,
                original_url,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let result = PixivDetail { work, pages };
    lease.check()?;
    if !lease.authenticated() {
        if let Ok(mut rows) = cache.lock() {
            rows.retain(|(key, _, _)| key != &id);
            rows.push_back((id, Instant::now(), result.clone()));
            while rows.len() > 32 {
                rows.pop_front();
            }
        }
    }
    Ok(result)
}
fn original_page_sync(id: &str, index: u32) -> Result<Vec<u8>, String> {
    original_page_with_session(id, index, &session::Lease::default())
}
fn original_page_with_session(
    id: &str,
    index: u32,
    lease: &session::Lease,
) -> Result<Vec<u8>, String> {
    if !valid_id(id) {
        return Err("올바른 작품 번호가 아닙니다.".into());
    }
    // Resolve authorized safe metadata; renderer-supplied original URLs remain forbidden.
    let detail = detail_with_session(id, lease)?;
    let page = detail
        .pages
        .get(index as usize)
        .ok_or("작품 페이지가 없습니다.")?;
    if u64::from(page.width) * u64::from(page.height) > 24_000_000 {
        return Err(DISPLAY_TOO_LARGE.into());
    }
    let _permit = MediaPermit::acquire()?;
    let bytes = limited_body(
        response_with_session(&page.original_url, true, lease)?,
        DISPLAY_ORIGINAL_LIMIT,
    ).map_err(|error| if error == BODY_TOO_LARGE { DISPLAY_TOO_LARGE.into() } else { error })?;
    lease.check()?;
    validate_display_original(&bytes, page.width, page.height)?;
    Ok(bytes)
}
fn validate_display_original(bytes: &[u8], width: u32, height: u32) -> Result<(), String> {
    let dimensions = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "이미지 형식이 올바르지 않습니다.")?
        .into_dimensions()
        .map_err(|_| "이미지가 손상되었습니다.")?;
    if dimensions != (width, height) {
        return Err("작품 정보와 원본 이미지 크기가 일치하지 않습니다. 다시 열어 주세요.".into());
    }
    if u64::from(width) * u64::from(height) > 24_000_000 {
        return Err(DISPLAY_TOO_LARGE.into());
    }
    Ok(())
}
fn media_sync(url: &str, detail: bool) -> Result<String, String> {
    let _permit = MediaPermit::acquire()?;
    // Original downloads have a separate streaming path. Display requests may
    // not request originals and accidentally inflate decoded-memory usage.
    let parsed = allowed_url(url, true)?;
    if parsed.path().starts_with("/img-original/") {
        return Err("원본은 저장 기능을 이용해 주세요.".into());
    }
    let bytes = limited_body(response(url, true)?, IMAGE_LIMIT)?;
    let encoded = display_jpeg(bytes, detail)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(encoded)
    ))
}
fn display_jpeg(bytes: Vec<u8>, detail: bool) -> Result<Vec<u8>, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "지원하지 않는 이미지입니다.")?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(10000);
    limits.max_image_height = Some(10000);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "이미지를 안전하게 표시할 수 없습니다.")?;
    let side = if detail { 1600 } else { 480 };
    let resized = decoded.thumbnail(side, side).to_rgb8();
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, if detail { 84 } else { 76 })
        .encode_image(&resized)
        .map_err(|_| "미리보기를 만들지 못했습니다.")?;
    let output_limit = if detail { 2 * 1024 * 1024 } else { 400 * 1024 };
    if encoded.len() > output_limit {
        return Err("미리보기 크기가 안전한 처리 범위를 초과합니다.".into());
    }
    Ok(encoded)
}

fn is_redirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}
fn root_path(download_root: &str, create: bool) -> Result<PathBuf, String> {
    let root = Path::new(download_root.trim());
    if download_root.trim().is_empty() || !root.is_absolute() {
        return Err("설정에서 다운로드 폴더를 먼저 지정해 주세요.".into());
    }
    if create {
        fs::create_dir_all(root).map_err(|_| "다운로드 폴더를 만들지 못했습니다.")?;
    }
    let root = root
        .canonicalize()
        .map_err(|_| "다운로드 폴더를 찾을 수 없습니다.")?;
    let pixiv = root.join("Pixiv");
    if create {
        fs::create_dir_all(&pixiv).map_err(|_| "Pixiv 저장 폴더를 만들지 못했습니다.")?;
    }
    if pixiv.exists()
        && (is_redirect(
            &fs::symlink_metadata(&pixiv).map_err(|_| "저장 폴더를 확인하지 못했습니다.")?,
        ) || !pixiv
            .canonicalize()
            .map_err(|_| "저장 폴더를 확인하지 못했습니다.")?
            .starts_with(&root))
    {
        return Err("Pixiv 저장 폴더가 지정한 경로 밖을 가리킵니다.".into());
    }
    Ok(pixiv)
}
fn original_extension(url: &str) -> Result<&str, String> {
    let path = url.split('?').next().unwrap_or(url);
    match path.rsplit('.').next() {
        Some(ext @ ("jpg" | "jpeg" | "png" | "webp")) => Ok(ext),
        _ => Err("지원하지 않는 원본 이미지 형식입니다.".into()),
    }
}
struct JobGuard {
    id: String,
    registry: Arc<JobRegistry>,
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        self.registry.lock().remove(&self.id);
    }
}
struct OriginalResponse {
    body: Box<dyn Read>,
    content_length: Option<u64>,
    mime: String,
}
struct DownloadPermit;
static DOWNLOAD_GATE: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
impl DownloadPermit {
    fn acquire(cancel: &AtomicBool) -> Result<Self, String> {
        let mut count = DOWNLOAD_GATE
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while *count >= 2 {
            if cancel.load(Ordering::Relaxed) {
                return Err("다운로드를 취소했습니다.".into());
            }
            count = DOWNLOAD_GATE
                .1
                .wait_timeout(count, Duration::from_millis(100))
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("다운로드를 취소했습니다.".into());
        }
        *count += 1;
        Ok(Self)
    }
}
impl Drop for DownloadPermit {
    fn drop(&mut self) {
        let mut count = DOWNLOAD_GATE
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *count = count.saturating_sub(1);
        DOWNLOAD_GATE.1.notify_one();
    }
}
fn original_response(url: &str) -> Result<OriginalResponse, String> {
    let response = response(url, true)?;
    let mime = response
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_owned();
    let content_length = response.content_length();
    Ok(OriginalResponse {
        body: Box::new(response),
        content_length,
        mime,
    })
}
fn download_sync(
    id: &str,
    download_root: &str,
    cancel: Arc<AtomicBool>,
) -> Result<PixivDownload, String> {
    download_with_session(id, download_root, cancel, &session::Lease::default())
}
fn download_with_session(
    id: &str,
    download_root: &str,
    cancel: Arc<AtomicBool>,
    lease: &session::Lease,
) -> Result<PixivDownload, String> {
    lease.check()?;
    let _download_permit = DownloadPermit::acquire(&cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("다운로드를 취소했습니다.".into());
    }
    if let Some(job) = jobs().lock().get_mut(id) {
        job.progress.queued = false;
    }
    let detail = detail_with_session(id, lease)?;
    save_detail_with_session(id, detail, download_root, cancel, original_response, lease)
}
fn save_detail_sync(
    id: &str,
    detail: PixivDetail,
    download_root: &str,
    cancel: Arc<AtomicBool>,
    fetch: impl FnMut(&str) -> Result<OriginalResponse, String>,
) -> Result<PixivDownload, String> {
    save_detail_with_session(
        id,
        detail,
        download_root,
        cancel,
        fetch,
        &session::Lease::default(),
    )
}
fn save_detail_with_session(
    id: &str,
    detail: PixivDetail,
    download_root: &str,
    cancel: Arc<AtomicBool>,
    mut fetch: impl FnMut(&str) -> Result<OriginalResponse, String>,
    lease: &session::Lease,
) -> Result<PixivDownload, String> {
    if detail.work.kind == "ugoira" {
        return Err(
            "움직이는 작품의 저장은 아직 지원하지 않습니다. 원문에서 확인해 주세요.".into(),
        );
    }
    if !valid_id(id)
        || detail.work.id != id
        || detail.pages.is_empty()
        || detail.pages.len() > 100
        || detail.pages.len() != detail.work.page_count as usize
    {
        return Err("저장할 작품 정보가 올바르지 않습니다.".into());
    }
    let check_cancel = || {
        lease.check()?;
        if cancel.load(Ordering::Relaxed) {
            Err("다운로드를 취소했습니다.".to_owned())
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let root = root_path(download_root, true)?;
    let destination = root.join(id);
    if destination.exists() {
        return Err("이미 저장된 작품 폴더가 있습니다. 기존 파일은 덮어쓰지 않습니다.".into());
    }
    let staging = root.join(format!(".{}-{}.partial", id, uuid::Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|_| "임시 저장 폴더를 만들지 못했습니다.")?;
    {
        let mut rows = jobs().lock();
        if let Some(job) = rows.get_mut(id) {
            job.progress.total_pages = detail.pages.len() as u32;
        }
    }
    let result = (|| {
        let mut total = 0u64;
        let mut saved_pages = Vec::new();
        for (page_index, page) in detail.pages.iter().enumerate() {
            check_cancel()?;
            if page.index as usize != page_index || page.width == 0 || page.height == 0 {
                return Err("작품 페이지 정보가 올바르지 않습니다.".into());
            }
            let parsed = allowed_url(&page.original_url, true)?;
            if !parsed.path().starts_with("/img-original/")
                || !parsed
                    .path()
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.starts_with(&format!("{id}_p{}.", page.index)))
            {
                return Err("작품과 원본 이미지 주소가 일치하지 않습니다.".into());
            }
            let extension = original_extension(&page.original_url)?;
            let mut response = fetch(&page.original_url)?;
            if !matches!(
                response.mime.as_str(),
                "image/jpeg" | "image/png" | "image/webp"
            ) {
                return Err("원본 응답이 이미지가 아닙니다.".into());
            }
            let expected = response.content_length;
            if expected.is_some_and(|length| length > PAGE_LIMIT || total + length > ALBUM_LIMIT) {
                return Err(
                    "작품이 안전한 저장 범위(쪽당 256 MB, 작품당 1 GB)를 초과합니다.".into(),
                );
            }
            let path = staging.join(format!("{id}_p{}.{}", page.index, extension));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .map_err(|_| "원본 파일을 만들지 못했습니다.")?;
            let mut buffer = [0u8; 64 * 1024];
            let mut page_bytes = 0;
            let mut digest = Sha256::new();
            loop {
                check_cancel()?;
                let count = response
                    .body
                    .read(&mut buffer)
                    .map_err(|_| "원본 다운로드 연결이 끊어졌습니다.")?;
                if count == 0 {
                    break;
                }
                page_bytes += count as u64;
                total += count as u64;
                if page_bytes > PAGE_LIMIT || total > ALBUM_LIMIT {
                    return Err("작품의 저장 크기 제한을 초과했습니다.".into());
                }
                file.write_all(&buffer[..count])
                    .map_err(|_| "원본 파일을 저장하지 못했습니다. 디스크 공간을 확인해 주세요.")?;
                digest.update(&buffer[..count]);
                {
                    let mut rows = jobs().lock();
                    if let Some(job) = rows.get_mut(id) {
                        job.progress.bytes = total;
                    }
                }
            }
            if page_bytes == 0 || expected.is_some_and(|length| length != page_bytes) {
                return Err("원본 파일이 완전히 다운로드되지 않았습니다.".into());
            }
            file.sync_all()
                .map_err(|_| "원본 파일 저장을 마무리하지 못했습니다.")?;
            let mut reader = ImageReader::open(&path)
                .map_err(|_| "저장한 파일을 확인하지 못했습니다.")?
                .with_guessed_format()
                .map_err(|_| "원본 이미지 형식을 확인하지 못했습니다.")?;
            // A valid JPEG/PNG header is not proof that the body is complete.
            // Decode before publishing; memory stays bounded even for hostile
            // dimensions. Originals beyond this bound fail explicitly.
            let mut limits = Limits::default();
            limits.max_image_width = Some(20000);
            limits.max_image_height = Some(20000);
            limits.max_alloc = Some(256 * 1024 * 1024);
            reader.limits(limits);
            let decoded = reader
                .decode()
                .map_err(|_| "저장한 원본이 올바른 이미지가 아닙니다.")?;
            let dimensions = (decoded.width(), decoded.height());
            drop(decoded);
            if dimensions.0 != page.width || dimensions.1 != page.height {
                return Err("저장한 이미지 크기가 작품 정보와 일치하지 않습니다.".into());
            }
            saved_pages.push(PixivLocalPage {
                index: page.index,
                file: format!("{id}_p{}.{}", page.index, extension),
                width: dimensions.0,
                height: dimensions.1,
                bytes: page_bytes,
                sha256: format!("{:x}", digest.finalize()),
            });
            {
                let mut rows = jobs().lock();
                if let Some(job) = rows.get_mut(id) {
                    job.progress.completed_pages += 1;
                }
            }
        }
        check_cancel()?;
        let record = PixivDownload {
            work: detail.work,
            saved_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string(),
            page_count: detail.pages.len() as u32,
            bytes: total,
            folder: destination.to_string_lossy().into_owned(),
            pages: saved_pages,
            issue: None,
            removed_token: None,
        };
        let bytes =
            serde_json::to_vec_pretty(&record).map_err(|_| "저장 기록을 만들지 못했습니다.")?;
        let mut metadata = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.join(RECORD_FILE))
            .map_err(|_| "저장 기록을 만들지 못했습니다.")?;
        metadata
            .write_all(&bytes)
            .and_then(|_| metadata.sync_all())
            .map_err(|_| "저장 기록을 기록하지 못했습니다.")?;
        drop(metadata);
        check_cancel()?;
        if destination.exists() {
            return Err("이미 저장된 작품 폴더가 있습니다. 기존 파일은 덮어쓰지 않습니다.".into());
        }
        lease.with_current(|| {
            fs::rename(&staging, &destination)
                .map_err(|_| "완료된 작품을 저장 폴더로 옮기지 못했습니다.".to_owned())
        })?;
        Ok(record)
    })();
    // A failed path may have been replaced since create_dir. Never follow it
    // with recursive cleanup: preserve unverified partials for safe recovery.
    // Successful publication renames the staging directory, leaving no copy.
    result
}
fn downloads_sync(download_root: &str) -> Result<Vec<PixivDownload>, String> {
    if download_root.trim().is_empty() || !Path::new(download_root).exists() {
        return Ok(vec![]);
    }
    let root = root_path(download_root, false)?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|_| "Pixiv 저장 목록을 읽지 못했습니다.")?
        .take(2000)
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_id(&name)
            || !fs::symlink_metadata(entry.path())
                .is_ok_and(|metadata| metadata.is_dir() && !is_redirect(&metadata))
        {
            continue;
        }
        // Cheap manifest/file-size integrity check only. Full decoding happened
        // at publication and per-page hashes are checked when a page is opened.
        if let Ok((_, record)) = management_record(&root, &name, &name) {
            records.push(record);
        }
    }
    let removed = root.join(".removed");
    if safe_child_directory(&root, ".removed").is_ok() {
        for entry in fs::read_dir(&removed)
            .map_err(|_| "복원 목록을 읽지 못했습니다.")?
            .take(2000usize.saturating_sub(records.len()))
            .flatten()
        {
            let token = entry.file_name().to_string_lossy().into_owned();
            let Some((id, nonce)) = token.split_once('-') else {
                continue;
            };
            if !valid_id(id) || uuid::Uuid::parse_str(nonce).is_err() {
                continue;
            }
            if let Ok((_, mut record)) = management_record(&removed, &token, id) {
                record.removed_token = Some(token);
                records.push(record);
            }
        }
    }
    records.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    Ok(records)
}

// Read-only, opaque-ID local access. Never trust a saved absolute folder or a
// frontend path. Deny reparse/symlink/hard-link files, and hold the read handle
// without Windows write/delete sharing while bytes and their digest are read.
fn local_file(path: &Path, limit: u64) -> Result<File, String> {
    local_file_checked(path, limit, false)
}
fn local_file_checked(path: &Path, limit: u64, allow_empty: bool) -> Result<File, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "저장된 파일을 찾을 수 없습니다.")?;
    if !metadata.is_file()
        || is_redirect(&metadata)
        || (!allow_empty && metadata.len() == 0)
        || metadata.len() > limit
    {
        return Err("저장된 파일을 안전하게 읽을 수 없습니다.".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|_| "저장된 파일을 열지 못했습니다.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "저장된 파일을 확인하지 못했습니다.")?;
    if is_redirect(&metadata)
        || !metadata.is_file()
        || (!allow_empty && metadata.len() == 0)
        || metadata.len() > limit
    {
        return Err("저장된 파일을 안전하게 읽을 수 없습니다.".into());
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
            .map_err(|_| "저장된 파일을 확인하지 못했습니다.")?;
        if info.nNumberOfLinks != 1 {
            return Err("연결된 파일은 열 수 없습니다.".into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("연결된 파일은 열 수 없습니다.".into());
        }
    }
    Ok(file)
}
fn local_record(download_root: &str, id: &str) -> Result<(PathBuf, PixivDownload), String> {
    if !valid_id(id) {
        return Err("올바른 작품 번호가 아닙니다.".into());
    }
    let root = root_path(download_root, false)?;
    local_record_under(&root, id, id)
}
fn local_record_under(
    root: &Path,
    name: &str,
    id: &str,
) -> Result<(PathBuf, PixivDownload), String> {
    let (resolved, mut record) = manifest_record(root, name, id)?;
    validate_local_pages(&resolved, id, &mut record)?;
    Ok((resolved, record))
}
fn safe_child_directory(root: &Path, name: &str) -> Result<PathBuf, String> {
    let album = root.join(name);
    let metadata = fs::symlink_metadata(&album).map_err(|_| "저장된 작품을 찾을 수 없습니다.")?;
    if !metadata.is_dir() || is_redirect(&metadata) {
        return Err("저장된 작품 경로가 올바르지 않습니다.".into());
    }
    let resolved = album
        .canonicalize()
        .map_err(|_| "저장된 작품을 찾을 수 없습니다.")?;
    if resolved.parent() != root.canonicalize().ok().as_deref() {
        return Err("저장된 작품 경로가 올바르지 않습니다.".into());
    }
    Ok(resolved)
}
fn manifest_record(root: &Path, name: &str, id: &str) -> Result<(PathBuf, PixivDownload), String> {
    let resolved = safe_child_directory(root, name)?;
    let file = local_file(&resolved.join(RECORD_FILE), RECORD_LIMIT)?;
    let mut record: PixivDownload = serde_json::from_reader(file.take(RECORD_LIMIT))
        .map_err(|_| "저장된 작품 정보가 손상되었습니다.")?;
    if record.work.id != id
        || !(1..=100).contains(&record.page_count)
        || record.work.page_count != record.page_count
        || record.work.kind == "ugoira"
        || (!record.pages.is_empty() && record.pages.len() != record.page_count as usize)
    {
        return Err("저장된 작품 정보가 올바르지 않습니다.".into());
    }
    for (index, page) in record.pages.iter().enumerate() {
        if page.index as usize != index
            || page.width == 0
            || page.height == 0
            || !["jpg", "jpeg", "png", "webp"]
                .iter()
                .any(|ext| page.file == format!("{id}_p{index}.{ext}"))
            || (!page.sha256.is_empty()
                && (page.sha256.len() != 64
                    || !page.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())))
        {
            return Err("저장된 페이지 정보가 올바르지 않습니다.".into());
        }
    }
    record.folder = resolved.to_string_lossy().into_owned();
    Ok((resolved, record))
}
fn management_record(
    root: &Path,
    name: &str,
    id: &str,
) -> Result<(PathBuf, PixivDownload), String> {
    let (resolved, mut record) = manifest_record(root, name, id)?;
    let mut verified = record.clone();
    match validate_local_pages(&resolved, id, &mut verified) {
        Ok(()) => record = verified,
        Err(_) => record.issue = Some("파일 확인 필요".into()),
    }
    Ok((resolved, record))
}
fn validate_local_pages(
    resolved: &Path,
    id: &str,
    record: &mut PixivDownload,
) -> Result<(), String> {
    // Older downloaded albums have no per-page digest. Discover only their
    // deterministic filenames; never write/migrate user files while opening.
    if record.pages.is_empty() {
        for index in 0..record.page_count {
            let names: Vec<_> = ["jpg", "jpeg", "png", "webp"]
                .iter()
                .map(|ext| format!("{id}_p{index}.{ext}"))
                .filter(|name| resolved.join(name).exists())
                .collect();
            if names.len() != 1 {
                return Err("저장된 페이지가 없거나 중복되어 있습니다.".into());
            }
            let file = local_file(&resolved.join(&names[0]), PAGE_LIMIT)?;
            let bytes = file
                .metadata()
                .map_err(|_| "파일 정보를 확인하지 못했습니다.")?
                .len();
            let (width, height) = ImageReader::new(std::io::BufReader::new(file))
                .with_guessed_format()
                .map_err(|_| "저장된 이미지 형식이 올바르지 않습니다.")?
                .into_dimensions()
                .map_err(|_| "저장된 이미지가 손상되었습니다.")?;
            record.pages.push(PixivLocalPage {
                index,
                file: names[0].clone(),
                width,
                height,
                bytes,
                sha256: String::new(),
            });
        }
    }
    let mut total = 0u64;
    for (index, page) in record.pages.iter().enumerate() {
        if page.index as usize != index
            || page.width == 0
            || page.height == 0
            || !["jpg", "jpeg", "png", "webp"]
                .iter()
                .any(|ext| page.file == format!("{id}_p{index}.{ext}"))
            || (!page.sha256.is_empty()
                && (page.sha256.len() != 64
                    || !page.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())))
        {
            return Err("저장된 페이지 정보가 올바르지 않습니다.".into());
        }
        let file = local_file(&resolved.join(&page.file), PAGE_LIMIT)?;
        if file
            .metadata()
            .map_err(|_| "파일 정보를 확인하지 못했습니다.")?
            .len()
            != page.bytes
        {
            return Err("저장된 페이지의 크기가 변경되었습니다.".into());
        }
        total = total
            .checked_add(page.bytes)
            .ok_or("저장 크기가 올바르지 않습니다.")?;
    }
    if total != record.bytes || total > ALBUM_LIMIT {
        return Err("저장된 작품의 크기가 일치하지 않습니다.".into());
    }
    Ok(())
}
fn library_move_sync(
    root: &str,
    id: &str,
    restore: Option<&str>,
) -> Result<PixivLibraryResult, String> {
    if !valid_id(id) {
        return Err("올바른 작품 번호가 아닙니다.".into());
    }
    let root = root_path(root, false)?;
    let removed = root.join(".removed");
    if removed.exists() {
        let metadata =
            fs::symlink_metadata(&removed).map_err(|_| "보관 경로를 확인하지 못했습니다.")?;
        if is_redirect(&metadata)
            || !metadata.is_dir()
            || removed
                .canonicalize()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                != root.canonicalize().ok()
        {
            return Err("보관 경로가 올바르지 않습니다.".into());
        }
    } else if restore.is_some() {
        return Err("복원할 작품을 찾을 수 없습니다.".into());
    }
    let (source, mut record, destination, token) = if let Some(token) = restore {
        let (token_id, nonce) = token
            .split_once('-')
            .ok_or("복원 정보가 올바르지 않습니다.")?;
        if token_id != id || uuid::Uuid::parse_str(nonce).is_err() {
            return Err("복원 정보가 올바르지 않습니다.".into());
        }
        let (source, record) = management_record(&removed, token, id)?;
        (source, record, root.join(id), None)
    } else {
        let (source, record) = management_record(&root, id, id)?;
        let token = format!("{id}-{}", uuid::Uuid::new_v4());
        if !removed.exists() {
            fs::create_dir(&removed).map_err(|_| "복원용 폴더를 만들지 못했습니다.")?;
        }
        (source, record, removed.join(&token), Some(token))
    };
    if destination.exists() {
        return Err("같은 작품이 이미 있어 덮어쓰지 않았습니다.".into());
    }
    // Never move user-added files with an app-owned album.
    for entry in fs::read_dir(&source).map_err(|_| "작품 폴더를 확인하지 못했습니다.")?
    {
        let entry = entry.map_err(|_| "작품 폴더를 확인하지 못했습니다.")?;
        let name = entry.file_name();
        let is_page = if record.pages.is_empty() {
            (0..record.page_count).any(|index| {
                ["jpg", "jpeg", "png", "webp"]
                    .iter()
                    .any(|ext| name.to_str() == Some(format!("{id}_p{index}.{ext}").as_str()))
            })
        } else {
            record
                .pages
                .iter()
                .any(|page| name.to_str() == Some(page.file.as_str()))
        };
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| "작품 폴더를 확인하지 못했습니다.")?;
        if !metadata.is_file()
            || is_redirect(&metadata)
            || (name.to_str() != Some(RECORD_FILE) && !is_page)
        {
            return Err("작품 폴더에 별도 파일이 있어 이동하지 않았습니다.".into());
        }
        // Missing/truncated originals are manageable, but that must not turn
        // the strict reader's hard-link/reparse rejection into a bypass.
        let _file = local_file_checked(&entry.path(), PAGE_LIMIT, true)?;
    }
    fs::rename(&source, &destination)
        .map_err(|_| "작품을 이동하지 못했습니다. 기존 파일은 유지했습니다.")?;
    record.folder = destination.to_string_lossy().into_owned();
    Ok(PixivLibraryResult {
        token,
        record: restore.map(|_| record),
    })
}
fn local_detail_sync(download_root: &str, id: &str) -> Result<PixivDetail, String> {
    let (_, record) = local_record(download_root, id)?;
    Ok(PixivDetail {
        work: record.work,
        pages: record
            .pages
            .iter()
            .map(|page| PixivPage {
                index: page.index,
                width: page.width,
                height: page.height,
                preview_url: format!("atsumi-pixiv:{id}:{}", page.index),
                original_url: format!("atsumi-pixiv:{id}:{}", page.index),
            })
            .collect(),
    })
}
fn local_media_sync(
    download_root: &str,
    id: &str,
    index: u32,
    detail: bool,
) -> Result<Vec<u8>, String> {
    let _permit = MediaPermit::acquire()?;
    let (folder, record) = local_record(download_root, id)?;
    let page = record
        .pages
        .get(index as usize)
        .ok_or("저장된 페이지가 없습니다.")?;
    let mut bytes = Vec::new();
    local_file(&folder.join(&page.file), DISPLAY_ORIGINAL_LIMIT)?
        .take(DISPLAY_ORIGINAL_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "저장된 이미지를 읽지 못했습니다.")?;
    if bytes.len() as u64 != page.bytes
        || bytes.len() as u64 > DISPLAY_ORIGINAL_LIMIT
        || (!page.sha256.is_empty() && format!("{:x}", Sha256::digest(&bytes)) != page.sha256)
    {
        return Err("저장된 이미지가 변경되었거나 손상되었습니다.".into());
    }
    validate_display_original(&bytes, page.width, page.height)?;
    if detail {
        Ok(bytes)
    } else {
        display_jpeg(bytes, false)
    }
}
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    // Reject excess work before spawning a blocking thread. The four-way media
    // gate below limits actual decoding/network activity; this also bounds its
    // waiting queue against bursts of IPC requests.
    static COMMANDS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let permit = COMMANDS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(12)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "Pixiv 작업이 많습니다. 잠시 후 다시 시도해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        operation()
    })
    .await
    .map_err(|_| "Pixiv 작업이 중단되었습니다.".to_owned())?
}
fn require_main(window: &tauri::Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| "앱 창을 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("Pixiv 기능은 Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn pixiv_discovery(
    window: tauri::Webview,
    account: State<'_, account::PixivAccount>,
    request: discovery::Request,
) -> Result<discovery::Response, String> {
    require_main(&window)?;
    let lease = account.lease(window.app_handle()).await?;
    blocking(move || observed("pixiv_discovery", || {
        let result = discovery::read(request, &lease)?;
        lease.check()?;
        Ok(result)
    })).await
}
#[tauri::command]
pub async fn pixiv_search(
    window: tauri::Webview,
    account: State<'_, account::PixivAccount>,
    request: PixivSearchRequest,
) -> Result<PixivSearchPage, String> {
    require_main(&window)?;
    let lease = account.lease(window.app_handle()).await?;
    blocking(move || {
        observed("pixiv_search", || {
            let result = search_with_session(request, &lease)?;
            lease.check()?;
            Ok(result)
        })
    })
    .await
}
#[tauri::command]
pub async fn pixiv_work(
    window: tauri::Webview,
    account: State<'_, account::PixivAccount>,
    input: String,
) -> Result<PixivDetail, String> {
    require_main(&window)?;
    let lease = account.lease(window.app_handle()).await?;
    blocking(move || {
        observed("pixiv_detail", || {
            let result = detail_with_session(&input, &lease)?;
            lease.check()?;
            Ok(result)
        })
    })
    .await
}
#[tauri::command]
pub async fn pixiv_media(
    window: tauri::Webview,
    url: String,
    detail: bool,
) -> Result<String, String> {
    require_main(&window)?;
    blocking(move || media_sync(&url, detail)).await
}
#[tauri::command]
pub async fn pixiv_download(
    window: tauri::Webview,
    state: State<'_, AppState>,
    account: State<'_, account::PixivAccount>,
    id: String,
) -> Result<PixivDownload, String> {
    require_main(&window)?;
    let id = artwork_id(&id)?;
    let lease = account.lease(window.app_handle()).await?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    let (cancel, guard) = state.begin_platform_work(|| {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut rows = jobs().lock();
        if rows.contains_key(&id) {
            return Err("이미 다운로드 중인 작품입니다.".into());
        }
        if rows.len() >= 2 {
            return Err("다른 작품의 다운로드가 끝난 뒤 다시 시도해 주세요.".into());
        }
        rows.insert(
            id.clone(),
            Job {
                cancel: cancel.clone(),
                progress: PixivProgress {
                    id: id.clone(),
                    completed_pages: 0,
                    total_pages: 1,
                    bytes: 0,
                    cancelled: false,
                    queued: false,
                },
            },
        );
        Ok((
            cancel,
            JobGuard {
                id: id.clone(),
                registry: jobs().clone(),
            },
        ))
    })?;
    blocking(move || {
        let _guard = guard;
        observed("pixiv_download", || {
            download_with_session(&id, &root, cancel, &lease)
        })
    })
    .await
}
#[tauri::command]
pub async fn pixiv_downloads_list(
    window: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Vec<PixivDownload>, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    blocking(move || downloads_sync(&root)).await
}
#[tauri::command]
pub async fn pixiv_download_batch(
    window: tauri::Webview,
    state: State<'_, AppState>,
    account: State<'_, account::PixivAccount>,
    ids: Vec<String>,
) -> Result<Vec<PixivDownloadOutcome>, String> {
    require_main(&window)?;
    if ids.is_empty() || ids.len() > 100 || ids.iter().any(|id| !valid_id(id)) {
        return Err("한 번에 작품 1~100개를 선택해 주세요.".into());
    }
    let lease = account.lease(window.app_handle()).await?;
    let mut unique = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    let work = state.begin_platform_work(|| {
        let mut rows = jobs().lock();
        if rows.len() + unique.iter().filter(|id| !rows.contains_key(*id)).count() > 100 {
            return Err("대기 중인 작업이 많습니다. 완료 후 다시 시도해 주세요.".into());
        }
        let mut work = Vec::new();
        for id in unique {
            if rows.contains_key(&id) {
                continue;
            }
            let cancel = Arc::new(AtomicBool::new(false));
            rows.insert(
                id.clone(),
                Job {
                    cancel: cancel.clone(),
                    progress: PixivProgress {
                        id: id.clone(),
                        completed_pages: 0,
                        total_pages: 0,
                        bytes: 0,
                        cancelled: false,
                        queued: true,
                    },
                },
            );
            work.push((
                id.clone(),
                cancel,
                JobGuard {
                    id,
                    registry: jobs().clone(),
                },
            ));
        }
        Ok(work)
    })?;
    let app = window.app_handle().clone();
    let mut outcomes = Vec::new();
    for (id, cancel, guard) in work {
        let output_id = id.clone();
        let root = root.clone();
        let lease = lease.clone();
        let result = blocking(move || {
            let _guard = guard;
            observed("pixiv_download", || {
                download_with_session(&id, &root, cancel, &lease)
            })
        })
        .await;
        let outcome = match result {
            Ok(saved) => PixivDownloadOutcome {
                id: output_id,
                saved: Some(saved),
                error: None,
            },
            Err(error) => PixivDownloadOutcome {
                id: output_id,
                saved: None,
                error: Some(error),
            },
        };
        // Each completed item is applied in place by the UI; no full-library
        // refresh or re-fetch of already rendered thumbnails is necessary.
        let _ = app.emit_to("main", "pixiv-download-result", &outcome);
        outcomes.push(outcome);
    }
    Ok(outcomes)
}
#[tauri::command]
pub async fn pixiv_local_work(
    window: tauri::Webview,
    state: State<'_, AppState>,
    id: String,
) -> Result<PixivDetail, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    blocking(move || observed("pixiv_local_detail", || local_detail_sync(&root, &id))).await
}
#[tauri::command]
pub async fn pixiv_local_media(
    window: tauri::Webview,
    state: State<'_, AppState>,
    id: String,
    page: u32,
    detail: bool,
) -> Result<tauri::ipc::Response, String> {
    require_main(&window)?;
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    blocking(move || {
        let result = local_media_sync(&root, &id, page, detail);
        if result.is_err() {
            tracing::warn!(diag_stage = "pixiv_local_image_failed", success = false);
        }
        result
    })
    .await
    .map(tauri::ipc::Response::new)
}
#[tauri::command]
pub async fn pixiv_page_preview(
    window: tauri::Webview,
    account: State<'_, account::PixivAccount>,
    id: String,
    page: u32,
) -> Result<tauri::ipc::Response, String> {
    require_main(&window)?;
    let lease = account.lease(window.app_handle()).await?;
    blocking(move || observed("pixiv_display_preview", || {
        let detail = detail_with_session(&id, &lease)?;
        let page = detail.pages.get(page as usize).ok_or("작품 페이지가 없습니다.")?;
        let _permit = MediaPermit::acquire()?;
        let bytes = limited_body(response_with_session(&page.preview_url, true, &lease)?, IMAGE_LIMIT)?;
        let encoded = display_jpeg(bytes, true)?;
        lease.check()?;
        Ok(encoded)
    })).await.map(tauri::ipc::Response::new)
}
#[tauri::command]
pub async fn pixiv_page_media(
    window: tauri::Webview,
    account: State<'_, account::PixivAccount>,
    id: String,
    page: u32,
) -> Result<tauri::ipc::Response, String> {
    require_main(&window)?;
    let lease = account.lease(window.app_handle()).await?;
    blocking(move || {
        let result = original_page_with_session(&id, page, &lease);
        if let Err(error) = &result {
            let stage = if error.starts_with("PIXIV_DISPLAY_LIMIT:") { "pixiv_original_display_limit" }
                else if error.contains("이미지 크기가 일치") { "pixiv_original_dimensions_mismatch" }
                else if error.contains("이미지가 손상") || error.contains("이미지 형식") { "pixiv_original_decode_failed" }
                else { "pixiv_original_image_failed" };
            tracing::warn!(diag_stage = stage, success = false);
        }
        lease.check()?;
        result
    })
    .await
    .map(tauri::ipc::Response::new)
}
#[tauri::command]
pub fn pixiv_download_progress(window: tauri::Webview) -> Result<Vec<PixivProgress>, String> {
    require_main(&window)?;
    Ok(download_snapshot())
}
#[tauri::command]
pub fn pixiv_download_cancel(window: tauri::Webview, id: String) -> Result<(), String> {
    require_main(&window)?;
    let id = artwork_id(&id)?;
    let rows = jobs().lock();
    if let Some(job) = rows.get(&id) {
        job.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}
#[tauri::command]
pub async fn pixiv_library_action(
    window: tauri::Webview,
    state: State<'_, AppState>,
    id: String,
    action: String,
    token: Option<String>,
) -> Result<PixivLibraryResult, String> {
    require_main(&window)?;
    if !valid_id(&id) || !["open", "remove", "restore"].contains(&action.as_str()) {
        return Err("올바른 보관함 작업이 아닙니다.".into());
    }
    if action == "restore" && token.is_none() {
        return Err("복원 정보가 없습니다.".into());
    }
    let root = state
        .settings_snapshot()
        .map_err(|_| "다운로드 설정을 읽지 못했습니다.")?
        .download_root;
    let guard = state.begin_platform_work(|| {
        let mut rows = jobs().lock();
        if rows.contains_key(&id) {
            return Err("이 작품의 진행 중인 작업을 먼저 완료해 주세요.".into());
        }
        rows.insert(
            id.clone(),
            Job {
                cancel: Arc::new(AtomicBool::new(false)),
                progress: PixivProgress {
                    id: id.clone(),
                    completed_pages: 0,
                    total_pages: 0,
                    bytes: 0,
                    cancelled: false,
                    queued: false,
                },
            },
        );
        Ok(JobGuard {
            id: id.clone(),
            registry: jobs().clone(),
        })
    })?;
    blocking(move || {
        let _guard = guard;
        observed("pixiv_library_action", || {
            if action == "open" {
                let root = root_path(&root, false)?;
                let (folder, _) = management_record(&root, &id, &id)?;
                open_native_target(&folder.to_string_lossy())?;
                Ok(PixivLibraryResult {
                    token: None,
                    record: None,
                })
            } else {
                library_move_sync(
                    &root,
                    &id,
                    if action == "restore" {
                        token.as_deref()
                    } else {
                        None
                    },
                )
            }
        })
    })
    .await
}
#[tauri::command]
pub fn pixiv_open(window: tauri::Webview, id: String) -> Result<(), String> {
    require_main(&window)?;
    let id = artwork_id(&id)?;
    open_native_target(&format!("{SITE}/artworks/{id}"))
}
fn open_native_target(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(operation.as_ptr()),
                PCWSTR(target.as_ptr()),
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
            "Pixiv",
            "150698216",
        ] {
            path.push(component);
            let metadata = fs::symlink_metadata(&path).expect("missing approved offline fixture");
            assert!(
                metadata.is_dir() && !is_redirect(&metadata),
                "unsafe approved fixture directory"
            );
        }
        let provider = path.parent().unwrap();
        let entries = fs::read_dir(provider)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            entries.len() == 1 && entries[0].file_name() == "150698216",
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
        let expected = std::iter::once(std::ffi::OsString::from(RECORD_FILE))
            .chain((0..7).map(|index| std::ffi::OsString::from(format!("150698216_p{index}.png"))))
            .collect::<Vec<_>>();
        assert!(names == expected, "unexpected approved fixture files");
        names
            .into_iter()
            .map(|name| {
                let mut bytes = Vec::new();
                local_file(&folder.join(&name), DISPLAY_ORIGINAL_LIMIT)
                    .expect("approved fixture read failed")
                    .take(DISPLAY_ORIGINAL_LIMIT + 1)
                    .read_to_end(&mut bytes)
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
    fn qa_saved_originals_reopen_offline_without_mutation() {
        let guard = OfflineNetworkGuard::new();
        let root = qa_media_root();
        let folder = root.join("Pixiv/150698216");
        let before = qa_snapshot(&folder);
        let expected = [
            "8092a0ee678e5d8f3ef2802797330e64fada4aeb6d6c752ae38529d323d632c3",
            "2b2f8aecd8d15045f8dc3b937d0f4090f0db658c15deca8d10750e15fa30065b",
            "a37118562ffe275b23620060922ee80125a5e898fb9fbaea9d060acae562709f",
            "0158dd8d4e04612284ff7088275de84fbc751d9821c6f04704a2e6e41e36d30f",
            "36f0dda7d2f689f0c13ada7ad0bdeec8eb8835a86ba7a4b2596a59bfb73da3bf",
            "67ad9a8b99f91e73118d270b350de4913b8aa4fef19701aaf44f5efb9915a62b",
            "af1eaba56097d1f559a1e6d007c1d0fb1a2331e82f5a2c651e5c08b29da7ba12",
            "67bcfca6806884859c93f7b4496d214312d214d22946dea29ddc354b70e18304",
        ];
        assert!(
            before
                .iter()
                .zip(expected)
                .all(|(item, hash)| item.2 == hash),
            "approved fixture baseline mismatch"
        );
        assert_eq!(before[0].1, 2414);
        assert_eq!(
            before.iter().skip(1).map(|item| item.1).sum::<u64>(),
            21_646_837
        );
        let records = downloads_sync(root.to_str().unwrap()).expect("offline list failed");
        assert!(
            records.len() == 1 && records[0].work.id == "150698216" && records[0].issue.is_none(),
            "offline list mismatch"
        );
        let detail =
            local_detail_sync(root.to_str().unwrap(), "150698216").expect("offline detail failed");
        assert_eq!(detail.pages.len(), 7);
        for (index, page) in detail.pages.iter().enumerate() {
            let bytes = local_media_sync(root.to_str().unwrap(), "150698216", index as u32, true)
                .expect("offline original reader failed");
            assert!(
                format!("{:x}", Sha256::digest(&bytes)) == before[index + 1].2
                    && bytes.len() as u64 == before[index + 1].1,
                "original reader changed stored bytes"
            );
            let mut reader = ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()
                .expect("original format invalid");
            let mut limits = Limits::default();
            limits.max_image_width = Some(10000);
            limits.max_image_height = Some(10000);
            limits.max_alloc = Some(96 * 1024 * 1024);
            reader.limits(limits);
            let image = reader.decode().expect("original decode failed");
            assert_eq!((image.width(), image.height()), (page.width, page.height));
        }
        let thumb = local_media_sync(root.to_str().unwrap(), "150698216", 0, false)
            .expect("offline thumbnail reader failed");
        let image = image::load_from_memory(&thumb).expect("thumbnail decode failed");
        assert!(
            thumb.len() <= 400 * 1024 && image.width() <= 480 && image.height() <= 480,
            "thumbnail exceeds display bounds"
        );
        assert_eq!(guard.attempts(), 0, "offline reader entered HTTP client");
        assert!(
            before == qa_snapshot(&folder),
            "approved originals or manifest changed during read"
        );
    }
    #[test]
    fn diagnostics_preserve_the_operation_result() {
        assert_eq!(observed("pixiv_search", || Ok(7u8)), Ok(7));
        assert_eq!(
            observed::<u8>("pixiv_download", || Err("fixture error".into())),
            Err("fixture error".into())
        );
    }
    #[test]
    fn poisoned_local_registry_recovers_real_work_cancel_progress_and_raii_cleanup() {
        // Deliberately poison only this test's private mutex, never global JOBS.
        let registry = Arc::new(JobRegistry::default());
        let cancel = Arc::new(AtomicBool::new(false));
        registry.lock().insert(
            "123".into(),
            Job {
                cancel: cancel.clone(),
                progress: PixivProgress {
                    id: "123".into(),
                    completed_pages: 0,
                    total_pages: 2,
                    bytes: 0,
                    cancelled: false,
                    queued: false,
                },
            },
        );
        let guard = JobGuard {
            id: "123".into(),
            registry: registry.clone(),
        };
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _locked = registry.entries.lock().unwrap();
            panic!("isolated registry fixture panic");
        }));
        assert!(panic_result.is_err());
        assert!(registry.entries.is_poisoned());
        assert_eq!(registry.count(), 1);
        assert_eq!(registry.ids(), vec!["123"]);
        assert_eq!(registry.snapshot()[0].total_pages, 2);
        registry
            .lock()
            .get_mut("123")
            .unwrap()
            .progress
            .completed_pages = 1;
        assert_eq!(registry.snapshot()[0].completed_pages, 1);
        registry.cancel_all();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(registry.snapshot()[0].cancelled);
        drop(guard);
        assert_eq!(registry.count(), 0);
        assert!(registry.ids().is_empty());
        assert!(registry.snapshot().is_empty());
        // The poisoned mutex remains recoverable for a later, fresh operation.
        registry.lock().insert(
            "456".into(),
            Job {
                cancel: Arc::new(AtomicBool::new(false)),
                progress: PixivProgress {
                    id: "456".into(),
                    completed_pages: 0,
                    total_pages: 1,
                    bytes: 0,
                    cancelled: false,
                    queued: false,
                },
            },
        );
        let next_guard = JobGuard {
            id: "456".into(),
            registry: registry.clone(),
        };
        assert_eq!(registry.ids(), vec!["456"]);
        drop(next_guard);
        assert_eq!(registry.count(), 0);
    }
    #[test]
    fn poisoned_empty_local_registry_does_not_invent_permanent_active_work() {
        let registry = JobRegistry::default();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _locked = registry.entries.lock().unwrap();
            panic!("isolated empty fixture panic");
        }));
        assert!(registry.entries.is_poisoned());
        assert_eq!(registry.count(), 0);
        assert!(registry.ids().is_empty());
        registry.cancel_all();
        assert!(registry.snapshot().is_empty());
    }
    #[test]
    fn image_and_metadata_gate_never_exceeds_four_workers() {
        use std::sync::atomic::AtomicUsize;
        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let current = current.clone();
                let peak = peak.clone();
                scope.spawn(move || {
                    let _permit = MediaPermit::acquire().unwrap();
                    let count = current.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(12));
                    current.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= 4);
        assert_eq!(current.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn job_guard_exposes_active_work_and_always_unregisters_on_drop() {
        let id = "998877661122".to_owned();
        jobs().lock().insert(
            id.clone(),
            Job {
                cancel: Arc::new(AtomicBool::new(false)),
                progress: PixivProgress {
                    id: id.clone(),
                    completed_pages: 0,
                    total_pages: 2,
                    bytes: 0,
                    cancelled: false,
                    queued: false,
                },
            },
        );
        let guard = JobGuard {
            id: id.clone(),
            registry: jobs().clone(),
        };
        assert!(active_download_ids().contains(&id));
        assert!(download_snapshot().iter().any(|row| row.id == id));
        drop(guard);
        assert!(!active_download_ids().contains(&id));
    }
    #[test]
    #[ignore = "Opt-in public network smoke; only writes into an isolated temporary fixture directory"]
    fn live_public_search_media_and_download_fixture() {
        for mode in ["daily", "weekly"] {
            let result = search_sync(PixivSearchRequest {
                query: String::new(),
                page: 1,
                mode: mode.into(),
                home_cursor: None,
                safe_only: false,
            })
            .unwrap();
            assert!(!result.items.is_empty(), "{mode} returned no public works");
            println!("Pixiv {mode}: {} public works", result.items.len());
        }
        let result = search_sync(PixivSearchRequest {
            query: "landscape".into(),
            page: 1,
            mode: "latest".into(),
            home_cursor: None,
            safe_only: false,
        })
        .unwrap();
        assert!(!result.items.is_empty());
        let selected = result
            .items
            .iter()
            .find(|item| {
                item.kind != "ugoira" && item.page_count <= 3 && item.thumbnail_url.is_some()
            })
            .unwrap();
        let thumbnail = media_sync(selected.thumbnail_url.as_ref().unwrap(), false).unwrap();
        assert!(thumbnail.starts_with("data:image/jpeg;base64,"));
        assert!(thumbnail.len() < 550_000);
        let detail = detail_sync(&selected.id).unwrap();
        assert_eq!(detail.pages.len(), selected.page_count as usize);
        let creator = artist_sync(&detail.work.artist_id, 1).unwrap();
        assert!(!creator.items.is_empty());
        assert!(creator
            .items
            .iter()
            .all(|item| item.artist_id == detail.work.artist_id));
        let original = original_page_sync(&selected.id, 0).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let saved = download_sync(
            &selected.id,
            directory.path().to_str().unwrap(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(saved.page_count, selected.page_count);
        assert!(saved.bytes > 0);
        assert_eq!(
            local_detail_sync(directory.path().to_str().unwrap(), &selected.id)
                .unwrap()
                .pages
                .len(),
            detail.pages.len()
        );
        assert_eq!(
            local_media_sync(directory.path().to_str().unwrap(), &selected.id, 0, true).unwrap(),
            original
        );
        assert_eq!(
            downloads_sync(directory.path().to_str().unwrap())
                .unwrap()
                .len(),
            1
        );
        println!(
            "Pixiv public fixture id={} pages={} bytes={} thumbnailChars={} creatorWorks={}; local original equals public original",
            saved.work.id,
            saved.page_count,
            saved.bytes,
            thumbnail.len(), creator.items.len()
        );
    }
    fn fixture_detail(pages: u32) -> PixivDetail {
        PixivDetail {
            work: work(&serde_json::json!({"id":"123","title":"Local fixture","pageCount":pages}))
                .unwrap(),
            pages: (0..pages)
                .map(|index| PixivPage {
                    index,
                    width: 4,
                    height: 3,
                    preview_url: format!(
                        "https://i.pximg.net/img-master/img/123_p{index}_master1200.jpg"
                    ),
                    original_url: format!("https://i.pximg.net/img-original/img/123_p{index}.jpg"),
                })
                .collect(),
        }
    }
    fn fixture_jpeg() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(4, 3, image::Rgb([30, 90, 120]));
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes).encode_image(&image).unwrap();
        bytes
    }
    #[test]
    fn library_remove_restore_is_recoverable_and_refuses_extra_files_or_overwrite() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let saved = save_detail_sync(
            "123",
            fixture_detail(1),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(fixture_response()),
        )
        .unwrap();
        let album = PathBuf::from(&saved.folder);
        fs::write(album.join("user-added.txt"), b"untouched").unwrap();
        assert!(library_move_sync(root, "123", None).is_err());
        assert_eq!(
            fs::read(album.join("user-added.txt")).unwrap(),
            b"untouched"
        );
        fs::remove_file(album.join("user-added.txt")).unwrap();
        let removed = library_move_sync(root, "123", None).unwrap();
        let token = removed.token.unwrap();
        assert!(!album.exists());
        let rediscovered = downloads_sync(root).unwrap();
        assert_eq!(rediscovered.len(), 1);
        assert_eq!(
            rediscovered[0].removed_token.as_deref(),
            Some(token.as_str())
        );
        assert!(library_move_sync(root, "123", Some("../outside")).is_err());
        fs::create_dir(&album).unwrap();
        assert!(library_move_sync(root, "123", Some(&token)).is_err());
        fs::remove_dir(&album).unwrap();
        let restored = library_move_sync(root, "123", Some(&token))
            .unwrap()
            .record
            .unwrap();
        assert_eq!(restored.work.id, "123");
        assert_eq!(
            local_media_sync(root, "123", 0, true).unwrap(),
            fixture_jpeg()
        );
    }
    fn fixture_response() -> OriginalResponse {
        let bytes = fixture_jpeg();
        OriginalResponse {
            content_length: Some(bytes.len() as u64),
            mime: "image/jpeg".into(),
            body: Box::new(Cursor::new(bytes)),
        }
    }
    #[test]
    fn failed_staging_replacement_never_removes_an_existing_album() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let pixiv = root_path(root, true).unwrap();
        let existing = pixiv.join("456");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("original.keep"), b"existing original").unwrap();
        let mut substituted = None;
        let error = save_detail_sync(
            "123",
            fixture_detail(1),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| {
                let staging = fs::read_dir(&pixiv)
                    .unwrap()
                    .flatten()
                    .find(|entry| entry.file_name().to_string_lossy().starts_with(".123-"))
                    .unwrap()
                    .path();
                fs::rename(&staging, pixiv.join("interrupted-staging")).unwrap();
                fs::rename(&existing, &staging).unwrap();
                substituted = Some(staging);
                Err("fixture transport failure".into())
            },
        )
        .unwrap_err();
        assert_eq!(error, "fixture transport failure");
        assert_eq!(
            fs::read(substituted.unwrap().join("original.keep")).unwrap(),
            b"existing original"
        );
        assert!(pixiv.join("interrupted-staging").exists());
    }
    #[test]
    fn damaged_albums_remain_manageable_and_removed_tokens_survive_new_queries() {
        for missing in [true, false] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().to_str().unwrap();
            let saved = save_detail_sync(
                "123",
                fixture_detail(2),
                root,
                Arc::new(AtomicBool::new(false)),
                |_| Ok(fixture_response()),
            )
            .unwrap();
            let folder = Path::new(&saved.folder);
            if missing {
                fs::remove_file(folder.join("123_p0.jpg")).unwrap();
            } else {
                fs::write(folder.join("123_p0.jpg"), b"truncated").unwrap();
            }
            let records = downloads_sync(root).unwrap();
            assert_eq!(records.len(), 1);
            assert!(records[0].issue.is_some());
            assert!(records[0].removed_token.is_none());
            assert!(local_detail_sync(root, "123").is_err());
            let token = library_move_sync(root, "123", None).unwrap().token.unwrap();
            let fresh_records = downloads_sync(root).unwrap();
            assert_eq!(fresh_records.len(), 1);
            assert_eq!(
                fresh_records[0].removed_token.as_deref(),
                Some(token.as_str())
            );
            assert!(fresh_records[0].issue.is_some());
            let restored =
                library_move_sync(root, "123", fresh_records[0].removed_token.as_deref())
                    .unwrap()
                    .record
                    .unwrap();
            assert!(restored.issue.is_some());
            assert!(restored.removed_token.is_none());
            assert_eq!(
                fs::read(Path::new(&restored.folder).join("123_p1.jpg")).unwrap(),
                fixture_jpeg()
            );
            assert!(downloads_sync(root).unwrap()[0].removed_token.is_none());
            let external = temporary.path().join("outside-original.jpg");
            fs::write(&external, fixture_jpeg()).unwrap();
            let retained_page = Path::new(&restored.folder).join("123_p1.jpg");
            fs::remove_file(&retained_page).unwrap();
            fs::hard_link(&external, &retained_page).unwrap();
            assert!(library_move_sync(root, "123", None).is_err());
            assert_eq!(fs::read(&external).unwrap(), fixture_jpeg());
        }
    }
    #[test]
    fn local_originals_roundtrip_without_network_and_validate_digest() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let saved = save_detail_sync(
            "123",
            fixture_detail(2),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(fixture_response()),
        )
        .unwrap();
        assert_eq!(saved.pages.len(), 2);
        let detail = local_detail_sync(root, "123").unwrap();
        assert_eq!(detail.pages[1].preview_url, "atsumi-pixiv:123:1");
        assert_eq!(
            local_media_sync(root, "123", 1, true).unwrap(),
            fixture_jpeg()
        );
        assert!(local_media_sync(root, "123", 0, false).unwrap().len() < 400 * 1024);
        let path = Path::new(&saved.folder).join("123_p0.jpg");
        let mut damaged = fs::read(&path).unwrap();
        damaged[100] ^= 1;
        fs::write(&path, damaged).unwrap();
        assert!(local_media_sync(root, "123", 0, true)
            .unwrap_err()
            .contains("손상"));
        assert!(local_detail_sync(root, "../123").is_err());
        assert!(local_media_sync(root, "123", 2, true).is_err());
    }
    #[test]
    fn local_originals_reject_missing_page_manifest_traversal_and_hard_link() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let mut saved = save_detail_sync(
            "123",
            fixture_detail(1),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(fixture_response()),
        )
        .unwrap();
        let folder = Path::new(&saved.folder);
        let path = folder.join("123_p0.jpg");
        let external = temporary.path().join("external.jpg");
        fs::write(&external, fixture_jpeg()).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(local_detail_sync(root, "123").is_err());
        fs::hard_link(&external, &path).unwrap();
        assert!(local_detail_sync(root, "123").is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, fixture_jpeg()).unwrap();
        saved.pages[0].file = "../../external.jpg".into();
        fs::write(
            folder.join(RECORD_FILE),
            serde_json::to_vec(&saved).unwrap(),
        )
        .unwrap();
        assert!(local_detail_sync(root, "123").is_err());
        assert_eq!(fs::read(&external).unwrap(), fixture_jpeg());
    }
    #[test]
    fn legacy_local_album_can_open_without_rewriting_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let mut saved = save_detail_sync(
            "123",
            fixture_detail(1),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(fixture_response()),
        )
        .unwrap();
        saved.pages.clear();
        let path = Path::new(&saved.folder).join(RECORD_FILE);
        let before = serde_json::to_vec(&saved).unwrap();
        fs::write(&path, &before).unwrap();
        assert_eq!(
            local_media_sync(root, "123", 0, true).unwrap(),
            fixture_jpeg()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    #[test]
    fn fixture_download_publishes_all_pages_atomically_and_refuses_overwrite() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        let saved = save_detail_sync(
            "123",
            fixture_detail(2),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(fixture_response()),
        )
        .unwrap();
        assert_eq!(saved.page_count, 2);
        assert_eq!(saved.bytes, 2 * fixture_jpeg().len() as u64);
        let destination = Path::new(&saved.folder);
        assert!(destination.join(RECORD_FILE).is_file());
        assert!(destination.join("123_p1.jpg").is_file());
        assert_eq!(downloads_sync(root).unwrap().len(), 1);
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            1
        );
        let before = fs::read(destination.join("123_p0.jpg")).unwrap();
        let error = save_detail_sync(
            "123",
            fixture_detail(2),
            root,
            Arc::new(AtomicBool::new(false)),
            |_| panic!("must not fetch an existing album"),
        )
        .unwrap_err();
        assert!(error.contains("덮어쓰지"));
        assert_eq!(before, fs::read(destination.join("123_p0.jpg")).unwrap());
    }
    #[test]
    fn truncated_or_wrong_mime_response_never_publishes_a_completed_album() {
        for wrong_mime in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().to_str().unwrap();
            let result = save_detail_sync(
                "123",
                fixture_detail(1),
                root,
                Arc::new(AtomicBool::new(false)),
                |_| {
                    let mut response = fixture_response();
                    if wrong_mime {
                        response.mime = "text/html".into();
                    } else {
                        response.content_length = response.content_length.map(|length| length + 1);
                    }
                    Ok(response)
                },
            );
            assert!(result.is_err());
            assert!(!temporary.path().join("Pixiv/123").exists());
            assert_eq!(
                fs::read_dir(temporary.path().join("Pixiv"))
                    .unwrap()
                    .count(),
                1
            );
            assert!(downloads_sync(root).unwrap().is_empty());
        }
    }
    #[test]
    fn cancellation_preserves_unverified_staging_and_unrelated_files() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_str().unwrap();
        fs::create_dir_all(temporary.path().join("Pixiv")).unwrap();
        fs::write(temporary.path().join("Pixiv/unrelated.keep"), "retain").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        struct CancellingRead {
            cursor: Cursor<Vec<u8>>,
            flag: Arc<AtomicBool>,
        }
        impl Read for CancellingRead {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let count = self.cursor.read(buffer)?;
                self.flag.store(true, Ordering::Relaxed);
                Ok(count)
            }
        }
        let result = save_detail_sync("123", fixture_detail(2), root, cancel, |_| {
            let bytes = fixture_jpeg();
            Ok(OriginalResponse {
                content_length: Some(bytes.len() as u64),
                mime: "image/jpeg".into(),
                body: Box::new(CancellingRead {
                    cursor: Cursor::new(bytes),
                    flag: flag.clone(),
                }),
            })
        });
        assert!(result.unwrap_err().contains("취소"));
        assert_eq!(
            fs::read_to_string(temporary.path().join("Pixiv/unrelated.keep")).unwrap(),
            "retain"
        );
        assert_eq!(
            fs::read_dir(temporary.path().join("Pixiv"))
                .unwrap()
                .count(),
            2
        );
    }
    #[test]
    fn mismatched_id_dimensions_or_original_host_are_rejected() {
        for variant in 0..3 {
            let temporary = tempfile::tempdir().unwrap();
            let mut detail = fixture_detail(1);
            match variant {
                0 => detail.work.id = "456".into(),
                1 => detail.pages[0].width = 9,
                _ => detail.pages[0].original_url = "https://localhost/123_p0.jpg".into(),
            }
            let result = save_detail_sync(
                "123",
                detail,
                temporary.path().to_str().unwrap(),
                Arc::new(AtomicBool::new(false)),
                |_| Ok(fixture_response()),
            );
            assert!(result.is_err());
            assert!(!temporary.path().join("Pixiv/123").exists());
        }
    }
    #[test]
    fn original_content_length_limit_is_checked_before_reading() {
        let temporary = tempfile::tempdir().unwrap();
        let result = save_detail_sync(
            "123",
            fixture_detail(1),
            temporary.path().to_str().unwrap(),
            Arc::new(AtomicBool::new(false)),
            |_| {
                Ok(OriginalResponse {
                    mime: "image/jpeg".into(),
                    content_length: Some(PAGE_LIMIT + 1),
                    body: Box::new(Cursor::new(Vec::<u8>::new())),
                })
            },
        );
        assert!(result.unwrap_err().contains("초과"));
        assert!(!temporary.path().join("Pixiv/123").exists());
    }
    #[test]
    fn every_ipc_command_checks_main_origin() {
        let source = include_str!("pixiv.rs");
        for name in [
            "pixiv_search",
            "pixiv_discovery",
            "pixiv_work",
            "pixiv_media",
            "pixiv_download",
            "pixiv_downloads_list",
            "pixiv_download_progress",
            "pixiv_download_cancel",
            "pixiv_open",
            "pixiv_local_work",
            "pixiv_local_media",
            "pixiv_page_media",
            "pixiv_page_preview",
            "pixiv_download_batch",
            "pixiv_library_action",
        ] {
            let marker = format!("fn {name}(");
            let definition = source.split_once(&marker).unwrap().1;
            let body = definition.split_once('{').unwrap().1;
            assert!(
                body.trim_start().starts_with("require_main(&window)?;"),
                "{name}"
            );
        }
        let config = Url::parse("http://127.0.0.1:1420").unwrap();
        assert!(crate::frontend_origin::trusted_main(
            "main",
            &config,
            true,
            Some(&config)
        ));
        assert!(!crate::frontend_origin::trusted_main(
            "official-browser",
            &config,
            true,
            Some(&config)
        ));
        assert!(!crate::frontend_origin::trusted_main(
            "main",
            &Url::parse("https://www.pixiv.net").unwrap(),
            true,
            Some(&config)
        ));
    }
    #[test]
    fn artwork_input_is_strict() {
        assert_eq!(
            artwork_id("https://www.pixiv.net/ko/artworks/150653396?from=test").unwrap(),
            "150653396"
        );
        assert_eq!(artwork_id("123").unwrap(), "123");
        for value in [
            "0",
            "01",
            "1234567890123",
            "http://www.pixiv.net/artworks/123",
            "https://pixiv.net.attacker.test/artworks/123",
            "https://user@pixiv.net/artworks/123",
            "https://pixiv.net:444/artworks/123",
            "https://pixiv.net/artworks/123/extra",
            "file:///123",
        ] {
            assert!(artwork_id(value).is_err(), "{value}");
        }
    }
    #[test]
    fn media_only_allows_expected_https_host_and_paths() {
        assert!(allowed_url(
            "https://i.pximg.net/c/480x480/img-master/img/test.jpg",
            true
        )
        .is_ok());
        for value in [
            "https://i.pximg.net.evil.test/img-original/x.jpg",
            "http://i.pximg.net/img-original/x.jpg",
            "https://localhost/img-original/x.jpg",
            "https://i.pximg.net:8443/img-original/x.jpg",
            "https://i.pximg.net/avatar.jpg",
            "https://i.pximg.net/img-original/x.jpg#fragment",
        ] {
            assert!(allowed_url(value, true).is_err(), "{value}");
        }
    }
    #[test]
    fn restrictions_are_not_silently_bypassed() {
        for value in [
            serde_json::json!({"xRestrict":1}),
            serde_json::json!({"restrict":1}),
            serde_json::json!({"sl":4}),
            serde_json::json!({"isLoginOnly":true}),
            serde_json::json!({"illust_content":{"sexual":true}}),
        ] {
            assert!(is_restricted(&value));
        }
    }
    #[test]
    fn verified_session_follows_server_access_not_an_unconditional_rating_filter() {
        assert!(!is_restricted_for_session(
            &serde_json::json!({"isLoginOnly":true}),
            true
        ));
        for value in [
            serde_json::json!({"xRestrict":1,"isLoginOnly":true}),
            serde_json::json!({"sl":4}),
            serde_json::json!({"illust_content":{"sexual":true}}),
        ] {
            assert!(!is_restricted_for_session(&value, true));
        }
        assert!(is_restricted_for_session(
            &serde_json::json!({"restrict":1}),
            true
        ));
        assert!(is_restricted_for_session(
            &serde_json::json!({"isMasked":true}),
            true
        ));
        let authority = session::Authority::default();
        let lease = authority.lease(None);
        authority.invalidate();
        assert!(search_with_session(
            PixivSearchRequest {
                query: String::new(),
                page: 1,
                mode: "daily".into(),
                home_cursor: None,
                safe_only: false,
            },
            &lease
        )
        .is_err());
        assert!(detail_with_session("123", &lease).is_err());
    }
    #[test]
    fn maps_search_and_ranking_without_remote_markup() {
        let search = work(&serde_json::json!({"id":"123","title":"a <b>title</b>","userId":"5","userName":"artist","pageCount":2,"illustType":1,"tags":["landscape"],"url":"https://i.pximg.net/c/250x250/img-master/img/x.jpg"})).unwrap();
        assert_eq!(search.page_count, 2);
        assert_eq!(search.kind, "manga");
        assert_eq!(search.tags, vec!["landscape"]);
        let rank = work(&serde_json::json!({"illust_id":456,"title":"rank","user_id":5,"user_name":"artist","illust_page_count":"3","illust_type":"0"})).unwrap();
        assert_eq!(rank.id, "456");
        assert_eq!(rank.page_count, 3);
    }
    #[test]
    fn missing_or_partial_downloads_are_not_complete() {
        let temporary = tempfile::tempdir().unwrap();
        let root = root_path(temporary.path().to_str().unwrap(), true).unwrap();
        let album = root.join("123");
        fs::create_dir(&album).unwrap();
        let mut record = PixivDownload {
            work: work(&serde_json::json!({"id":"123","title":"fixture","pageCount":1})).unwrap(),
            saved_at: "123".into(),
            page_count: 1,
            bytes: 100,
            folder: "untrusted".into(),
            pages: vec![],
            issue: None,
            removed_token: None,
        };
        fs::write(
            album.join(RECORD_FILE),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(
            downloads_sync(temporary.path().to_str().unwrap()).unwrap()[0]
                .issue
                .is_some()
        );
        fs::write(album.join("123_p0.jpg"), b"fixture").unwrap();
        assert!(
            downloads_sync(temporary.path().to_str().unwrap()).unwrap()[0]
                .issue
                .is_some()
        );
        let bytes = fixture_jpeg();
        record.bytes = bytes.len() as u64;
        fs::write(album.join("123_p0.jpg"), bytes).unwrap();
        fs::write(
            album.join(RECORD_FILE),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let rows = downloads_sync(temporary.path().to_str().unwrap()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].folder, album.to_string_lossy());
    }
    #[test]
    fn bad_sort_and_empty_search_fail_before_network() {
        assert!(search_sync(PixivSearchRequest {
            query: "".into(),
            page: 1,
            mode: "popular".into(),
            home_cursor: None,
            safe_only: false,
        })
        .is_err());
        assert!(search_sync(PixivSearchRequest {
            query: "".into(),
            page: 1,
            mode: "latest".into(),
            home_cursor: None,
            safe_only: false,
        })
        .is_err());
        assert!(search_sync(PixivSearchRequest {
            query: "landscape".into(),
            page: 1,
            mode: "daily".into(),
            home_cursor: None,
            safe_only: false,
        })
        .is_err());
    }

    #[test]
    fn request_defaults_to_account_scope_and_validates_the_local_switch() {
        let request: PixivSearchRequest = serde_json::from_value(
            serde_json::json!({"query":"landscape","mode":"latest","page":1}),
        )
        .unwrap();
        assert!(!request.safe_only);
        for invalid in [
            serde_json::json!("false"),
            serde_json::json!(1),
            serde_json::Value::Null,
        ] {
            assert!(serde_json::from_value::<PixivSearchRequest>(
                serde_json::json!({"query":"landscape","mode":"latest","page":1,"safeOnly":invalid})
            )
            .is_err());
        }
        for (authenticated, safe_only, expected) in [
            (true, false, "all"),
            (true, true, "safe"),
            (false, false, "safe"),
        ] {
            let url =
                keyword_search_url("landscape", 1, DisplayPolicy::new(authenticated, safe_only))
                    .unwrap();
            assert_eq!(
                url.query_pairs().find(|(key, _)| key == "mode").unwrap().1,
                expected
            );
            assert_eq!(url.host_str(), Some("www.pixiv.net"));
        }
    }

    #[test]
    #[ignore = "Opt-in ordinary public metadata only; no session export, media or writes"]
    fn live_public_display_modes_metadata_only() {
        let lease = session::Lease::default();
        // Probe the official query variants without authentication. This is not
        // evidence of any account's age/visibility eligibility.
        for safe_only in [true, false] {
            let display = DisplayPolicy::new(true, safe_only);
            let url = keyword_search_url("landscape", 1, display).unwrap();
            let response = json_with_session(url.as_str(), &lease).unwrap();
            let rows = response
                .pointer("/body/illustManga/data")
                .and_then(Value::as_array)
                .unwrap();
            assert!(!rows.is_empty());
            let projected = rows
                .iter()
                .filter_map(|row| work_with_display(row, DisplayPolicy::new(false, safe_only)))
                .count();
            assert!(projected > 0);
            println!("Pixiv ordinary public query mode={}: server_rows={}, mapped_public_rows={}, total={}; no images or account credentials", display.query_mode(), rows.len(), projected, number(&response["body"]["illustManga"]["total"]));
        }
    }
}
