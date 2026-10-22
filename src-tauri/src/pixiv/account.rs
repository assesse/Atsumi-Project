//! Pixiv-owned authentication proof with a shared browser identity. Secrets stay native.
use super::session::{session_header, Authority, Lease};
use reqwest::{blocking::Client, header::HeaderValue, redirect::Policy, Url};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Manager, State, Webview, WebviewUrl, WebviewWindowBuilder};

const HOLDER: &str = "pixiv-session-profile";
const WINDOW: &str = "pixiv-account";
const UNAVAILABLE: &str = "Pixiv 로그인 창을 준비하지 못했습니다.";
const VERIFY_FAILED: &str = "Pixiv 로그인 상태를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.";
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub authenticated: Option<bool>,
    pub open: bool,
    pub busy: bool,
    pub revision: u64,
    pub error: Option<String>,
}
#[derive(Default)]
struct Observed {
    cookie: Option<HeaderValue>,
    authenticated: Option<bool>,
    checked: Option<Instant>,
    error: Option<String>,
    operation: u64,
    logging_out: bool,
    window_logout: Option<bool>,
}
#[derive(Clone, Default)]
pub struct PixivAccount {
    authority: Authority,
    state: Arc<Mutex<Observed>>,
    lane: Arc<tokio::sync::Mutex<()>>,
    stopped: Arc<AtomicBool>,
}
impl PixivAccount {
    fn admission(&self, app: &tauri::AppHandle) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(super::session::STALE.into());
        }
        let state = app
            .try_state::<crate::interface::commands::AppState>()
            .ok_or(UNAVAILABLE)?;
        state.begin_platform_work(|| {
            if self.stopped.load(Ordering::Acquire) {
                Err(super::session::STALE.into())
            } else {
                Ok(())
            }
        })
    }
    /// Data reload/reset revokes pending remote work but never removes the official profile.
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.operation = state.operation.wrapping_add(1);
        state.cookie = None;
        state.authenticated = None;
        state.checked = None;
        state.logging_out = false;
        state.window_logout = None;
        state.error = None;
        self.authority.invalidate();
    }
    /// Called by the normal drain before windows are destroyed; cannot be reopened after shutdown.
    pub fn shutdown(&self, app: &tauri::AppHandle) {
        self.stopped.store(true, Ordering::Release);
        self.invalidate();
        for label in [WINDOW, HOLDER] {
            if let Some(window) = app.get_webview_window(label) {
                let _ = window.close();
            }
        }
        for (label, window) in app.webview_windows() {
            if label.starts_with("pixiv-account-social-") { let _ = window.close(); }
        }
    }
    fn snapshot(&self, app: &tauri::AppHandle) -> AccountSnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        AccountSnapshot {
            authenticated: state.authenticated,
            open: app.get_webview_window(WINDOW).is_some(),
            busy: state.logging_out,
            revision: self.authority.revision(),
            error: state.error.clone(),
        }
    }
    fn finish_verified_window(&self, app: &tauri::AppHandle, operation: u64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(logout) = state.window_logout else {
            return;
        };
        if state.operation != operation
            || state.authenticated != Some(!logout)
            || self.stopped.load(Ordering::Acquire)
        {
            return;
        }
        drop(state);
        if let Some(window) = app.get_webview_window(WINDOW) {
            if window
                .url()
                .ok()
                .is_some_and(|url| completion_url(&url, logout))
            {
                let _ = window.close();
            }
        }
    }
    fn profile(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
        crate::browser_identity::profile(app).map_err(|_| UNAVAILABLE.into())
    }
    fn holder(&self, app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
        self.admission(app)?;
        if let Some(view) = app.get_webview_window(HOLDER) {
            return Ok(view);
        }
        let window = WebviewWindowBuilder::new(
            app,
            HOLDER,
            WebviewUrl::External("about:blank".parse().unwrap()),
        )
        .title("Pixiv session")
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .data_directory(Self::profile(app)?)
        .browser_extensions_enabled(false)
        .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
        .on_navigation(|url| url.as_str() == "about:blank")
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()
        .map_err(|_| UNAVAILABLE.to_owned())?;
        // Never hold the managed-work mutex across a native UI dispatch.
        if let Err(error) = self.admission(app) {
            let _ = window.close();
            return Err(error);
        }
        Ok(window)
    }
    async fn refresh(
        &self,
        app: &tauri::AppHandle,
        force: bool,
    ) -> Result<AccountSnapshot, String> {
        let _lane = self.lane.lock().await;
        self.admission(app)?;
        {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !force
                && state
                    .checked
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(10))
            {
                drop(state);
                return Ok(self.snapshot(app));
            }
        }
        let holder = match self.holder(app) {
            Ok(holder) => holder,
            Err(error) => {
                mark_unknown(
                    &mut self.state.lock().unwrap_or_else(|e| e.into_inner()),
                    &self.authority,
                );
                return Err(error);
            }
        };
        let operation = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .operation;
        // cookies_for_url deadlocks in a Windows UI callback. This worker is the only reader.
        let observation =
            tauri::async_runtime::spawn_blocking(move || -> Result<Option<HeaderValue>, String> {
                let cookies = holder
                    .cookies_for_url(Url::parse("https://www.pixiv.net/").unwrap())
                    .map_err(|_| VERIFY_FAILED)?;
                if cookies.len() > 128 {
                    return Err(VERIFY_FAILED.into());
                }
                let candidates: Vec<_> = cookies
                    .iter()
                    .filter(|cookie| cookie.name() == "PHPSESSID")
                    .collect();
                if candidates.len() > 1 {
                    return Err(VERIFY_FAILED.into());
                }
                let Some(cookie) = candidates.first() else {
                    return Ok(None);
                };
                if !cookie_scope(cookie.domain(), cookie.path()) {
                    return Err(VERIFY_FAILED.into());
                }
                let header = session_header(cookie.value()).ok_or(VERIFY_FAILED)?;
                match verify_session(&header)? {
                    SessionProof::Authenticated => Ok(Some(header)),
                    SessionProof::SignedOut => Ok(None),
                }
            })
            .await
            .unwrap_or_else(|_| Err(VERIFY_FAILED.to_owned()));
        self.admission(app)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.operation != operation {
            return Err(super::session::STALE.into());
        }
        state.checked = Some(Instant::now());
        apply_observation(&mut state, &self.authority, observation);
        drop(state);
        // Cookie commit can lag behind the first PageLoadEvent. A later verified poll
        // may close an official landing, but never an authentication/security document.
        self.finish_verified_window(app, operation);
        Ok(self.snapshot(app))
    }
    pub(super) async fn lease(&self, app: &tauri::AppHandle) -> Result<Lease, String> {
        self.refresh(app, false).await?;
        self.admission(app)?;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.logging_out
            || state.authenticated.is_none()
            || self.stopped.load(Ordering::Acquire)
        {
            return Err(VERIFY_FAILED.into());
        }
        Ok(self.authority.lease(state.cookie.clone()))
    }
    async fn open(&self, app: &tauri::AppHandle, logout: bool) -> Result<AccountSnapshot, String> {
        let _lane = self.lane.lock().await;
        self.admission(app)?;
        if let Some(window) = app.get_webview_window(WINDOW) {
            window.show().map_err(|_| UNAVAILABLE)?;
            window.set_focus().map_err(|_| UNAVAILABLE)?;
            return Ok(self.snapshot(app));
        }
        let operation = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.operation = state.operation.wrapping_add(1);
            state.checked = None;
            state.error = None;
            state.window_logout = Some(logout);
            if logout {
                self.authority.invalidate();
                state.cookie = None;
                state.authenticated = None;
                state.logging_out = true;
            }
            state.operation
        };
        let entry = if logout {
            "https://www.pixiv.net/logout.php"
        } else {
            "https://accounts.pixiv.net/login?return_to=https%3A%2F%2Fwww.pixiv.net%2F&source=pc&view_type=page"
        };
        let host = self.clone();
        let current = self.clone();
        let popups = crate::browser_identity::LoginPopups::new(
            crate::browser_identity::Service::Pixiv, WINDOW,
            move || !current.stopped.load(Ordering::Acquire)
                && current.state.lock().unwrap_or_else(|e| e.into_inner()).operation == operation,
        );
        let popup_open = popups.clone();
        let popup_app = app.clone();
        let navigation = self.clone();
        let window_result =
            WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(entry.parse().unwrap()))
                .title("Pixiv 로그인")
                .inner_size(720.0, 780.0)
                .min_inner_size(400.0, 450.0)
                .data_directory(Self::profile(app)?)
                .browser_extensions_enabled(false)
                .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
                .on_navigation(move |url| !navigation.stopped.load(Ordering::Acquire)
                    && navigation.state.lock().unwrap_or_else(|e| e.into_inner()).operation == operation
                    && crate::browser_identity::account_url(crate::browser_identity::Service::Pixiv, url))
                .on_new_window(move |url, features| popup_open.open(&popup_app, url, features))
                .on_download(|_, _| false)
                .on_page_load(move |view, payload| {
                    if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
                        || !completion_url(payload.url(), logout)
                    {
                        return;
                    }
                    let expected_url = payload.url().clone();
                    let app = view.app_handle().clone();
                    let host = host.clone();
                    tauri::async_runtime::spawn(async move {
                        let Ok(snapshot) = host.refresh(&app, true).await else {
                            return;
                        };
                        if snapshot.authenticated != Some(!logout) {
                            return;
                        }
                        let current = host
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .operation
                            == operation;
                        if current {
                            if let Some(window) = app.get_webview_window(WINDOW) {
                                if window.url().ok().as_ref() == Some(&expected_url) {
                                    let _ = window.close();
                                }
                            }
                        }
                    });
                })
                .build();
        let window = match window_result {
            Ok(window) => window,
            Err(_) => {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.operation == operation {
                    state.logging_out = false;
                    state.error = Some(UNAVAILABLE.into());
                    state.window_logout = None;
                }
                return Err(UNAVAILABLE.into());
            }
        };
        if let Err(error) = self.admission(app) {
            let _ = window.close();
            return Err(error);
        }
        if self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .operation
            != operation
        {
            let _ = window.close();
            return Err(super::session::STALE.into());
        }
        let host = self.clone();
        let close_app = app.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                popups.close(&close_app);
                let mut state = host.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.operation == operation {
                    state.logging_out = false;
                    state.checked = None;
                    state.window_logout = None;
                }
            }
        });
        Ok(self.snapshot(app))
    }
}
fn apply_observation(
    state: &mut Observed,
    authority: &Authority,
    observation: Result<Option<HeaderValue>, String>,
) {
    match observation {
        Ok(cookie) if !state.logging_out || cookie.is_none() => {
            if state.cookie != cookie || state.authenticated != Some(cookie.is_some()) {
                authority.invalidate();
            }
            state.authenticated = Some(cookie.is_some());
            state.cookie = cookie;
            state.error = None;
            if state.authenticated == Some(false) {
                state.logging_out = false;
            }
        }
        Ok(_) | Err(_) => mark_unknown(state, authority),
    }
}
fn mark_unknown(state: &mut Observed, authority: &Authority) {
    state.cookie = None;
    authority.invalidate();
    state.authenticated = None;
    state.error = Some(VERIFY_FAILED.into());
    // Unverified logout is not success; permit retry rather than an endless busy state.
    state.logging_out = false;
}
fn cookie_scope(domain: Option<&str>, path: Option<&str>) -> bool {
    // `None` is a host-only cookie already selected by cookies_for_url on the exact
    // HTTPS www.pixiv.net origin. Scope is necessary, never proof of authentication.
    domain.is_none_or(|domain| {
        matches!(
            domain.trim_start_matches('.'),
            "pixiv.net" | "www.pixiv.net"
        )
    }) && path.unwrap_or("/") == "/"
}
fn allowed_account_url(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(url.host_str(), Some("accounts.pixiv.net" | "www.pixiv.net"))
        && url.port().is_none_or(|p| p == 443)
        && url.username().is_empty()
        && url.password().is_none()
}
fn completion_url(url: &Url, logout: bool) -> bool {
    allowed_account_url(url)
        && if logout {
            url.host_str() == Some("accounts.pixiv.net") && url.path() == "/login"
                || url.host_str() == Some("www.pixiv.net")
                    && matches!(url.path(), "/" | "/en/" | "/ko/" | "/ja/")
        } else {
            url.host_str() == Some("www.pixiv.net")
                && matches!(url.path(), "/" | "/en/" | "/ko/" | "/ja/")
        }
}
fn verified_payload(value: &Value) -> bool {
    // Exact authenticated endpoint, not a page URL/cookie heuristic. Unknown schema fails closed.
    value["error"].as_bool() == Some(false)
        && value["body"].as_object().is_some_and(|body| {
            body.get("following").is_some_and(|v| v.as_u64().is_some())
                && body
                    .get("mypixivCount")
                    .is_some_and(|v| v.as_u64().is_some())
        })
}
#[derive(Debug, PartialEq, Eq)]
enum SessionProof {
    Authenticated,
    SignedOut,
}
fn session_proof(status: u16, value: &Value) -> Result<SessionProof, String> {
    if status == 200 && verified_payload(value) {
        return Ok(SessionProof::Authenticated);
    }
    // Pixiv also issues PHPSESSID to anonymous visitors. Its exact account
    // endpoint explicitly returns this bounded JSON response for that session;
    // the cookie must not become authorization or block public searches.
    if status == 401
        && value["error"].as_bool() == Some(true)
        && value["body"].as_array().is_some_and(Vec::is_empty)
    {
        return Ok(SessionProof::SignedOut);
    }
    Err(VERIFY_FAILED.into())
}
fn verify_session(header: &HeaderValue) -> Result<SessionProof, String> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 Atsumi/PixivPublicReader")
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| VERIFY_FAILED)?;
    let mut response = client
        .get("https://www.pixiv.net/ajax/user/extra?lang=en")
        .header("Referer", "https://www.pixiv.net/")
        .header("Cookie", header.clone())
        .send()
        .map_err(|_| VERIFY_FAILED)?;
    let status = response.status().as_u16();
    if !matches!(status, 200 | 401) || response.content_length().is_some_and(|n| n > 65536) {
        return Err(VERIFY_FAILED.into());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| VERIFY_FAILED)?;
    if bytes.len() > 65536 {
        return Err(VERIFY_FAILED.into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| VERIFY_FAILED)?;
    session_proof(status, &value)
}
#[tauri::command]
pub async fn pixiv_account_status(
    window: Webview,
    account: State<'_, PixivAccount>,
) -> Result<AccountSnapshot, String> {
    super::require_main(&window)?;
    account.refresh(window.app_handle(), false).await
}
#[tauri::command]
pub async fn pixiv_account_login(
    window: Webview,
    account: State<'_, PixivAccount>,
) -> Result<AccountSnapshot, String> {
    super::require_main(&window)?;
    account.open(window.app_handle(), false).await
}
#[tauri::command]
pub async fn pixiv_account_logout(
    window: Webview,
    account: State<'_, PixivAccount>,
) -> Result<AccountSnapshot, String> {
    super::require_main(&window)?;
    account.open(window.app_handle(), true).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anonymous_account_proof_is_distinct_from_unknown_or_authenticated() {
        let guest = serde_json::json!({"error":true,"message":"fixture","body":[]});
        assert_eq!(session_proof(401, &guest), Ok(SessionProof::SignedOut));
        let authenticated =
            serde_json::json!({"error":false,"body":{"following":0,"mypixivCount":0}});
        assert_eq!(
            session_proof(200, &authenticated),
            Ok(SessionProof::Authenticated)
        );
        for status in [200, 302, 400, 403, 429, 500, 503] {
            assert!(session_proof(status, &guest).is_err());
        }
        for value in [
            serde_json::json!({}),
            serde_json::json!({"error":false,"body":[]}),
            serde_json::json!({"error":"true","body":[]}),
            serde_json::json!({"error":true,"body":{}}),
            serde_json::json!({"error":true,"body":["fixture"]}),
            serde_json::json!({"error":true,"body":null}),
            authenticated,
        ] {
            assert!(session_proof(401, &value).is_err());
        }
    }
    #[test]
    fn repeated_guest_proof_keeps_public_work_and_clears_no_login_error() {
        let authority = Authority::default();
        let mut state = Observed::default();
        mark_unknown(&mut state, &authority);
        apply_observation(&mut state, &authority, Ok(None));
        assert_eq!(state.authenticated, Some(false));
        assert!(state.cookie.is_none());
        assert!(state.error.is_none());
        let revision = authority.revision();
        let lease = authority.lease(state.cookie.clone());
        assert!(!lease.authenticated());
        apply_observation(&mut state, &authority, Ok(None));
        assert_eq!(authority.revision(), revision);
        assert!(lease.check().is_ok());
    }
    #[test]
    fn account_transitions_revoke_previous_work_without_reusing_guest_cookies() {
        let authority = Authority::default();
        let mut state = Observed::default();
        apply_observation(&mut state, &authority, Ok(None));
        let public = authority.lease(state.cookie.clone());
        apply_observation(&mut state, &authority, Ok(session_header("123_fixture")));
        assert!(public.check().is_err());
        assert_eq!(state.authenticated, Some(true));
        let authenticated = authority.lease(state.cookie.clone());
        assert!(authenticated.authenticated());
        state.logging_out = true;
        apply_observation(&mut state, &authority, Ok(None));
        assert!(authenticated.check().is_err());
        assert_eq!(state.authenticated, Some(false));
        assert!(state.cookie.is_none());
        assert!(!state.logging_out);
        assert!(!authority.lease(state.cookie.clone()).authenticated());
    }
    #[test]
    fn failed_or_not_yet_signed_out_observation_never_confirms_logout() {
        let authority = Authority::default();
        let mut state = Observed::default();
        for observation in [Ok(session_header("123_fixture")), Err(VERIFY_FAILED.into())] {
            apply_observation(&mut state, &authority, Ok(session_header("123_fixture")));
            let authenticated = authority.lease(state.cookie.clone());
            state.logging_out = true;
            apply_observation(&mut state, &authority, observation);
            assert!(authenticated.check().is_err());
            assert_eq!(state.authenticated, None);
            assert!(state.cookie.is_none());
            assert!(state.error.is_some());
        }
    }
    #[test]
    fn exact_origin_host_only_cookie_is_scoped_but_not_authentication_proof() {
        for domain in [
            None,
            Some("pixiv.net"),
            Some(".pixiv.net"),
            Some("www.pixiv.net"),
        ] {
            assert!(cookie_scope(domain, Some("/")));
        }
        for domain in [
            Some(""),
            Some("accounts.pixiv.net"),
            Some("www.pixiv.net.evil.test"),
            Some("pximg.net"),
        ] {
            assert!(!cookie_scope(domain, Some("/")));
        }
        assert!(!cookie_scope(None, Some("/ajax")));
        assert!(!verified_payload(
            &serde_json::json!({"PHPSESSID":"fixture"})
        ));
        assert!(completion_url(
            &Url::parse("https://www.pixiv.net/ko/").unwrap(),
            true
        ));
    }
    #[test]
    fn every_unknown_observation_revokes_prior_work_without_claiming_logout() {
        let authority = Authority::default();
        let mut state = Observed::default();
        let lease = authority.lease(None);
        mark_unknown(&mut state, &authority);
        assert!(lease.check().is_err());
        assert_eq!(state.authenticated, None);
        assert!(state.cookie.is_none());
        assert!(state.error.is_some());
        let lease = authority.lease(None);
        mark_unknown(&mut state, &authority);
        assert!(lease.check().is_err());
    }
    #[test]
    fn security_pages_never_count_as_completion() {
        for url in [
            "https://accounts.pixiv.net/login",
            "https://accounts.pixiv.net/two-factor-authentication",
            "https://accounts.google.com/",
            "https://consent.google.com/",
            "https://www.youtube.com/",
            "http://www.pixiv.net/",
            "https://www.pixiv.net.evil.test/",
        ] {
            assert!(!completion_url(&Url::parse(url).unwrap(), false));
        }
        assert!(completion_url(
            &Url::parse("https://www.pixiv.net/ko/").unwrap(),
            false
        ));
    }
    #[test]
    fn missing_or_failed_auth_payload_is_never_success() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"error":true,"body":[]}),
            serde_json::json!({"error":false,"body":{}}),
            serde_json::json!({"error":false,"body":{"following":"1","mypixivCount":0}}),
        ] {
            assert!(!verified_payload(&value));
        }
        assert!(verified_payload(
            &serde_json::json!({"error":false,"body":{"following":0,"mypixivCount":0}})
        ));
        let snapshot = AccountSnapshot {
            authenticated: None,
            open: true,
            busy: false,
            revision: 1,
            error: None,
        };
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 5);
        assert!(json.get("cookie").is_none());
    }
}
