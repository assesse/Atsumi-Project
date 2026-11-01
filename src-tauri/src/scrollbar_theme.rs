//! Presentation only. Remote authentication/service windows keep their own UI.
use tauri::{Runtime, Url, Webview};

const CSS: &str = include_str!("../../src/scrollbars.css");

fn instance(label: &str, prefix: &str) -> bool {
    label.strip_prefix(prefix).is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

fn content_document(label: &str, url: &Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    let path = url.path();
    if instance(label, "x-surface-") {
        return matches!(host, "x.com" | "www.x.com" | "twitter.com" | "www.twitter.com")
            && !["/i/flow", "/account", "/login", "/logout", "/oauth"].iter().any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")));
    }
    if label == "chzzk-official" || instance(label, "chzzk-mado-") || instance(label, "chzzk-auto-") || instance(label, "chzzk-chat-") {
        return host == "chzzk.naver.com";
    }
    if instance(label, "soop-original-") || instance(label, "soop-original-vod-") {
        return matches!(host, "play.sooplive.com" | "vod.sooplive.com");
    }
    if instance(label, "youtube-original-") || instance(label, "youtube-original-chat-") || instance(label, "youtube-original-vod-") {
        return host == "www.youtube.com" && matches!(path, "/watch" | "/live_chat" | "/live_chat_replay");
    }
    if label == "pinterest-official" {
        return matches!(host, "pinterest.com" | "www.pinterest.com" | "kr.pinterest.com")
            && !["/login", "/logout", "/signup", "/oauth"].iter().any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")));
    }
    false
}

pub fn apply<R: Runtime>(view: &Webview<R>, url: &Url) {
    if !content_document(view.label(), url) {
        return;
    }
    // A pending evaluation must not style a document that has navigated away.
    // No event/input hooks, network access, IPC or page API replacements.
    let expected = serde_json::to_string(url.as_str()).expect("URL string");
    let css = serde_json::to_string(CSS).expect("static CSS string");
    let script = format!(r#"(() => {{
        if (location.href !== {expected}) return;
        let style = document.querySelector('style[data-atsumi-scrollbars]');
        if (!style) {{
            style = document.createElement('style');
            style.setAttribute('data-atsumi-scrollbars', '');
            (document.head || document.documentElement).appendChild(style);
        }}
        if (style.textContent !== {css}) style.textContent = {css};
    }})();"#);
    if let Err(error) = view.eval(&script) {
        tracing::debug!(%error, "content scrollbar theme unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn only_embedded_content_documents_are_themed() {
        for (label, address) in [
            (format!("x-surface-{ID}"), "https://x.com/home"),
            (format!("x-surface-{ID}"), "https://x.com/artist/status/123/photo/2"),
            ("chzzk-official".into(), "https://chzzk.naver.com/live/channel"),
            (format!("chzzk-mado-{ID}"), "https://chzzk.naver.com/chat/channel"),
            (format!("chzzk-auto-{ID}"), "https://chzzk.naver.com/live/channel"),
            (format!("chzzk-chat-{ID}"), "https://chzzk.naver.com/chat/channel"),
            (format!("soop-original-{ID}"), "https://play.sooplive.com/channel/123/embed"),
            (format!("soop-original-vod-{ID}"), "https://vod.sooplive.com/player/123/embed"),
            (format!("youtube-original-{ID}"), "https://www.youtube.com/watch?v=test"),
            (format!("youtube-original-chat-{ID}"), "https://www.youtube.com/live_chat?v=test"),
            (format!("youtube-original-vod-{ID}"), "https://www.youtube.com/watch?v=test"),
            ("pinterest-official".into(), "https://kr.pinterest.com/pin/123456789/"),
        ] {
            assert!(content_document(&label, &address.parse().unwrap()), "{label}");
        }
    }

    #[test]
    fn authentication_popups_foreign_hosts_and_transport_are_untouched() {
        for (label, address) in [
            ("main".into(), "https://x.com/home"),
            ("x-login".into(), "https://x.com/home"),
            (format!("x-surface-{ID}"), "https://x.com/i/flow/login"),
            (format!("x-surface-{ID}"), "https://x.com/account/access"),
            (format!("x-surface-{ID}"), "https://x.com.evil.test/home"),
            (format!("x-surface-{ID}"), "http://x.com/home"),
            (format!("x-surface-{ID}"), "https://x.com:8443/home"),
            (format!("x-surface-{ID}"), "https://user@x.com/home"),
            ("chzzk-login".into(), "https://chzzk.naver.com/"),
            ("chzzk-official".into(), "https://nid.naver.com/nidlogin.login"),
            ("youtube-original-account".into(), "https://www.youtube.com/watch?v=test"),
            (format!("youtube-original-popup-{ID}"), "https://www.youtube.com/watch?v=test"),
            (format!("youtube-original-{ID}"), "https://www.youtube.com/signin"),
            (format!("soop-original-popup-{ID}"), "https://play.sooplive.com/channel/123/embed"),
            (format!("soop-original-{ID}"), "https://login.sooplive.com/"),
            ("pinterest-official".into(), "https://www.pinterest.com/login/"),
            ("pinterest-official".into(), "https://accounts.google.com/"),
        ] {
            assert!(!content_document(&label, &address.parse().unwrap()), "{label} {address}");
        }
    }
}
