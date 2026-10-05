//! Auxiliary official-site windows: no capture bridge, recording reservation,
//! local navigation, or app IPC. Keep the opener so payments can report back.
use super::*;
use tauri::{
    webview::{NewWindowFeatures, NewWindowResponse},
    WebviewUrl, WebviewWindowBuilder,
};

pub(super) fn blank() -> WebviewUrl {
    // Tauri leaves about:blank un-navigated. Navigating the requested URL here
    // makes WebView2's SetNewWindow fail (or loses opener/POST state).
    WebviewUrl::External("about:blank".parse().expect("static blank URL"))
}

#[derive(Default)]
pub(super) struct InitialNavigation(AtomicBool);

impl InitialNavigation {
    pub(super) fn allows(&self, url: &tauri::Url, allowed: bool) -> bool {
        if url.as_str() == "about:blank" {
            return !self.0.load(Ordering::Acquire);
        }
        if allowed {
            self.0.store(true, Ordering::Release);
        }
        allowed
    }
}

fn official_service(url: &tauri::Url) -> bool {
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.as_str().len() > 16_384
    {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    matches!(host, "chzzk.naver.com" | "nid.naver.com" | "bill.naver.com")
        || host == "pay.naver.com"
        || host.ends_with(".pay.naver.com")
        || host == "npay.com"
        || host.ends_with(".npay.com")
}

pub(super) fn open(
    host: &OfficialBrowser,
    app: &AppHandle,
    url: tauri::Url,
    features: NewWindowFeatures,
    channel: &str,
    depth: u8,
) -> NewWindowResponse<tauri::Wry> {
    if depth >= 4
        || host.inner.closing.load(Ordering::Acquire)
        || channel.len() != 32
        || !channel.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !(official_service(&url) || url.as_str() == "about:blank")
        || app
            .webview_windows()
            .keys()
            .filter(|label| label.starts_with("chzzk-service-") || label.starts_with("chzzk-clip-"))
            .count()
            >= 8
    {
        // Never log query strings: checkout and authentication URLs have tokens.
        tracing::warn!(
            destination_host = url.host_str().unwrap_or_default(),
            "official auxiliary popup was blocked"
        );
        return NewWindowResponse::Deny;
    }
    let label = format!("chzzk-service-{channel}-{}", uuid::Uuid::new_v4().simple());
    let expected_label = label.clone();
    let child_host = host.clone();
    let child_app = app.clone();
    let child_channel = channel.to_owned();
    let initial = InitialNavigation::default();
    let result = WebviewWindowBuilder::new(app, &label, blank())
        .data_directory(host.inner.data_dir.join("chzzk-browser-profile"))
        .browser_extensions_enabled(!super::super::browser_compat::GRID_FREE_PLAYBACK)
        .window_features(features)
        .title("CHZZK 서비스")
        .inner_size(770.0, 790.0)
        .min_inner_size(360.0, 420.0)
        .on_navigation(move |next| {
            let allowed = initial.allows(next, official_service(next));
            if !allowed {
                tracing::warn!(
                    destination_host = next.host_str().unwrap_or_default(),
                    "official auxiliary navigation was blocked"
                );
            }
            allowed
        })
        .on_new_window(move |next, next_features| {
            // Hidden privacy windows must not reveal a nested checkout.
            if !child_app
                .get_webview_window(&expected_label)
                .is_some_and(|window| window.is_visible().unwrap_or(false))
            {
                return NewWindowResponse::Deny;
            }
            open(
                &child_host,
                &child_app,
                next,
                next_features,
                &child_channel,
                depth + 1,
            )
        })
        .build();
    match result {
        Ok(window) => NewWindowResponse::Create { window },
        Err(cause) => {
            tracing::warn!(error = %cause, "could not open official auxiliary popup");
            NewWindowResponse::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_payment_and_verification_origins_are_allowed_without_app_access() {
        for url in [
            "https://chzzk.naver.com/close-window",
            "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/profile/user",
            "https://order.pay.naver.com/order?paymentId=synthetic",
            "https://pay.naver.com/",
            "https://m.pay.naver.com/",
            "https://bill.naver.com/",
            "https://nid.naver.com/user2/help/realNameCheck.nhn?type=2",
            "https://nid.naver.com/user2/help/commonTermAgree?cpcd=300",
            "https://order.npay.com/",
        ] {
            assert!(official_service(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "http://chzzk.naver.com/",
            "https://chzzk.naver.com:8443/",
            "https://chzzk.naver.com.evil.invalid/",
            "https://fakepay.naver.com/",
            "https://pay.naver.com.evil.invalid/",
            "https://evilnpay.com/",
            "https://user:secret@chzzk.naver.com/",
            "https://127.0.0.1/",
            "https://tauri.localhost/",
            "http://ipc.localhost/",
            "file:///C:/private",
            "javascript:alert(1)",
            "data:text/html,test",
            "about:blank",
        ] {
            assert!(!official_service(&url.parse().unwrap()), "{url}");
        }
    }

    #[test]
    fn blank_bootstrap_cannot_be_reentered_after_official_navigation() {
        let gate = InitialNavigation::default();
        let blank = "about:blank".parse().unwrap();
        assert!(gate.allows(&blank, false));
        assert!(!gate.allows(&"file:///C:/private".parse().unwrap(), false));
        assert!(gate.allows(&"https://order.pay.naver.com/".parse().unwrap(), true));
        assert!(!gate.allows(&blank, false));
        assert!(gate.allows(
            &"https://chzzk.naver.com/close-window".parse().unwrap(),
            true
        ));
    }

    #[test]
    fn auxiliary_windows_never_inherit_capture_or_account_controllers() {
        let source = include_str!("browser_service_popup.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!source.contains("reserve_account_window"));
        assert!(!source.contains(".initialization_script("));
        assert!(source.contains("WebviewWindowBuilder::new(app, &label, blank())"));
        assert!(source.contains(".window_features(features)"));
        for source in [
            include_str!("browser_chat_popup.rs"),
            include_str!("browser_clip_popup.rs"),
        ] {
            assert!(
                source.contains("WebviewWindowBuilder::new(app, &label, service_popup::blank())")
            );
            assert!(source.contains(".window_features(features)"));
            assert!(source.contains("initial.allows(next,"));
        }
    }
}
