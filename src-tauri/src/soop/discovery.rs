//! Provider-owned anonymous discovery. Every advertised category was checked against
//! the official client and a nonempty service response, not just HTTP status.
use super::{bounded_get, clean_text, client, valid_id, IMAGE_LIMIT};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::Manager;

const PAGE_SIZE: usize = 20;
type ThumbnailCache = HashMap<String, (Instant, String)>;
static THUMBNAILS: OnceLock<Mutex<ThumbnailCache>> = OnceLock::new();
// Search must never lose its permit to background presence or preview requests.
static SEARCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static IMAGE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub kind: String,
    pub keyword: String,
    pub page: u32,
    pub channel_id: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub channel_id: String,
    pub channel_name: String,
    pub thumbnail_key: Option<String>,
    pub views: Option<u64>,
    pub duration: Option<String>,
    pub restricted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite_count: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub page: u32,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSection {
    pub title: String,
    pub items: Vec<SearchItem>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomePage {
    pub sections: Vec<HomeSection>,
    pub personalized: bool,
    pub notice: Option<String>,
}
static HOME_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_u64().map(|n| n.to_string()))
}
fn positive(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| *n > 0)
}
fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn number_id(value: &Value) -> Option<String> {
    text(value).filter(|v| {
        v.len() <= 20 && !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v != "0"
    })
}
// SOOP title fields contain escaped characters. Decode text once, never HTML,
// and leave unsupported/malformed entities visible rather than dropping data.
fn display_text(value: Option<&str>, limit: usize) -> Option<String> {
    let bounded = clean_text(value, limit.saturating_mul(12))?;
    let mut rest = bounded.as_str();
    let mut output = String::with_capacity(bounded.len());
    while let Some(at) = rest.find('&') {
        output.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let entity = rest.find(';').filter(|end| *end <= 12).map(|end| (&rest[..end], end));
        let decoded = entity.and_then(|(entity, end)| {
            let character = match entity {
                "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'),
                "quot" => Some('"'), "apos" => Some('\''), "nbsp" => Some(' '),
                _ => entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|digits| digits.parse::<u32>().ok()))
                    .and_then(char::from_u32),
            };
            character.map(|character| (character, end))
        });
        if let Some((character, end)) = decoded {
            output.push(character);
            rest = &rest[end + 1..];
        } else {
            output.push('&');
        }
    }
    output.push_str(rest);
    clean_text(Some(&output), limit)
}
fn clock_duration(value: &Value) -> Option<String> {
    let raw = value.as_str()?.trim();
    if raw.len() > 20 { return None; }
    let parts = raw.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit())) {
        return None;
    }
    let numbers = parts.iter().map(|part| part.parse::<u64>()).collect::<Result<Vec<_>, _>>().ok()?;
    if numbers[0] > 999 || *numbers.last()? >= 60 || (numbers.len() == 3 && numbers[1] >= 60) {
        return None;
    }
    Some(if numbers.len() == 3 {
        format!("{}:{:02}:{:02}", numbers[0], numbers[1], numbers[2])
    } else {
        format!("{}:{:02}", numbers[0], numbers[1])
    })
}
fn display_duration(row: &Value) -> Option<String> {
    clock_duration(&row["vod_duration"])
        .or_else(|| clock_duration(&row["duration"]))
        // Official preferred cards accept a Number in seconds. Numeric strings
        // are ambiguous: search vod_duration and legacy duration can be ms.
        .or_else(|| row["vod_duration"].as_u64().filter(|seconds| *seconds <= 3_599_999)
            .map(|seconds| format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)))
}
fn thumbnail_url(raw: &str) -> Option<Url> {
    if raw.len() > 2048 {
        return None;
    }
    let mut url = Url::parse(&if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.into()
    })
    .ok()?;
    // Current official VOD search still emits legacy HTTP thumbnail URLs.
    // Upgrade only the same fixed CDN paths; never allow an HTTP request.
    if url.scheme() == "http"
        && url.host_str() == Some("videoimg.sooplive.com")
        && url.path() == "/php/SnapshotLoad.php"
    {
        url.set_scheme("https").ok()?;
    }
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let allowed = match host {
        "liveimg.sooplive.com" => url
            .path()
            .strip_prefix("/m/")
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit())),
        "profile.img.sooplive.com"
        | "profile.img.sooplive.co.kr"
        | "profile.img.afreecatv.com"
        | "stimg.sooplive.com" => url.path().starts_with("/LOGO/"),
        "videoimg.sooplive.com" => url.path() == "/php/SnapshotLoad.php",
        _ => {
            host.strip_prefix("iflv")
                .and_then(|s| s.strip_suffix(".sooplive.com"))
                .is_some_and(|n| {
                    !n.is_empty() && n.len() <= 3 && n.bytes().all(|c| c.is_ascii_digit())
                })
                && url.path().starts_with("/save/")
        }
    };
    allowed.then_some(url)
}
fn remember_thumbnail(raw: Option<&str>) -> Option<String> {
    let url = thumbnail_url(raw?)?;
    let mut cache = THUMBNAILS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(1800));
    while cache.len() >= 512 {
        let key = cache
            .iter()
            .min_by_key(|(_, v)| v.0)
            .map(|(k, _)| k.clone())?;
        cache.remove(&key);
    }
    let key = uuid::Uuid::new_v4().to_string();
    cache.insert(key.clone(), (Instant::now(), url.into()));
    Some(key)
}
fn request_url(request: &SearchRequest) -> Result<Url, String> {
    if !(1..=1000).contains(&request.page)
        || !matches!(request.kind.as_str(), "all" | "live" | "video" | "channel")
    {
        return Err("검색 조건을 확인하세요.".into());
    }
    if let Some(id) = &request.channel_id {
        if request.kind != "video" || !valid_id(id) {
            return Err("채널을 확인하세요.".into());
        }
        let mut url = Url::parse(&format!("https://chapi.sooplive.com/api/{id}/vods")).unwrap();
        url.query_pairs_mut()
            .append_pair("page", &request.page.to_string())
            .append_pair("per_page", "20");
        return Ok(url);
    }
    let keyword = request.keyword.trim();
    if keyword.is_empty() || keyword.chars().count() > 160 || keyword.chars().any(char::is_control)
    {
        return Err("검색어를 160자 이내로 입력하세요.".into());
    }
    let mut url = Url::parse("https://sch.sooplive.com/api.php").unwrap();
    let (method, version, tab) = match request.kind.as_str() {
        "live" => ("liveSearch", "2.0", "live"),
        "video" => ("vodSearch", "5.0", "vod"),
        _ => ("bjSearch", "3.0", "bj"),
    };
    url.query_pairs_mut()
        .extend_pairs([
            ("m", method),
            ("v", version),
            ("l", "DF"),
            ("c", "UTF-8"),
            ("w", "webk"),
            ("isMobile", "0"),
            ("szType", "json"),
            ("szKeyword", keyword),
            ("nListCnt", "20"),
            ("szOrder", ""),
            ("location", "total_search"),
            ("isHashSearch", "0"),
            ("tab", tab),
        ])
        .append_pair("nPageNo", &request.page.to_string());
    if request.kind == "video" {
        url.query_pairs_mut().append_pair("szFileType", "ALL");
    }
    if request.kind == "live" {
        url.query_pairs_mut().append_pair("onlyParent", "1");
    }
    Ok(url)
}
fn public_tags(row: &Value) -> Vec<String> {
    let mut tags = Vec::new();
    for key in ["lang_tags", "category_tags", "hash_tags"] {
        if let Some(values) = row[key].as_array() {
            for value in values.iter().take(8) {
                if let Some(tag) = display_text(value.as_str(), 32) {
                    if !tags.contains(&tag) && tags.len() < 8 { tags.push(tag); }
                }
            }
        }
    }
    tags
}
fn parse_row(row: &Value, kind: &str, section: Option<&str>) -> Option<SearchItem> {
    let channel_id = row["user_id"].as_str().or(row["bj_id"].as_str()).filter(|id| valid_id(id))?;
    let id = match kind {
        "video" => number_id(&row["title_no"]),
        "live" => number_id(&row["broad_no"]),
        "channel" => Some(channel_id.into()),
        _ => None,
    }?;
    let channel_name = display_text(row["user_nick"].as_str().or(row["bj_nick"].as_str()), 120)
        .unwrap_or_else(|| channel_id.into());
    let title = display_text(row["title_name"].as_str().or(row["title"].as_str())
        .or(row["broad_title"].as_str()).or(row["station_title"].as_str()), 300)
        .unwrap_or_else(|| channel_name.clone());
    // An obsolete clip URL must not prevent a later, valid official snapshot.
    let thumbnail_key = ["thumbnail_path", "thumbnail", "broad_img", "broad_thumb", "thumb", "station_logo", "webp_path", "mobile_thumbnail_path"]
        .iter().find_map(|key| remember_thumbnail(row[*key].as_str()))
        .or_else(|| remember_thumbnail(row["ucc"]["thumb"].as_str()));
    Some(SearchItem {
        id, kind: kind.into(), title, channel_id: channel_id.into(), channel_name, thumbnail_key,
        views: count(&row["view_cnt"]).or_else(|| count(&row["total_view_cnt"]))
            .or_else(|| count(&row["count"]["vod_read_cnt"])),
        duration: display_duration(row),
        restricted: row["is_password"] == "Y" || row["ppv"] == true || row["ppv_yn"] == "Y"
            || positive(&row["subscription_only"]).is_some() || positive(&row["fan_flag"]).is_some()
            || positive(&row["subs_flag"]).is_some(),
        section: section.map(str::to_string), tags: public_tags(row),
        favorite_count: count(&row["favorite_cnt"]).or_else(|| count(&row["fan_count"])),
    })
}
fn parse_page(value: &Value, request: &SearchRequest) -> Result<SearchPage, String> {
    let creator = request.channel_id.is_some();
    if !creator && text(&value["RESULT"]).as_deref() != Some("1") {
        return Err("SOOP 검색 요청이 제한됐습니다. 잠시 후 다시 시도하세요.".into());
    }
    let key = if creator {
        "data"
    } else if request.kind == "live" {
        "REAL_BROAD"
    } else {
        "DATA"
    };
    let list = value[key]
        .as_array()
        .ok_or("SOOP 검색 응답 형식이 변경됐습니다.")?;
    let items = list.iter().take(PAGE_SIZE).filter_map(|row| parse_row(row, &request.kind, None)).collect();
    let has_more = if creator {
        value["links"]["next"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    } else {
        value["HAS_MORE_LIST"].as_bool().unwrap_or_else(|| {
            positive(&value["TOTAL_CNT"])
                .is_some_and(|n| n > u64::from(request.page) * PAGE_SIZE as u64)
        })
    };
    Ok(SearchPage {
        items,
        page: request.page,
        has_more,
        warnings: Vec::new(),
    })
}
fn profile_theme_url(request: &SearchRequest) -> Result<Url, String> {
    request_url(request)?;
    let mut url = Url::parse("https://sch.sooplive.com/api.php").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("l", "DF"), ("m", "profileTheme"), ("v", "6.0"), ("w", "webk"),
        ("pttype", "all"), ("isMobile", "0"), ("location", "total_search"), ("tab", "total"),
        ("d", request.keyword.trim()), ("order", "current_sum_viewer"), ("orderBy", "desc"),
    ]);
    Ok(url)
}
fn catch_url(request: &SearchRequest) -> Result<Url, String> {
    let mut url = request_url(&SearchRequest { kind: "video".into(), ..request.clone() })?;
    let pairs = url.query_pairs().filter(|(key, _)| key != "szFileType")
        .map(|(key, value)| (key.into_owned(), value.into_owned())).collect::<Vec<_>>();
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(pairs).append_pair("szFileType", "CATCH");
    Ok(url)
}
fn read_search(url: Url) -> Result<Value, String> {
    let bytes = bounded_get(&client()?, url.as_str(), 4 * 1024 * 1024)?;
    serde_json::from_slice(&bytes).map_err(|_| "SOOP 검색 응답을 읽지 못했습니다.".into())
}
fn parse_profile_theme(value: &Value) -> Result<Vec<SearchItem>, String> {
    if text(&value["RESULT"]).as_deref() != Some("1") {
        return Err("관련 스트리머 응답을 불러오지 못했습니다.".into());
    }
    let mut items = Vec::new();
    let mut schema = false;
    for key in ["THEME_BJ", "RECOMMEND_BJ", "RELATED_BJ", "PROFILE"] {
        let Some(rows) = value[key].as_array() else {
            if !value[key].is_null() { return Err("관련 스트리머 응답 형식이 변경됐습니다.".into()); }
            continue;
        };
        schema = true;
        if rows.is_empty() { continue; }
        let mut seen = std::collections::HashSet::new();
        items.extend(rows.iter().take(PAGE_SIZE).filter_map(|row| parse_row(row, "channel", Some("streamers")))
            .filter(|item| seen.insert(item.channel_id.clone())));
        if items.is_empty() { return Err("관련 스트리머 정보를 확인하지 못했습니다.".into()); }
        break;
    }
    if !schema { return Err("관련 스트리머 응답 형식이 변경됐습니다.".into()); }
    let rows = value["RECOMMEND_CONTENTS"].as_array().ok_or("관련 콘텐츠 응답 형식이 변경됐습니다.")?;
    let mut seen = std::collections::HashSet::new();
    items.extend(rows.iter().take(PAGE_SIZE).filter_map(|row| {
        let kind = match row["recommend_type"].as_str()? { "VOD" => "video", "LIVE" => "live", "BJ" => "channel", _ => return None };
        parse_row(row, kind, Some("related"))
    }).filter(|item| seen.insert(format!("{}:{}", item.kind, item.id))));
    Ok(items)
}
fn parse_catch(value: &Value, request: &SearchRequest) -> Result<SearchPage, String> {
    let mut page = parse_page(value, &SearchRequest { kind: "video".into(), ..request.clone() })?;
    if value["DATA"].as_array().is_some_and(|rows| rows.iter().take(PAGE_SIZE).any(|row| row["file_type"] != "CATCH")) {
        return Err("Catch 검색 응답 형식이 변경됐습니다.".into());
    }
    for item in &mut page.items { item.section = Some("catch".into()); }
    Ok(page)
}
fn merge_part(page: &mut SearchPage, label: &str, result: Result<SearchPage, String>) {
    match result {
        Ok(part) => { page.items.extend(part.items); page.has_more |= part.has_more; }
        Err(error) => page.warnings.push(format!("{label}: {error}")),
    }
}
fn search(request: SearchRequest) -> Result<SearchPage, String> {
    if request.kind == "all" {
        request_url(&request)?;
        let mut page = SearchPage {
            items: Vec::new(),
            page: request.page,
            has_more: false,
            warnings: Vec::new(),
        };
        if request.page == 1 {
            let result = profile_theme_url(&request).and_then(read_search).and_then(|value| parse_profile_theme(&value))
                .map(|items| SearchPage { items, page: 1, has_more: false, warnings: Vec::new() });
            merge_part(&mut page, "관련 스트리머·콘텐츠", result);
        }
        let live = search(SearchRequest { kind: "live".into(), ..request.clone() }).map(|mut part| {
            for item in &mut part.items { item.section = Some("live".into()); }
            part
        });
        merge_part(&mut page, "LIVE", live);
        let catches = catch_url(&request).and_then(read_search).and_then(|value| parse_catch(&value, &request));
        merge_part(&mut page, "Catch", catches);
        if page.warnings.len() == if request.page == 1 { 3 } else { 2 } {
            return Err("SOOP 통합검색을 불러오지 못했습니다. 다시 시도하세요.".into());
        }
        return Ok(page);
    }
    let value = read_search(request_url(&request)?)?;
    parse_page(&value, &request)
}
fn home_rows(rows: &[Value], kind: &str) -> Result<Vec<SearchItem>, String> {
    let key = if kind == "live" { "REAL_BROAD" } else { "DATA" };
    let request = SearchRequest {
        kind: kind.into(),
        keyword: String::new(),
        page: 1,
        channel_id: None,
    };
    let mut value = serde_json::json!({"RESULT":1});
    value[key] = Value::Array(rows.to_vec());
    Ok(parse_page(&value, &request)?.items)
}
fn preferred_page(value: &Value) -> Result<HomePage, String> {
    let mut sections = Vec::new();
    for (key, kind, title) in [
        ("live", "live", "추천 LIVE"),
        ("video", "video", "관심 VOD"),
    ] {
        let rows = value[key]
            .as_array()
            .ok_or("추천 응답 형식이 변경됐습니다.")?;
        if rows.len() > 40 {
            return Err("추천 응답이 너무 큽니다.".into());
        }
        let items = home_rows(rows, kind)?;
        if !items.is_empty() {
            sections.push(HomeSection {
                title: title.into(),
                items,
            });
        }
    }
    Ok(HomePage {
        personalized: !sections.is_empty(),
        sections,
        notice: None,
    })
}
fn public_home(notice: Option<String>) -> Result<HomePage, String> {
    let client = client()?;
    let bytes = bounded_get(
        &client,
        "https://static.file.sooplive.com/pc/ko_KR/main_popular_list.js",
        4 * 1024 * 1024,
    )?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "SOOP 메인 응답을 읽지 못했습니다.")?;
    let rows = value["DATA"]
        .as_array()
        .filter(|_| text(&value["RESULT"]).as_deref() == Some("1"))
        .ok_or("SOOP 메인 응답 형식이 변경됐습니다.")?;
    let mut sections = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for section in rows.iter().take(30) {
        let code = section["SECTION_CODE"].as_str().unwrap_or_default();
        let (kind, title) = if code == "POP_LIVE_01" {
            ("live", "인기 LIVE")
        } else if code.starts_with("POP_LIVE") {
            ("live", "지금 방송 중")
        } else if code.starts_with("POP_CATCH") {
            ("video", "인기 Catch")
        } else {
            continue;
        };
        let Some(rows) = section["DATA"].as_array() else {
            continue;
        };
        let items = home_rows(rows, kind)?
            .into_iter()
            .filter(|item| seen.insert(format!("{}:{}", item.kind, item.id)))
            .collect::<Vec<_>>();
        if !items.is_empty() {
            sections.push(HomeSection {
                title: title.into(),
                items,
            });
        }
        if sections.len() >= 5 {
            break;
        }
    }
    if sections.is_empty() {
        return Err("SOOP 메인 콘텐츠를 불러오지 못했습니다.".into());
    }
    Ok(HomePage {
        sections,
        personalized: false,
        notice,
    })
}
#[tauri::command]
pub async fn soop_home(window: tauri::Webview) -> Result<HomePage, String> {
    trusted(&window)?;
    let _slot = HOME_SLOTS
        .try_acquire()
        .map_err(|_| "메인을 불러오는 중입니다. 잠시 후 다시 시도하세요.")?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let preferred =
            super::discovery_session::preferred(&app).and_then(|value| preferred_page(&value));
        match preferred {
            Ok(page) if page.personalized => Ok(page),
            Ok(_) => public_home(None),
            Err(error) => public_home(Some(error)),
        }
    })
    .await
    .map_err(|_| "SOOP 메인을 불러오지 못했습니다.".to_string())?
}
fn trusted(window: &tauri::Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| "앱 창을 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("Atsumi 기본 창에서 검색하세요.".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn soop_search(
    window: tauri::Webview,
    request: SearchRequest,
) -> Result<SearchPage, String> {
    trusted(&window)?;
    request_url(&request)?;
    let _slot = SEARCH_SLOTS
        .try_acquire()
        .map_err(|_| "다른 검색을 처리 중입니다. 잠시 후 다시 시도하세요.")?;
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(move || search(request))
        .await
        .map_err(|_| "검색을 완료하지 못했습니다.".to_string())?;
    tracing::info!(
        diag_operation = "soop_search",
        success = result.is_ok(),
        elapsed_ms = started.elapsed().as_millis() as u64
    );
    result
}
#[tauri::command]
pub async fn soop_thumbnail(
    window: tauri::Webview,
    key: String,
    video_id: Option<String>,
) -> Result<tauri::ipc::Response, String> {
    trusted(&window)?;
    if key.len() > 40
        || video_id
            .as_ref()
            .is_some_and(|id| number_id(&Value::String(id.clone())).is_none())
    {
        return Err("미리보기 식별자가 올바르지 않습니다.".into());
    }
    let cached = THUMBNAILS.get().and_then(|cache| {
        cache
            .lock()
            .ok()?
            .get(&key)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(1800))
            .map(|(_, url)| url.clone())
    });
    let _slot = IMAGE_SLOTS
        .try_acquire()
        .map_err(|_| "다른 미리보기를 준비 중입니다.")?;
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let url = match cached {
            Some(url) => url,
            None => video_thumbnail_url(
                video_id
                    .as_deref()
                    .ok_or("미리보기가 만료됐습니다. 목록을 다시 불러오세요.")?,
            )?,
        };
        let bytes = bounded_get(&client()?, &url, IMAGE_LIMIT * 4)?;
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "이미지 형식을 읽지 못했습니다.")?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| "미리보기를 읽지 못했습니다.")?
            .thumbnail(480, 270);
        let mut output = Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Jpeg)
            .map_err(|_| "미리보기를 만들지 못했습니다.")?;
        Ok::<_, String>(output.into_inner())
    })
    .await
    .map_err(|_| "미리보기를 준비하지 못했습니다.")??;
    Ok(tauri::ipc::Response::new(bytes))
}
fn video_thumbnail_url(id: &str) -> Result<String, String> {
    if number_id(&Value::String(id.into())).is_none() {
        return Err("영상 번호를 확인하세요.".into());
    }
    let response = client()?
        .post("https://api.m.sooplive.com/station/video/a/view")
        .header(
            reqwest::header::REFERER,
            format!("https://vod.sooplive.com/player/{id}"),
        )
        .form(&[("nTitleNo", id), ("nApiLevel", "10")])
        .send()
        .map_err(|_| "영상 미리보기를 확인하지 못했습니다.")?;
    if !response.status().is_success() {
        return Err("영상 미리보기 요청이 제한됐습니다.".into());
    }
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "영상 정보를 읽지 못했습니다.")?;
    if bytes.len() > 1024 * 1024 {
        return Err("영상 정보가 너무 큽니다.".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "영상 정보를 읽지 못했습니다.")?;
    thumbnail_url(
        value["data"]["thumb"]
            .as_str()
            .ok_or("영상 미리보기가 없습니다.")?,
    )
    .map(Into::into)
    .ok_or("영상 미리보기 주소를 확인하지 못했습니다.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(kind: &str) -> SearchRequest {
        SearchRequest {
            kind: kind.into(),
            keyword: "음악".into(),
            page: 1,
            channel_id: None,
        }
    }
    #[test]
    fn native_queries_are_bounded_and_provider_owned() {
        assert!(request_url(&request("video"))
            .unwrap()
            .as_str()
            .contains("vodSearch"));
        let mut r = request("live");
        r.page = 0;
        assert!(request_url(&r).is_err());
        r.page = 1;
        r.channel_id = Some("../evil".into());
        assert!(request_url(&r).is_err());
    }
    #[test]
    fn successful_http_shell_is_not_an_empty_result() {
        assert!(parse_page(&serde_json::json!({"RESULT":-1}), &request("live")).is_err());
        assert!(parse_page(&serde_json::json!({"RESULT":1}), &request("video")).is_err());
    }
    #[test]
    fn uppercase_response_maps_ids_and_auth() {
        let p=parse_page(&serde_json::json!({"RESULT":1,"HAS_MORE_LIST":true,"DATA":[{"title_no":"123","user_id":"creator","user_nick":"이름","title":"영상","ppv":true}]}),&request("video")).unwrap();
        assert_eq!(p.items[0].id, "123");
        assert!(p.items[0].restricted);
        assert!(p.has_more);
    }
    #[test]
    fn preferred_feed_only_maps_valid_public_content_and_keeps_zero() {
        let page = preferred_page(&serde_json::json!({"live":[{"user_id":"creator","broad_no":"123","broad_title":"방송","total_view_cnt":0}],"video":[{"bj_id":"creator","title_no":"456","title_name":"영상","subscription_only":"1"}]})).unwrap();
        assert!(page.personalized);
        assert_eq!(page.sections.len(), 2);
        assert_eq!(page.sections[0].items[0].views, Some(0));
        assert!(page.sections[1].items[0].restricted);
        assert!(preferred_page(&serde_json::json!({"live":[],"video":[]}))
            .unwrap()
            .sections
            .is_empty());
        assert!(preferred_page(&serde_json::json!({"live":[],"video":"invalid"})).is_err());
    }
    #[test]
    fn preferred_cards_keep_official_titles_thumbnails_and_formatted_duration() {
        let page = preferred_page(&serde_json::json!({"live":[{"user_id":"creator","broad_no":"123","broad_title":"&lt;방송&gt; &amp; 채팅","user_nick":"이름"}],"video":[{"user_id":"creator","user_nick":"채널 이름","title_no":"209244227","title":"실제 영상 제목","thumbnail":"https://videoimg.sooplive.com/php/SnapshotLoad.php?rowKey=public","duration":"11109467","vod_duration":"3:05:09","ppv":true}]})).unwrap();
        assert_eq!(page.sections[0].items[0].title, "<방송> & 채팅");
        let video = &page.sections[1].items[0];
        assert_eq!(video.id, "209244227");
        assert_eq!(video.title, "실제 영상 제목");
        assert_eq!(video.channel_name, "채널 이름");
        assert_eq!(video.duration.as_deref(), Some("3:05:09"));
        assert!(video.thumbnail_key.is_some());
        assert!(video.restricted);
    }
    #[test]
    fn duration_units_are_not_guessed_from_numeric_strings() {
        assert_eq!(display_duration(&serde_json::json!({"duration":"0:28","vod_duration":"28676"})).as_deref(), Some("0:28"));
        assert_eq!(display_duration(&serde_json::json!({"vod_duration":9})).as_deref(), Some("0:00:09"));
        assert_eq!(display_duration(&serde_json::json!({"vod_duration":"1:2:3"})).as_deref(), Some("1:02:03"));
        for row in [serde_json::json!({"duration":"11109467"}), serde_json::json!({"duration":11109467}), serde_json::json!({"duration":"1:99"}), serde_json::json!({"duration":"-1:30"}), serde_json::json!({"duration":"1:60:00"}), serde_json::json!({"vod_duration":"28676"})] {
            assert!(display_duration(&row).is_none());
        }
    }
    #[test]
    fn escaped_titles_stay_bounded_plain_text_and_decode_only_once() {
        assert_eq!(display_text(Some("&lt;img onerror=alert(1)&gt; &#x1F600; &#34; &apos; &nbsp;"), 300).as_deref(), Some("<img onerror=alert(1)> 😀 \" '  "));
        assert_eq!(display_text(Some("&amp;lt; &unknown; &#xD800; &#0;Z"), 300).as_deref(), Some("&lt; &unknown; &#xD800; Z"));
        assert_eq!(display_text(Some("&#65;&#66;&#67;"), 2).as_deref(), Some("AB"));
    }
    #[test]
    fn integrated_search_has_a_valid_bounded_request_and_no_channel_mix() {
        assert!(request_url(&request("all")).is_ok());
        let mut value = request("all");
        value.channel_id = Some("creator".into());
        assert!(request_url(&value).is_err());
    }
    #[test]
    fn integrated_sources_use_official_ranking_and_catch_only_paging() {
        let url = profile_theme_url(&request("all")).unwrap();
        let pairs = url.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(pairs.get("m").unwrap(), "profileTheme");
        assert_eq!(pairs.get("d").unwrap(), "음악");
        let url = catch_url(&SearchRequest { page: 2, ..request("all") }).unwrap();
        let pairs = url.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(pairs.get("tab").unwrap(), "vod");
        assert_eq!(pairs.get("szFileType").unwrap(), "CATCH");
        assert_eq!(pairs.get("nPageNo").unwrap(), "2");
    }
    #[test]
    fn official_streamer_order_dedup_and_related_types_are_not_nickname_search() {
        let value = serde_json::json!({"RESULT":1,"THEME_BJ":[
            {"user_id":"sikhye1004","user_nick":"거대별","favorite_cnt":"22894"},
            {"user_id":"bboringirl","user_nick":"뽀린걸"},
            {"user_id":"mygomiee","user_nick":"마이곰이"}, {"user_id":"mygomiee"}],
            "RECOMMEND_BJ":[{"user_id":"nickname_match"}],"RECOMMEND_CONTENTS":[
                {"recommend_type":"VOD","user_id":"mygomiee","title_no":"123"},
                {"recommend_type":"LIVE","user_id":"mygomiee","broad_no":"456"},
                {"recommend_type":"BJ","user_id":"bboringirl"},
                {"recommend_type":"POST","user_id":"hidden"},
                {"recommend_type":"VOD","user_id":"../invalid","title_no":"999"}]});
        let items = parse_profile_theme(&value).unwrap();
        assert_eq!(items.len(), 6);
        assert_eq!(items[..3].iter().map(|item| item.channel_id.as_str()).collect::<Vec<_>>(), ["sikhye1004", "bboringirl", "mygomiee"]);
        assert_eq!(items[0].favorite_count, Some(22894));
        assert!(items[..3].iter().all(|item| item.section.as_deref() == Some("streamers")));
        assert_eq!(items[3..].iter().map(|item| item.kind.as_str()).collect::<Vec<_>>(), ["video", "live", "channel"]);
        assert!(items[3..].iter().all(|item| item.section.as_deref() == Some("related")));
    }
    #[test]
    fn exact_profile_fallback_and_invalid_theme_are_distinct_from_empty() {
        let items = parse_profile_theme(&serde_json::json!({"RESULT":1,"THEME_BJ":[],"RECOMMEND_BJ":[],"PROFILE":[{"user_id":"townboy","fan_count":12}],"RECOMMEND_CONTENTS":[]})).unwrap();
        assert_eq!(items[0].id, "townboy");
        assert_eq!(items[0].favorite_count, Some(12));
        assert!(parse_profile_theme(&serde_json::json!({"RESULT":1,"THEME_BJ":[],"RECOMMEND_CONTENTS":[]})).unwrap().is_empty());
        for value in [serde_json::json!({"RESULT":1}), serde_json::json!({"RESULT":1,"THEME_BJ":"invalid","RECOMMEND_CONTENTS":[]}), serde_json::json!({"RESULT":1,"THEME_BJ":[{"user_id":"../invalid"}],"PROFILE":[{"user_id":"townboy"}],"RECOMMEND_CONTENTS":[]})] {
            assert!(parse_profile_theme(&value).is_err());
        }
    }
    #[test]
    fn catch_rows_remain_videos_and_use_their_own_page_signal() {
        let rows = (1..=30).map(|id| serde_json::json!({"title_no":id,"user_id":"creator","file_type":"CATCH"})).collect::<Vec<_>>();
        let page = parse_catch(&serde_json::json!({"RESULT":1,"DATA":rows,"HAS_MORE_LIST":true}), &request("all")).unwrap();
        assert_eq!(page.items.len(), PAGE_SIZE);
        assert!(page.has_more);
        assert!(page.items.iter().all(|item| item.kind == "video" && item.section.as_deref() == Some("catch")));
        assert!(parse_catch(&serde_json::json!({"RESULT":1,"DATA":[{"title_no":1,"user_id":"creator","file_type":"REVIEW"}]}), &request("all")).is_err());
    }
    #[test]
    fn official_tags_zero_viewers_and_valid_snapshot_fallback_are_preserved() {
        let item = parse_row(&serde_json::json!({"user_id":"creator","broad_no":123,"total_view_cnt":0,
            "lang_tags":["한국어"],"category_tags":["버추얼"],"hash_tags":["버추얼","태그",{},7],
            "thumbnail_path":"http://iflv14.sooplive.com/clip/invalid.jpg",
            "webp_path":"https://videoimg.sooplive.com/php/SnapshotLoad.php?rowKey=public"}), "live", None).unwrap();
        assert_eq!(item.views, Some(0));
        assert_eq!(item.tags, ["한국어", "버추얼", "태그"]);
        assert!(item.thumbnail_key.is_some());
        let mut page = SearchPage { items: vec![item], page: 1, has_more: false, warnings: vec![] };
        merge_part(&mut page, "Catch", Err("실패".into()));
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.warnings, ["Catch: 실패"]);
    }
    #[test]
    #[ignore = "explicit anonymous official four-section comparison and page2"]
    fn public_official_four_sections() {
        let first = search(SearchRequest { keyword: "곰이".into(), ..request("all") }).unwrap();
        assert!(first.warnings.is_empty(), "{:?}", first.warnings);
        let streamers = first.items.iter().filter(|item| item.section.as_deref() == Some("streamers")).map(|item| item.channel_id.as_str()).collect::<Vec<_>>();
        assert_eq!(streamers, ["sikhye1004", "bboringirl", "mygomiee"]);
        for section in ["streamers", "live", "related", "catch"] {
            let items = first.items.iter().filter(|item| item.section.as_deref() == Some(section)).collect::<Vec<_>>();
            assert!(!items.is_empty(), "{section}");
            println!("SOOP {section}: {}, first IDs {:?}", items.len(), items.iter().take(3).map(|item| &item.id).collect::<Vec<_>>());
        }
        let second = search(SearchRequest { keyword: "곰이".into(), page: 2, ..request("all") }).unwrap();
        assert!(second.items.iter().all(|item| matches!(item.section.as_deref(), Some("live" | "catch"))));
        let first_catch = first.items.iter().find(|item| item.section.as_deref() == Some("catch")).unwrap();
        let next_catch = second.items.iter().find(|item| item.section.as_deref() == Some("catch")).unwrap();
        assert_ne!(first_catch.id, next_catch.id);
    }
    #[test]
    #[ignore = "explicit anonymous official home and integrated-search response test"]
    fn public_home_and_integrated_smoke() {
        let home = public_home(None).unwrap();
        assert!(!home.sections.is_empty());
        assert!(!home.personalized);
        let all = search(request("all")).unwrap();
        for kind in ["channel", "live", "video"] {
            let count = all.items.iter().filter(|item| item.kind == kind).count();
            assert!(count > 0, "{kind}");
            println!("SOOP integrated {kind}: {count}");
        }
        println!("SOOP official home: {} sections", home.sections.len());
    }
    #[test]
    fn thumbnails_do_not_become_an_open_proxy() {
        for url in [
            "http://liveimg.sooplive.com/m/123",
            "https://liveimg.sooplive.com.evil.test/m/123",
            "https://user@liveimg.sooplive.com/m/123",
            "https://iflv14.sooplive.com/api/secret",
            "https://127.0.0.1/a.jpg",
        ] {
            assert!(thumbnail_url(url).is_none());
        }
        assert!(thumbnail_url("https://liveimg.sooplive.com/m/123").is_some());
        assert_eq!(
            thumbnail_url("http://videoimg.sooplive.com/php/SnapshotLoad.php?rowKey=public")
                .unwrap()
                .scheme(),
            "https"
        );
        assert!(thumbnail_url("http://videoimg.sooplive.com/private").is_none());
        assert!(thumbnail_url("http://user@videoimg.sooplive.com/php/SnapshotLoad.php").is_none());
    }
    #[test]
    #[ignore = "explicit anonymous discovery test"]
    fn public_discovery_smoke() {
        for kind in ["live", "video", "channel"] {
            let p = search(request(kind)).unwrap();
            assert!(!p.items.is_empty(), "{kind}");
            println!("SOOP {kind}: {} items, more={}", p.items.len(), p.has_more);
        }
    }
}
