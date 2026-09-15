//! Boolean-only UI observation, never authority for any privileged operation.
use super::*;

const ENDPOINT: &str = "https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus";
const PROBE_PREFIX: &str = "chzzk-auth-probe-";
pub(super) const DEADLINE: Duration = Duration::from_secs(8);

fn allowed_probe_url(url: &tauri::Url) -> bool {
    url.as_str() == ENDPOINT
}

impl ViewState {
    pub(super) fn begin_auth_probe(&mut self, force: bool, now: Instant) -> Option<u64> {
        if self.account_busy
            || self.auth_checking
            || (!force
                && self
                    .auth_last_probe
                    .is_some_and(|last| now.duration_since(last) < Duration::from_secs(30)))
        {
            return None;
        }
        self.auth_generation = self.auth_generation.wrapping_add(1);
        self.auth_last_probe = Some(now);
        self.auth_checking = true;
        self.auth_error = None;
        // A background refresh must not flash an already verified account as
        // logged out or re-enable the login button while its request is pending.
        if self.auth_status == AuthStatus::Unknown {
            self.auth_status = AuthStatus::Checking;
        }
        Some(self.auth_generation)
    }

    pub(super) fn finish_auth_probe(&mut self, generation: u64, status: AuthStatus) {
        if self.auth_generation != generation || !self.auth_checking || self.account_busy {
            return;
        }
        self.auth_checking = false;
        self.auth_status = status;
        self.auth_last_probe = Some(Instant::now());
        self.auth_error = (status == AuthStatus::Unknown).then(||
            "로그인 상태를 확인하지 못했습니다. 인터넷 연결을 확인한 뒤 ‘상태 확인’을 다시 눌러 주세요.".into());
    }

    pub(super) fn invalidate_auth(&mut self) {
        self.auth_generation = self.auth_generation.wrapping_add(1);
        self.auth_checking = false;
        self.auth_error = None;
        self.auth_status = AuthStatus::Unknown;
        self.auth_last_probe = None;
    }
}

/// Read-only observation in the same browser profile, even with no player open.
/// Only the small account endpoint is loaded: no live stream or extra player.
pub(super) fn probe_profile(host: &OfficialBrowser, app: &AppHandle, generation: u64) {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let host = host.clone();
    let app = app.clone();
    let label = format!("{PROBE_PREFIX}{}", uuid::Uuid::new_v4().simple());
    let completed = Arc::new(AtomicBool::new(false));
    let finish: Arc<dyn Fn(AuthStatus) + Send + Sync> = {
        let host = host.clone();
        let app = app.clone();
        let label = label.clone();
        let completed = completed.clone();
        Arc::new(move |status| {
            if completed.swap(true, Ordering::AcqRel) {
                return;
            }
            if let Ok(mut state) = host.inner.view.lock() {
                state.finish_auth_probe(generation, status);
            }
            // The unique label prevents a late timeout from closing a newer check.
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        })
    };
    let timeout = finish.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(DEADLINE).await;
        timeout(AuthStatus::Unknown);
    });
    tauri::async_runtime::spawn_blocking(move || {
        let current = || {
            !completed.load(Ordering::Acquire)
                && !host.inner.closing.load(Ordering::Acquire)
                && host.inner.view.lock().is_ok_and(|s| {
                    s.auth_generation == generation && s.auth_checking && !s.account_busy
                })
        };
        if !current() {
            finish(AuthStatus::Unknown);
            return;
        }
        let profile = host.inner.data_dir.join("chzzk-browser-profile");
        if std::fs::create_dir_all(&profile).is_err() {
            finish(AuthStatus::Unknown);
            return;
        }
        let page_finish = finish.clone();
        let page_started = Arc::new(AtomicBool::new(false));
        let window = WebviewWindowBuilder::new(
            &app,
            &label,
            WebviewUrl::External(ENDPOINT.parse().unwrap()),
        )
        .title("CHZZK 계정 상태 확인")
        .data_directory(profile)
        .browser_extensions_enabled(true)
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .inner_size(1.0, 1.0)
        .on_navigation(allowed_probe_url)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_page_load(move |view, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
                && allowed_probe_url(payload.url())
                && !page_started.swap(true, Ordering::AcqRel)
            {
                let callback = page_finish.clone();
                if probe(view.as_ref(), move |status| callback(status)).is_err() {
                    page_finish(AuthStatus::Unknown);
                }
            }
        })
        .build();
        match window {
            Ok(window) => {
                // Creation can race logout or the deadline. Never leave its
                // profile owner behind or accept a result from an old account.
                if !current() {
                    let _ = window.destroy();
                    finish(AuthStatus::Unknown);
                }
            }
            Err(_) => finish(AuthStatus::Unknown),
        }
    });
}

pub(super) fn cancel(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(PROBE_PREFIX) {
            let _ = window.destroy();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AuthStatus {
    Unknown,
    Checking,
    SignedIn,
    SignedOut,
}

fn parse_response(json: &str) -> AuthStatus {
    if json.len() > 512 {
        return AuthStatus::Unknown;
    }
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return AuthStatus::Unknown;
    };
    if value.get("exceptionDetails").is_some() {
        return AuthStatus::Unknown;
    }
    match value.pointer("/result/value").and_then(Value::as_str) {
        Some("signed_in") => AuthStatus::SignedIn,
        Some("signed_out") => AuthStatus::SignedOut,
        _ => AuthStatus::Unknown,
    }
}

#[cfg(windows)]
pub(super) fn probe(
    view: &Webview,
    callback: impl FnOnce(AuthStatus) + Send + 'static,
) -> Result<(), StreamError> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    let url = view.url().map_err(|_| unavailable())?;
    if !allowed_probe_url(&url) {
        return Err(unavailable());
    }
    type Completion = Arc<Mutex<Option<Box<dyn FnOnce(AuthStatus) + Send>>>>;
    fn complete(callback: &Completion, state: AuthStatus) {
        let action = callback.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(action) = action {
            action(state);
        }
    }
    let callback: Completion = Arc::new(Mutex::new(Some(Box::new(callback))));
    let deadline = callback.clone();
    thread::Builder::new()
        .name("chzzk-auth-deadline".into())
        .spawn(move || {
            thread::sleep(Duration::from_secs(6));
            complete(&deadline, AuthStatus::Unknown);
        })
        .map_err(|_| unavailable())?;
    let dispatched = callback.clone();
    let result = view.with_webview(move |platform| unsafe {
        let outcome = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let completed = dispatched.clone();
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, json| {
                complete(&completed, if status.is_ok() { parse_response(&json) } else { AuthStatus::Unknown });
                Ok(())
            }));
            let params = json!({ "expression": include_str!("browser_auth.js"), "awaitPromise": true, "returnByValue": true }).to_string();
            core.CallDevToolsProtocolMethod(&HSTRING::from("Runtime.evaluate"), &HSTRING::from(params), &handler)
        })();
        if outcome.is_err() { complete(&dispatched, AuthStatus::Unknown); }
    });
    if result.is_err() {
        complete(&callback, AuthStatus::Unknown);
    }
    Ok(())
}
#[cfg(not(windows))]
pub(super) fn probe(
    _: &Webview,
    callback: impl FnOnce(AuthStatus) + Send + 'static,
) -> Result<(), StreamError> {
    callback(AuthStatus::Unknown);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_check_works_without_a_player_and_manual_check_bypasses_cache() {
        let mut state = ViewState::default();
        assert!(!state.open);
        let first = state.begin_auth_probe(false, Instant::now()).unwrap();
        assert_eq!(state.auth_status, AuthStatus::Checking);
        assert!(state.begin_auth_probe(true, Instant::now()).is_none());
        state.finish_auth_probe(first, AuthStatus::SignedIn);
        assert!(!state.auth_checking);
        assert!(state.auth_error.is_none());
        assert!(state.begin_auth_probe(false, Instant::now()).is_none());
        let second = state.begin_auth_probe(true, Instant::now()).unwrap();
        assert_ne!(first, second);
        assert_eq!(state.auth_status, AuthStatus::SignedIn);
        assert!(state.auth_checking);
        state.finish_auth_probe(first, AuthStatus::SignedOut);
        assert_eq!(state.auth_status, AuthStatus::SignedIn);
        assert!(state.auth_checking);
        state.finish_auth_probe(second, AuthStatus::Unknown);
        assert_eq!(state.auth_status, AuthStatus::Unknown);
        assert!(state.auth_error.is_some());
        assert!(!state.auth_checking);
    }

    #[test]
    fn account_changes_and_duplicate_completions_cannot_restore_stale_login() {
        let mut state = ViewState::default();
        let old = state.begin_auth_probe(false, Instant::now()).unwrap();
        state.invalidate_auth();
        state.account_busy = true;
        state.finish_auth_probe(old, AuthStatus::SignedIn);
        assert_eq!(state.auth_status, AuthStatus::Unknown);
        assert!(state.begin_auth_probe(true, Instant::now()).is_none());
        state.account_busy = false;
        let next = state.begin_auth_probe(true, Instant::now()).unwrap();
        state.finish_auth_probe(next, AuthStatus::SignedOut);
        state.finish_auth_probe(next, AuthStatus::Unknown);
        assert_eq!(state.auth_status, AuthStatus::SignedOut);
        assert!(state.auth_error.is_none());
    }

    #[test]
    fn probe_navigation_is_limited_to_exact_official_account_endpoint() {
        assert!(allowed_probe_url(&ENDPOINT.parse().unwrap()));
        for url in [
            "about:blank",
            "https://chzzk.naver.com/live/a",
            "https://nid.naver.com/",
            "http://comm-api.game.naver.com/nng_main/v1/user/getUserStatus",
            "https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus?next=1",
            "https://comm-api.game.naver.com.evil.invalid/nng_main/v1/user/getUserStatus",
        ] {
            assert!(!allowed_probe_url(&url.parse().unwrap()));
        }
    }

    #[test]
    fn only_exact_projection_states_are_accepted() {
        assert_eq!(
            parse_response(r#"{"result":{"value":"signed_in"}}"#),
            AuthStatus::SignedIn
        );
        assert_eq!(
            parse_response(r#"{"result":{"value":"signed_out"}}"#),
            AuthStatus::SignedOut
        );
        for value in [
            r#"{"result":{"value":true}}"#,
            r#"{"result":{"value":{"loggedIn":true}}}"#,
            r#"{"result":{"value":"signed_in"},"exceptionDetails":{}}"#,
            "invalid",
            "{}",
        ] {
            assert_eq!(parse_response(value), AuthStatus::Unknown);
        }
    }
}
