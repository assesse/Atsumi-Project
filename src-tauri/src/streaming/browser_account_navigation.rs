//! Naver's receipt challenge can redirect HTTPS POST /nidlogin.rcaptcha to
//! HTTP GET /nidlogin.login?chp=... (observed 2026-10-12). Cancel that insecure
//! navigation and resume the exact continuation over HTTPS in the SAME view.
//! Never submit/replay a password or challenge answer, or log the URL/token.
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Url};

#[derive(Default)]
struct State {
    generation: u64,
    challenge_pending: bool,
    closed: bool,
}

#[derive(Default)]
pub(super) struct AccountNavigation(Mutex<State>);

// Deliberately no Debug/Serialize: target contains a one-use continuation token.
pub(super) struct Upgrade {
    generation: u64,
    target: Url,
}

fn identity_url(url: &Url, scheme: &str, path: &str) -> bool {
    url.scheme() == scheme
        && url.host_str() == Some("nid.naver.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == path
        && url.fragment().is_none()
        && url.as_str().len() <= 16_384
}

fn https_continuation(url: &Url) -> Option<Url> {
    if !identity_url(url, "http", "/nidlogin.login") {
        return None;
    }
    let mut tokens = url.query_pairs().filter(|(name, _)| name == "chp");
    let (_, token) = tokens.next()?;
    if token.is_empty() || tokens.next().is_some() {
        return None;
    }
    let mut secure = url.clone();
    secure.set_scheme("https").ok()?;
    // Do not reconstruct query_pairs: escaping, order and token bytes must stay
    // exactly as the server sent them. Nothing is persisted outside this task.
    Some(secure)
}

impl AccountNavigation {
    pub(super) fn observe(&self, url: &Url) -> Option<Upgrade> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.generation = state.generation.wrapping_add(1);
        if state.closed {
            return None;
        }
        let was_challenge = state.challenge_pending;
        state.challenge_pending = identity_url(url, "https", "/nidlogin.rcaptcha");
        // Only the immediate continuation of the official challenge qualifies.
        // Ordinary HTTP logins, arbitrary POSTs and redirect loops stay denied.
        was_challenge
            .then(|| https_continuation(url))
            .flatten()
            .map(|target| Upgrade {
                generation: state.generation,
                target,
            })
    }

    fn is_current(&self, generation: u64) -> bool {
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        !state.closed && state.generation == generation
    }

    pub(super) fn close(&self) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        state.challenge_pending = false;
        state.generation = state.generation.wrapping_add(1);
    }
}

pub(super) fn enqueue(
    app: &AppHandle,
    label: &str,
    guard: Arc<AccountNavigation>,
    upgrade: Upgrade,
) {
    // Capture this exact window, not a future login window reusing its label.
    let Some(window) = app.get_webview_window(label) else {
        tracing::warn!(diag_stage = "chzzk_account_https_resume_unavailable");
        return;
    };
    let app = app.clone();
    // NavigationStarting runs under the browser dispatcher. Defer navigation
    // until that callback has returned; a synchronous navigate can deadlock.
    std::thread::spawn(move || {
        if app
            .run_on_main_thread(move || {
                if !guard.is_current(upgrade.generation) {
                    return;
                }
                if window.navigate(upgrade.target).is_err() {
                    tracing::warn!(diag_stage = "chzzk_account_https_resume_failed");
                } else {
                    tracing::info!(diag_stage = "chzzk_account_https_resume_started");
                }
            })
            .is_err()
        {
            tracing::warn!(diag_stage = "chzzk_account_https_resume_failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHALLENGE: &str = "https://nid.naver.com/nidlogin.rcaptcha";
    const CONTINUATION: &str = "http://nid.naver.com/nidlogin.login?url=https%3A%2F%2Fchzzk.naver.com%2F&chp=synthetic%2Bvalue%2F%3D&locale=ko_KR&svctype=1";

    fn prepared() -> AccountNavigation {
        let guard = AccountNavigation::default();
        assert!(guard.observe(&CHALLENGE.parse().unwrap()).is_none());
        guard
    }

    #[test]
    fn challenge_continuation_changes_only_scheme_and_keeps_query_bytes() {
        let guard = prepared();
        let original: Url = CONTINUATION.parse().unwrap();
        let upgrade = guard.observe(&original).unwrap();
        assert_eq!(
            upgrade.target.as_str(),
            CONTINUATION.replacen("http:", "https:", 1)
        );
        assert_eq!(upgrade.target.query(), original.query());
        assert!(super::super::allowed_navigation(&upgrade.target));
        assert!(!super::super::allowed_navigation(&original));
        assert!(guard.is_current(upgrade.generation));
    }

    #[test]
    fn ordinary_http_or_unrelated_navigation_is_never_upgraded() {
        for url in [
            "http://nid.naver.com/nidlogin.login",
            "http://nid.naver.com/nidlogin.login?chp=",
            "http://nid.naver.com/nidlogin.login?chp=one&chp=two",
            "http://nid.naver.com/nidlogin.rcaptcha?chp=synthetic",
            "http://nid.naver.com/other?chp=synthetic",
            "http://nid.naver.com.evil.invalid/nidlogin.login?chp=synthetic",
            "http://nid.naver.com:8080/nidlogin.login?chp=synthetic",
            "http://name:secret@nid.naver.com/nidlogin.login?chp=synthetic",
            "http://nid.naver.com/nidlogin.login?chp=synthetic#fragment",
            "http://chzzk.naver.com/nidlogin.login?chp=synthetic",
            "https://nid.naver.com/nidlogin.login?chp=synthetic",
            "file:///C:/nidlogin.login?chp=synthetic",
        ] {
            assert!(prepared().observe(&url.parse().unwrap()).is_none(), "{url}");
        }
        let oversized = format!(
            "http://nid.naver.com/nidlogin.login?chp={}",
            "x".repeat(16_384)
        );
        assert!(prepared().observe(&oversized.parse().unwrap()).is_none());
    }

    #[test]
    fn requires_immediately_preceding_official_challenge() {
        let target: Url = CONTINUATION.parse().unwrap();
        let guard = AccountNavigation::default();
        assert!(guard.observe(&target).is_none());
        for wrong_source in [
            "http://nid.naver.com/nidlogin.rcaptcha",
            "https://nid.naver.com/nidlogin.login",
            "https://nid.naver.com.evil.invalid/nidlogin.rcaptcha",
            "https://nid.naver.com:8443/nidlogin.rcaptcha",
            "https://name@nid.naver.com/nidlogin.rcaptcha",
        ] {
            guard.observe(&wrong_source.parse().unwrap());
            assert!(guard.observe(&target).is_none());
        }
        let guard = prepared();
        guard.observe(&"https://nid.naver.com/other".parse().unwrap());
        assert!(guard.observe(&target).is_none());
    }

    #[test]
    fn redirect_cannot_loop_but_fresh_user_challenge_can_retry() {
        let guard = prepared();
        let target: Url = CONTINUATION.parse().unwrap();
        let first = guard.observe(&target).unwrap();
        assert!(guard.observe(&target).is_none());
        assert!(!guard.is_current(first.generation));
        guard.observe(&first.target);
        assert!(guard.observe(&target).is_none());
        guard.observe(&CHALLENGE.parse().unwrap());
        assert!(guard.observe(&target).is_some());
    }

    #[test]
    fn back_navigation_and_closed_window_cancel_queued_continuation() {
        let guard = prepared();
        let target: Url = CONTINUATION.parse().unwrap();
        let upgrade = guard.observe(&target).unwrap();
        guard.observe(&"https://nid.naver.com/nidlogin.login".parse().unwrap());
        assert!(!guard.is_current(upgrade.generation));
        guard.observe(&CHALLENGE.parse().unwrap());
        let upgrade = guard.observe(&target).unwrap();
        guard.close();
        assert!(!guard.is_current(upgrade.generation));
        guard.observe(&CHALLENGE.parse().unwrap());
        assert!(guard.observe(&target).is_none());
    }
}
