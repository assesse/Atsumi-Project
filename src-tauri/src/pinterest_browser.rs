//! Official Pinterest discovery surface. Remote pages receive no app IPC capability.
//! Only a trusted-main, explicit action may import the current public pin identity.
use std::sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State, Webview, WebviewUrl};
use crate::interface::commands::AppState;

const LABEL: &str = "pinterest-official";
const STALE: &str = "이전 Pinterest 탐색 요청입니다. 현재 화면에서 다시 선택해 주세요.";
const UNAVAILABLE: &str = "Pinterest 공식 탐색 창을 열지 못했습니다. 다시 시도해 주세요.";
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct PinterestBrowser {
    inner: Arc<Mutex<BrowserState>>,
    lane: Arc<tokio::sync::Mutex<()>>,
    paint: Arc<AtomicU64>,
}
#[derive(Default)]
struct BrowserState {
    epoch: u64,
    sequence: u64,
    open: bool,
    loading: Option<Instant>,
    error: Option<String>,
    viewport: Option<BrowserViewport>,
    navigation_target: Option<String>,
    login_popups: Option<crate::browser_identity::LoginPopups>,
}
impl BrowserState {
    fn clear_loading(&mut self) {
        self.loading = None;
        self.navigation_target = None;
    }
    fn expire_loading(&mut self, now: Instant) {
        if self.loading.is_some_and(|start| now.saturating_duration_since(start) >= LOAD_TIMEOUT) {
            self.clear_loading();
            self.error = Some("공식 페이지 응답이 늦습니다. 로그인 또는 네트워크 상태를 확인하고 새로고침해 주세요.".into());
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserViewport {
    pub epoch: u64,
    pub sequence: u64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    epoch: u64,
    open: bool,
    loading: bool,
    current_pin_id: Option<String>,
    error: Option<String>,
}
fn lock(host: &PinterestBrowser) -> MutexGuard<'_, BrowserState> {
    host.inner.lock().unwrap_or_else(|poison| poison.into_inner())
}
fn allowed_url(url: &tauri::Url) -> bool {
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        && url.port().is_none()
        && matches!(url.host_str(), Some("pinterest.com" | "www.pinterest.com" | "kr.pinterest.com"))
}
fn selected_pin(url: &tauri::Url) -> Option<String> {
    if !allowed_url(url) { return None; }
    let path = url.path().strip_prefix("/pin/")?.trim_end_matches('/');
    if path.contains('/') { return None; }
    let id = path.rsplit_once("--").map_or(path, |(_, id)| id);
    ((5..=25).contains(&id.len()) && !id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}
fn search_url(query: &str) -> Result<tauri::Url, String> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 200 || query.chars().any(char::is_control) {
        return Err("검색어는 1~200자로 입력해 주세요.".into());
    }
    let mut url: tauri::Url = "https://www.pinterest.com/search/pins/".parse().map_err(|_| UNAVAILABLE)?;
    url.query_pairs_mut().append_pair("q", query);
    Ok(url)
}
fn same_navigation_target(target: &str, candidate: &tauri::Url) -> bool {
    let Ok(expected) = tauri::Url::parse(target) else { return false; };
    if !allowed_url(&expected) || !allowed_url(candidate) { return false; }
    expected == *candidate || (
        expected.path() == "/search/pins/" && candidate.path() == expected.path()
        && expected.query() == candidate.query()
        && expected.fragment() == candidate.fragment()
    )
}
fn validate_viewport(viewport: &BrowserViewport) -> Result<(), String> {
    if [viewport.x, viewport.y, viewport.width, viewport.height].iter().any(|n| !n.is_finite() || *n < 0.0 || *n > 32768.0)
        || viewport.visible && (viewport.width < 1.0 || viewport.height < 1.0) {
        return Err("Pinterest 탐색 화면 크기가 올바르지 않습니다.".into());
    }
    Ok(())
}
fn require_main(window: &Webview) -> Result<(), String> {
    let url = window.url().map_err(|_| UNAVAILABLE)?;
    if !crate::frontend_origin::trusted_main(window.label(), &url, cfg!(debug_assertions), window.app_handle().config().build.dev_url.as_ref()) {
        return Err("Pinterest 탐색은 Atsumi 기본 창에서만 사용할 수 있습니다.".into());
    }
    Ok(())
}
impl PinterestBrowser {
    fn begin(&self, url: Option<&tauri::Url>) -> u64 {
        let mut state = lock(self);
        state.epoch = state.epoch.saturating_add(1);
        state.sequence = 0;
        state.open = true;
        state.loading = Some(Instant::now());
        state.error = None;
        state.viewport = None;
        state.navigation_target = url.map(ToString::to_string);
        self.paint.fetch_add(1, Ordering::AcqRel);
        state.epoch
    }
    fn current(&self, epoch: u64) -> bool { let state = lock(self); state.open && state.epoch == epoch }
    fn page_load(&self, epoch: u64, current_url: &tauri::Url, event_url: &tauri::Url, started: bool, now: Instant) -> bool {
        if !allowed_url(event_url) || current_url != event_url { return false; }
        let mut state = lock(self);
        if !state.open || state.epoch != epoch { return false; }
        if state.navigation_target.as_deref().is_some_and(|target| !same_navigation_target(target, event_url)) { return false; }
        if started {
            // Keep ownership through Finished, including regional redirects. A
            // redirect must neither release the old-query fence nor reset its deadline.
            if state.navigation_target.is_none() { state.navigation_target = Some(event_url.to_string()); }
            state.loading.get_or_insert(now);
        } else {
            state.clear_loading();
        }
        true
    }
    fn update_viewport(&self, viewport: BrowserViewport) -> Result<u64, String> {
        validate_viewport(&viewport)?;
        let mut state = lock(self);
        if !state.open || state.epoch != viewport.epoch || viewport.sequence <= state.sequence { return Err(STALE.into()); }
        state.sequence = viewport.sequence;
        state.viewport = Some(viewport);
        Ok(self.paint.fetch_add(1, Ordering::AcqRel) + 1)
    }
    /// Fail closed on main document reload/shutdown; this does not clear cookies.
    pub fn detach(&self, app: &tauri::AppHandle) {
        let _ = self.detach_epoch(app, None);
    }
    fn invalidate(&self, expected_epoch: Option<u64>) -> Result<(u64, Option<crate::browser_identity::LoginPopups>), String> {
        let mut state = lock(self);
        if expected_epoch.is_some_and(|epoch| !state.open || state.epoch != epoch) { return Err(STALE.into()); }
        state.epoch = state.epoch.saturating_add(1);
        state.open = false;
        state.clear_loading();
        state.viewport = None;
        let popups = state.login_popups.take();
        let revision = self.paint.fetch_add(1, Ordering::AcqRel) + 1;
        Ok((revision, popups))
    }
    fn detach_epoch(&self, app: &tauri::AppHandle, expected_epoch: Option<u64>) -> Result<(), String> {
        let (revision, popups) = self.invalidate(expected_epoch)?;
        if let Some(popups) = popups { popups.close(app); }
        if let Some(view) = app.get_webview(LABEL) { let _ = paint_surface(&view, self.paint.clone(), revision, None); }
        Ok(())
    }
    fn failed_open(&self, epoch: u64, error: String) -> Option<crate::browser_identity::LoginPopups> {
        let mut state = lock(self);
        if !state.open || state.epoch != epoch { return None; }
        state.clear_loading();
        state.error = Some(error);
        state.login_popups.take()
    }
    fn snapshot(&self, app: &tauri::AppHandle, epoch: u64) -> Result<BrowserSnapshot, String> {
        if !self.current(epoch) { return Err(STALE.into()); }
        let url = app.get_webview(LABEL).and_then(|view| view.url().ok());
        let mut state = lock(self);
        if state.epoch != epoch || !state.open { return Err(STALE.into()); }
        state.expire_loading(Instant::now());
        Ok(BrowserSnapshot { epoch, open: state.open, loading: state.loading.is_some(), current_pin_id: url.as_ref().and_then(selected_pin), error: state.error.clone() })
    }
    fn open_native(&self, app: &tauri::AppHandle, epoch: u64, url: tauri::Url) -> Result<(), String> {
        if !self.current(epoch) { return Err(STALE.into()); }
        let weak = Arc::downgrade(&self.inner);
        let popups = crate::browser_identity::LoginPopups::new(crate::browser_identity::Service::Pinterest, LABEL, move || {
            weak.upgrade().is_some_and(|inner| {
                let state = inner.lock().unwrap_or_else(|p| p.into_inner());
                state.open && state.epoch == epoch
            })
        });
        let old_popups = {
            let mut state = lock(self);
            if !state.open || state.epoch != epoch { return Err(STALE.into()); }
            state.login_popups.replace(popups)
        };
        if let Some(popups) = old_popups { popups.close(app); }
        let view = if let Some(view) = app.get_webview(LABEL) { view } else {
            let parent = app.get_window("main").ok_or(UNAVAILABLE)?;
            let profile = crate::browser_identity::profile(app).map_err(|_| UNAVAILABLE)?;
            let navigation = self.clone();
            let lifecycle = self.clone();
            let popups = self.clone();
            let popup_app = app.clone();
            let builder = tauri::webview::WebviewBuilder::new(LABEL, WebviewUrl::External("about:blank".parse().map_err(|_| UNAVAILABLE)?))
                .data_directory(profile).focused(false).browser_extensions_enabled(false)
                .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
                .on_navigation(move |url| {
                    if allowed_url(url) { return true; }
                    // Only the builder's inert initial document is accepted.
                    if url.as_str() == "about:blank" { return true; }
                    lock(&navigation).error = Some("Pinterest 외부 페이지는 이 탐색 창에서 열 수 없습니다.".into());
                    false
                })
                .on_new_window(move |url, features| {
                    let owner = lock(&popups).login_popups.clone();
                    owner.map_or(tauri::webview::NewWindowResponse::Deny, |owner| owner.open(&popup_app, url, features))
                })
                .on_download(|_, _| false)
                .on_page_load(move |view, payload| {
                    // Fence a concurrent begin while obtaining the native URL.
                    // No remote script or metadata decides document identity.
                    let epoch = lock(&lifecycle).epoch;
                    let Ok(current_url) = view.url() else { return; };
                    lifecycle.page_load(epoch, &current_url, payload.url(),
                        matches!(payload.event(), tauri::webview::PageLoadEvent::Started), Instant::now());
                });
            parent.add_child(builder, tauri::LogicalPosition::new(-10_000.0, -10_000.0), tauri::LogicalSize::new(1.0, 1.0)).map_err(|_| UNAVAILABLE)?
        };
        let revision = self.paint.load(Ordering::Acquire);
        paint_surface(&view, self.paint.clone(), revision, None)?;
        if !self.current(epoch) { return Err(STALE.into()); }
        view.navigate(url).map_err(|_| UNAVAILABLE)?;
        Ok(())
    }
}

/// Apply identity-fenced visibility on the UI thread, not a delayed renderer show.
#[cfg(windows)]
fn paint_surface(view: &Webview, revision: Arc<AtomicU64>, expected: u64, viewport: Option<BrowserViewport>) -> Result<(), String> {
    use windows::{core::Interface, Win32::{Foundation::{HWND, RECT}, UI::WindowsAndMessaging::{SetWindowPos, ShowWindow, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER}}};
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    let scale = view.window().scale_factor().map_err(|_| UNAVAILABLE)?;
    let size = view.window().inner_size().map_err(|_| UNAVAILABLE)?;
    view.with_webview(move |platform| unsafe {
        if revision.load(Ordering::Acquire) != expected { return; }
        let controller = platform.controller();
        let mut hwnd = HWND::default();
        if controller.ParentWindow(&mut hwnd).is_err() { return; }
        // This surface is for image discovery, never a background media player.
        if let Ok(audio) = controller.CoreWebView2().and_then(|core| core.cast::<ICoreWebView2_8>()) { let _ = audio.SetIsMuted(true); }
        let visible = viewport.as_ref().is_some_and(|v| v.visible);
        let _ = controller.SetIsVisible(false);
        let _ = ShowWindow(hwnd, SW_HIDE);
        if !visible { return; }
        let v = viewport.unwrap();
        let x = (v.x * scale).round().clamp(0.0, size.width as f64) as i32;
        let y = (v.y * scale).round().clamp(0.0, size.height as f64) as i32;
        let w = (v.width * scale).round().min(size.width as f64 - x as f64).max(0.0) as i32;
        let h = (v.height * scale).round().min(size.height as f64 - y as f64).max(0.0) as i32;
        if w < 1 || h < 1 { return; }
        if SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER).is_err()
            || controller.SetBounds(RECT { left: 0, top: 0, right: w, bottom: h }).is_err() { return; }
        if revision.load(Ordering::Acquire) != expected { return; }
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        let _ = controller.SetIsVisible(true);
        if revision.load(Ordering::Acquire) != expected { let _ = controller.SetIsVisible(false); let _ = ShowWindow(hwnd, SW_HIDE); }
    }).map_err(|_| UNAVAILABLE.to_owned())
}
#[cfg(not(windows))]
fn paint_surface(view: &Webview, _: Arc<AtomicU64>, _: u64, _: Option<BrowserViewport>) -> Result<(), String> {
    let _ = view.hide();
    Err("공식 Pinterest 탐색은 현재 Windows 앱에서 지원합니다.".into())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_browser_open(window: Webview, state: State<'_, AppState>, browser: State<'_, PinterestBrowser>, query: String) -> Result<BrowserSnapshot, String> {
    require_main(&window)?;
    let url = search_url(&query)?;
    let host = browser.inner().clone();
    let epoch = state.begin_platform_work(|| Ok(host.begin(Some(&url))))?;
    let app = window.app_handle().clone();
    let _lane = host.lane.lock().await;
    let worker = host.clone(); let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || worker.open_native(&worker_app, epoch, url)).await.unwrap_or_else(|_| Err(UNAVAILABLE.into()));
    if let Err(error) = result {
        if let Some(owner) = host.failed_open(epoch, error.clone()) { owner.close(&app); }
        return Err(error);
    }
    host.snapshot(&app, epoch)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_browser_snapshot(window: Webview, browser: State<'_, PinterestBrowser>, epoch: u64) -> Result<BrowserSnapshot, String> {
    require_main(&window)?;
    browser.snapshot(window.app_handle(), epoch)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_browser_set_viewport(window: Webview, browser: State<'_, PinterestBrowser>, viewport: BrowserViewport) -> Result<(), String> {
    require_main(&window)?;
    let revision = browser.update_viewport(viewport.clone())?;
    if let Some(view) = window.app_handle().get_webview(LABEL) { paint_surface(&view, browser.paint.clone(), revision, Some(viewport))?; }
    Ok(())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_browser_select_current(window: Webview, browser: State<'_, PinterestBrowser>, epoch: u64, expected_pin_id: String) -> Result<String, String> {
    require_main(&window)?;
    let snapshot = browser.snapshot(window.app_handle(), epoch)?;
    let id = snapshot.current_pin_id.ok_or("공식 검색에서 가져올 핀을 먼저 열어 주세요.")?;
    if id != expected_pin_id || !browser.current(epoch) { return Err(STALE.into()); }
    let current = window.app_handle().get_webview(LABEL).and_then(|view| view.url().ok()).as_ref().and_then(selected_pin);
    if current.as_deref() != Some(&id) || !browser.current(epoch) { return Err(STALE.into()); }
    Ok(id)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn pinterest_browser_action(window: Webview, browser: State<'_, PinterestBrowser>, epoch: u64, action: String) -> Result<(), String> {
    require_main(&window)?;
    if !matches!(action.as_str(), "back" | "forward" | "reload" | "stop" | "close") { return Err("지원하지 않는 탐색 동작입니다.".into()); }
    if !browser.current(epoch) { return Err(STALE.into()); }
    let app = window.app_handle().clone();
    if action == "close" {
        browser.detach_epoch(&app, Some(epoch))?;
        // Retain the isolated page context, but no visible surface or active session.
        return Ok(());
    }
    let view = app.get_webview(LABEL).ok_or(UNAVAILABLE)?;
    #[cfg(windows)]
    {
        let host = browser.inner().clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| unsafe {
            let result = (|| -> Result<(), String> {
                if !host.current(epoch) { return Err(STALE.into()); }
                let core = platform.controller().CoreWebView2().map_err(|_| UNAVAILABLE)?;
                match action.as_str() { "back" => core.GoBack(), "forward" => core.GoForward(), "reload" => core.Reload(), "stop" => core.Stop(), _ => unreachable!() }.map_err(|_| "탐색 동작을 완료하지 못했습니다.".to_owned())?;
                let mut s = lock(&host);
                if !s.open || s.epoch != epoch { return Err(STALE.into()); }
                s.error = None; if action == "stop" { s.clear_loading(); }
                Ok(())
            })();
            let _ = tx.send(result);
        }).map_err(|_| UNAVAILABLE)?;
        tokio::time::timeout(Duration::from_secs(3), rx).await.map_err(|_| UNAVAILABLE)?.map_err(|_| UNAVAILABLE)?
    }
    #[cfg(not(windows))]
    { let _ = view; Err(UNAVAILABLE.into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_official_urls_only_and_pin_identity_is_not_remote_metadata() {
        assert_eq!(selected_pin(&"https://www.pinterest.com/pin/tree--1076430748452447388/?q=x".parse().unwrap()).as_deref(), Some("1076430748452447388"));
        for input in ["http://www.pinterest.com/pin/12345/", "https://www.pinterest.com.evil.test/pin/12345/", "https://user@www.pinterest.com/pin/12345/", "https://www.pinterest.com:8080/pin/12345/", "https://127.0.0.1/pin/12345/", "tauri://localhost/pin/12345", "https://www.pinterest.com/pin/12345/extra", "https://www.pinterest.com/pin/00000/"] { assert!(selected_pin(&input.parse().unwrap()).is_none(), "{input}"); }
        assert!(!allowed_url(&"https://accounts.google.com/".parse().unwrap()));
    }
    #[test]
    fn search_query_is_encoded_bounded_and_never_a_navigation_url() {
        let url = search_url("풍경 & x=https://evil.test").unwrap();
        assert_eq!(url.host_str(), Some("www.pinterest.com"));
        assert_eq!(url.query_pairs().count(), 1);
        for query in ["".to_owned(), "x".repeat(201), "a\nb".to_owned()] { assert!(search_url(&query).is_err()); }
    }
    #[test]
    fn old_epoch_and_out_of_order_show_cannot_replace_new_hide() {
        let host = PinterestBrowser::default(); let first = host.begin(None);
        let mut viewport = BrowserViewport { epoch: first, sequence: 1, x: 0., y: 0., width: 300., height: 200., visible: true };
        let show = host.update_viewport(viewport.clone()).unwrap();
        viewport.sequence = 2; viewport.visible = false;
        let hide = host.update_viewport(viewport.clone()).unwrap(); assert!(hide > show);
        viewport.sequence = 1; viewport.visible = true; assert!(host.update_viewport(viewport.clone()).is_err());
        host.begin(None); viewport.sequence = 3; assert!(host.update_viewport(viewport).is_err());
    }
    #[test]
    fn geometry_rejects_non_finite_negative_and_empty_visible_rectangles() {
        let mut v = BrowserViewport { epoch: 1, sequence: 1, x: 0., y: 0., width: 300., height: 200., visible: true };
        assert!(validate_viewport(&v).is_ok()); v.x = f64::NAN; assert!(validate_viewport(&v).is_err()); v.x = -1.; assert!(validate_viewport(&v).is_err()); v.x = 0.; v.width = 0.; assert!(validate_viewport(&v).is_err()); v.visible = false; assert!(validate_viewport(&v).is_ok());
    }
    #[test]
    fn late_close_checks_epoch_atomically_and_cannot_invalidate_new_search() {
        let host = PinterestBrowser::default();
        let obsolete = host.begin(None);
        let current = host.begin(None);
        let revision = host.paint.load(Ordering::Acquire);
        assert!(host.invalidate(Some(obsolete)).is_err());
        assert!(host.current(current));
        assert_eq!(host.paint.load(Ordering::Acquire), revision);
        assert!(host.invalidate(Some(current)).is_ok());
        assert!(!host.current(current));
        assert!(host.invalidate(Some(current)).is_err());
        // Main reload and shutdown must still invalidate unconditionally.
        let next = host.begin(None);
        assert!(host.invalidate(None).is_ok());
        assert!(!host.current(next));
    }
    #[test]
    fn obsolete_close_or_open_failure_cannot_take_a_new_popup_owner() {
        use crate::browser_identity::{LoginPopups, Service};
        let host = PinterestBrowser::default();
        let first = host.begin(None);
        lock(&host).login_popups = Some(LoginPopups::new(Service::Pinterest, LABEL, || true));
        let (_, detached_owner) = host.invalidate(Some(first)).unwrap();
        assert!(detached_owner.is_some());
        assert!(lock(&host).login_popups.is_none());
        let next = host.begin(None);
        lock(&host).login_popups = Some(LoginPopups::new(Service::Pinterest, LABEL, || true));
        assert!(host.invalidate(Some(first)).is_err());
        assert!(host.failed_open(first, "stale".into()).is_none());
        assert!(lock(&host).login_popups.is_some());
        assert!(lock(&host).error.is_none());
        assert!(host.failed_open(next, "current failure".into()).is_some());
        assert!(lock(&host).login_popups.is_none());
        assert!(lock(&host).loading.is_none());
        assert_eq!(lock(&host).error.as_deref(), Some("current failure"));
    }
    #[test]
    fn regional_search_finished_can_settle_without_an_initial_started_callback() {
        let initial = search_url("forest landscape").unwrap();
        for domain in ["pinterest.com", "www.pinterest.com", "kr.pinterest.com"] {
            let host = PinterestBrowser::default();
            let epoch = host.begin(Some(&initial));
            let mut regional = initial.clone(); regional.set_host(Some(domain)).unwrap();
            assert!(host.page_load(epoch, &regional, &regional, false, Instant::now()));
            let state = lock(&host);
            assert!(state.loading.is_none());
            assert!(state.navigation_target.is_none());
            assert!(state.error.is_none());
            // Search completion does not manufacture a selected pin identity.
            assert!(selected_pin(&regional).is_none());
        }
    }
    #[test]
    fn regional_equivalence_never_changes_query_path_pin_owner_or_trusted_origin() {
        let initial = search_url("forest landscape").unwrap();
        for input in [
            "https://kr.pinterest.com/search/pins/?q=forest",
            "https://kr.pinterest.com/search/pins/?q=forest+landscape&extra=1",
            "https://kr.pinterest.com/search/pins/?q=forest+landscape#other",
            "https://kr.pinterest.com/search/boards/?q=forest+landscape",
            "https://kr.pinterest.com/pin/1076430748452447388/",
            "https://kr.pinterest.com/login/?q=forest+landscape",
            "https://kr.pinterest.com.evil.test/search/pins/?q=forest+landscape",
            "http://kr.pinterest.com/search/pins/?q=forest+landscape",
            "https://user@kr.pinterest.com/search/pins/?q=forest+landscape",
            "https://kr.pinterest.com:8443/search/pins/?q=forest+landscape",
        ] {
            let candidate = input.parse().unwrap();
            let host = PinterestBrowser::default(); let epoch = host.begin(Some(&initial));
            assert!(!same_navigation_target(initial.as_str(), &candidate), "{input}");
            assert!(!host.page_load(epoch, &candidate, &candidate, false, Instant::now()), "{input}");
            assert!(lock(&host).loading.is_some());
        }
        let pin: tauri::Url = "https://www.pinterest.com/pin/1076430748452447388/".parse().unwrap();
        assert!(same_navigation_target(pin.as_str(), &pin));
        for input in ["https://kr.pinterest.com/pin/1076430748452447388/", "https://www.pinterest.com/pin/123456789/"] {
            assert!(!same_navigation_target(pin.as_str(), &input.parse().unwrap()));
        }
        let host = PinterestBrowser::default(); let epoch = host.begin(Some(&initial));
        assert!(!host.page_load(epoch, &pin, &initial, false, Instant::now()));
        assert!(lock(&host).loading.is_some());
    }
    #[test]
    fn stale_started_and_finished_keep_the_new_query_and_epoch_fence() {
        let host = PinterestBrowser::default();
        let old = search_url("forest").unwrap(); let old_epoch = host.begin(Some(&old));
        let current = search_url("mountains").unwrap(); let epoch = host.begin(Some(&current));
        let now = Instant::now();
        let start = lock(&host).loading.unwrap();
        assert!(!host.page_load(old_epoch, &current, &current, true, now));
        assert!(!host.page_load(old_epoch, &current, &current, false, now));
        assert!(!host.page_load(epoch, &old, &old, true, now));
        assert!(host.page_load(epoch, &current, &current, true, now));
        // Started must not discard the target fence before completion.
        assert_eq!(lock(&host).navigation_target.as_deref(), Some(current.as_str()));
        assert!(!host.page_load(epoch, &old, &old, true, now));
        assert!(!host.page_load(epoch, &old, &old, false, now));
        assert_eq!(lock(&host).loading, Some(start));
        let mut regional = current.clone(); regional.set_host(Some("kr.pinterest.com")).unwrap();
        assert!(host.page_load(epoch, &regional, &regional, false, now));
        assert!(lock(&host).loading.is_none());
    }
    #[test]
    fn regional_started_cannot_extend_deadline_and_timeout_or_stop_releases_target() {
        let host = PinterestBrowser::default(); let initial = search_url("forest landscape").unwrap();
        let epoch = host.begin(Some(&initial)); let start = Instant::now();
        lock(&host).loading = Some(start);
        let mut regional = initial.clone(); regional.set_host(Some("kr.pinterest.com")).unwrap();
        assert!(host.page_load(epoch, &regional, &regional, true, start + Duration::from_secs(29)));
        {
            let mut state = lock(&host);
            assert_eq!(state.loading, Some(start));
            state.expire_loading(start + Duration::from_millis(29_999));
            assert!(state.loading.is_some()); assert!(state.error.is_none());
            state.expire_loading(start + LOAD_TIMEOUT);
            assert!(state.loading.is_none()); assert!(state.navigation_target.is_none());
            assert!(state.error.as_deref().unwrap().contains("응답이 늦습니다"));
        }
        let pin: tauri::Url = "https://kr.pinterest.com/pin/1076430748452447388/".parse().unwrap();
        assert!(host.page_load(epoch, &pin, &pin, true, start + Duration::from_secs(31)));
        assert_eq!(lock(&host).navigation_target.as_deref(), Some(pin.as_str()));
        lock(&host).clear_loading(); // The same operation used by explicit Stop.
        assert!(lock(&host).navigation_target.is_none());
        assert!(host.page_load(epoch, &regional, &regional, true, start + Duration::from_secs(32)));
        assert!(host.page_load(epoch, &regional, &regional, false, start + Duration::from_secs(33)));
        assert!(lock(&host).loading.is_none());
    }
}
