//! Anonymous YouTube discovery and metadata. No credentials, cookies, or account writes.
pub mod original;
pub mod original_commands;
pub mod auto_record;
mod auto_record_observation;
#[path = "youtube/search.rs"]
pub(crate) mod search;
mod portrait;
pub(crate) use portrait::read as recording_portrait;
pub(crate) use portrait::read_video as current_video_portrait;
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, Url};
pub use search::{youtube_search, youtube_thumbnail, YouTubeSearchPage, YouTubeSearchRequest};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read},
    sync::OnceLock,
    time::Duration,
};
use tauri::Manager;

static METADATA_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeTarget {
    kind: String,
    id: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_seconds: Option<u64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeMetadata {
    #[serde(flatten)]
    target: YouTubeTarget,
    title: String,
    author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_live: Option<bool>,
}
#[derive(Deserialize)]
struct OEmbed {
    title: Option<String>,
    author_name: Option<String>,
    author_url: Option<String>,
}
fn ascii_id(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || v == b'_' || v == b'-')
}
fn video_target(id: &str) -> YouTubeTarget {
    YouTubeTarget {
        kind: "video".into(),
        id: id.into(),
        url: format!("https://www.youtube.com/watch?v={id}"),
        start_seconds: None,
    }
}
fn decode_path(input: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(ch) = chars.next() {
        if ch == b'%' {
            let first = (chars.next()? as char).to_digit(16)?;
            let second = (chars.next()? as char).to_digit(16)?;
            bytes.push((first * 16 + second) as u8);
        } else {
            bytes.push(ch);
        }
    }
    String::from_utf8(bytes).ok()
}
fn channel_path(path: &str) -> bool {
    if let Some(handle) = path.strip_prefix("/@") {
        return !handle.is_empty()
            && handle.chars().count() <= 100
            && handle
                .chars()
                .all(|v| v.is_alphanumeric() || matches!(v, '_' | '.' | '-'));
    }
    if let Some(id) = path.strip_prefix("/channel/") {
        return ascii_id(id, 24, 24);
    }
    path.strip_prefix("/c/")
        .or_else(|| path.strip_prefix("/user/"))
        .is_some_and(|name| {
            !name.is_empty()
                && name.len() <= 100
                && name
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'_' | b'.' | b'-'))
        })
}
fn parse_target(input: &str) -> Result<YouTubeTarget, String> {
    let input = input.trim();
    if input.len() > 2048 {
        return Err("YouTube 주소가 너무 깁니다.".into());
    }
    if ascii_id(input, 11, 11) {
        return Ok(video_target(input));
    }
    let input = if input.starts_with("https://") || input.starts_with("http://") {
        input.into()
    } else {
        format!("https://{input}")
    };
    let url = Url::parse(&input).map_err(|_| "YouTube 주소를 확인해 주세요.")?;
    let host = url.host_str().unwrap_or("");
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(
            host,
            "youtube.com"
                | "www.youtube.com"
                | "m.youtube.com"
                | "music.youtube.com"
                | "youtu.be"
                | "www.youtu.be"
                | "www.youtube-nocookie.com"
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("YouTube 주소만 열 수 있습니다.".into());
    }
    let path = decode_path(url.path()).ok_or("YouTube 주소를 확인해 주세요.")?;
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let id = if matches!(host, "youtu.be" | "www.youtu.be") {
        path.strip_prefix('/').unwrap_or("").split('/').next()
    } else if path == "/watch" {
        query.get("v").map(String::as_str)
    } else {
        path.strip_prefix("/shorts/")
            .or_else(|| path.strip_prefix("/live/"))
            .or_else(|| path.strip_prefix("/embed/"))
            .map(|v| v.split('/').next().unwrap_or(""))
    };
    if let Some(id) = id.filter(|value| ascii_id(value, 11, 11)) {
        return Ok(video_target(id));
    }
    if matches!(path.as_str(), "/playlist" | "/embed/videoseries") {
        if let Some(list) = query.get("list").filter(|v| ascii_id(v, 10, 128)) {
            return Ok(YouTubeTarget {
                kind: "playlist".into(),
                id: list.clone(),
                url: format!("https://www.youtube.com/playlist?list={list}"),
                start_seconds: None,
            });
        }
    }
    let path = path.trim_end_matches('/');
    if channel_path(path) {
        return Ok(YouTubeTarget {
            kind: "channel".into(),
            id: path[1..].into(),
            url: format!("https://www.youtube.com{path}"),
            start_seconds: None,
        });
    }
    Err("YouTube 영상·재생목록·채널 주소를 입력해 주세요.".into())
}
fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .timeout(Duration::from_secs(12))
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("Atsumi/2.2 YouTube public metadata")
                .build()
                .map_err(|_| "YouTube 연결을 준비하지 못했습니다.".into())
        })
        .as_ref()
        .map_err(Clone::clone)
}
fn bounded_get(client: &Client, url: Url, max_bytes: usize) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|_| "YouTube에 연결하지 못했습니다. 네트워크를 확인해 주세요.")?;
    if !response.status().is_success() {
        return Err(
            "영상 정보를 가져오지 못했습니다. 삭제·비공개·제한된 영상인지 확인해 주세요.".into(),
        );
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err("YouTube 응답이 허용 크기를 넘었습니다.".into());
    }
    let mut bytes = Vec::new();
    response
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "영상 정보를 읽지 못했습니다.")?;
    if bytes.len() > max_bytes {
        return Err("YouTube 응답이 허용 크기를 넘었습니다.".into());
    }
    Ok(bytes)
}
fn thumbnail(client: &Client, id: &str) -> Option<String> {
    // Never fetch a URL supplied by metadata or preferences; only this exact public CDN.
    let url = Url::parse(&format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")).ok()?;
    let bytes = bounded_get(client, url, 512 * 1024).ok()?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(640, 360);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Jpeg).ok()?;
    if output.get_ref().len() > 96 * 1024 {
        return None;
    }
    Some(format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(output.into_inner())
    ))
}
fn player_live_state(player: &serde_json::Value, id: &str) -> Option<bool> {
    let details = &player["videoDetails"];
    if details["videoId"].as_str() != Some(id)
        || player["playabilityStatus"]["status"].as_str() != Some("OK") {
        return None;
    }
    let explicit = details["isLive"].as_bool();
    let broadcast = player["microformat"]["playerMicroformatRenderer"]["liveBroadcastDetails"]["isLiveNow"].as_bool();
    if explicit.zip(broadcast).is_some_and(|(a,b)| a != b) { return None; }
    explicit.or(broadcast).or_else(|| (details["isLiveContent"] == false).then_some(false))
}
fn metadata(target: YouTubeTarget) -> Result<YouTubeMetadata, String> {
    if target.kind != "video" {
        let channel=(target.kind=="channel").then(||portrait::read_channel(&target.url)).flatten();
        return Ok(YouTubeMetadata {
            title: if let Some((title,_))=&channel{title.clone()}else if target.kind == "playlist" {
                "재생목록".into()
            } else {
                target.id.trim_start_matches('@').into()
            },
            author: String::new(),
            author_url: None,
            thumbnail: channel.and_then(|(_,image)|image),
            warning: None,
            is_live: None,
            target,
        });
    }
    let client = client()?;
    let mut endpoint = Url::parse("https://www.youtube.com/oembed")
        .map_err(|_| "YouTube 연결을 준비하지 못했습니다.")?;
    endpoint
        .query_pairs_mut()
        .append_pair("url", &target.url)
        .append_pair("format", "json");
    let info: OEmbed = serde_json::from_slice(&bounded_get(client, endpoint, 128 * 1024)?)
        .map_err(|_| "영상 정보를 확인하지 못했습니다.")?;
    let title = info
        .title
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| target.id.clone())
        .chars()
        .take(300)
        .collect();
    let author = info
        .author_name
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect();
    let author_url = info
        .author_url
        .and_then(|url| parse_target(&url).ok())
        .filter(|v| v.kind == "channel")
        .map(|v| v.url);
    let thumbnail = thumbnail(client, &target.id);
    // oEmbed does not distinguish a currently-live video from a finished video.
    // Inspect only the anonymous, bounded initial player JSON; never run its JS.
    let is_live = bounded_get(
        client,
        Url::parse(&target.url).map_err(|_| "영상 주소 오류")?,
        4 * 1024 * 1024,
    )
    .ok()
    .and_then(|bytes| {
        let html = std::str::from_utf8(&bytes).ok()?;
        let raw = search::object_after(html, "var ytInitialPlayerResponse =")
            .or_else(|| search::object_after(html, "ytInitialPlayerResponse ="))?;
        let player: serde_json::Value = serde_json::from_str(raw).ok()?;
        player_live_state(&player, &target.id)
    });
    Ok(YouTubeMetadata {
        target,
        title,
        author,
        author_url,
        thumbnail,
        warning: is_live.is_none().then(||"현재 라이브인지 확인하지 못했습니다. 다시 확인해 주세요.".into()),
        is_live,
    })
}
fn require_main(window: &tauri::Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| "앱 창을 확인하지 못했습니다.")?;
    if !crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        return Err("YouTube 기능은 Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn youtube_metadata(
    window: tauri::Webview,
    input: String,
) -> Result<YouTubeMetadata, String> {
    require_main(&window)?;
    let target = parse_target(&input)?;
    let permit = METADATA_SLOTS
        .try_acquire()
        .map_err(|_| "영상 정보를 불러오는 중입니다. 잠시 후 다시 시도해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        metadata(target)
    })
    .await
    .map_err(|_| "영상 정보를 불러오지 못했습니다.")?
}
#[tauri::command]
pub fn youtube_open_external(window: tauri::Webview, input: String) -> Result<(), String> {
    require_main(&window)?;
    let target = parse_target(&input)?;
    #[cfg(windows)]
    {
        use windows::{
            core::{w, PCWSTR},
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let url = target.url.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(url.as_ptr()),
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
        let _ = target;
        Err("외부 브라우저 열기는 Windows에서 지원합니다.".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_supported_urls_without_tracking() {
        for input in [
            "M7lc1UVf-VE",
            "https://youtu.be/M7lc1UVf-VE?si=secret",
            "https://www.youtube.com/watch?v=M7lc1UVf-VE&tracking=x",
            "https://m.youtube.com/shorts/M7lc1UVf-VE",
            "https://youtube.com/live/M7lc1UVf-VE",
        ] {
            assert_eq!(parse_target(input).unwrap(), video_target("M7lc1UVf-VE"));
        }
    }
    #[test]
    fn rejects_external_or_credential_urls() {
        for input in [
            "https://youtube.com.evil.com/watch?v=M7lc1UVf-VE",
            "https://youtube.com@evil.com/watch?v=M7lc1UVf-VE",
            "https://secret@youtube.com/watch?v=M7lc1UVf-VE",
            "javascript:alert(1)",
            "file:///C:/Users",
            "https://youtube.com:444/watch?v=M7lc1UVf-VE",
            "https://www.youtube.com/redirect?q=https://evil.com",
            "https://youtube.com/@../x",
        ] {
            assert!(parse_target(input).is_err(), "{input}");
        }
    }
    #[test]
    fn accepts_channels_and_playlists() {
        assert_eq!(
            parse_target("https://youtube.com/@GoogleDevelopers")
                .unwrap()
                .kind,
            "channel"
        );
        assert_eq!(
            parse_target("https://youtube.com/@%EC%9C%A4%ED%82%A4%ED%82%A4")
                .unwrap()
                .id,
            "@윤키키"
        );
        assert_eq!(
            parse_target("https://youtube.com/playlist?list=PL590L5WQmH8fJ54F369BLDSqIwcs-TCfs")
                .unwrap()
                .kind,
            "playlist"
        );
    }
    #[test]
    fn public_metadata_work_is_bounded() {
        let first = METADATA_SLOTS.try_acquire().unwrap();
        let second = METADATA_SLOTS.try_acquire().unwrap();
        assert!(METADATA_SLOTS.try_acquire().is_err());
        drop(first);
        drop(second);
        assert!(METADATA_SLOTS.try_acquire().is_ok());
    }
    #[test]
    fn exact_player_live_state_does_not_invent_false() {
        use serde_json::json;
        let id = "cswFQYam8Ic";
        let mut value = json!({"playabilityStatus":{"status":"OK"},"videoDetails":{"videoId":id,"isLive":true,"isLiveContent":true},"microformat":{"playerMicroformatRenderer":{"liveBroadcastDetails":{"isLiveNow":true}}}});
        assert_eq!(player_live_state(&value,id),Some(true));
        assert_eq!(player_live_state(&value,"jNQXAC9IVRw"),None);
        value["microformat"] = json!({});
        value["videoDetails"] = json!({"videoId":id,"isLiveContent":false});
        assert_eq!(player_live_state(&value,id),Some(false));
        value["videoDetails"] = json!({"videoId":id});
        assert_eq!(player_live_state(&value,id),None);
        value["videoDetails"]["isLive"] = json!("false");
        assert_eq!(player_live_state(&value,id),None);
        value["videoDetails"]["isLive"] = json!(true);
        value["playabilityStatus"]["status"] = json!("LOGIN_REQUIRED");
        assert_eq!(player_live_state(&value,id),None);
        value["playabilityStatus"]["status"] = json!("OK");
        value["microformat"] = json!({"playerMicroformatRenderer":{"liveBroadcastDetails":{"isLiveNow":false}}});
        assert_eq!(player_live_state(&value,id),None);
    }
    #[test]
    #[ignore = "explicit public metadata requests only; no playback or account"]
    fn public_metadata_distinguishes_live_from_vod() {
        let vod = metadata(video_target("jNQXAC9IVRw")).unwrap();
        assert_eq!(vod.is_live, Some(false));
        let live = metadata(video_target("cswFQYam8Ic")).unwrap();
        assert_eq!(live.is_live, Some(true));
        eprintln!("public metadata: VOD=false, preflight live=true");
    }
}
