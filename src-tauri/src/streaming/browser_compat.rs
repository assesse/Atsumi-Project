//! A playback-only compatibility identity. Never applied to login, Hitomi, or
//! the trusted app document. No extension/connector or second media request.
use super::model::StreamError;

pub(crate) const GRID_FREE_PLAYBACK: bool = true;
pub(crate) const STATUS: &str = "그리드 없이 직접 재생";

// The official player excludes Opera from its Chrome connector branch:
// player-vendor-B7YwD4RF.js, inspected 2026-10-04. The compatibility product
// token is also used by RRRF0214/chzzk-anti-grid. Keep the ACTUAL Chromium
// engine version instead of that extension's frozen Chrome 78 identity.
const COMPAT_PRODUCT: &str = "OPR/65.0.3467.48";

fn unavailable() -> StreamError {
    StreamError::new(
        "BROWSER_COMPATIBILITY_FAILED",
        "그리드 없는 재생 환경을 준비하지 못했습니다. 시청 영역을 다시 열어 주세요.",
        true,
    )
}

fn playback_identity(native: &str) -> Result<String, StreamError> {
    if native.len() > 1024
        || !native.is_ascii()
        || native.bytes().any(|b| b.is_ascii_control())
        || !native.starts_with("Mozilla/5.0 ")
        || !native.contains("AppleWebKit/")
        || !native.contains("Safari/")
    {
        return Err(unavailable());
    }
    let parts: Vec<_> = native.split_ascii_whitespace().collect();
    let versions: Vec<_> = parts
        .iter()
        .filter_map(|p| p.strip_prefix("Chrome/"))
        .collect();
    if versions.len() != 1
        || versions[0].split('.').count() != 4
        || !versions[0]
            .split('.')
            .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(unavailable());
    }
    let mut selected = parts
        .into_iter()
        .filter(|p| !p.starts_with("Edg/") && !p.starts_with("OPR/"))
        .collect::<Vec<_>>()
        .join(" ");
    selected.push(' ');
    selected.push_str(COMPAT_PRODUCT);
    Ok(selected)
}

#[cfg(windows)]
pub(crate) fn configure(view: &tauri::Webview) -> Result<(), StreamError> {
    use webview2_com::{CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2};
    use windows::core::{Interface, HSTRING, PWSTR};
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let result = (|| {
            let settings = platform
                .controller()
                .CoreWebView2()
                .and_then(|c| c.Settings())
                .and_then(|s| s.cast::<ICoreWebView2Settings2>())
                .map_err(|_| unavailable())?;
            let mut raw = PWSTR::null();
            settings.UserAgent(&mut raw).map_err(|_| unavailable())?;
            let selected = playback_identity(&CoTaskMemPWSTR::from(raw).to_string())?;
            // Native setter changes BOTH HTTP User-Agent and navigator.userAgent
            // before navigation. No late JS getter patch or reload race.
            settings
                .SetUserAgent(&HSTRING::from(&selected))
                .map_err(|_| unavailable())?;
            let mut actual = PWSTR::null();
            settings.UserAgent(&mut actual).map_err(|_| unavailable())?;
            if CoTaskMemPWSTR::from(actual).to_string() != selected {
                return Err(unavailable());
            }
            Ok(())
        })();
        // Never call back into Tauri while holding its native dispatcher lock.
        let _ = send.try_send(result);
    })
    .map_err(|_| unavailable())?;
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_else(|_| Err(unavailable()))
}

#[cfg(not(windows))]
pub(crate) fn configure(_: &tauri::Webview) -> Result<(), StreamError> {
    Err(unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    const NATIVE: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.3800.12 Safari/537.36 Edg/145.0.3800.12";
    #[test]
    fn direct_identity_preserves_engine_and_is_idempotent() {
        let identity = playback_identity(NATIVE).unwrap();
        assert!(identity.contains("Chrome/145.0.3800.12"));
        assert!(!identity.contains("Edg/"));
        assert!(identity.ends_with(COMPAT_PRODUCT));
        assert_eq!(playback_identity(&identity).unwrap(), identity);
    }
    #[test]
    fn invalid_identities_fail_without_guessing_an_engine() {
        for value in [
            "",
            "Firefox/100",
            &NATIVE.replace("145.0.3800.12", "bad"),
            &format!("{NATIVE}\r\nheader: bad"),
            &format!("{NATIVE} Chrome/1.2.3.4"),
        ] {
            assert!(playback_identity(value).is_err());
        }
    }
    #[test]
    fn both_receiver_builders_set_identity_before_first_navigation() {
        for source in [
            include_str!("browser_host.rs"),
            include_str!("browser_multiview.rs"),
        ] {
            assert!(source.contains(
                "browser_extensions_enabled(!super::super::browser_compat::GRID_FREE_PLAYBACK)"
            ));
            let prepared = source.find("browser_compat::configure(&view)?;").unwrap();
            assert!(source[prepared..].contains("view.navigate("));
        }
    }
    #[test]
    fn every_shared_profile_window_disables_extensions_but_login_keeps_native_identity() {
        for source in [
            include_str!("browser_host.rs"),
            include_str!("browser_multiview.rs"),
            include_str!("browser_auth.rs"),
            include_str!("browser_chat_popup.rs"),
            include_str!("browser_clip_popup.rs"),
            include_str!("browser_service_popup.rs"),
        ] {
            assert!(!source.contains(".browser_extensions_enabled(true)"));
            assert!(source.contains(
                "browser_extensions_enabled(!super::super::browser_compat::GRID_FREE_PLAYBACK)"
            ));
        }
        assert!(!include_str!("browser_auth.rs").contains("browser_compat::configure"));
    }
}
