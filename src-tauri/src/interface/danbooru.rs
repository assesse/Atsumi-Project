use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use md5::{Digest, Md5};
use reqwest::{blocking::Client, header::CONTENT_TYPE, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::State;

use super::{commands::AppState, ApiAction, ApiError, ApiResult};

const API_ROOT: &str = "https://danbooru.donmai.us";
const API_START_INTERVAL: Duration = Duration::from_millis(250);
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DISPLAY_MEDIA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CONCURRENT_MEDIA_REQUESTS: usize = 6;
const INDEX_FILE: &str = ".atsumi-danbooru-index.json";
const MEDIA_PROXY_HTTP_ROOT: &str = "http://danbooru-media.localhost/";
const MEDIA_PROXY_SCHEME_ROOT: &str = "danbooru-media://localhost/";
const RELATION_POST_LIMIT: u32 = 50;
const RELATED_POOL_LIMIT: usize = 8;
const RELATED_POOL_WINDOW: usize = 9;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DanbooruSearchRequest {
    pub tags: String,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruSearchPage {
    pub items: Vec<DanbooruPost>,
    pub page: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DanbooruRelatedRequest {
    pub post_id: u64,
    pub parent_id: Option<u64>,
    pub has_children: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruPoolRelation {
    pub id: u64,
    pub name: String,
    pub category: String,
    pub post_count: u64,
    pub current_index: usize,
    pub items: Vec<DanbooruPost>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruRelatedPosts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<DanbooruPost>,
    pub siblings: Vec<DanbooruPost>,
    pub children: Vec<DanbooruPost>,
    pub pools: Vec<DanbooruPoolRelation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruPost {
    pub id: u64,
    pub created_at: String,
    pub rating: String,
    pub score: i64,
    pub favorite_count: u64,
    pub image_width: u32,
    pub image_height: u32,
    pub file_ext: String,
    pub file_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_url: Option<String>,
    pub artists: Vec<String>,
    pub copyrights: Vec<String>,
    pub characters: Vec<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u64>,
    pub has_children: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruAutocompleteItem {
    pub label: String,
    pub value: String,
    pub category: u32,
    pub post_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruDownloadRecord {
    pub post: DanbooruPost,
    pub file_name: String,
    /// Unix epoch milliseconds encoded as a string to preserve integer precision in JavaScript.
    pub downloaded_at: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DanbooruDownloadsRequest {
    pub page: u32,
    pub page_size: u32,
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanbooruDownloadsPage {
    pub items: Vec<DanbooruDownloadRecord>,
    pub page: u32,
    pub total: u64,
    pub total_pages: u32,
}

#[derive(Debug, Deserialize)]
struct RemotePost {
    id: u64,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    fav_count: u64,
    #[serde(default)]
    image_width: u32,
    #[serde(default)]
    image_height: u32,
    #[serde(default)]
    file_ext: String,
    #[serde(default)]
    file_size: u64,
    md5: Option<String>,
    source: Option<String>,
    preview_file_url: Option<String>,
    large_file_url: Option<String>,
    file_url: Option<String>,
    media_asset: Option<RemoteMediaAsset>,
    #[serde(default)]
    tag_string_general: String,
    #[serde(default)]
    tag_string_artist: String,
    #[serde(default)]
    tag_string_character: String,
    #[serde(default)]
    tag_string_copyright: String,
    parent_id: Option<u64>,
    #[serde(default)]
    has_children: bool,
}

#[derive(Debug, Deserialize)]
struct RemotePool {
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    post_count: u64,
    #[serde(default)]
    post_ids: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct RemoteMediaAsset {
    #[serde(default)]
    variants: Vec<RemoteMediaVariant>,
}

#[derive(Debug, Deserialize)]
struct RemoteMediaVariant {
    url: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    file_ext: String,
}

#[derive(Debug, Deserialize)]
struct RemoteAutocompleteItem {
    #[serde(default)]
    label: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    category: u32,
    #[serde(default)]
    post_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DanbooruError {
    InvalidRequest(&'static str),
    TagLimit,
    NotFound,
    RateLimited,
    Unavailable,
    Protocol,
    DownloadRootMissing,
    MediaUnavailable,
    UnsafeMediaUrl,
    UnsupportedMedia,
    DownloadTooLarge,
    Integrity,
    Storage,
}

#[derive(Clone)]
pub struct DanbooruClient {
    http: Client,
    next_start: Arc<Mutex<Instant>>,
    download_index: Arc<Mutex<()>>,
    media_gate: Arc<MediaGate>,
}

struct MediaGate {
    active: Mutex<usize>,
    available: Condvar,
}

struct MediaPermit<'a> {
    gate: &'a MediaGate,
}

pub(crate) struct DanbooruMedia {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl MediaGate {
    fn acquire(&self) -> Result<MediaPermit<'_>, DanbooruError> {
        let mut active = self.active.lock().map_err(|_| DanbooruError::Unavailable)?;
        while *active >= MAX_CONCURRENT_MEDIA_REQUESTS {
            active = self
                .available
                .wait(active)
                .map_err(|_| DanbooruError::Unavailable)?;
        }
        *active += 1;
        Ok(MediaPermit { gate: self })
    }
}

impl Drop for MediaPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.gate.active.lock() {
            *active = active.saturating_sub(1);
            self.gate.available.notify_one();
        }
    }
}

impl DanbooruClient {
    pub fn new() -> Result<Self, DanbooruError> {
        let http = Client::builder()
            .user_agent(format!(
                "Atsumi/{} (https://github.com/assesse/Atsumi-Project)",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| DanbooruError::Unavailable)?;
        Ok(Self {
            http,
            next_start: Arc::new(Mutex::new(Instant::now())),
            download_index: Arc::new(Mutex::new(())),
            media_gate: Arc::new(MediaGate {
                active: Mutex::new(0),
                available: Condvar::new(),
            }),
        })
    }

    pub fn search(
        &self,
        request: DanbooruSearchRequest,
    ) -> Result<DanbooruSearchPage, DanbooruError> {
        validate_page(request.page, request.page_size)?;
        let tags = normalize_query(&request.tags)?;
        if let Ok(post_id) = tags.parse::<u64>() {
            if post_id == 0 {
                return Err(DanbooruError::InvalidRequest("postId"));
            }
            return Ok(DanbooruSearchPage {
                items: vec![self.post(post_id)?],
                page: 1,
                has_more: false,
            });
        }
        let mut url =
            Url::parse(&format!("{API_ROOT}/posts.json")).map_err(|_| DanbooruError::Protocol)?;
        url.query_pairs_mut()
            .append_pair("limit", &request.page_size.to_string())
            .append_pair("page", &request.page.to_string());
        if !tags.is_empty() {
            url.query_pairs_mut().append_pair("tags", &tags);
        }
        let posts: Vec<RemotePost> = self.get_json(url)?;
        let has_more = posts.len() == request.page_size as usize;
        Ok(DanbooruSearchPage {
            items: posts.into_iter().map(DanbooruPost::from).collect(),
            page: request.page,
            has_more,
        })
    }

    pub fn random(&self) -> Result<DanbooruPost, DanbooruError> {
        let page = self.search(DanbooruSearchRequest {
            tags: "order:random".into(),
            page: 1,
            page_size: 1,
        })?;
        page.items.into_iter().next().ok_or(DanbooruError::NotFound)
    }

    pub fn post(&self, post_id: u64) -> Result<DanbooruPost, DanbooruError> {
        if post_id == 0 {
            return Err(DanbooruError::InvalidRequest("postId"));
        }
        let url = Url::parse(&format!("{API_ROOT}/posts/{post_id}.json"))
            .map_err(|_| DanbooruError::Protocol)?;
        self.get_json::<RemotePost>(url).map(DanbooruPost::from)
    }

    pub fn related(
        &self,
        request: DanbooruRelatedRequest,
    ) -> Result<DanbooruRelatedPosts, DanbooruError> {
        if request.post_id == 0
            || request.parent_id == Some(0)
            || request.parent_id == Some(request.post_id)
        {
            return Err(DanbooruError::InvalidRequest("postId"));
        }

        let (parent, siblings) = if let Some(parent_id) = request.parent_id {
            let parent = self.post(parent_id)?;
            let siblings = self
                .relation_group(parent_id)?
                .into_iter()
                .filter(|post| post.parent_id == Some(parent_id) && post.id != request.post_id)
                .collect();
            (Some(parent), siblings)
        } else {
            (None, Vec::new())
        };
        let children = if request.has_children {
            self.relation_group(request.post_id)?
                .into_iter()
                .filter(|post| post.parent_id == Some(request.post_id))
                .collect()
        } else {
            Vec::new()
        };

        let mut pools_url =
            Url::parse(&format!("{API_ROOT}/pools.json")).map_err(|_| DanbooruError::Protocol)?;
        pools_url
            .query_pairs_mut()
            .append_pair("search[post_ids_include_any]", &request.post_id.to_string())
            .append_pair("limit", &RELATED_POOL_LIMIT.to_string());
        let remote_pools: Vec<RemotePool> = self.get_json(pools_url)?;
        let pool_windows: Vec<(RemotePool, usize, Vec<u64>)> = remote_pools
            .into_iter()
            .filter_map(|pool| {
                let current_index = pool.post_ids.iter().position(|id| *id == request.post_id)?;
                let ids = centered_window(&pool.post_ids, current_index, RELATED_POOL_WINDOW);
                Some((pool, current_index, ids))
            })
            .collect();

        let selected_ids: Vec<u64> = pool_windows
            .iter()
            .flat_map(|(_, _, ids)| ids.iter().copied())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let pool_posts = self.posts_by_id(&selected_ids)?;
        let pools = pool_windows
            .into_iter()
            .map(|(pool, current_index, ids)| DanbooruPoolRelation {
                id: pool.id,
                name: pool.name,
                category: pool.category,
                post_count: pool.post_count.max(pool.post_ids.len() as u64),
                current_index,
                items: ids
                    .iter()
                    .filter_map(|id| pool_posts.get(id).cloned())
                    .collect(),
            })
            .collect();

        Ok(DanbooruRelatedPosts {
            parent,
            siblings,
            children,
            pools,
        })
    }

    fn relation_group(&self, parent_id: u64) -> Result<Vec<DanbooruPost>, DanbooruError> {
        self.search(DanbooruSearchRequest {
            tags: format!("parent:{parent_id}"),
            page: 1,
            page_size: RELATION_POST_LIMIT,
        })
        .map(|page| page.items)
    }

    fn posts_by_id(&self, ids: &[u64]) -> Result<HashMap<u64, DanbooruPost>, DanbooruError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut url =
            Url::parse(&format!("{API_ROOT}/posts.json")).map_err(|_| DanbooruError::Protocol)?;
        let tags = format!(
            "id:{}",
            ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
        );
        url.query_pairs_mut()
            .append_pair("tags", &tags)
            .append_pair("limit", &ids.len().to_string());
        let posts: Vec<RemotePost> = self.get_json(url)?;
        Ok(posts
            .into_iter()
            .map(DanbooruPost::from)
            .map(|post| (post.id, post))
            .collect())
    }

    pub fn autocomplete(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<DanbooruAutocompleteItem>, DanbooruError> {
        let query = query.trim();
        if query.len() < 2 {
            return Ok(Vec::new());
        }
        if query.len() > 100 || !(1..=10).contains(&limit) {
            return Err(DanbooruError::InvalidRequest("query"));
        }
        let mut url = Url::parse(&format!("{API_ROOT}/autocomplete.json"))
            .map_err(|_| DanbooruError::Protocol)?;
        url.query_pairs_mut()
            .append_pair("search[query]", query)
            .append_pair("search[type]", "tag_query")
            .append_pair("limit", &limit.to_string());
        let items: Vec<RemoteAutocompleteItem> = self.get_json(url)?;
        Ok(items
            .into_iter()
            .filter(|item| !item.value.trim().is_empty())
            .map(|item| DanbooruAutocompleteItem {
                label: item.label,
                value: item.value,
                category: item.category,
                post_count: item.post_count,
            })
            .collect())
    }

    pub fn download(
        &self,
        download_root: &str,
        post_id: u64,
    ) -> Result<DanbooruDownloadRecord, DanbooruError> {
        let root = danbooru_root(download_root)?;
        fs::create_dir_all(&root).map_err(|_| DanbooruError::Storage)?;
        let post = self.post(post_id)?;
        let extension = validate_extension(&post.file_ext)?;
        if post.file_size > MAX_DOWNLOAD_BYTES {
            return Err(DanbooruError::DownloadTooLarge);
        }
        let file_url = post
            .file_url
            .as_deref()
            .ok_or(DanbooruError::MediaUnavailable)?;
        validate_media_url(file_url)?;
        let file_name = format!("{}.{}", post.id, extension);
        let target = root.join(&file_name);
        let sidecar = root.join(format!("{}.atsumi.json", post.id));

        let _index_guard = self
            .download_index
            .lock()
            .map_err(|_| DanbooruError::Storage)?;
        if target.is_file() && sidecar.is_file() {
            if let Ok(record) = read_json_file::<DanbooruDownloadRecord>(&sidecar) {
                if record.post.id == post.id && record.file_name == file_name {
                    return Ok(record);
                }
            }
        }

        self.wait_for_slot()?;
        let response = self
            .http
            .get(file_url)
            .send()
            .map_err(|_| DanbooruError::Unavailable)?;
        map_http_status(response.status(), None)?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
        {
            return Err(DanbooruError::DownloadTooLarge);
        }

        let part = root.join(format!("{}.{}.part", post.id, extension));
        let bytes = write_verified_media(
            response,
            &part,
            post.file_size,
            post.md5.as_deref().unwrap_or(""),
        )?;
        if target.exists() {
            fs::remove_file(&target).map_err(|_| DanbooruError::Storage)?;
        }
        fs::rename(&part, &target).map_err(|_| DanbooruError::Storage)?;

        let record = DanbooruDownloadRecord {
            post,
            file_name,
            downloaded_at: now_unix_ms(),
            bytes,
        };
        write_json_atomic(&sidecar, &record)?;
        let mut records = load_records(&root)?;
        records.retain(|item| item.post.id != record.post.id);
        records.push(record.clone());
        records.sort_by(|left, right| right.downloaded_at.cmp(&left.downloaded_at));
        write_json_atomic(&root.join(INDEX_FILE), &records)?;
        Ok(record)
    }

    pub fn downloads(
        &self,
        download_root: &str,
        request: DanbooruDownloadsRequest,
    ) -> Result<DanbooruDownloadsPage, DanbooruError> {
        validate_page(request.page, request.page_size)?;
        let root = danbooru_root(download_root)?;
        if !root.is_dir() {
            return Ok(DanbooruDownloadsPage {
                items: Vec::new(),
                page: 1,
                total: 0,
                total_pages: 1,
            });
        }
        let _index_guard = self
            .download_index
            .lock()
            .map_err(|_| DanbooruError::Storage)?;
        let query = request.query.trim().to_lowercase();
        let mut records = load_records(&root)?;
        records.retain(|record| root.join(&record.file_name).is_file());
        records.sort_by(|left, right| right.downloaded_at.cmp(&left.downloaded_at));
        if !query.is_empty() {
            records.retain(|record| record_matches(record, &query));
        }
        let total = records.len() as u64;
        let total_pages = total
            .div_ceil(u64::from(request.page_size))
            .max(1)
            .min(u64::from(u32::MAX)) as u32;
        let page = request.page.min(total_pages);
        let offset = usize::try_from((page - 1) * request.page_size).unwrap_or(usize::MAX);
        let mut items: Vec<DanbooruDownloadRecord> = records
            .into_iter()
            .skip(offset)
            .take(request.page_size as usize)
            .collect();
        for record in &mut items {
            record.post.proxy_display_media();
        }
        Ok(DanbooruDownloadsPage {
            items,
            page,
            total,
            total_pages,
        })
    }

    pub(crate) fn media(&self, token: &str) -> Result<DanbooruMedia, DanbooruError> {
        let remote_url = decode_media_proxy_token(token)?;
        let _permit = self.media_gate.acquire()?;
        let mut response = self
            .http
            .get(remote_url)
            .send()
            .map_err(|_| DanbooruError::Unavailable)?;
        map_http_status(response.status(), None)?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DISPLAY_MEDIA_BYTES)
        {
            return Err(DanbooruError::DownloadTooLarge);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| is_display_image_content_type(value))
            .ok_or(DanbooruError::UnsupportedMedia)?
            .to_owned();
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or_default()
                .min(MAX_DISPLAY_MEDIA_BYTES) as usize,
        );
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let count = response
                .read(&mut chunk)
                .map_err(|_| DanbooruError::Unavailable)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > MAX_DISPLAY_MEDIA_BYTES as usize {
                return Err(DanbooruError::DownloadTooLarge);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        if bytes.is_empty() {
            return Err(DanbooruError::Protocol);
        }
        Ok(DanbooruMedia {
            bytes,
            content_type,
        })
    }

    fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T, DanbooruError> {
        self.wait_for_slot()?;
        let response = self
            .http
            .get(url)
            .send()
            .map_err(|_| DanbooruError::Unavailable)?;
        let status = response.status();
        let body = response.text().map_err(|_| DanbooruError::Unavailable)?;
        map_http_status(status, Some(&body))?;
        serde_json::from_str(&body).map_err(|_| DanbooruError::Protocol)
    }

    fn wait_for_slot(&self) -> Result<(), DanbooruError> {
        let mut next = self
            .next_start
            .lock()
            .map_err(|_| DanbooruError::Unavailable)?;
        let now = Instant::now();
        if *next > now {
            thread::sleep(*next - now);
        }
        *next = Instant::now() + API_START_INTERVAL;
        Ok(())
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn danbooru_search(
    state: State<'_, AppState>,
    request: DanbooruSearchRequest,
) -> Result<ApiResult<DanbooruSearchPage>, ApiError> {
    let client = state.danbooru.clone();
    Ok(run_blocking("danbooru_search", move || client.search(request)).await)
}

#[tauri::command]
pub async fn danbooru_random(
    state: State<'_, AppState>,
) -> Result<ApiResult<DanbooruPost>, ApiError> {
    let client = state.danbooru.clone();
    Ok(run_blocking("danbooru_random", move || client.random()).await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn danbooru_related(
    state: State<'_, AppState>,
    request: DanbooruRelatedRequest,
) -> Result<ApiResult<DanbooruRelatedPosts>, ApiError> {
    let client = state.danbooru.clone();
    Ok(run_blocking("danbooru_related", move || client.related(request)).await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn danbooru_autocomplete(
    state: State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<ApiResult<Vec<DanbooruAutocompleteItem>>, ApiError> {
    let client = state.danbooru.clone();
    Ok(run_blocking("danbooru_autocomplete", move || {
        client.autocomplete(&query, limit)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn danbooru_download(
    state: State<'_, AppState>,
    post_id: u64,
) -> Result<ApiResult<DanbooruDownloadRecord>, ApiError> {
    let download_root = match state.settings_snapshot() {
        Ok(settings) => settings.download_root,
        Err(error) => return Ok(ApiResult::failure(error.into())),
    };
    let client = state.danbooru.clone();
    Ok(run_blocking("danbooru_download", move || {
        client.download(&download_root, post_id)
    })
    .await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn danbooru_downloads_list(
    state: State<'_, AppState>,
    request: DanbooruDownloadsRequest,
) -> Result<ApiResult<DanbooruDownloadsPage>, ApiError> {
    let download_root = match state.settings_snapshot() {
        Ok(settings) => settings.download_root,
        Err(error) => return Ok(ApiResult::failure(error.into())),
    };
    let client = state.danbooru.clone();
    Ok(run_blocking("danbooru_downloads_list", move || {
        client.downloads(&download_root, request)
    })
    .await)
}

async fn run_blocking<T, F>(operation_id: &'static str, operation: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, DanbooruError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(operation).await {
        Ok(Ok(value)) => ApiResult::success(value),
        Ok(Err(error)) => ApiResult::failure(api_error(error)),
        Err(error) => {
            tracing::error!(operation_id, error = %error, "Danbooru blocking task did not complete");
            ApiResult::failure(ApiError {
                code: "BACKEND_TASK_FAILED".into(),
                message: "요청을 처리하는 작업이 완료되지 않았습니다.".into(),
                retryable: true,
                action: Some(ApiAction::Retry),
                details: None,
            })
        }
    }
}

fn api_error(error: DanbooruError) -> ApiError {
    let (code, message, retryable, action) = match error {
        DanbooruError::InvalidRequest(_) => (
            "DANBOORU_VALIDATION",
            "검색 조건이나 페이지 값이 올바르지 않습니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::TagLimit => (
            "DANBOORU_TAG_LIMIT",
            "Danbooru 비로그인 검색은 제한 대상 조건을 최대 2개까지 사용할 수 있습니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::NotFound => (
            "DANBOORU_POST_NOT_FOUND",
            "해당 Danbooru post를 찾을 수 없습니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::RateLimited => (
            "DANBOORU_RATE_LIMIT",
            "Danbooru 요청 제한에 도달했습니다. 잠시 뒤 다시 시도해 주세요.",
            true,
            ApiAction::Retry,
        ),
        DanbooruError::Unavailable => (
            "DANBOORU_UNAVAILABLE",
            "Danbooru에 연결하지 못했습니다.",
            true,
            ApiAction::Reconnect,
        ),
        DanbooruError::Protocol => (
            "DANBOORU_PROTOCOL",
            "Danbooru 응답 형식이 예상과 다릅니다.",
            true,
            ApiAction::Retry,
        ),
        DanbooruError::DownloadRootMissing => (
            "DOWNLOAD_ROOT_NOT_CONFIGURED",
            "설정에서 다운로드 폴더를 먼저 선택해 주세요.",
            false,
            ApiAction::Reveal,
        ),
        DanbooruError::MediaUnavailable => (
            "DANBOORU_MEDIA_UNAVAILABLE",
            "이 post는 현재 원본 파일을 제공하지 않습니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::UnsafeMediaUrl => (
            "DANBOORU_UNSAFE_MEDIA_URL",
            "허용되지 않은 미디어 주소여서 다운로드를 중단했습니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::UnsupportedMedia => (
            "DANBOORU_MEDIA_UNSUPPORTED",
            "아직 지원하지 않는 Danbooru 파일 형식입니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::DownloadTooLarge => (
            "DANBOORU_MEDIA_TOO_LARGE",
            "파일이 512 MiB 안전 상한을 초과해 다운로드를 중단했습니다.",
            false,
            ApiAction::None,
        ),
        DanbooruError::Integrity => (
            "DANBOORU_INTEGRITY_FAILED",
            "받은 원본 파일의 크기 또는 해시가 Danbooru 정보와 일치하지 않습니다.",
            true,
            ApiAction::Retry,
        ),
        DanbooruError::Storage => (
            "DANBOORU_STORAGE_FAILED",
            "Danbooru 파일 또는 로컬 인덱스를 저장하지 못했습니다.",
            true,
            ApiAction::Retry,
        ),
    };
    ApiError {
        code: code.into(),
        message: message.into(),
        retryable,
        action: Some(action),
        details: None,
    }
}

impl From<RemotePost> for DanbooruPost {
    fn from(value: RemotePost) -> Self {
        let file_ext = value.file_ext.to_lowercase();
        let preview_url = clean_optional(value.preview_file_url);
        let large_url = if is_display_image_extension(&file_ext) {
            clean_optional(value.large_file_url).or_else(|| preview_url.clone())
        } else {
            best_static_media_variant(value.media_asset.as_ref()).or_else(|| preview_url.clone())
        };
        let mut post = Self {
            id: value.id,
            created_at: value.created_at,
            rating: value.rating,
            score: value.score,
            favorite_count: value.fav_count,
            image_width: value.image_width,
            image_height: value.image_height,
            file_ext,
            file_size: value.file_size,
            md5: clean_optional(value.md5),
            source: clean_optional(value.source),
            preview_url,
            large_url,
            file_url: clean_optional(value.file_url),
            artists: split_tags(&value.tag_string_artist),
            copyrights: split_tags(&value.tag_string_copyright),
            characters: split_tags(&value.tag_string_character),
            tags: split_tags(&value.tag_string_general),
            parent_id: value.parent_id,
            has_children: value.has_children,
        };
        post.proxy_display_media();
        post
    }
}

fn best_static_media_variant(media_asset: Option<&RemoteMediaAsset>) -> Option<String> {
    media_asset?
        .variants
        .iter()
        .filter(|variant| is_display_image_extension(&variant.file_ext.to_lowercase()))
        .filter_map(|variant| {
            let url = clean_optional(variant.url.clone())?;
            validate_media_url(&url).ok()?;
            let area = u64::from(variant.width).saturating_mul(u64::from(variant.height));
            Some((area, url))
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, url)| url)
}

impl DanbooruPost {
    fn proxy_display_media(&mut self) {
        self.preview_url = self.preview_url.take().and_then(proxy_media_url);
        self.large_url = self.large_url.take().and_then(proxy_media_url);
    }
}

fn validate_page(page: u32, page_size: u32) -> Result<(), DanbooruError> {
    if page == 0 || page > 1_000 {
        return Err(DanbooruError::InvalidRequest("page"));
    }
    if !(1..=100).contains(&page_size) {
        return Err(DanbooruError::InvalidRequest("pageSize"));
    }
    Ok(())
}

fn write_verified_media(
    mut response: reqwest::blocking::Response,
    part: &Path,
    expected_size: u64,
    expected_md5: &str,
) -> Result<u64, DanbooruError> {
    let result = (|| {
        let mut output = File::create(part).map_err(|_| DanbooruError::Storage)?;
        let mut hasher = Md5::new();
        let mut bytes = 0_u64;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let count = response
                .read(&mut chunk)
                .map_err(|_| DanbooruError::Unavailable)?;
            if count == 0 {
                break;
            }
            bytes = bytes.saturating_add(count as u64);
            if bytes > MAX_DOWNLOAD_BYTES {
                return Err(DanbooruError::DownloadTooLarge);
            }
            output
                .write_all(&chunk[..count])
                .map_err(|_| DanbooruError::Storage)?;
            hasher.update(&chunk[..count]);
        }
        output.flush().map_err(|_| DanbooruError::Storage)?;
        output.sync_all().map_err(|_| DanbooruError::Storage)?;
        let actual_md5 = format!("{:x}", hasher.finalize());
        if bytes == 0
            || (expected_size > 0 && bytes != expected_size)
            || (!expected_md5.is_empty() && !actual_md5.eq_ignore_ascii_case(expected_md5))
        {
            return Err(DanbooruError::Integrity);
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = fs::remove_file(part);
    }
    result
}

fn normalize_query(value: &str) -> Result<String, DanbooruError> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() > 500 {
        return Err(DanbooruError::InvalidRequest("tags"));
    }
    Ok(normalized)
}

fn map_http_status(status: StatusCode, body: Option<&str>) -> Result<(), DanbooruError> {
    if status.is_success() {
        return Ok(());
    }
    match status {
        StatusCode::NOT_FOUND => Err(DanbooruError::NotFound),
        StatusCode::TOO_MANY_REQUESTS => Err(DanbooruError::RateLimited),
        StatusCode::UNPROCESSABLE_ENTITY
            if body.is_some_and(|value| value.contains("TagLimitError")) =>
        {
            Err(DanbooruError::TagLimit)
        }
        status if status.is_server_error() => Err(DanbooruError::Unavailable),
        _ => Err(DanbooruError::Protocol),
    }
}

fn validate_media_url(value: &str) -> Result<Url, DanbooruError> {
    let url = Url::parse(value).map_err(|_| DanbooruError::UnsafeMediaUrl)?;
    if url.scheme() != "https"
        || url.host_str() != Some("cdn.donmai.us")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(DanbooruError::UnsafeMediaUrl);
    }
    Ok(url)
}

fn proxy_media_url(value: String) -> Option<String> {
    if value.starts_with(MEDIA_PROXY_HTTP_ROOT) || value.starts_with(MEDIA_PROXY_SCHEME_ROOT) {
        return Some(value);
    }
    validate_media_url(&value).ok()?;
    let token = URL_SAFE_NO_PAD.encode(value.as_bytes());
    #[cfg(target_os = "windows")]
    let root = MEDIA_PROXY_HTTP_ROOT;
    #[cfg(not(target_os = "windows"))]
    let root = MEDIA_PROXY_SCHEME_ROOT;
    Some(format!("{root}{token}"))
}

fn decode_media_proxy_token(token: &str) -> Result<Url, DanbooruError> {
    if token.is_empty() || token.len() > 4_096 || token.contains('/') {
        return Err(DanbooruError::UnsafeMediaUrl);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| DanbooruError::UnsafeMediaUrl)?;
    let value = String::from_utf8(bytes).map_err(|_| DanbooruError::UnsafeMediaUrl)?;
    validate_media_url(&value)
}

fn is_display_image_extension(extension: &str) -> bool {
    matches!(extension, "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif")
}

fn is_display_image_content_type(content_type: &str) -> bool {
    matches!(
        content_type.to_ascii_lowercase().as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/avif"
    )
}

fn validate_extension(value: &str) -> Result<String, DanbooruError> {
    let extension = value.trim().to_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "webm" | "mp4" | "zip" => {
            Ok(if extension == "jpeg" {
                "jpg".into()
            } else {
                extension
            })
        }
        _ => Err(DanbooruError::UnsupportedMedia),
    }
}

fn danbooru_root(download_root: &str) -> Result<PathBuf, DanbooruError> {
    let root = download_root.trim();
    if root.is_empty() {
        return Err(DanbooruError::DownloadRootMissing);
    }
    Ok(PathBuf::from(root).join("Danbooru"))
}

fn load_records(root: &Path) -> Result<Vec<DanbooruDownloadRecord>, DanbooruError> {
    let index = root.join(INDEX_FILE);
    if let Ok(records) = read_json_file::<Vec<DanbooruDownloadRecord>>(&index) {
        return Ok(records);
    }
    let mut records = Vec::new();
    let entries = fs::read_dir(root).map_err(|_| DanbooruError::Storage)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file()
            || !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.ends_with(".atsumi.json"))
        {
            continue;
        }
        if let Ok(record) = read_json_file::<DanbooruDownloadRecord>(&path) {
            if safe_file_name(&record.file_name) && root.join(&record.file_name).is_file() {
                records.push(record);
            }
        }
    }
    Ok(records)
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, DanbooruError> {
    let file = File::open(path).map_err(|_| DanbooruError::Storage)?;
    serde_json::from_reader(file).map_err(|_| DanbooruError::Storage)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), DanbooruError> {
    let temporary = path.with_extension("json.new");
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| DanbooruError::Storage)?;
    let mut file = File::create(&temporary).map_err(|_| DanbooruError::Storage)?;
    file.write_all(&bytes).map_err(|_| DanbooruError::Storage)?;
    file.flush().map_err(|_| DanbooruError::Storage)?;
    file.sync_all().map_err(|_| DanbooruError::Storage)?;
    drop(file);
    if path.exists() {
        fs::remove_file(path).map_err(|_| DanbooruError::Storage)?;
    }
    fs::rename(temporary, path).map_err(|_| DanbooruError::Storage)
}

fn safe_file_name(value: &str) -> bool {
    !value.is_empty() && Path::new(value).components().count().eq(&1)
}

fn record_matches(record: &DanbooruDownloadRecord, query: &str) -> bool {
    record.post.id.to_string().contains(query)
        || record
            .post
            .artists
            .iter()
            .chain(record.post.copyrights.iter())
            .chain(record.post.characters.iter())
            .chain(record.post.tags.iter())
            .any(|value| value.to_lowercase().contains(query))
}

fn split_tags(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.filter(|item| !item.trim().is_empty())
}

fn centered_window(values: &[u64], current_index: usize, limit: usize) -> Vec<u64> {
    if values.is_empty() || limit == 0 || current_index >= values.len() {
        return Vec::new();
    }
    let count = limit.min(values.len());
    let start = current_index
        .saturating_sub(count / 2)
        .min(values.len().saturating_sub(count));
    values[start..start + count].to_vec()
}

fn now_unix_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: u64) -> DanbooruPost {
        DanbooruPost {
            id,
            created_at: "2026-01-01T00:00:00Z".into(),
            rating: "g".into(),
            score: 1,
            favorite_count: 2,
            image_width: 100,
            image_height: 200,
            file_ext: "jpg".into(),
            file_size: 3,
            md5: None,
            source: None,
            preview_url: None,
            large_url: None,
            file_url: None,
            artists: vec!["sample_artist".into()],
            copyrights: vec![],
            characters: vec![],
            tags: vec!["blue_sky".into()],
            parent_id: None,
            has_children: false,
        }
    }

    #[test]
    fn media_allowlist_rejects_lookalikes_and_plain_http() {
        assert!(validate_media_url("https://cdn.donmai.us/data/a.jpg").is_ok());
        assert!(validate_media_url("http://cdn.donmai.us/data/a.jpg").is_err());
        assert!(validate_media_url("https://cdn.donmai.us.evil.example/a.jpg").is_err());
        assert!(validate_media_url("https://user@cdn.donmai.us/a.jpg").is_err());
    }

    #[test]
    fn media_proxy_round_trip_preserves_only_allowlisted_urls() {
        let remote = "https://cdn.donmai.us/180x180/aa/bb/example.jpg";
        let proxied = proxy_media_url(remote.into()).unwrap();
        let token = proxied.rsplit('/').next().unwrap();
        assert_eq!(decode_media_proxy_token(token).unwrap().as_str(), remote);
        assert_eq!(proxy_media_url(proxied.clone()), Some(proxied));
        assert!(proxy_media_url("https://cdn.donmai.us.evil.example/a.jpg".into()).is_none());
    }

    #[test]
    fn pool_preview_window_stays_centered_and_preserves_pool_order() {
        let ids: Vec<u64> = (1..=20).collect();
        assert_eq!(centered_window(&ids, 0, 9), (1..=9).collect::<Vec<_>>());
        assert_eq!(centered_window(&ids, 10, 9), (7..=15).collect::<Vec<_>>());
        assert_eq!(centered_window(&ids, 19, 9), (12..=20).collect::<Vec<_>>());
        assert!(centered_window(&ids, 20, 9).is_empty());
    }

    #[test]
    fn video_posts_use_the_largest_static_media_variant_for_display() {
        let converted = DanbooruPost::from(RemotePost {
            id: 9,
            created_at: String::new(),
            rating: "g".into(),
            score: 0,
            fav_count: 0,
            image_width: 1,
            image_height: 1,
            file_ext: "webm".into(),
            file_size: 1,
            md5: None,
            source: None,
            preview_file_url: Some("https://cdn.donmai.us/180x180/aa/bb/preview.jpg".into()),
            large_file_url: Some("https://cdn.donmai.us/original/aa/bb/video.webm".into()),
            file_url: Some("https://cdn.donmai.us/original/aa/bb/video.webm".into()),
            media_asset: Some(RemoteMediaAsset {
                variants: vec![
                    RemoteMediaVariant {
                        url: Some("https://cdn.donmai.us/180x180/aa/bb/preview.jpg".into()),
                        width: 123,
                        height: 180,
                        file_ext: "jpg".into(),
                    },
                    RemoteMediaVariant {
                        url: Some("https://cdn.donmai.us/720x720/aa/bb/poster.webp".into()),
                        width: 491,
                        height: 720,
                        file_ext: "webp".into(),
                    },
                    RemoteMediaVariant {
                        url: Some("https://cdn.donmai.us/original/aa/bb/video.webm".into()),
                        width: 720,
                        height: 1_056,
                        file_ext: "webm".into(),
                    },
                    RemoteMediaVariant {
                        url: Some(
                            "https://cdn.donmai.us.evil.example/1440x1440/poster.webp".into(),
                        ),
                        width: 982,
                        height: 1_440,
                        file_ext: "webp".into(),
                    },
                ],
            }),
            tag_string_general: String::new(),
            tag_string_artist: String::new(),
            tag_string_character: String::new(),
            tag_string_copyright: String::new(),
            parent_id: None,
            has_children: false,
        });
        let large_url = converted.large_url.as_deref().unwrap();
        assert_ne!(converted.large_url, converted.preview_url);
        let token = large_url.rsplit('/').next().unwrap();
        assert_eq!(
            decode_media_proxy_token(token).unwrap().as_str(),
            "https://cdn.donmai.us/720x720/aa/bb/poster.webp"
        );
    }

    #[test]
    fn video_posts_fall_back_to_the_small_preview_without_static_variants() {
        let converted = DanbooruPost::from(RemotePost {
            id: 10,
            created_at: String::new(),
            rating: "g".into(),
            score: 0,
            fav_count: 0,
            image_width: 1,
            image_height: 1,
            file_ext: "mp4".into(),
            file_size: 1,
            md5: None,
            source: None,
            preview_file_url: Some("https://cdn.donmai.us/180x180/aa/bb/preview.jpg".into()),
            large_file_url: Some("https://cdn.donmai.us/original/aa/bb/video.mp4".into()),
            file_url: Some("https://cdn.donmai.us/original/aa/bb/video.mp4".into()),
            media_asset: None,
            tag_string_general: String::new(),
            tag_string_artist: String::new(),
            tag_string_character: String::new(),
            tag_string_copyright: String::new(),
            parent_id: None,
            has_children: false,
        });
        assert_eq!(converted.large_url, converted.preview_url);
    }

    #[test]
    #[ignore = "opt-in live Danbooru general-rated thumbnail transport smoke"]
    fn live_general_thumbnail_round_trips_through_the_app_proxy() {
        let client = DanbooruClient::new().unwrap();
        let page = client
            .search(DanbooruSearchRequest {
                tags: "rating:g".into(),
                page: 1,
                page_size: 1,
            })
            .unwrap();
        let proxy_url = page.items[0].preview_url.as_deref().unwrap();
        let token = proxy_url.rsplit('/').next().unwrap();
        let media = client.media(token).unwrap();
        assert!(!media.bytes.is_empty());
        assert!(is_display_image_content_type(&media.content_type));
    }

    #[test]
    fn server_tag_limit_error_is_preserved() {
        assert_eq!(
            map_http_status(
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("PostQuery::TagLimitError"),
            ),
            Err(DanbooruError::TagLimit)
        );
    }

    #[test]
    fn sidecars_rebuild_the_download_index() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let media = root.join("7.jpg");
        fs::write(&media, [1, 2, 3]).unwrap();
        let record = DanbooruDownloadRecord {
            post: post(7),
            file_name: "7.jpg".into(),
            downloaded_at: "2".into(),
            bytes: 3,
        };
        write_json_atomic(&root.join("7.atsumi.json"), &record).unwrap();
        assert_eq!(load_records(root).unwrap(), vec![record]);
    }
}
