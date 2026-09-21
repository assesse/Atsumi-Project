//! Official chat windows and shared auxiliary-popup routing/lifecycle. The
//! separate clip editor module owns its narrow URL policy and video preview.
//! Neither popup receives capture bridges, account reservations or main IPC.
use super::*;
use tauri::{
    webview::NewWindowFeatures, webview::NewWindowResponse, WebviewUrl, WebviewWindowBuilder,
};

fn chat_channel(url: &tauri::Url) -> Option<String> {
    if url.scheme() != "https"
        || url.host_str() != Some("chzzk.naver.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let channel = url.path().strip_prefix("/live/")?.strip_suffix("/chat")?;
    (channel.len() == 32
        && channel
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then(|| channel.to_owned())
}

fn same_chat(url: &tauri::Url, channel: &str) -> bool {
    chat_channel(url).as_deref() == Some(channel)
}

/// Called only by the native new-window callback of a known receiver. A popup
/// cannot navigate the recorder or turn an external URL into an account window.
pub(super) fn open(
    host: &OfficialBrowser,
    app: &AppHandle,
    url: tauri::Url,
    features: NewWindowFeatures,
    channel: &str,
) -> NewWindowResponse<tauri::Wry> {
    if clip_popup::is_editor(&url) {
        return clip_popup::open(host, app, url, features, channel);
    }
    if !same_chat(&url, channel) {
        return NewWindowResponse::Deny;
    }
    let label = format!("chzzk-chat-{channel}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return NewWindowResponse::Deny;
    }
    // Same WebView2 environment/profile as the opener, so the existing login
    // works without copying/exposing cookies or creating a second login flow.
    let expected = channel.to_owned();
    let result = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .data_directory(host.inner.data_dir.join("chzzk-browser-profile"))
        .browser_extensions_enabled(true)
        .window_features(features)
        .title("CHZZK 채팅")
        .inner_size(420.0, 720.0)
        .min_inner_size(300.0, 360.0)
        .initialization_script(include_str!("browser_chat_enhancements.js"))
        // This script detects /chat and only augments the header and silences
        // accidental media; it does not receive the recorder's native bridge.
        .initialization_script(include_str!("browser_multiview.js"))
        .on_navigation(move |next| same_chat(next, &expected))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build();
    match result {
        Ok(window) => NewWindowResponse::Create { window },
        Err(cause) => {
            tracing::warn!(channel_id = channel, error = %cause, "could not open official chat popup");
            NewWindowResponse::Deny
        }
    }
}

pub(super) fn is_chat(url: &tauri::Url) -> bool {
    // Malformed chat requests must not fall through to the account popup path.
    url.path().starts_with("/live/") && url.path().contains("/chat")
}

pub(super) fn sync_privacy(
    app: &AppHandle,
    channel: &str,
    private: bool,
    revision: Arc<AtomicU64>,
    expected: u64,
) {
    clip_popup::sync_privacy(app, channel, private, revision.clone(), expected);
    let Some(window) = app.get_webview_window(&format!("chzzk-chat-{channel}")) else {
        return;
    };
    let _ = app.run_on_main_thread(move || {
        if revision.load(Ordering::Acquire) != expected {
            return;
        }
        if private {
            let _ = window.hide();
        } else if !window.is_visible().unwrap_or(true) {
            let _ = window.show();
        }
    });
}

pub(super) fn close_all(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("chzzk-chat-") || label.starts_with("chzzk-clip-") {
            let _ = window.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn only_exact_official_chat_for_the_owning_channel_is_allowed() {
        let url: tauri::Url = format!("https://chzzk.naver.com/live/{CHANNEL}/chat")
            .parse()
            .unwrap();
        assert!(same_chat(&url, CHANNEL));
        assert!(!same_chat(&url, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
        for candidate in [
            format!("http://chzzk.naver.com/live/{CHANNEL}/chat"),
            format!("https://chzzk.naver.com.evil.invalid/live/{CHANNEL}/chat"),
            format!("https://user@chzzk.naver.com/live/{CHANNEL}/chat"),
            format!("https://chzzk.naver.com:8443/live/{CHANNEL}/chat"),
            format!("https://chzzk.naver.com/live/{CHANNEL}"),
            format!("https://chzzk.naver.com/live/{CHANNEL}/chat?studio=true"),
            format!("https://chzzk.naver.com/live/{CHANNEL}/chat#other"),
            format!("https://chzzk.naver.com/live/{CHANNEL}/chat/"),
            "https://nid.naver.com/nidlogin.login".into(),
            "javascript:alert(1)".into(),
            "about:blank".into(),
        ] {
            assert!(
                !same_chat(&candidate.parse().unwrap(), CHANNEL),
                "{candidate}"
            );
        }
    }
}
