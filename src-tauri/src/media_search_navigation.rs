//! Route only destinations already selected by the native indexed search/link
//! controllers. There is deliberately no IPC that accepts a caller-provided URL.
use serde::Serialize;
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State, Url, Webview};

#[cfg(debug_assertions)]
#[path = "media_search_navigation_probe.rs"]
pub(crate) mod verification;

const EVENT: &str = "atsumi:open-supported-link";
const ERROR: &str = "선택한 링크를 열지 못했습니다. 다시 선택해 주세요.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Destination {
    id: String,
    source: &'static str,
    kind: &'static str,
    target_id: String,
    url: String,
}

static PENDING: OnceLock<Mutex<Option<Destination>>> = OnceLock::new();
fn pending() -> &'static Mutex<Option<Destination>> {
    PENDING.get_or_init(|| Mutex::new(None))
}
static X_PENDING: OnceLock<Mutex<Option<(Destination, Instant)>>> = OnceLock::new();
fn x_pending() -> &'static Mutex<Option<(Destination, Instant)>> {
    X_PENDING.get_or_init(|| Mutex::new(None))
}

fn public_url(value: &str) -> Option<Url> {
    if value.len() > 8192
        || value.chars().any(|c| c.is_control() || c == '\\')
        || value.contains("/../")
        || value.contains("/./")
        || value.to_ascii_lowercase().contains("%2e")
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.host_str().is_none()
    {
        return None;
    }
    Some(url)
}

fn decimal(value: &str) -> bool {
    !value.starts_with('0')
        && (1..=12).contains(&value.len())
        && value.bytes().all(|c| c.is_ascii_digit())
}
fn long_decimal(value: &str, min: usize, max: usize) -> bool {
    !value.starts_with('0')
        && (min..=max).contains(&value.len())
        && value.bytes().all(|c| c.is_ascii_digit())
}
fn ascii_id(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
fn one_query(url: &Url, key: &str) -> Option<String> {
    let values = url
        .query_pairs()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    (values.len() == 1).then(|| values[0].clone())
}

fn query_id(url: &Url, key: &str) -> Option<String> {
    let ids = url
        .query_pairs()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    (ids.len() == 1 && decimal(&ids[0])).then(|| ids[0].clone())
}

fn classify(url: &Url) -> Option<(&'static str, &'static str, String, String)> {
    // Encoded separators/traversal and opaque Google redirects are never
    // interpreted as a platform destination, even if their query names one.
    let encoded = url.path().to_ascii_lowercase();
    if encoded.contains("%2f") || encoded.contains("%5c") || encoded.contains("%2e") {
        return None;
    }
    let mut parts = url
        .path()
        .trim_end_matches('/')
        .split('/')
        .skip(1)
        .collect::<Vec<_>>();
    match url.host_str()? {
        "www.pixiv.net" | "pixiv.net" => {
            if parts
                .first()
                .is_some_and(|part| part.len() == 2 && part.bytes().all(|c| c.is_ascii_lowercase()))
            {
                parts.remove(0);
            }
            let (kind, id) = match parts.as_slice() {
                ["artworks", id] if decimal(id) => ("artwork", (*id).to_owned()),
                ["users", id] if decimal(id) => ("artist", (*id).to_owned()),
                ["member_illust.php"] => ("artwork", query_id(url, "illust_id")?),
                ["member.php"] => ("artist", query_id(url, "id")?),
                _ => return None,
            };
            let segment = if kind == "artist" {
                "users"
            } else {
                "artworks"
            };
            Some((
                "pixiv",
                kind,
                id.clone(),
                format!("https://www.pixiv.net/{segment}/{id}"),
            ))
        }
        "danbooru.donmai.us" => {
            let id = match parts.as_slice() {
                ["posts", id] | ["post", "show", id] if decimal(id) => *id,
                _ => return None,
            };
            Some((
                "danbooru",
                "post",
                id.into(),
                format!("https://danbooru.donmai.us/posts/{id}"),
            ))
        }
        host if host == "pinterest.com"
            || host == "www.pinterest.com"
            || host.strip_suffix(".pinterest.com").is_some_and(|prefix| {
                prefix.len() == 2 && prefix.bytes().all(|c| c.is_ascii_lowercase())
            }) =>
        {
            let ["pin", slug] = parts.as_slice() else {
                return None;
            };
            let id = slug.rsplit_once("--").map_or(*slug, |(_, id)| id);
            if !long_decimal(id, 5, 25) {
                return None;
            }
            Some((
                "pinterest",
                "pin",
                id.into(),
                format!("https://www.pinterest.com/pin/{id}/"),
            ))
        }
        "hitomi.la" | "www.hitomi.la" => {
            let [category, slug] = parts.as_slice() else {
                return None;
            };
            if !matches!(
                *category,
                "galleries"
                    | "reader"
                    | "doujinshi"
                    | "manga"
                    | "cg"
                    | "gamecg"
                    | "anime"
                    | "imageset"
            ) {
                return None;
            }
            let stem = slug.strip_suffix(".html")?;
            let id = stem.rsplit_once('-').map_or(stem, |(_, id)| id);
            if !decimal(id) {
                return None;
            }
            Some((
                "hitomi",
                "gallery",
                id.into(),
                format!("https://hitomi.la/galleries/{id}.html"),
            ))
        }
        "youtube.com"
        | "www.youtube.com"
        | "m.youtube.com"
        | "music.youtube.com"
        | "www.youtube-nocookie.com"
        | "youtu.be"
        | "www.youtu.be" => {
            let video = match parts.as_slice() {
                [id] if matches!(url.host_str(), Some("youtu.be" | "www.youtu.be")) => {
                    Some((*id).to_owned())
                }
                ["watch"] => one_query(url, "v"),
                ["shorts" | "live" | "embed", id] if ascii_id(id, 11, 11) => Some((*id).to_owned()),
                _ => None,
            };
            if let Some(id) = video.filter(|id| ascii_id(id, 11, 11)) {
                return Some((
                    "youtube",
                    "video",
                    id.clone(),
                    format!("https://www.youtube.com/watch?v={id}"),
                ));
            }
            if matches!(parts.as_slice(), ["playlist"] | ["embed", "videoseries"]) {
                let id = one_query(url, "list")?;
                if ascii_id(&id, 10, 128) {
                    return Some((
                        "youtube",
                        "playlist",
                        id.clone(),
                        format!("https://www.youtube.com/playlist?list={id}"),
                    ));
                }
            }
            if matches!(parts.last(), Some(&"live" | &"streams")) {
                parts.pop();
            }
            let channel = match parts.as_slice() {
                ["channel", id] if ascii_id(id, 24, 24) => format!("channel/{id}"),
                ["c" | "user", id]
                    if !id.is_empty()
                        && id.len() <= 100
                        && id
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) =>
                {
                    parts.join("/")
                }
                [handle]
                    if handle.starts_with('@')
                        && handle.len() > 1
                        && handle.chars().count() <= 101
                        && handle[1..]
                            .chars()
                            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-')) =>
                {
                    (*handle).into()
                }
                _ => return None,
            };
            Some((
                "youtube",
                "channel",
                channel.clone(),
                format!("https://www.youtube.com/{channel}"),
            ))
        }
        "vod.sooplive.com" | "vod.sooplive.co.kr" | "vod.afreecatv.com" => {
            let ["player", id] = parts.as_slice() else {
                return None;
            };
            if !long_decimal(id, 1, 20) {
                return None;
            }
            Some((
                "soop",
                "video",
                (*id).into(),
                format!("https://vod.sooplive.com/player/{id}"),
            ))
        }
        host @ ("play.sooplive.com"
        | "play.sooplive.co.kr"
        | "play.afreecatv.com"
        | "ch.sooplive.com"
        | "ch.sooplive.co.kr"
        | "bj.sooplive.co.kr"
        | "bj.afreecatv.com"
        | "www.sooplive.com"
        | "www.sooplive.co.kr") => {
            let id = if host.starts_with("www.") {
                match parts.as_slice() {
                    ["station", id] | ["station", id, "board"] => *id,
                    _ => return None,
                }
            } else if host.starts_with("play.") {
                match parts.as_slice() {
                    [id] | [id, "embed"] => *id,
                    [id, broadcast] | [id, broadcast, "embed"]
                        if long_decimal(broadcast, 1, 20) =>
                    {
                        *id
                    }
                    _ => return None,
                }
            } else {
                match parts.as_slice() {
                    [id] => *id,
                    _ => return None,
                }
            };
            if !ascii_id(id, 1, 40) {
                return None;
            }
            let id = id.to_ascii_lowercase();
            Some((
                "soop",
                "channel",
                id.clone(),
                format!("https://play.sooplive.com/{id}"),
            ))
        }
        "chzzk.naver.com" => {
            let id = match parts.as_slice() {
                ["live", id] | [id] => *id,
                _ => return None,
            };
            if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let id = id.to_ascii_lowercase();
            Some((
                "chzzk",
                "channel",
                id.clone(),
                format!("https://chzzk.naver.com/live/{id}"),
            ))
        }
        "x.com" | "www.x.com" | "mobile.x.com" | "twitter.com" | "www.twitter.com"
        | "mobile.twitter.com" | "m.twitter.com" => {
            let path = url.path().trim_end_matches('/');
            if !crate::x_feed::surface::media_search_path(path) {
                return None;
            }
            Some((
                "x",
                if path.contains("/status/") {
                    "post"
                } else {
                    "profile"
                },
                path.into(),
                format!("https://x.com{path}"),
            ))
        }
        _ => None,
    }
}

/// Existing controllers must finish their account/job/index checks before this
/// call. A pending destination survives the main listener and lazy-view startup.
pub(crate) fn open_supported_or_external<R: tauri::Runtime>(
    app: &AppHandle<R>,
    value: &str,
) -> Result<(), String> {
    let url = public_url(value).ok_or(ERROR)?;
    if url.host_str() == Some("www.google.com") && matches!(url.path(), "/goto" | "/url") {
        return Err("SEARCH_RESULT_REDIRECT_UNRESOLVED".into());
    }
    let Some((source, kind, target_id, url)) = classify(&url) else {
        if matches!(
            url.host_str(),
            Some(
                "x.com"
                    | "www.x.com"
                    | "mobile.x.com"
                    | "twitter.com"
                    | "www.twitter.com"
                    | "mobile.twitter.com"
                    | "m.twitter.com"
            )
        ) {
            return Err("앱에서 지원하지 않는 X 링크입니다.".into());
        }
        return external(value);
    };
    // X and the official players attach additional native Webviews to this
    // Window. get_webview_window intentionally rejects those multi-view hosts,
    // even when the extra views are hidden by the source-search dialog.
    let main = app.get_window("main").ok_or(ERROR)?;
    let destination = Destination {
        id: uuid::Uuid::new_v4().to_string(),
        source,
        kind,
        target_id,
        url,
    };
    let id = destination.id.clone();
    *x_pending().lock().unwrap_or_else(|p| p.into_inner()) =
        (source == "x").then(|| (destination.clone(), Instant::now()));
    *pending().lock().unwrap_or_else(|p| p.into_inner()) = Some(destination.clone());
    if app.emit_to("main", EVENT, &destination).is_err() {
        let mut pending = pending().lock().unwrap_or_else(|p| p.into_inner());
        if pending.as_ref().is_some_and(|item| item.id == id) {
            *pending = None;
        }
        return Err(ERROR.into());
    }
    main.show()
        .and_then(|_| main.unminimize())
        .and_then(|_| main.set_focus())
        .map_err(|_| ERROR.into())
}

fn trusted(window: &Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| ERROR)?;
    if crate::frontend_origin::trusted_main(
        window.label(),
        &url,
        cfg!(debug_assertions),
        window.app_handle().config().build.dev_url.as_ref(),
    ) {
        Ok(())
    } else {
        Err(ERROR.into())
    }
}

#[tauri::command]
pub(crate) fn media_search_navigation_pending(
    window: Webview,
) -> Result<Option<Destination>, String> {
    trusted(&window)?;
    Ok(pending().lock().unwrap_or_else(|p| p.into_inner()).clone())
}

#[tauri::command]
pub(crate) fn media_search_navigation_ack(window: Webview, id: String) -> Result<(), String> {
    trusted(&window)?;
    let mut pending = pending().lock().unwrap_or_else(|p| p.into_inner());
    if pending.as_ref().is_some_and(|item| item.id == id) {
        *pending = None;
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn media_search_navigation_open_x(
    window: Webview,
    app: AppHandle,
    session: State<'_, crate::x_feed::XSession>,
    id: String,
    generation: u64,
) -> Result<crate::x_feed::surface::SurfaceSnapshot, String> {
    trusted(&window)?;
    let path = {
        let mut stored = x_pending().lock().unwrap_or_else(|p| p.into_inner());
        let (destination, _) = stored
            .as_ref()
            .filter(|(item, since)| item.id == id && since.elapsed() < Duration::from_secs(120))
            .ok_or(ERROR)?;
        let path = destination.target_id.clone();
        *stored = None;
        path
    };
    crate::x_feed::surface::open_media_search_target(&session, &app, generation, &path).await
}

fn external(value: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let wide = value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
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
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err("외부 브라우저를 열지 못했습니다.".into())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = value;
        Err("이 환경에서 외부 브라우저를 열 수 없습니다.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(value: &str) -> Option<(&'static str, &'static str, String, String)> {
        classify(&public_url(value)?)
    }
    #[test]
    fn exact_platform_resources_and_legacy_aliases_have_canonical_targets() {
        for (url, source, kind, id) in [
            (
                "https://www.pixiv.net/artworks/144440262",
                "pixiv",
                "artwork",
                "144440262",
            ),
            (
                "https://pixiv.net/en/users/4912/?from=profile#works",
                "pixiv",
                "artist",
                "4912",
            ),
            (
                "http://www.pixiv.net/member_illust.php?mode=medium&illust_id=123",
                "pixiv",
                "artwork",
                "123",
            ),
            (
                "https://www.pixiv.net/member.php?id=4912",
                "pixiv",
                "artist",
                "4912",
            ),
            (
                "https://danbooru.donmai.us/posts/123?tags=sky",
                "danbooru",
                "post",
                "123",
            ),
            (
                "https://danbooru.donmai.us/post/show/123",
                "danbooru",
                "post",
                "123",
            ),
            (
                "https://se.pinterest.com/pin/landscape--621004236103700020/?x=1",
                "pinterest",
                "pin",
                "621004236103700020",
            ),
            (
                "https://hitomi.la/galleries/fixture-landscape-424242.html",
                "hitomi",
                "gallery",
                "424242",
            ),
            (
                "https://hitomi.la/manga/%E9%A2%A8%E6%99%AF-424242.html",
                "hitomi",
                "gallery",
                "424242",
            ),
            (
                "https://youtu.be/abcdefghijk",
                "youtube",
                "video",
                "abcdefghijk",
            ),
            (
                "https://www.youtube.com/@example/streams",
                "youtube",
                "channel",
                "@example",
            ),
            (
                "https://www.youtube.com/playlist?list=PL1234567890",
                "youtube",
                "playlist",
                "PL1234567890",
            ),
            (
                "https://ch.sooplive.com/Example_1",
                "soop",
                "channel",
                "example_1",
            ),
            (
                "https://vod.sooplive.co.kr/player/123456",
                "soop",
                "video",
                "123456",
            ),
            (
                "https://chzzk.naver.com/live/0123456789abcdef0123456789abcdef",
                "chzzk",
                "channel",
                "0123456789abcdef0123456789abcdef",
            ),
            (
                "https://twitter.com/example/status/12345/photo/1",
                "x",
                "post",
                "/example/status/12345/photo/1",
            ),
            (
                "https://x.com/i/web/status/12345",
                "x",
                "post",
                "/i/web/status/12345",
            ),
            ("https://x.com/example", "x", "profile", "/example"),
            (
                "https://mobile.x.com/example/status/12345",
                "x",
                "post",
                "/example/status/12345",
            ),
            ("https://m.twitter.com/example", "x", "profile", "/example"),
        ] {
            let found = target(url).unwrap();
            assert_eq!((found.0, found.1, found.2.as_str()), (source, kind, id));
        }
    }
    #[test]
    fn unowned_redirects_and_unsupported_shapes_never_become_app_targets() {
        for url in [
            "https://www.google.com/goto?url=https://www.pixiv.net/users/4912",
            "https://www.pixiv.net.evil.example/artworks/123",
            "https://www.pixiv.net@evil.example/artworks/123",
            "https://evil.example@www.pixiv.net/artworks/123",
            "https://www.pixiv.net:444/artworks/123",
            "https://www.pixiv.net/artworks%2f123",
            "https://www.pixiv.net/artworks/0",
            "https://www.pixiv.net/artworks/01",
            "https://www.pixiv.net/member.php?id=123&id=456",
            "https://www.pixiv.net/users/123/following",
            "https://www.pixiv.net/users/1/../4912",
            "https://www.pixiv.net/users/%2e%2e/artworks/123",
            "https://www.pixiv.net/artworks/123/extra",
            "https://danbooru.donmai.us/posts?tags=sky",
            "javascript:alert(1)",
            "https://www.pixiv.net\\@evil.example/artworks/123",
            "https://www.pinterest.com.evil.example/pin/621004236103700020/",
            "https://pin.it/opaque",
            "https://hitomi.la/artist/example-424242.html",
            "https://youtu.be/abcdefghijk/extra",
            "https://www.youtube.com/watch?v=abcdefghijk&v=lmnopqrstuv",
            "https://play.sooplive.com/example/extra",
            "https://chzzk.naver.com/video/12345",
            "https://x.com/settings",
            "https://x.com/example/status/012345",
        ] {
            assert!(target(url).is_none(), "{url}");
        }
    }

    #[test]
    fn supported_destinations_survive_visible_and_hidden_native_children() {
        let app = tauri::test::mock_app();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        assert!(app.get_webview_window("main").is_some());
        let child = main
            .as_ref()
            .window()
            .add_child(
                tauri::WebviewBuilder::new("source-search-child", Default::default()),
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(100.0, 100.0),
            )
            .unwrap();

        for hidden in [false, true] {
            if hidden {
                child.hide().unwrap();
            }
            assert!(app.get_window("main").is_some());
            assert!(app.get_webview("main").is_some());
            assert!(app.get_webview_window("main").is_none());
            for (url, source, kind, id) in [
                (
                    "https://danbooru.donmai.us/post/show/12439739",
                    "danbooru",
                    "post",
                    "12439739",
                ),
                (
                    "https://danbooru.donmai.us/posts/12439739",
                    "danbooru",
                    "post",
                    "12439739",
                ),
                (
                    "https://www.pixiv.net/artworks/144440262",
                    "pixiv",
                    "artwork",
                    "144440262",
                ),
                (
                    "https://www.pixiv.net/users/32715112",
                    "pixiv",
                    "artist",
                    "32715112",
                ),
                (
                    "https://www.pinterest.com/pin/621004236103700020/",
                    "pinterest",
                    "pin",
                    "621004236103700020",
                ),
                (
                    "https://hitomi.la/galleries/424242.html",
                    "hitomi",
                    "gallery",
                    "424242",
                ),
                (
                    "https://youtu.be/abcdefghijk",
                    "youtube",
                    "video",
                    "abcdefghijk",
                ),
                (
                    "https://www.youtube.com/@example",
                    "youtube",
                    "channel",
                    "@example",
                ),
                (
                    "https://www.youtube.com/playlist?list=PL1234567890",
                    "youtube",
                    "playlist",
                    "PL1234567890",
                ),
                (
                    "https://vod.sooplive.com/player/123456",
                    "soop",
                    "video",
                    "123456",
                ),
                (
                    "https://play.sooplive.com/example_1",
                    "soop",
                    "channel",
                    "example_1",
                ),
                (
                    "https://chzzk.naver.com/live/0123456789abcdef0123456789abcdef",
                    "chzzk",
                    "channel",
                    "0123456789abcdef0123456789abcdef",
                ),
                (
                    "https://x.com/example/status/12345",
                    "x",
                    "post",
                    "/example/status/12345",
                ),
                ("https://x.com/example", "x", "profile", "/example"),
            ] {
                open_supported_or_external(app.handle(), url).unwrap();
                let delivered = pending()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                    .unwrap();
                assert_eq!(
                    (
                        delivered.source,
                        delivered.kind,
                        delivered.target_id.as_str()
                    ),
                    (source, kind, id)
                );
                assert_eq!(delivered.url, target(url).unwrap().3);
                assert_eq!(
                    x_pending()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .is_some(),
                    source == "x"
                );
            }
        }
        *pending().lock().unwrap_or_else(|p| p.into_inner()) = None;
        *x_pending().lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}
