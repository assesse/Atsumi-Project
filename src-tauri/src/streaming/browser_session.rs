//! A profile folder persists durable cookies, not a WebView2 session. Retain one
//! inert owner until app shutdown so closing the last login/player/probe does not
//! discard session cookies. Never copy cookies or change their expiry policy.
use super::*;
use tauri::{WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "chzzk-profile-session";

fn allowed_navigation(url: &tauri::Url) -> bool {
    url.as_str() == "about:blank"
}

fn session_unavailable() -> StreamError {
    error(
        "BROWSER_SESSION_UNAVAILABLE",
        "로그인 세션을 준비하지 못했습니다. 잠시 후 다시 시도해 주세요.",
    )
}

impl OfficialBrowser {
    /// Lazily retain the shared browser session without opening a site or media.
    /// Call on a worker, without holding view/context gates; not a native callback.
    /// This is intentionally not part of `new`: other workspaces/startup pay no cost.
    pub fn prepare_profile_session(&self, app: &AppHandle) -> Result<(), StreamError> {
        let _guard = self
            .inner
            .contexts
            .profile_session
            .lock()
            .map_err(|_| session_unavailable())?;
        if self.inner.closing.load(Ordering::Acquire) || self.inner.detached.load(Ordering::Acquire)
        {
            return Err(session_unavailable());
        }
        if app.get_webview_window(LABEL).is_some() {
            return Ok(());
        }
        let profile = self.inner.data_dir.join("chzzk-browser-profile");
        std::fs::create_dir_all(&profile).map_err(|_| session_unavailable())?;
        let window = WebviewWindowBuilder::new(
            app,
            LABEL,
            WebviewUrl::External("about:blank".parse().unwrap()),
        )
        .title("CHZZK session")
        .data_directory(profile)
        .browser_extensions_enabled(!super::super::browser_compat::GRID_FREE_PLAYBACK)
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .inner_size(1.0, 1.0)
        .on_navigation(allowed_navigation)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build()
        .map_err(|_| {
            tracing::warn!(diag_stage = "chzzk_session_owner_failed");
            session_unavailable()
        })?;
        // Shutdown may start while WebView2 is being created. Do not leave a
        // late owner behind or let that request open another login/player.
        if self.inner.closing.load(Ordering::Acquire) || self.inner.detached.load(Ordering::Acquire)
        {
            let _ = window.destroy();
            return Err(session_unavailable());
        }
        tracing::info!(diag_stage = "chzzk_session_owner_ready");
        Ok(())
    }
}

pub(super) fn release(host: &OfficialBrowser, app: &AppHandle) {
    let _guard = host
        .inner
        .contexts
        .profile_session
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.destroy();
        tracing::info!(diag_stage = "chzzk_session_owner_closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_owner_cannot_navigate_to_accounts_media_or_app_content() {
        assert!(allowed_navigation(&"about:blank".parse().unwrap()));
        for url in [
            "about:blank#redirect",
            "https://chzzk.naver.com/",
            "https://nid.naver.com/",
            "http://tauri.localhost/",
            "data:text/html,test",
            "file:///C:/Windows/win.ini",
        ] {
            assert!(!allowed_navigation(&url.parse().unwrap()));
        }
    }

    #[test]
    fn owner_is_not_a_login_probe_player_or_recording_context() {
        for prefix in [
            "chzzk-login",
            "chzzk-auth-probe-",
            "chzzk-official",
            "chzzk-auto-",
            "chzzk-mado-",
            "chzzk-chat-",
        ] {
            assert!(!LABEL.starts_with(prefix));
        }
    }
}
