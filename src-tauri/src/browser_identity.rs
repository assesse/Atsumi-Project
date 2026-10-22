//! Browser identity shared only by YouTube, Pinterest and Pixiv.
//! The historical YouTube directory is deliberately retained: no credential
//! copying/migration, no profile wipe, and no sharing of service authorization.
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

#[path = "youtube/original_account_regions.rs"]
mod google_regions;

const PROFILE: &str = "youtube-original-profile";
// WebView2 shares a browser process for this profile. Every surface must pass
// identical environment options, regardless of which service opens first.
pub(crate) const BROWSER_ARGS: &str = "--autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";
const MAX_POPUPS: usize = 4;
const MAX_DEPTH: u8 = 2;

pub(crate) fn profile(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app.path().app_data_dir().map_err(|_| "로그인 저장소를 준비하지 못했습니다.")?.join(PROFILE);
    std::fs::create_dir_all(&path).map_err(|_| "로그인 저장소를 준비하지 못했습니다.")?;
    Ok(path)
}

#[derive(Clone, Copy)]
pub(crate) enum Service { Pinterest, Pixiv, X }

fn https(url: &Url) -> bool {
    url.scheme() == "https" && url.port().is_none()
        && url.username().is_empty() && url.password().is_none()
}

/// Account-window navigation only. Never use this for queries, IPC, completion,
/// downloads or content/player surfaces. Google tokens remain in the browser.
pub(crate) fn account_url(service: Service, url: &Url) -> bool {
    if !https(url) { return false; }
    let service_host = match service {
        Service::Pinterest => matches!(url.host_str(), Some("pinterest.com" | "www.pinterest.com" | "kr.pinterest.com")),
        Service::Pixiv => matches!(url.host_str(), Some("accounts.pixiv.net" | "www.pixiv.net")),
        Service::X => matches!(url.host_str(), Some("x.com" | "www.x.com" | "twitter.com" | "www.twitter.com")),
    };
    service_host || matches!(url.host_str(), Some("accounts.google.com" | "consent.google.com"))
        || url.path() == "/accounts/SetSID"
            && url.host_str().is_some_and(google_regions::is_known_regional_account_host)
}

#[derive(Default)]
struct PopupState { closed: bool, labels: HashSet<String> }
impl PopupState {
    fn reserve(&mut self, label: String, depth: u8) -> bool {
        if self.closed || depth >= MAX_DEPTH || self.labels.len() >= MAX_POPUPS { return false; }
        self.labels.insert(label)
    }
    fn close(&mut self) -> Vec<String> {
        self.closed = true;
        self.labels.drain().collect()
    }
}

/// One parent account flow owns the popup family. Create preserves the official
/// opener via window_features (including WebView2's original environment).
#[derive(Clone)]
pub(crate) struct LoginPopups {
    service: Service,
    parent: &'static str,
    state: Arc<Mutex<PopupState>>,
    current: Arc<dyn Fn() -> bool + Send + Sync>,
    directory: Option<PathBuf>,
}
impl LoginPopups {
    pub(crate) fn new(service: Service, parent: &'static str, current: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self { service, parent, state: Arc::default(), current: Arc::new(current), directory: None }
    }
    pub(crate) fn with_profile(mut self, directory: PathBuf) -> Self { self.directory = Some(directory); self }
    fn active(&self, app: &AppHandle) -> bool {
        (self.current)() && !self.state.lock().unwrap_or_else(|p| p.into_inner()).closed
            && app.get_webview(self.parent).is_some()
            && app.try_state::<crate::interface::AppState>()
                .is_some_and(|state| state.begin_platform_work(|| Ok(())).is_ok())
    }
    pub(crate) fn close(&self, app: &AppHandle) {
        let labels = self.state.lock().unwrap_or_else(|p| p.into_inner()).close();
        for label in labels {
            if let Some(window) = app.get_webview_window(&label) { let _ = window.close(); }
        }
    }
    pub(crate) fn open(&self, app: &AppHandle, url: Url, features: tauri::webview::NewWindowFeatures) -> tauri::webview::NewWindowResponse<tauri::Wry> {
        self.open_at(app, url, features, 0)
    }
    fn open_at(&self, app: &AppHandle, url: Url, features: tauri::webview::NewWindowFeatures, depth: u8) -> tauri::webview::NewWindowResponse<tauri::Wry> {
        use tauri::webview::NewWindowResponse;
        if !self.active(app) || !(url.as_str() == "about:blank" || account_url(self.service, &url)) {
            return NewWindowResponse::Deny;
        }
        let label = format!("{}-social-{}", self.parent, uuid::Uuid::new_v4().simple());
        if !self.state.lock().unwrap_or_else(|p| p.into_inner()).reserve(label.clone(), depth) {
            return NewWindowResponse::Deny;
        }
        let Ok(directory) = self.directory.clone().map(Ok).unwrap_or_else(|| profile(app)) else {
            self.state.lock().unwrap_or_else(|p| p.into_inner()).labels.remove(&label);
            return NewWindowResponse::Deny;
        };
        let navigation = self.clone();
        let navigation_app = app.clone();
        let nested = self.clone();
        let nested_app = app.clone();
        let initial = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let result = WebviewWindowBuilder::new(app, &label, WebviewUrl::External("about:blank".parse().unwrap()))
            .title(match self.service { Service::Pinterest => "Pinterest · Google 로그인", Service::Pixiv => "Pixiv · Google 로그인", Service::X => "X · Google 로그인" })
            .data_directory(directory)
            .browser_extensions_enabled(false)
            .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
            .window_features(features)
            .inner_size(720.0, 780.0)
            .min_inner_size(400.0, 450.0)
            .on_navigation(move |next| {
                use std::sync::atomic::Ordering;
                if !navigation.active(&navigation_app) { return false; }
                if next.as_str() == "about:blank" { return initial.load(Ordering::Acquire); }
                let allowed = account_url(navigation.service, next);
                if allowed { initial.store(false, Ordering::Release); }
                allowed
            })
            .on_new_window(move |next, features| nested.open_at(&nested_app, next, features, depth + 1))
            .on_download(|_, _| false)
            .build();
        let window = match result {
            Ok(window) => window,
            Err(_) => {
                self.state.lock().unwrap_or_else(|p| p.into_inner()).labels.remove(&label);
                return NewWindowResponse::Deny;
            }
        };
        let state = self.state.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                state.lock().unwrap_or_else(|p| p.into_inner()).labels.remove(&label);
            }
        });
        if !self.active(app) {
            let _ = window.close();
            return NewWindowResponse::Deny;
        }
        NewWindowResponse::Create { window }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_environment_preserves_existing_youtube_playback_arguments() {
        assert_eq!(BROWSER_ARGS.split_whitespace().collect::<Vec<_>>(), [
            "--autoplay-policy=no-user-gesture-required",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
        ]);
    }
    #[test]
    fn every_shared_profile_builder_uses_identical_environment_options() {
        let own_source = include_str!("browser_identity.rs").split("#[cfg(test)]").next().unwrap();
        for (name, source, expected) in [
            ("browser_identity", own_source, 1),
            ("youtube live/account/popup", include_str!("youtube/original.rs"), 3),
            ("youtube VOD", include_str!("youtube/original_vod.rs"), 1),
            ("pixiv holder/account", include_str!("pixiv/account.rs"), 2),
            ("pinterest query/account", include_str!("pinterest_session.rs"), 2),
            ("pinterest discovery", include_str!("pinterest_browser.rs"), 1),
            ("X reader/account", include_str!("x_feed.rs").split("#[cfg(test)]").next().unwrap(), 2),
        ] {
            let builders = source.split(".data_directory(").skip(1).collect::<Vec<_>>();
            assert_eq!(builders.len(), expected, "{name}: review every shared-profile builder");
            for builder in builders {
                let options = builder.split(".on_navigation(").next().unwrap();
                assert_eq!(options.matches(".additional_browser_args(").count(), 1, "{name}");
                assert!(options.contains(".additional_browser_args(crate::browser_identity::BROWSER_ARGS)"), "{name}");
                assert!(options.contains(".browser_extensions_enabled(false)"), "{name}");
            }
        }
    }
    #[test]
    fn retains_existing_youtube_profile_without_credential_migration() {
        assert_eq!(PROFILE, "youtube-original-profile");
        for source in [include_str!("youtube/original.rs"), include_str!("pinterest_session.rs"), include_str!("pinterest_browser.rs"), include_str!("pixiv/account.rs")] {
            assert!(source.contains("crate::browser_identity::profile(app)"));
            assert!(!source.contains("join(\"pinterest-browser-profile\")"));
            assert!(!source.contains("join(\"pixiv-browser-profile\")"));
        }
    }
    #[test]
    fn google_navigation_is_exact_and_service_specific() {
        for service in [Service::Pinterest, Service::Pixiv, Service::X] {
            for url in ["https://accounts.google.com/gsi/select?state=fixture", "https://accounts.google.com/o/oauth2/v2/auth", "https://accounts.google.com/v3/signin/challenge", "https://consent.google.com/", "https://accounts.google.co.kr/accounts/SetSID?state=fixture"] {
                assert!(account_url(service, &url.parse().unwrap()), "{url}");
            }
            for url in ["http://accounts.google.com/", "https://accounts.google.com.evil.test/", "https://evil.accounts.google.com/", "https://user:secret@accounts.google.com/", "https://accounts.google.com:444/", "https://accounts.google.co.kr/", "https://accounts.google.co.kr.evil.test/accounts/SetSID", "https://nid.naver.com/", "https://login.sooplive.com/", "https://www.youtube.com/", "https://s.pinimg.com/", "https://s.pximg.net/", "about:blank", "data:text/html,hi", "file:///C:/fixture"] {
                assert!(!account_url(service, &url.parse().unwrap()), "{url}");
            }
        }
        assert!(account_url(Service::Pinterest, &"https://www.pinterest.com/login/".parse().unwrap()));
        assert!(!account_url(Service::Pixiv, &"https://www.pinterest.com/".parse().unwrap()));
        assert!(account_url(Service::Pixiv, &"https://accounts.pixiv.net/pigya/start".parse().unwrap()));
        assert!(!account_url(Service::Pinterest, &"https://accounts.pixiv.net/login".parse().unwrap()));
    }
    #[test]
    fn popup_budget_is_owned_bounded_and_irrevocably_closed() {
        let mut first = PopupState::default(); let mut other = PopupState::default();
        assert!(!first.reserve("too-deep".into(), MAX_DEPTH));
        for n in 0..MAX_POPUPS { assert!(first.reserve(format!("first-{n}"), 0)); }
        assert!(!first.reserve("overflow".into(), 0));
        assert!(other.reserve("other".into(), 0));
        first.labels.remove("first-0"); assert!(first.reserve("replacement".into(), 1));
        assert_eq!(first.close().len(), MAX_POPUPS);
        assert!(!first.reserve("stale".into(), 0));
        assert!(first.close().is_empty()); assert_eq!(other.labels.len(), 1);
    }
}
