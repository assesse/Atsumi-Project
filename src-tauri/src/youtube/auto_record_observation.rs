//! Public channel identity and current LIVE discovery. Absence is never end proof.
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Read,
    time::{Duration, Instant},
};

const LIMIT: usize = 4 * 1024 * 1024;
const ERROR: &str = "YouTube 채널의 방송 상태를 확인하지 못했습니다.";
const MAX_REDIRECTS: usize = 3;

fn redirect_target(
    current: &reqwest::Url,
    location: &str,
    requested: &super::YouTubeTarget,
    streams: bool,
) -> Result<reqwest::Url, String> {
    if location.len() > 2048 {
        return Err(ERROR.into());
    }
    let mut next = current.join(location).map_err(|_| ERROR)?;
    if next.scheme() != "https"
        || next.host_str() != Some("www.youtube.com")
        || next.port().is_some()
        || !next.username().is_empty()
        || next.password().is_some()
        || next.fragment().is_some()
        || next.path().contains('%')
        || next.query_pairs().count() > 1
        || next
            .query_pairs()
            .any(|(key, value)| key != "hl" || value != "ko")
    {
        return Err(ERROR.into());
    }
    let path = next.path().trim_end_matches('/');
    let path = if streams {
        path.strip_suffix("/streams").ok_or(ERROR)?
    } else {
        path
    };
    let reserved = [
        "signin", "login", "logout", "account", "accounts", "consent", "oops", "error", "watch",
        "redirect", "results", "feed", "shorts", "live", "embed", "playlist",
    ];
    let bare = path.strip_prefix('/').ok_or(ERROR)?;
    let requested_alias = requested
        .id
        .strip_prefix('@')
        .or_else(|| requested.id.strip_prefix("c/"))
        .or_else(|| requested.id.strip_prefix("user/"));
    let legacy_alias = !bare.contains('/')
        && !reserved.contains(&bare.to_ascii_lowercase().as_str())
        && requested_alias.is_some_and(|alias| alias.eq_ignore_ascii_case(bare))
        && !bare.is_empty()
        && bare.len() <= 100
        && bare
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    if !super::channel_path(path) && !legacy_alias {
        return Err(ERROR.into());
    }
    if let Some(id) = path.strip_prefix("/channel/") {
        if !channel_id(id)
            || requested
                .id
                .strip_prefix("channel/")
                .is_some_and(|expected| expected != id)
        {
            return Err(ERROR.into());
        }
    }
    if next.query().is_none() {
        next.query_pairs_mut().append_pair("hl", "ko");
    }
    Ok(next)
}

fn channel_response(
    mut url: reqwest::Url,
    target: &super::YouTubeTarget,
    streams: bool,
) -> Result<reqwest::blocking::Response, String> {
    let started = Instant::now();
    let mut visited = HashSet::new();
    for hop in 0..=MAX_REDIRECTS {
        if !visited.insert(url.as_str().to_owned()) {
            return Err(ERROR.into());
        }
        let remaining = Duration::from_secs(12)
            .checked_sub(started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or(ERROR)?;
        let response = super::client()?
            .get(url.clone())
            .header("User-Agent", "Mozilla/5.0")
            .timeout(remaining)
            .send()
            .map_err(|_| ERROR)?;
        if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if hop == MAX_REDIRECTS {
            return Err(ERROR.into());
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(ERROR)?;
        url = redirect_target(&url, location, target, streams)?;
    }
    Err(ERROR.into())
}

pub(super) fn channel_id(id: &str) -> bool {
    id.starts_with("UC") && super::ascii_id(id, 24, 24)
}
pub(super) fn video_id(id: &str) -> bool {
    super::ascii_id(id, 11, 11)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub channel_id: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub(super) struct Observation {
    pub identity: Identity,
    pub videos: Vec<String>,
    /// Only a complete, validated channel listing may be presented as offline.
    /// Even that is not sufficient evidence to stop an existing capture.
    pub complete: bool,
}

fn document(input: &str, streams: bool) -> Result<(super::YouTubeTarget, Value), String> {
    let input = if channel_id(input) {
        format!("https://www.youtube.com/channel/{input}")
    } else if input.starts_with('@') {
        format!("https://www.youtube.com/{input}")
    } else {
        input.to_owned()
    };
    let target = super::parse_target(&input)?;
    if target.kind != "channel" {
        return Err("자동 녹화할 채널 주소를 입력하세요.".into());
    }
    let suffix = if streams { "/streams" } else { "" };
    let mut url = reqwest::Url::parse(&format!("{}{suffix}", target.url)).map_err(|_| ERROR)?;
    url.query_pairs_mut().append_pair("hl", "ko");
    let response = channel_response(url, &target, streams)?;
    if response.status() != reqwest::StatusCode::OK
        || response.content_length().is_some_and(|n| n > LIMIT as u64)
    {
        return Err(ERROR.into());
    }
    let mut bytes = Vec::new();
    response
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() > LIMIT {
        return Err(ERROR.into());
    }
    let html = std::str::from_utf8(&bytes).map_err(|_| ERROR)?;
    let raw = super::search::object_after(html, "var ytInitialData =")
        .or_else(|| super::search::object_after(html, "window[\"ytInitialData\"] ="))
        .ok_or(ERROR)?;
    let data = serde_json::from_str(raw).map_err(|_| ERROR)?;
    Ok((target, data))
}

fn identity(data: &Value, target: &super::YouTubeTarget) -> Result<Identity, String> {
    if data.get("error").is_some() {
        return Err(ERROR.into());
    }
    let meta = &data["metadata"]["channelMetadataRenderer"];
    let id = meta["externalId"]
        .as_str()
        .filter(|id| channel_id(id))
        .ok_or(ERROR)?;
    let canonical = super::parse_target(meta["channelUrl"].as_str().ok_or(ERROR)?)?;
    if canonical.id != format!("channel/{id}")
        || canonical.kind != "channel"
        || target
            .id
            .strip_prefix("channel/")
            .is_some_and(|expected| expected != id)
    {
        return Err(ERROR.into());
    }
    let name = meta["title"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>();
    if name.trim().is_empty() {
        return Err(ERROR.into());
    }
    Ok(Identity {
        channel_id: id.into(),
        name,
    })
}

pub(super) fn resolve(input: &str) -> Result<Identity, String> {
    let (target, data) = document(input, false)?;
    identity(&data, &target)
}

fn observe_page(target: &super::YouTubeTarget, data: &Value) -> Result<Observation, String> {
    let identity = identity(data, target)?;
    // Never scan recommendations, an unselected tab, a consent document or an
    // unknown fallback as if it were this channel's current streams.
    let tabs = data["contents"]["twoColumnBrowseResultsRenderer"]["tabs"]
        .as_array()
        .ok_or(ERROR)?;
    let selected = tabs
        .iter()
        .filter_map(|v| v.get("tabRenderer"))
        .find(|tab| tab["selected"] == true)
        .ok_or(ERROR)?;
    let endpoint = selected["endpoint"]["commandMetadata"]["webCommandMetadata"]["url"]
        .as_str()
        .ok_or(ERROR)?;
    if !endpoint
        .split('?')
        .next()
        .unwrap_or("")
        .ends_with("/streams")
        || !selected["content"].is_object()
    {
        return Err("이 채널의 라이브 목록을 확인할 수 없습니다.".into());
    }
    let (items, continuation) = super::search::parse_page(&selected["content"]);
    let videos = items
        .iter()
        .filter(|item| item.kind == "video" && item.is_live == Some(true) && video_id(&item.id))
        .map(|item| item.id.clone())
        .take(32)
        .collect();
    Ok(Observation {
        identity,
        videos,
        complete: continuation.is_none() && items.len() < 40
            && !items.iter().any(|item| item.kind == "video" && item.is_live.is_none()),
    })
}

pub(super) fn observe(id: &str) -> Result<Observation, String> {
    if !channel_id(id) {
        return Err(ERROR.into());
    }
    let (target, data) = document(id, true)?;
    observe_page(&target, &data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const CHANNEL: &str = "UC4QobU6STFB0P71PMvOGN5A";
    #[test]
    fn official_channel_alias_redirect_is_bounded_and_keeps_endpoint_kind() {
        let target = super::super::parse_target("https://www.youtube.com/@KitkatPlayroom").unwrap();
        let current = "https://www.youtube.com/@KitkatPlayroom?hl=ko"
            .parse()
            .unwrap();
        assert_eq!(
            redirect_target(
                &current,
                "https://www.youtube.com/kitkatplayroom?hl=ko",
                &target,
                false
            )
            .unwrap()
            .as_str(),
            "https://www.youtube.com/kitkatplayroom?hl=ko"
        );
        assert_eq!(
            redirect_target(&current, "/kitkatplayroom/streams", &target, true)
                .unwrap()
                .as_str(),
            "https://www.youtube.com/kitkatplayroom/streams?hl=ko"
        );
        assert!(redirect_target(&current, "/kitkatplayroom/streams", &target, false).is_err());
        assert!(redirect_target(&current, "/kitkatplayroom", &target, true).is_err());
        assert_eq!(MAX_REDIRECTS, 3);
    }
    #[test]
    fn redirect_rejects_auth_foreign_hosts_ambiguous_query_and_unbound_legacy_alias() {
        let target = super::super::parse_target("https://www.youtube.com/@KitkatPlayroom").unwrap();
        let current = "https://www.youtube.com/@KitkatPlayroom?hl=ko"
            .parse()
            .unwrap();
        for location in [
            "https://evil.test/kitkatplayroom",
            "http://www.youtube.com/kitkatplayroom",
            "https://accounts.google.com/",
            "https://user@www.youtube.com/kitkatplayroom",
            "https://www.youtube.com:8443/kitkatplayroom",
            "/signin",
            "/logout",
            "/oops",
            "/consent",
            "/redirect?q=x",
            "/watch?v=abcdefghijk",
            "/@KitkatPlayroom?x=y",
            "/@KitkatPlayroom?hl=ko&hl=ko",
            "/@KitkatPlayroom#fragment",
            "/%40KitkatPlayroom",
            "/anotherchannel",
        ] {
            assert!(
                redirect_target(&current, location, &target, false).is_err(),
                "{location}"
            );
        }
    }
    #[test]
    fn canonical_requested_channel_cannot_be_changed_by_a_redirect() {
        let target =
            super::super::parse_target(&format!("https://www.youtube.com/channel/{CHANNEL}"))
                .unwrap();
        let current = target.url.parse().unwrap();
        assert!(redirect_target(
            &current,
            &format!("/channel/{CHANNEL}/streams"),
            &target,
            true
        )
        .is_ok());
        assert!(redirect_target(
            &current,
            "/channel/UC0000000000000000000000/streams",
            &target,
            true
        )
        .is_err());
    }
    fn fixture() -> (super::super::YouTubeTarget, Value) {
        let url = format!("https://www.youtube.com/channel/{CHANNEL}");
        (
            super::super::parse_target(&url).unwrap(),
            json!({"metadata":{"channelMetadataRenderer":{"externalId":CHANNEL,"channelUrl":url,"title":"Fixture"}},"contents":{"twoColumnBrowseResultsRenderer":{"tabs":[{"tabRenderer":{"selected":true,"endpoint":{"commandMetadata":{"webCommandMetadata":{"url":format!("/channel/{CHANNEL}/streams")}}},"content":{"richGridRenderer":{"contents":[{"videoRenderer":{"videoId":"abcdefghijk","title":{"simpleText":"live"},"badges":[{"metadataBadgeRenderer":{"style":"BADGE_STYLE_TYPE_LIVE_NOW"}}]}},{"videoRenderer":{"videoId":"bcdefghijkl","title":{"simpleText":"second"},"thumbnailOverlays":[{"thumbnailOverlayTimeStatusRenderer":{"style":"LIVE"}}]}},{"videoRenderer":{"videoId":"cdefghijklm","title":{"simpleText":"old"}}}]}}}}]}}}),
        )
    }
    #[test]
    fn multiple_live_videos_are_distinct_and_nonlive_is_not_scheduled() {
        let (target, data) = fixture();
        let result = observe_page(&target, &data).unwrap();
        assert_eq!(result.videos, vec!["abcdefghijk", "bcdefghijkl"]);
        assert!(!result.complete); // The badge-less historical-looking row is unknown.
        assert_eq!(result.identity.channel_id, CHANNEL);
    }
    #[test]
    fn wrong_channel_missing_streams_and_error_never_become_offline() {
        let (target, mut data) = fixture();
        data["metadata"]["channelMetadataRenderer"]["externalId"] =
            json!("UC0000000000000000000000");
        assert!(observe_page(&target, &data).is_err());
        let (target, mut data) = fixture();
        data["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["endpoint"]
            ["commandMetadata"]["webCommandMetadata"]["url"] = json!("/videos");
        assert!(observe_page(&target, &data).is_err());
        assert!(observe_page(&target, &json!({"error":{}})).is_err());
        assert!(!channel_id("abcdefghijk"));
        assert!(!channel_id("XX4QobU6STFB0P71PMvOGN5A"));
    }
    #[test]
    fn paginated_empty_listing_is_not_claimed_complete() {
        let (target, mut data) = fixture();
        data["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["content"] = json!({"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"opaque"}}}});
        let value = observe_page(&target, &data).unwrap();
        assert!(!value.complete);
        assert!(value.videos.is_empty());
    }
    #[test]
    fn badge_less_listing_never_claims_offline_but_empty_verified_listing_can() {
        let (target, mut data) = fixture();
        let contents = &mut data["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["content"];
        *contents = json!({"richGridRenderer":{"contents":[{"videoRenderer":{"videoId":"cswFQYam8Ic","title":{"simpleText":"LIVE"}}}]}});
        let unknown = observe_page(&target, &data).unwrap();
        assert!(unknown.videos.is_empty());
        assert!(!unknown.complete);
        data["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["content"] = json!({"richGridRenderer":{"contents":[]}});
        assert!(observe_page(&target, &data).unwrap().complete);
    }
    #[test]
    #[ignore = "Public read-only YouTube channel metadata; opt in explicitly"]
    fn actual_public_channel_live_discovery() {
        let identity = resolve("https://www.youtube.com/@KitkatPlayroom").unwrap();
        let observation = observe(&identity.channel_id).unwrap();
        assert_eq!(observation.identity.channel_id, identity.channel_id);
        assert!(!observation.videos.is_empty());
        println!(
            "verified canonical channel; current live videos={}; listing complete={}",
            observation.videos.len(),
            observation.complete
        );
    }
}
