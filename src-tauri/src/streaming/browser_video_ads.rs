//! A small, app-owned CHZZK live video-ad filter, independent of extensions.
//! This is not a general AdGuard/List-KR engine. See docs/CHZZK_VIDEO_ADS.md.

use super::model::StreamError;

const MAX_URL_BYTES: usize = 8 * 1024;
const LIVE_SCHEDULE: &str = "LIVE_CHZZK_NDP_SCH";
// A locally generated empty response, readable only by the exact CHZZK origin.
// Never reflect Origin or expose any original server response or credentials.
const EMPTY_RESPONSE_HEADERS: &str = "Content-Length: 0\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: https://chzzk.naver.com\r\nAccess-Control-Allow-Credentials: true";

// WebView2 glob filters only reduce callback traffic. The parsed matcher below
// is the actual boundary, including method, main document, host and full path.
#[cfg(windows)]
const REQUEST_FILTERS: [&str; 3] = [
    "https://api.chzzk.naver.com/service/v1/lives/*/ads/current*",
    "https://api.chzzk.naver.com/ad-polling/v1/lives/*/ad*",
    "https://nam.veta.naver.com/gfp/v1/vas/vas*",
];

fn parsed_https(input: &str) -> Option<tauri::Url> {
    if input.len() > MAX_URL_BYTES || input.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    let url = tauri::Url::parse(input).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

fn live_document(input: &str) -> bool {
    let Some(url) = parsed_https(input) else {
        return false;
    };
    let Some(channel) = url.path().strip_prefix("/live/") else {
        return false;
    };
    url.host_str() == Some("chzzk.naver.com")
        && channel.len() == 32
        && channel.bytes().all(|b| b.is_ascii_hexdigit())
        && !url.query_pairs().any(|(name, _)| name == "chat")
}

fn live_id(input: &str) -> bool {
    // Live IDs in these two ad-state APIs are decimal, not channel UUIDs.
    !input.is_empty() && input.len() <= 20 && input.bytes().all(|b| b.is_ascii_digit())
}

fn should_block(document: &str, request: &str, method: &str, is_fetch_or_xhr: bool) -> bool {
    if method != "GET" || !is_fetch_or_xhr || !live_document(document) {
        return false;
    }
    let Some(url) = parsed_https(request) else {
        return false;
    };
    match url.host_str() {
        Some("api.chzzk.naver.com") => {
            let path = url.path();
            let id = path
                .strip_prefix("/service/v1/lives/")
                .and_then(|p| p.strip_suffix("/ads/current"))
                .or_else(|| {
                    path.strip_prefix("/ad-polling/v1/lives/")
                        .and_then(|p| p.strip_suffix("/ad"))
                });
            id.is_some_and(live_id)
        }
        Some("nam.veta.naver.com") if url.path() == "/gfp/v1/vas/vas" => {
            // This endpoint is shared with other NAVER products. Require the
            // one publicly identified CHZZK live video schedule, exactly once.
            let mut schedules = url.query_pairs().filter(|(name, _)| name == "vsi");
            schedules
                .next()
                .is_some_and(|(_, value)| value == LIVE_SCHEDULE)
                && schedules.next().is_none()
        }
        _ => false,
    }
}

fn enabled(value: Option<&str>) -> bool {
    value != Some("0")
}

/// Attach only to app-owned video views, before navigating to CHZZK. An
/// installation failure leaves all requests untouched and is reported to the
/// caller, which keeps viewing available. No headers, bodies or URLs are logged.
#[cfg(windows)]
pub(super) fn install(view: &tauri::Webview) -> Result<(), StreamError> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
        },
        WebResourceRequestedEventHandler,
    };
    use windows::{
        core::{HSTRING, PWSTR},
        Win32::System::Com::IStream,
    };

    if !enabled(
        std::env::var("ATSUMI_CHZZK_VIDEO_AD_FILTER")
            .ok()
            .as_deref(),
    ) {
        return Ok(());
    }
    let failure = || {
        StreamError::new(
            "VIDEO_AD_FILTER_UNAVAILABLE",
            "영상 광고 필터를 초기화하지 못했습니다. 공식 시청은 계속 사용할 수 있습니다.",
            false,
        )
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let active = Arc::new(AtomicBool::new(false));
    let callback_active = active.clone();
    view.with_webview(move |platform| unsafe {
        let outcome = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let environment = platform.environment();
            let mut token = 0;
            core.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
                    if !callback_active.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let (Some(sender), Some(args)) = (sender, args) else {
                        return Ok(());
                    };
                    let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH;
                    if args.ResourceContext(&mut context).is_err()
                        || !matches!(
                            context,
                            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH
                                | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST
                        )
                    {
                        return Ok(());
                    }
                    let Ok(request) = args.Request() else {
                        return Ok(());
                    };
                    let mut document = PWSTR::null();
                    let mut uri = PWSTR::null();
                    let mut method = PWSTR::null();
                    let source_result = sender.Source(&mut document);
                    let uri_result = request.Uri(&mut uri);
                    let method_result = request.Method(&mut method);
                    let document = CoTaskMemPWSTR::from(document).to_string();
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    let method = CoTaskMemPWSTR::from(method).to_string();
                    if source_result.is_err()
                        || uri_result.is_err()
                        || method_result.is_err()
                        || !should_block(&document, &uri, &method, true)
                    {
                        return Ok(());
                    }
                    if let Ok(response) = environment.CreateWebResourceResponse(
                        None::<&IStream>,
                        204,
                        &HSTRING::from("No Content"),
                        &HSTRING::from(EMPTY_RESPONSE_HEADERS),
                    ) {
                        let _ = args.SetResponse(&response);
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            for pattern in REQUEST_FILTERS {
                for context in [
                    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
                    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
                ] {
                    core.AddWebResourceRequestedFilter(&HSTRING::from(pattern), context)?;
                }
            }
            Ok(())
        })();
        let _ = tx.try_send(outcome.is_ok());
    })
    .map_err(|_| failure())?;
    if !rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or(false)
    {
        return Err(failure());
    }
    // Only the caller enables the callback, after successful installation. A
    // delayed dispatcher callback cannot enable it after the caller times out.
    active.store(true, Ordering::Release);
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn install(_: &tauri::Webview) -> Result<(), StreamError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CURRENT: &str = "https://api.chzzk.naver.com/service/v1/lives/123456/ads/current";
    const POLLING: &str = "https://api.chzzk.naver.com/ad-polling/v1/lives/123456/ad";
    const SCHEDULE: &str =
        "https://nam.veta.naver.com/gfp/v1/vas/vas?vsi=LIVE_CHZZK_NDP_SCH&ct=web";

    #[test]
    fn only_three_identified_video_ad_routes_match() {
        for request in [CURRENT, POLLING, SCHEDULE] {
            assert!(should_block(PAGE, request, "GET", true), "{request}");
        }
        assert!(should_block(
            PAGE,
            &format!("{CURRENT}?cache=1"),
            "GET",
            true
        ));
    }

    #[test]
    fn unknown_and_shared_naver_routes_remain_available() {
        for request in [
            "https://nam.veta.naver.com/gfp/v1?u=display_banner",
            "https://nam.veta.naver.com/vas",
            "https://nam.veta.naver.com/call",
            "https://nam.veta.naver.com/nac/1",
            "https://gfp.veta.naver.com/gfp/v1",
            "https://siape.veta.naver.com/openrtb/nbackimp",
            "https://api.chzzk.naver.com/service/v1/seoraksan",
            "https://api.chzzk.naver.com/service/v1/channels/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/live-detail",
            "https://api.chzzk.naver.com/service/v1/lives/123456/ads",
            "https://ssl.pstatic.net/static/nng/sdk/refresh-detector-obfuscated.js",
            "https://livecloud.pstatic.net/live/master.m3u8",
            "https://livecloud.pstatic.net/live/video.m4s",
            "https://nid.naver.com/nidlogin.login",
            "https://apis.naver.com/nng_main/v1/chats/access-token",
            "https://lcs.naver.com/m",
            "https://127.0.0.1:12345/native-grid",
        ] {
            assert!(!should_block(PAGE, request, "GET", true), "{request}");
        }
    }

    #[test]
    fn schedule_must_identify_one_exact_chzzk_live_video_unit() {
        let base = "https://nam.veta.naver.com/gfp/v1/vas/vas";
        for suffix in [
            "",
            "?vsi=",
            "?vsi=LIVE_OTHER_NDP_SCH",
            "?vsi=VOD_CHZZK_NDP_SCH",
            "?vsi=LIVE_CHZZK_NDP_SCH_EXTRA",
            "?u=LIVE_CHZZK_NDP_SCH",
            "?vsi=LIVE_CHZZK_NDP_SCH&vsi=display",
            "?vsi=display&vsi=LIVE_CHZZK_NDP_SCH",
            "/extra?vsi=LIVE_CHZZK_NDP_SCH",
        ] {
            assert!(!should_block(PAGE, &format!("{base}{suffix}"), "GET", true));
        }
    }

    #[test]
    fn account_chat_and_other_documents_are_never_filtered() {
        for document in [
            "about:blank",
            "https://chzzk.naver.com/",
            "https://nid.naver.com/",
            "https://chzzk.naver.com/video/123456",
            "https://chzzk.naver.com/live/invalid",
            "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?chat=1",
            "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/chat",
            "https://chzzk.naver.com.evil.test/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(!should_block(document, CURRENT, "GET", true), "{document}");
        }
    }

    #[test]
    fn request_methods_and_media_contexts_remain_untouched() {
        for method in ["POST", "OPTIONS", "HEAD", "DELETE", "get", ""] {
            assert!(!should_block(PAGE, CURRENT, method, true));
        }
        assert!(!should_block(PAGE, CURRENT, "GET", false));
    }

    #[test]
    fn host_path_and_input_boundaries_fail_open() {
        for request in [
            "http://api.chzzk.naver.com/service/v1/lives/123456/ads/current",
            "https://api.chzzk.naver.com.evil.test/service/v1/lives/123456/ads/current",
            "https://user@api.chzzk.naver.com/service/v1/lives/123456/ads/current",
            "https://api.chzzk.naver.com:8443/service/v1/lives/123456/ads/current",
            "https://api.chzzk.naver.com/service/v1/lives/123456/ads/current/extra",
            "https://api.chzzk.naver.com/service/v2/lives/123456/ads/current",
            "https://api.chzzk.naver.com/service/v1/lives/123/other/ads/current",
            "https://api.chzzk.naver.com/service/v1/lives/abc/ads/current",
            "https://api.chzzk.naver.com/service/v1/lives/%31%32%33/ads/current",
            "https://api.chzzk.naver.com/service/v1/lives//ads/current",
            "https://api.chzzk.naver.com/service/v1/lives/123456789012345678901/ads/current",
        ] {
            assert!(!should_block(PAGE, request, "GET", true), "{request}");
        }
        assert!(!should_block(PAGE, &format!("{CURRENT}#ad"), "GET", true));
        assert!(!should_block(PAGE, &format!("{CURRENT}\n"), "GET", true));
        assert!(!should_block(
            PAGE,
            &format!("{CURRENT}?{}", "a".repeat(MAX_URL_BYTES)),
            "GET",
            true
        ));
    }

    #[test]
    fn troubleshooting_switch_is_explicit() {
        assert!(!enabled(Some("0")));
        assert!(enabled(None));
        assert!(enabled(Some("1")));
    }

    #[test]
    fn empty_response_cors_allows_only_the_exact_chzzk_origin() {
        let origins: Vec<_> = EMPTY_RESPONSE_HEADERS
            .lines()
            .filter_map(|line| line.strip_prefix("Access-Control-Allow-Origin: "))
            .collect();
        assert_eq!(origins, ["https://chzzk.naver.com"]);
        assert!(!EMPTY_RESPONSE_HEADERS.contains('*'));
        assert!(!EMPTY_RESPONSE_HEADERS
            .to_ascii_lowercase()
            .contains("set-cookie"));
    }
}
