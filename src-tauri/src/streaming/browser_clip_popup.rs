//! Official live clip editor, separate from account and recording controllers.
//! The editor keeps its opener and the shared browser login, but receives no
//! capture scripts, Tauri permissions, filesystem paths or native recording IPC.
use super::*;
use tauri::{
    webview::{NewWindowFeatures, NewWindowResponse},
    WebviewUrl, WebviewWindowBuilder,
};

#[derive(Debug, PartialEq, Eq)]
struct ClipTarget {
    live_id: u64,
}

impl ClipTarget {
    // Confirmed in CHZZK's public player on 2026-09-20:
    // window.open('/clip-editor?contentType=live&contentId=...&offsetTime=...',
    //             'chzzkClipEditor', 'width=1020,height=690').
    fn parse(url: &tauri::Url) -> Option<Self> {
        if url.scheme() != "https"
            || url.host_str() != Some("chzzk.naver.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/clip-editor"
            || url.fragment().is_some()
            || url.as_str().len() > 1024
        {
            return None;
        }
        let (mut kind, mut id, mut offset) = (None, None, None);
        for (key, value) in url.query_pairs() {
            let slot = match key.as_ref() {
                "contentType" => &mut kind,
                "contentId" => &mut id,
                "offsetTime" => &mut offset,
                _ => return None,
            };
            if slot.replace(value.into_owned()).is_some() {
                return None;
            }
        }
        if kind.as_deref() != Some("live") {
            return None;
        }
        let id = id?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let live_id = id.parse::<u64>().ok()?;
        let offset = offset?.parse::<f64>().ok()?;
        (live_id > 0
            && live_id <= 9_007_199_254_740_991
            && offset.is_finite()
            && offset.abs() <= 1e9)
            .then_some(Self { live_id })
    }

    fn allows(&self, url: &tauri::Url) -> bool {
        Self::parse(url).as_ref() == Some(self)
    }
}

/// Even malformed editor requests must not fall through into account handling.
pub(super) fn is_editor(url: &tauri::Url) -> bool {
    url.path().starts_with("/clip-editor")
}

pub(super) fn open(
    host: &OfficialBrowser,
    app: &AppHandle,
    url: tauri::Url,
    features: NewWindowFeatures,
    channel: &str,
) -> NewWindowResponse<tauri::Wry> {
    let Some(target) = ClipTarget::parse(&url) else {
        return NewWindowResponse::Deny;
    };
    if channel.len() != 32 || !channel.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return NewWindowResponse::Deny;
    }
    let label = format!("chzzk-clip-{channel}-{}", target.live_id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return NewWindowResponse::Deny;
    }
    if app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with("chzzk-clip-"))
        .count()
        >= 8
    {
        return NewWindowResponse::Deny;
    }
    // Do not reserve an account window: doing so blocks a running recording.
    // window_features retains the official opener/close notifications. CHZZK
    // itself temporarily mutes that live player's audio while editing a clip.
    let result = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .data_directory(host.inner.data_dir.join("chzzk-browser-profile"))
        .browser_extensions_enabled(true)
        .window_features(features)
        .title("CHZZK 클립 만들기")
        .inner_size(1020.0, 690.0)
        .min_inner_size(760.0, 540.0)
        .on_navigation(move |next| target.allows(next))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build();
    match result {
        Ok(window) => NewWindowResponse::Create { window },
        Err(cause) => {
            tracing::warn!(channel_id = channel, error = %cause, "could not open official clip editor");
            NewWindowResponse::Deny
        }
    }
}

pub(super) fn sync_privacy(
    app: &AppHandle,
    channel: &str,
    private: bool,
    revision: Arc<AtomicU64>,
    expected: u64,
) {
    let prefix = format!("chzzk-clip-{channel}-");
    for (label, window) in app.webview_windows() {
        if !label.starts_with(&prefix) {
            continue;
        }
        let revision = revision.clone();
        let _ = app.run_on_main_thread(move || {
            if revision.load(Ordering::Acquire) != expected {
                return;
            }
            if private {
                let _ = window.hide();
            }
            let revision_for_show = revision.clone();
            #[cfg(windows)]
            {
                use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
                use windows::core::Interface;
                let _ = window.with_webview(move |platform| unsafe {
                    if revision.load(Ordering::Acquire) != expected {
                        return;
                    }
                    if let Ok(core) = platform
                        .controller()
                        .CoreWebView2()
                        .and_then(|core| core.cast::<ICoreWebView2_8>())
                    {
                        // Native privacy mute does not overwrite the editor's
                        // own HTML media volume/mute preference.
                        let _ = core.SetIsMuted(private);
                    }
                });
            }
            if !private
                && revision_for_show.load(Ordering::Acquire) == expected
                && !window.is_visible().unwrap_or(true)
            {
                let _ = window.show();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(query: &str) -> tauri::Url {
        format!("https://chzzk.naver.com/clip-editor?{query}")
            .parse()
            .unwrap()
    }

    #[test]
    fn accepts_official_live_editor_and_locks_navigation_to_that_broadcast() {
        let target =
            ClipTarget::parse(&editor("contentType=live&contentId=21213148&offsetTime=0")).unwrap();
        assert_eq!(target.live_id, 21213148);
        assert!(target.allows(&editor(
            "offsetTime=-12.75&contentId=21213148&contentType=live"
        )));
        assert!(!target.allows(&editor("contentType=live&contentId=21210314&offsetTime=0")));
        assert!(!target.allows(&"https://nid.naver.com/nidlogin.login".parse().unwrap()));
    }

    #[test]
    fn rejects_foreign_origins_credentials_paths_and_non_live_targets() {
        let query = "contentType=live&contentId=21213148&offsetTime=0";
        for base in [
            "http://chzzk.naver.com/clip-editor",
            "https://chzzk.naver.com.evil.invalid/clip-editor",
            "https://user@chzzk.naver.com/clip-editor",
            "https://chzzk.naver.com:8443/clip-editor",
            "https://nid.naver.com/clip-editor",
            "https://chzzk.naver.com/clip-editor/",
            "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(
                ClipTarget::parse(&format!("{base}?{query}").parse().unwrap()).is_none(),
                "{base}"
            );
        }
        assert!(ClipTarget::parse(
            &format!("https://chzzk.naver.com/clip-editor?{query}#other")
                .parse()
                .unwrap()
        )
        .is_none());
        for query in [
            "",
            "contentType=video&contentId=21213148&offsetTime=0",
            "contentType=live&contentId=21213148",
            "contentType=live&contentId=0&offsetTime=0",
            "contentType=live&contentId=bad&offsetTime=0",
            "contentType=live&contentId=9007199254740992&offsetTime=0",
            "contentType=live&contentId=21213148&offsetTime=NaN",
            "contentType=live&contentId=21213148&offsetTime=inf",
            "contentType=live&contentId=21213148&offsetTime=1e20",
            "contentType=live&contentId=21213148&offsetTime=0&next=https://evil.invalid",
            "contentType=live&contentId=21213148&offsetTime=0&contentId=1",
            "contentType=live&contentId=21213148&offsetTime=0&contentType=video",
            "contentType=live&contentId=21213148&offsetTime=0&offsetTime=1",
        ] {
            assert!(ClipTarget::parse(&editor(query)).is_none(), "{query}");
        }
    }

    #[test]
    fn malformed_editor_routes_cannot_be_mistaken_for_account_windows() {
        for route in ["/clip-editor", "/clip-editor/", "/clip-editor-unknown"] {
            assert!(is_editor(
                &format!("https://chzzk.naver.com{route}").parse().unwrap()
            ));
        }
        assert!(!is_editor(
            &"https://nid.naver.com/nidlogin.login".parse().unwrap()
        ));
    }
}
