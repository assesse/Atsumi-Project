use super::*;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Instant,
};
static SEARCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
static THUMBNAIL_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeSearchRequest {
    pub query: String,
    pub filter: String,
    pub sort: String,
    pub channel: Option<String>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeSearchItem {
    pub kind: String,
    pub id: String,
    pub url: String,
    pub title: String,
    pub author: String,
    pub author_url: Option<String>,
    pub duration: Option<String>,
    pub views: Option<String>,
    pub published: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_live: Option<bool>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeSearchPage {
    pub items: Vec<YouTubeSearchItem>,
    pub cursor: Option<String>,
}
struct Continuation {
    created: Instant,
    token: String,
    context: Value,
    browse: bool,
    channel: Option<(String, Option<String>)>,
}
fn continuations() -> &'static Mutex<HashMap<String, Continuation>> {
    static STORE: OnceLock<Mutex<HashMap<String, Continuation>>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

// Extract one balanced JSON object without executing any remote JavaScript.
pub(super) fn object_after<'a>(source: &'a str, marker: &str) -> Option<&'a str> {
    let offset = source.find(marker)? + marker.len();
    let tail = &source[offset..];
    let begin = tail.find('{')?;
    let bytes = tail.as_bytes();
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for i in begin..bytes.len() {
        let b = bytes[i];
        if quoted {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                quoted = false;
            }
            continue;
        }
        match b {
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&tail[begin..=i]);
                }
            }
            _ => {}
        }
    }
    None
}
fn config_context(source: &str) -> Value {
    for (offset, _) in source.match_indices("ytcfg.set(").take(32) {
        if let Some(raw) = object_after(&source[offset..], "ytcfg.set(") {
            if let Ok(config) = serde_json::from_str::<Value>(raw) {
                if config["INNERTUBE_CONTEXT"].is_object() {
                    return config["INNERTUBE_CONTEXT"].clone();
                }
            }
        }
    }
    Value::Null
}
fn text(value: &Value) -> String {
    value["simpleText"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| {
            value["runs"]
                .as_array()
                .map(|runs| runs.iter().filter_map(|r| r["text"].as_str()).collect())
                .unwrap_or_default()
        })
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect()
}
fn author_url(value: &Value) -> Option<String> {
    let raw = value["runs"].as_array()?.first()?["navigationEndpoint"]["browseEndpoint"]
        ["canonicalBaseUrl"]
        .as_str()?;
    let parsed = super::parse_target(&format!("https://www.youtube.com{raw}")).ok()?;
    (parsed.kind == "channel").then_some(parsed.url)
}
fn renderer(value: &Value, channel: bool) -> Option<YouTubeSearchItem> {
    let id = value[if channel { "channelId" } else { "videoId" }].as_str()?;
    if !super::ascii_id(
        id,
        if channel { 24 } else { 11 },
        if channel { 24 } else { 11 },
    ) {
        return None;
    }
    let byline = if value["ownerText"].is_object() {
        &value["ownerText"]
    } else {
        &value["longBylineText"]
    };
    let title = text(&value["title"]);
    if title.is_empty() {
        return None;
    }
    let is_live = value["badges"].as_array().is_some_and(|badges| {
        badges
            .iter()
            .any(|b| b["metadataBadgeRenderer"]["style"] == "BADGE_STYLE_TYPE_LIVE_NOW")
    }) || value["thumbnailOverlays"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|v| v["thumbnailOverlayTimeStatusRenderer"]["style"] == "LIVE")
    });
    Some(YouTubeSearchItem {
        kind: if channel { "channel" } else { "video" }.into(),
        id: if channel {
            format!("channel/{id}")
        } else {
            id.into()
        },
        url: if channel {
            format!("https://www.youtube.com/channel/{id}")
        } else {
            format!("https://www.youtube.com/watch?v={id}")
        },
        title,
        author: text(byline),
        author_url: author_url(byline),
        duration: Some(text(&value["lengthText"])).filter(|v| !v.is_empty()),
        views: Some(text(&value["viewCountText"])).filter(|v| !v.is_empty()),
        published: Some(text(&value["publishedTimeText"])).filter(|v| !v.is_empty()),
        // A current live may have no search badge at all. Only watch/player
        // metadata can resolve that absence; it is not proof of a VOD.
        is_live: is_live.then_some(true),
    })
}
fn lockup(value: &Value) -> Option<YouTubeSearchItem> {
    let id = value["contentId"].as_str()?;
    if value["contentType"] != "LOCKUP_CONTENT_TYPE_VIDEO" || !super::ascii_id(id, 11, 11) {
        return None;
    }
    let metadata = &value["metadata"]["lockupMetadataViewModel"];
    let title = metadata["title"]["content"]
        .as_str()?
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect::<String>();
    if title.is_empty() {
        return None;
    }
    let parts: Vec<_> = metadata["metadata"]["contentMetadataViewModel"]["metadataRows"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row["metadataParts"].as_array().into_iter().flatten())
        .collect();
    let views = parts
        .iter()
        .find(|part| part["leadingIcon"]["name"] == "PLAY_ARROW_OUTLINED")
        .and_then(|part| {
            part["accessibilityLabel"]
                .as_str()
                .or(part["text"]["content"].as_str())
        })
        .map(|v| v.chars().take(80).collect());
    let published = parts
        .iter()
        .filter_map(|part| part["text"]["content"].as_str())
        .last()
        .map(|v| v.chars().take(80).collect());
    let badges: Vec<_> = value["contentImage"]["thumbnailViewModel"]["overlays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|overlay| {
            overlay["thumbnailBottomOverlayViewModel"]["badges"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .map(|badge| &badge["thumbnailBadgeViewModel"])
        .collect();
    let is_live = badges.iter().any(|badge| {
        badge["badgeStyle"]
            .as_str()
            .is_some_and(|s| s.contains("LIVE"))
    });
    let duration = badges
        .iter()
        .filter_map(|badge| badge["text"].as_str())
        .find(|s| s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit() || b == b':'))
        .map(String::from);
    Some(YouTubeSearchItem {
        kind: "video".into(),
        id: id.into(),
        url: format!("https://www.youtube.com/watch?v={id}"),
        title,
        author: String::new(),
        author_url: None,
        duration,
        views,
        published,
        is_live: is_live.then_some(true),
    })
}
pub(super) fn parse_page(root: &Value) -> (Vec<YouTubeSearchItem>, Option<String>) {
    // Channel headers have their own "About" continuation. Only traverse the
    // result contents; otherwise its later token replaces the video-page token.
    let scope = root.get("contents").unwrap_or(root);
    let mut stack = vec![(scope, 0usize)];
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut token = None;
    let mut visited = 0;
    while let Some((value, depth)) = stack.pop() {
        visited += 1;
        if depth > 60 || visited > 120_000 {
            continue;
        }
        if let Some(obj) = value.as_object() {
            // Ad payloads are not discovery results.
            if obj
                .keys()
                .any(|key| key == "adSlotRenderer" || key == "promotedSparklesWebRenderer")
            {
                continue;
            }
            for (key, channel) in [("videoRenderer", false), ("channelRenderer", true)] {
                if let Some(r) = obj.get(key) {
                    if let Some(item) = renderer(r, channel) {
                        if items.len() < 40 && seen.insert(item.id.clone()) {
                            items.push(item);
                        }
                    }
                }
            }
            if let Some(r) = obj.get("lockupViewModel") {
                if let Some(item) = lockup(r) {
                    if items.len() < 40 && seen.insert(item.id.clone()) {
                        items.push(item);
                    }
                }
            }
            if token.is_none() {
                if let Some(item) = obj.get("continuationItemRenderer") {
                    if let Some(value) =
                        item["continuationEndpoint"]["continuationCommand"]["token"].as_str()
                    {
                        if value.len() < 16384 {
                            token = Some(value.into());
                        }
                    }
                }
            }
            for child in obj.values().rev() {
                stack.push((child, depth + 1));
            }
        } else if let Some(array) = value.as_array() {
            for child in array.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    (items, token)
}
fn read_response(response: reqwest::blocking::Response) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err("YouTube 검색에 응답이 없습니다. 잠시 후 다시 시도해 주세요.".into());
    }
    if response.content_length().is_some_and(|n| n > LIMIT as u64) {
        return Err("검색 응답이 너무 큽니다.".into());
    }
    let mut bytes = Vec::new();
    response
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "검색 응답을 읽지 못했습니다.")?;
    if bytes.len() > LIMIT {
        return Err("검색 응답이 너무 큽니다.".into());
    }
    Ok(bytes)
}
fn channel_browse_url(input:&str,live:bool)->Result<Url,String>{
    let target=super::parse_target(input)?;
    if target.kind!="channel"{return Err("채널 주소를 확인해 주세요.".into());}
    Url::parse(&format!("{}/{}",target.url.trim_end_matches('/'),if live{"streams"}else{"videos"})).map_err(|_|"채널 주소 오류".into())
}
pub(crate) fn search(request: YouTubeSearchRequest) -> Result<YouTubeSearchPage, String> {
    let client = super::client()?;
    let (root, context, browse, saved_channel) = if let Some(cursor) = request.cursor {
        let continuation = {
            let mut saved = continuations().lock().map_err(|_| "검색 상태 오류")?;
            saved.retain(|_, v| v.created.elapsed() < Duration::from_secs(900));
            let c = saved
                .get(&cursor)
                .ok_or("검색이 만료됐습니다. 다시 검색해 주세요.")?;
            (
                c.token.clone(),
                c.context.clone(),
                c.browse,
                c.channel.clone(),
            )
        };
        let url = if continuation.2 {
            "https://www.youtube.com/youtubei/v1/browse?prettyPrint=false"
        } else {
            "https://www.youtube.com/youtubei/v1/search?prettyPrint=false"
        };
        let body = serde_json::to_vec(
            &serde_json::json!({"context":continuation.1,"continuation":continuation.0}),
        )
        .map_err(|_| "검색 상태 오류")?;
        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .map_err(|_| "YouTube에 연결하지 못했습니다.")?;
        let root: Value = serde_json::from_slice(&read_response(response)?)
            .map_err(|_| "검색 결과를 읽지 못했습니다.")?;
        (root, continuation.1, continuation.2, continuation.3)
    } else {
        if request.query.chars().count() > 200
            || !matches!(
                request.filter.as_str(),
                "all" | "video" | "channel" | "live"
            )
            || !matches!(request.sort.as_str(), "relevance" | "popular")
        {
            return Err("검색 조건을 확인해 주세요.".into());
        }
        let mut url = if let Some(channel) = request.channel {
            channel_browse_url(&channel,request.filter=="live")?
        } else {
            if request.query.trim().is_empty() {
                return Err("검색어를 입력해 주세요.".into());
            }
            if request.sort == "popular" && request.filter != "all" {
                return Err("인기순은 전체 검색에서 사용할 수 있습니다.".into());
            }
            let mut url = Url::parse("https://www.youtube.com/results").unwrap();
            url.query_pairs_mut()
                .append_pair("search_query", request.query.trim());
            let filter = if request.sort == "popular" {
                "CAM%3D"
            } else {
                match request.filter.as_str() {
                    "video" => "EgIQAQ%3D%3D",
                    "channel" => "EgIQAg%3D%3D",
                    "live" => "EgJAAQ%3D%3D",
                    _ => "",
                }
            };
            if !filter.is_empty() {
                url.query_pairs_mut().append_pair("sp", filter);
            }
            url
        };
        let browse = url.path() != "/results";
        url.query_pairs_mut().append_pair("hl", "ko");
        let response = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .map_err(|_| "YouTube에 연결하지 못했습니다.")?;
        let bytes = read_response(response)?;
        let html = std::str::from_utf8(&bytes).map_err(|_| "검색 결과를 읽지 못했습니다.")?;
        let raw = object_after(html, "var ytInitialData =")
            .or_else(|| object_after(html, "window[\"ytInitialData\"] ="))
            .ok_or("검색 형식이 변경됐거나 접근이 제한됐습니다.")?;
        let root: Value = serde_json::from_str(raw).map_err(|_| "검색 결과를 읽지 못했습니다.")?;
        (root, config_context(html), browse, None)
    };
    let (mut items, token) = parse_page(&root);
    let channel = if browse {
        let metadata = &root["metadata"]["channelMetadataRenderer"];
        let name = metadata["title"]
            .as_str()
            .map(|name| name.chars().take(160).collect::<String>())
            .or_else(|| saved_channel.as_ref().map(|v| v.0.clone()))
            .unwrap_or_default();
        let url = metadata["channelUrl"]
            .as_str()
            .and_then(|url| super::parse_target(url).ok())
            .filter(|target| target.kind == "channel")
            .map(|target| target.url)
            .or_else(|| saved_channel.as_ref().and_then(|v| v.1.clone()));
        for item in &mut items {
            if item.author.is_empty() {
                item.author = name.clone();
                item.author_url = url.clone();
            }
        }
        Some((name, url))
    } else {
        None
    };
    if root.get("error").is_some() {
        return Err("YouTube 검색 요청을 처리하지 못했습니다.".into());
    }
    let cursor = if let Some(token) = token.filter(|_| context.is_object()) {
        let key = uuid::Uuid::new_v4().to_string();
        let mut saved = continuations().lock().map_err(|_| "검색 상태 오류")?;
        saved.retain(|_, v| v.created.elapsed() < Duration::from_secs(900));
        if saved.len() >= 128 {
            if let Some(oldest) = saved
                .iter()
                .min_by_key(|(_, v)| v.created)
                .map(|(k, _)| k.clone())
            {
                saved.remove(&oldest);
            }
        }
        saved.insert(
            key.clone(),
            Continuation {
                created: Instant::now(),
                token,
                context,
                browse,
                channel,
            },
        );
        Some(key)
    } else {
        None
    };
    Ok(YouTubeSearchPage { items, cursor })
}
#[tauri::command]
pub async fn youtube_search(
    window: tauri::Webview,
    request: YouTubeSearchRequest,
) -> Result<YouTubeSearchPage, String> {
    super::require_main(&window)?;
    let permit = SEARCH_SLOTS
        .try_acquire()
        .map_err(|_| "검색 중입니다. 잠시 후 다시 시도해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        search(request)
    })
    .await
    .map_err(|_| "검색에 실패했습니다.")?
}
#[derive(Debug, serde::Serialize)]
pub struct ThumbnailError {
    code: ThumbnailErrorCode,
    message: String,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ThumbnailErrorCode { Busy, Unavailable }
impl ThumbnailError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self { code: ThumbnailErrorCode::Unavailable, message: message.into() }
    }
    fn busy() -> Self {
        Self { code: ThumbnailErrorCode::Busy, message: "미리보기를 준비 중입니다.".into() }
    }
}
#[tauri::command]
pub async fn youtube_thumbnail(
    window: tauri::Webview,
    id: String,
) -> Result<Option<String>, ThumbnailError> {
    super::require_main(&window).map_err(ThumbnailError::unavailable)?;
    let channel=super::parse_target(&format!("https://www.youtube.com/{id}")).ok().filter(|target|target.kind=="channel");
    if !super::ascii_id(&id, 11, 11)&&channel.is_none() {
        return Err(ThumbnailError::unavailable("영상 식별자를 확인해 주세요."));
    }
    let permit = THUMBNAIL_SLOTS
        .try_acquire()
        .map_err(|_| ThumbnailError::busy())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if let Some(channel)=channel{
            Ok(super::portrait::read_channel(&channel.url).and_then(|(_,image)|image))
        }else{super::client().map(|client| super::thumbnail(client, &id))}
    })
    .await
    .map_err(|_| ThumbnailError::unavailable("미리보기를 읽지 못했습니다."))?
    .map_err(ThumbnailError::unavailable)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnail_busy_is_a_closed_transport_code_not_localized_message_matching() {
        assert_eq!(serde_json::to_value(ThumbnailError::busy()).unwrap(), serde_json::json!({"code":"busy", "message":"미리보기를 준비 중입니다."}));
        assert_eq!(serde_json::to_value(ThumbnailError::unavailable("미리보기를 준비 중입니다.")).unwrap()["code"], "unavailable");
    }
    #[test]
    fn balanced_json_handles_escaped_braces() {
        assert_eq!(
            object_after("prefix x={\"a\":\"}\\\"{\",\"b\":{}}; more", "x="),
            Some("{\"a\":\"}\\\"{\",\"b\":{}}")
        );
    }
    #[test]
    fn renderer_ids_are_validated_and_deduplicated() {
        let v = serde_json::json!({"items":[{"videoRenderer":{"videoId":"M7lc1UVf-VE","title":{"runs":[{"text":"Sample"}]}}},{"videoRenderer":{"videoId":"../private","title":{"simpleText":"bad"}}},{"videoRenderer":{"videoId":"M7lc1UVf-VE","title":{"simpleText":"duplicate"}}}]});
        assert_eq!(parse_page(&v).0.len(), 1);
    }
    #[test]
    fn discovery_limit_is_bounded() {
        let items=(0..200).map(|i|serde_json::json!({"videoRenderer":{"videoId":format!("{i:011}"),"title":{"simpleText":"test"}}})).collect::<Vec<_>>();
        assert_eq!(parse_page(&serde_json::json!({"items":items})).0.len(), 40);
    }
    #[test]
    fn configuration_skips_unrelated_earlier_blocks() {
        let html="ytcfg.set({\"OTHER\":true});ytcfg.set({\"INNERTUBE_CONTEXT\":{\"client\":{\"clientName\":\"WEB\"}}});";
        assert_eq!(config_context(html)["client"]["clientName"], "WEB");
    }
    #[test]
    fn channel_continuation_ignores_about_header() {
        let root = serde_json::json!({"contents":{"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"videos"}}}},"header":{"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"about"}}}}});
        assert_eq!(parse_page(&root).1.as_deref(), Some("videos"));
    }
    #[test]
    fn lockup_video_cards_are_supported() {
        let root = serde_json::json!({"lockupViewModel":{"contentType":"LOCKUP_CONTENT_TYPE_VIDEO","contentId":"M7lc1UVf-VE","metadata":{"lockupMetadataViewModel":{"title":{"content":"Test video"}}}}});
        let (items, _) = parse_page(&root);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Test video");
        assert_eq!(items[0].is_live, None);
    }
    #[test]
    fn absent_live_badge_is_unknown_even_with_live_title_or_duration() {
        for extra in [serde_json::json!({}), serde_json::json!({"lengthText":{"simpleText":"1:23"}})] {
            let mut item = serde_json::json!({"videoId":"cswFQYam8Ic","title":{"simpleText":"Kitkat Playroom LIVE"}});
            item.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let row = renderer(&item, false).unwrap();
            assert_eq!(row.is_live, None);
            assert!(serde_json::to_value(row).unwrap().get("isLive").is_none());
        }
        let item = serde_json::json!({"videoId":"cswFQYam8Ic","title":{"simpleText":"sample"},"badges":[{"metadataBadgeRenderer":{"style":"BADGE_STYLE_TYPE_LIVE_NOW"}}]});
        assert_eq!(renderer(&item, false).unwrap().is_live, Some(true));
    }
    #[test]
    fn channel_live_resolution_uses_streams_not_uploaded_videos(){
        assert_eq!(channel_browse_url("https://www.youtube.com/@KittenAcademy",true).unwrap().path(),"/@KittenAcademy/streams");
        assert_eq!(channel_browse_url("https://www.youtube.com/@KittenAcademy",false).unwrap().path(),"/@KittenAcademy/videos");
        assert!(channel_browse_url("https://www.youtube.com/watch?v=M7lc1UVf-VE",true).is_err());
        assert!(channel_browse_url("https://youtube.com.evil.invalid/@channel",true).is_err());
    }
    #[test]
    #[ignore = "explicit public channel-live result; no account or media download"]
    fn public_channel_live_resolution(){
        let page=search(YouTubeSearchRequest{query:String::new(),filter:"live".into(),sort:"relevance".into(),channel:Some("https://www.youtube.com/@KittenAcademy".into()),cursor:None}).unwrap();
        assert!(page.items.iter().any(|item|item.is_live == Some(true)));
        eprintln!("public channel streams: {} results, {} live",page.items.len(),page.items.iter().filter(|item|item.is_live == Some(true)).count());
    }
    #[test]
    #[ignore = "explicit anonymous network discovery; no account or media download"]
    fn public_discovery_and_continuation() {
        for (filter, sort, channel) in [
            ("all", "relevance", None),
            ("all", "popular", None),
            ("channel", "relevance", None),
            (
                "all",
                "relevance",
                Some("https://www.youtube.com/@NASA".to_string()),
            ),
        ] {
            let request = YouTubeSearchRequest {
                query: "NASA Artemis".into(),
                filter: filter.into(),
                sort: sort.into(),
                channel,
                cursor: None,
            };
            let first = search(request.clone()).unwrap();
            assert!(!first.items.is_empty());
            eprintln!(
                "anonymous YouTube discovery filter={} sort={} channel={} items={} continuation={}",
                filter,
                sort,
                request.channel.is_some(),
                first.items.len(),
                first.cursor.is_some()
            );
            if filter == "all" && sort == "relevance" {
                let mut next = request;
                next.cursor = first.cursor;
                assert!(next.cursor.is_some());
                let page = search(next).unwrap();
                assert!(!page.items.is_empty());
                assert!(page
                    .items
                    .iter()
                    .any(|v| !first.items.iter().any(|p| p.id == v.id)));
            }
        }
    }
}
