//! Native host layout and account windows. The remote player remains a separate
//! Webview identity even though its parent OS window is the trusted main window.
use super::*;
use tauri::{LogicalPosition, LogicalSize, WebviewBuilder, WebviewUrl, WebviewWindowBuilder};

const LOGIN_LABEL: &str = "chzzk-login";
const INSTALL_URL: &str =
    "https://chromewebstore.google.com/detail/ooadnieabchijkibjpeieeliohjidnjj";
const EDGE_INSTALL_URL: &str =
    "https://microsoftedge.microsoft.com/addons/detail/jedbgfnhnpbfcbplibkacnmiafbojobk";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallerBrowser {
    #[default]
    Chrome,
    Edge,
}
impl InstallerBrowser {
    fn url(self) -> &'static str {
        match self {
            Self::Chrome => INSTALL_URL,
            Self::Edge => EDGE_INSTALL_URL,
        }
    }
    fn relative_executable(self) -> &'static str {
        match self {
            Self::Chrome => "Google/Chrome/Application/chrome.exe",
            Self::Edge => "Microsoft/Edge/Application/msedge.exe",
        }
    }
    fn missing(self) -> StreamError {
        error("INSTALL_BROWSER_NOT_FOUND", match self {
            Self::Chrome => "Chrome 설치 위치를 확인하지 못했습니다. Chrome을 설치하거나 연결 설정에서 Edge용 설치를 선택해 주세요.",
            Self::Edge => "Edge 설치 위치를 확인하지 못했습니다. Edge를 설치하거나 연결 설정에서 Chrome용 설치를 선택해 주세요.",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserClip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserViewport {
    #[serde(default)]
    pub epoch: u64,
    #[serde(default)]
    pub request_sequence: Option<u64>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
    /// A trusted modal masks pixels/input without hiding the media controller.
    /// Privacy, inactive workspaces and detached documents must use visible=false.
    #[serde(default)]
    pub occluded: bool,
    /// Only trusted measured popups may retain a clipped background. Native
    /// input remains disabled for the entire surface, not only the sheet.
    #[serde(default)]
    pub preserve_background: bool,
    /// Stage-local trusted popup rectangles, subtracted from the scroll clip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occlusions: Vec<BrowserClip>,
    #[serde(default)]
    pub clip: Option<BrowserClip>,
}
impl Default for BrowserViewport {
    fn default() -> Self {
        Self {
            epoch: 0,
            request_sequence: None,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            visible: false,
            occluded: false,
            preserve_background: false,
            occlusions: vec![],
            clip: None,
        }
    }
}
impl BrowserViewport {
    pub(super) fn validate(&self) -> Result<(), StreamError> {
        if self
            .request_sequence
            .is_some_and(|n| n == 0 || n > 9_007_199_254_740_991)
        {
            return Err(error(
                "VIEWPORT_INVALID",
                "시청 영역 요청 순서가 올바르지 않습니다.",
            ));
        }
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 32768.0)
            || self.width < 0.0
            || self.height < 0.0
        {
            return Err(error(
                "VIEWPORT_INVALID",
                "시청 영역의 크기가 올바르지 않습니다.",
            ));
        }
        if let Some(c) = &self.clip {
            if [c.x, c.y, c.width, c.height]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
                || c.x + c.width > self.width + 1.0
                || c.y + c.height > self.height + 1.0
            {
                return Err(error(
                    "VIEWPORT_INVALID",
                    "시청 영역의 잘림 범위가 올바르지 않습니다.",
                ));
            }
        }
        if self.occluded && (!self.visible || self.width < 1.0 || self.height < 1.0) {
            return Err(error(
                "VIEWPORT_INVALID",
                "가려진 시청 영역은 원래 화면 크기를 유지해야 합니다.",
            ));
        }
        if self.preserve_background && (!self.occluded || self.clip.is_none()) {
            return Err(error(
                "VIEWPORT_INVALID",
                "확인 창의 시청 범위를 확인하지 못했습니다.",
            ));
        }
        if self.occlusions.len() > 8
            || (!self.occlusions.is_empty() && !self.preserve_background)
            || self.occlusions.iter().any(|c| {
                [c.x, c.y, c.width, c.height]
                    .iter()
                    .any(|v| !v.is_finite() || *v < 0.0)
                    || c.width <= 0.0
                    || c.height <= 0.0
                    || c.x + c.width > self.width
                    || c.y + c.height > self.height
            })
        {
            return Err(error(
                "VIEWPORT_INVALID",
                "팝업의 가림 범위가 올바르지 않습니다.",
            ));
        }
        Ok(())
    }
}

pub(super) fn viewport_sequence(
    incoming: &BrowserViewport,
    previous: &BrowserViewport,
    epoch: u64,
) -> Result<Option<u64>, StreamError> {
    if incoming
        .request_sequence
        .is_some_and(|n| n == 0 || n > 9_007_199_254_740_991)
    {
        return Err(error(
            "VIEWPORT_INVALID",
            "시청 영역 요청 순서가 올바르지 않습니다.",
        ));
    }
    if incoming.visible && incoming.epoch != epoch {
        return Err(viewport_stale());
    }
    if incoming.epoch == epoch {
        if let (Some(next), Some(last)) = (incoming.request_sequence, previous.request_sequence) {
            // Equal is only an exact internal replay, never an older request
            // with changed visibility/geometry. UI submissions are monotonic.
            if next < last || (next == last && incoming != previous) {
                return Err(viewport_stale());
            }
        }
        Ok(incoming.request_sequence.or(previous.request_sequence))
    } else {
        // Detach already hid the old document. Once a fresh document submits
        // anything, an older hide must not obscure its acknowledged surface.
        if previous.request_sequence.is_some() {
            return Err(viewport_stale());
        }
        Ok(previous.request_sequence)
    }
}

fn account_label(label: &str) -> bool {
    label == LOGIN_LABEL
        || label
            .strip_prefix("chzzk-login-")
            .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

// Round inward: a fractional CSS boundary must never expose remote pixels over
// adjacent trusted controls. Wry rounds the complete child HWND size separately.
fn clip_pixels(viewport: &BrowserViewport, scale: f64) -> Result<[i32; 4], StreamError> {
    viewport.validate()?;
    if !scale.is_finite() || scale <= 0.0 || scale > 16.0 {
        return Err(error(
            "VIEWPORT_INVALID",
            "시청 영역의 배율이 올바르지 않습니다.",
        ));
    }
    if viewport.occluded && !viewport.preserve_background {
        return Ok([0; 4]);
    }
    let full = BrowserClip {
        x: 0.0,
        y: 0.0,
        width: viewport.width,
        height: viewport.height,
    };
    let clip = viewport.clip.as_ref().unwrap_or(&full);
    let width = (viewport.width * scale).round() as i32;
    let height = (viewport.height * scale).round() as i32;
    let left = ((clip.x * scale).ceil() as i32).clamp(0, width);
    let top = ((clip.y * scale).ceil() as i32).clamp(0, height);
    let right = (((clip.x + clip.width) * scale).floor() as i32).clamp(left, width);
    let bottom = (((clip.y + clip.height) * scale).floor() as i32).clamp(top, height);
    Ok([left, top, right, bottom])
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PixelOcclusions {
    rectangles: [[i32; 4]; 8],
    length: usize,
}
impl PixelOcclusions {
    fn rectangles(&self) -> &[[i32; 4]] {
        &self.rectangles[..self.length]
    }
    fn is_empty(&self) -> bool {
        self.length == 0
    }
}
fn occlusion_pixels(
    viewport: &BrowserViewport,
    scale: f64,
) -> Result<PixelOcclusions, StreamError> {
    // Validates scale as well as shape before any float-to-integer conversion.
    let _ = clip_pixels(viewport, scale)?;
    let mut result = PixelOcclusions::default();
    if !viewport.occluded || !viewport.preserve_background {
        return Ok(result);
    }
    let width = (viewport.width * scale).round() as i32;
    let height = (viewport.height * scale).round() as i32;
    for (index, rect) in viewport.occlusions.iter().enumerate() {
        // Removed pixels round outward, the opposite of the visible scroll clip.
        result.rectangles[index] = [
            ((rect.x * scale).floor() as i32).clamp(0, width),
            ((rect.y * scale).floor() as i32).clamp(0, height),
            (((rect.x + rect.width) * scale).ceil() as i32).clamp(0, width),
            (((rect.y + rect.height) * scale).ceil() as i32).clamp(0, height),
        ];
        result.length += 1;
    }
    Ok(result)
}

#[cfg(windows)]
mod native_region {
    use super::{viewport_failed, PixelOcclusions, StreamError};
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            CombineRgn, CreateRectRgn, DeleteObject, EqualRgn, GetWindowRgn, SetWindowRgn, HGDIOBJ,
            HRGN, RGN_DIFF,
        },
    };

    /// Owns temporary GDI regions until SetWindowRgn explicitly takes ownership.
    pub(super) struct Region(pub(super) HRGN);
    impl Region {
        pub(super) fn new(
            rect: [i32; 4],
            occlusions: PixelOcclusions,
        ) -> Result<Self, StreamError> {
            let region = unsafe { CreateRectRgn(rect[0], rect[1], rect[2], rect[3]) };
            if region.0.is_null() {
                return Err(viewport_failed());
            }
            let result = Self(region);
            for hole in occlusions.rectangles() {
                let mask = Self::new(*hole, PixelOcclusions::default())?;
                if unsafe { CombineRgn(Some(result.0), Some(result.0), Some(mask.0), RGN_DIFF) }.0
                    == 0
                {
                    return Err(viewport_failed());
                }
            }
            Ok(result)
        }
        pub(super) fn apply(self, hwnd: HWND) -> Result<(), StreamError> {
            if unsafe { SetWindowRgn(hwnd, Some(self.0), true) } == 0 {
                return Err(viewport_failed());
            }
            // Windows owns and releases the successful replacement region.
            std::mem::forget(self);
            Ok(())
        }
        pub(super) fn matches_window(&self, hwnd: HWND) -> Result<bool, StreamError> {
            let actual = Self::new([0; 4], PixelOcclusions::default())?;
            if unsafe { GetWindowRgn(hwnd, actual.0) }.0 == 0 {
                return Ok(false);
            }
            // Bounding boxes cannot detect missing or misplaced popup holes.
            Ok(unsafe { EqualRgn(self.0, actual.0) }.as_bool())
        }
    }
    impl Drop for Region {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0 .0));
            }
        }
    }
}

fn installation_browser(url: &tauri::Url) -> Option<InstallerBrowser> {
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 2048
    {
        return None;
    }
    let (browser, prefix, id) = match url.host_str()? {
        "chromewebstore.google.com" => (
            InstallerBrowser::Chrome,
            "/detail/",
            super::super::browser_extension::CHROME_EXTENSION_ID,
        ),
        "chrome.google.com" => (
            InstallerBrowser::Chrome,
            "/webstore/detail/",
            super::super::browser_extension::CHROME_EXTENSION_ID,
        ),
        "microsoftedge.microsoft.com" => (
            InstallerBrowser::Edge,
            "/addons/detail/",
            super::super::browser_extension::EDGE_EXTENSION_ID,
        ),
        _ => return None,
    };
    let tail = url.path().strip_prefix(prefix)?.trim_end_matches('/');
    let parts: Vec<_> = tail.split('/').collect();
    // The store's optional product slug is cosmetic. Navigation always opens
    // our canonical product URL, never a page-controlled path/query/argument.
    (parts.last() == Some(&id) && (parts.len() == 1 || (parts.len() == 2 && !parts[0].is_empty())))
        .then_some(browser)
}

impl OfficialBrowser {
    pub fn open(&self, app: &AppHandle, input: &str) -> Result<(), StreamError> {
        let channel = normalize_channel_input(input)?;
        let url: tauri::Url = format!("https://chzzk.naver.com/live/{channel}")
            .parse()
            .map_err(|_| unavailable())?;
        {
            let mut s = self.inner.view.lock().map_err(|_| unavailable())?;
            if self.inner.closing.load(Ordering::Acquire)
                || s.account_busy
                || self.multiview_active()
            {
                return Err(unavailable());
            }
            if (s.recording.is_some() || s.arm.is_some()) && s.channel.as_ref() != Some(&channel) {
                return Err(error(
                    "BROWSER_RECORDING_ACTIVE",
                    "녹화를 중지한 뒤 채널을 변경해 주세요.",
                ));
            }
            if s.channel.as_ref() != Some(&channel) {
                s.ready = false;
                s.status = "loading".into();
            }
            s.channel = Some(channel);
            s.error = None;
        }
        if let Some(view) = app.get_webview(WINDOW_LABEL) {
            if view.url().ok().as_ref() != Some(&url) {
                view.navigate(url).map_err(|_| unavailable())?;
            }
            let viewport = self
                .inner
                .view
                .lock()
                .map_err(|_| unavailable())?
                .viewport
                .clone();
            return self.set_viewport(app, viewport);
        }
        let parent = app.get_window("main").ok_or_else(unavailable)?;
        {
            let mut s = self.inner.view.lock().map_err(|_| unavailable())?;
            s.extension_generation = s.extension_generation.wrapping_add(1);
            s.extension_connecting = false;
            s.loaded_extensions.clear();
            s.extension = "not_connected".into();
        }
        let profile = self.inner.data_dir.join("chzzk-browser-profile");
        std::fs::create_dir_all(&profile).map_err(|_| unavailable())?;
        let nav_host = self.clone();
        let nav_app = app.clone();
        let popup_host = self.clone();
        let popup_app = app.clone();
        let page_host = self.clone();
        let startup_blank = Arc::new(AtomicBool::new(true));
        let nav_startup_blank = startup_blank.clone();
        // Install native request policy before the first live-page request.
        let builder = WebviewBuilder::new(
            WINDOW_LABEL,
            WebviewUrl::External("about:blank".parse().map_err(|_| unavailable())?),
        )
        .data_directory(profile)
        .browser_extensions_enabled(true)
        .initialization_script(include_str!("browser_chat_enhancements.js"))
        .initialization_script(include_str!("browser_page_chat.js"))
        .initialization_script(include_str!("browser_encoded_capture.js"))
        .initialization_script(include_str!("browser_capture.js"))
        .initialization_script(include_str!("browser_player_ui.js"))
        .on_navigation(move |url| {
            if let Some(browser) = installation_browser(url) {
                nav_host.install_from_page(browser);
                return false;
            }
            if url.as_str() == "about:blank"
                && (nav_startup_blank.load(Ordering::Acquire)
                    || nav_host
                        .inner
                        .view
                        .lock()
                        .map(|s| s.account_busy)
                        .unwrap_or(false))
            {
                return true;
            }
            if !allowed_navigation(url) {
                return false;
            }
            let state = nav_host
                .inner
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if state.recording.is_some() || state.arm.is_some() {
                return live_channel(url).as_ref() == state.channel.as_ref();
            }
            drop(state);
            if url.host_str() == Some("nid.naver.com") {
                let host = nav_host.clone();
                let app = nav_app.clone();
                // Queue after this WebView2 navigation callback unwinds.
                thread::spawn(move || {
                    let dispatch = app.clone();
                    let _ = dispatch.run_on_main_thread(move || {
                        let _ = host.login(&app);
                    });
                });
                return false;
            }
            true
        })
        .on_new_window(move |url, features| {
            if let Some(browser) = installation_browser(&url) {
                popup_host.install_from_page(browser);
                return tauri::webview::NewWindowResponse::Deny;
            }
            if !allowed_navigation(&url) {
                return tauri::webview::NewWindowResponse::Deny;
            }
            if popup_host.account_window_open(&popup_app)
                || popup_host.reserve_account_window().is_err()
            {
                return tauri::webview::NewWindowResponse::Deny;
            }
            let label = format!("{LOGIN_LABEL}-{}", uuid::Uuid::new_v4().simple());
            let host = popup_host.clone();
            let app = popup_app.clone();
            let result = WebviewWindowBuilder::new(&popup_app, &label, WebviewUrl::External(url))
                .data_directory(popup_host.inner.data_dir.join("chzzk-browser-profile"))
                .browser_extensions_enabled(true)
                .window_features(features)
                .title("CHZZK 로그인·계정 관리")
                .inner_size(700.0, 820.0)
                .on_navigation(allowed_navigation)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build();
            popup_host.account_window_created(result.is_ok());
            match result {
                Ok(window) => {
                    window.on_window_event(move |event| {
                        if matches!(event, tauri::WindowEvent::Destroyed) {
                            host.account_window_closed(&app, &label);
                        }
                    });
                    tauri::webview::NewWindowResponse::Create { window }
                }
                Err(_) => tauri::webview::NewWindowResponse::Deny,
            }
        })
        .on_page_load(move |view, payload| {
            // The initial empty document has no player lifecycle to stop.
            if payload.url().as_str() == "about:blank" {
                return;
            }
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                let generation = {
                    let mut s = page_host
                        .inner
                        .view
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    s.page_generation = s.page_generation.wrapping_add(1);
                    s.ready = false;
                    s.status = "loading".into();
                    s.page_generation
                };
                let host = page_host.clone();
                thread::spawn(move || {
                    if host
                        .inner
                        .view
                        .lock()
                        .map(|s| s.page_generation == generation)
                        .unwrap_or(false)
                    {
                        host.interrupt("page_hidden");
                    }
                });
            } else if live_channel(payload.url()).is_some() {
                let _ = view.eval(include_str!("browser_page_chat.js"));
                let _ = view.eval(include_str!("browser_encoded_capture.js"));
                let _ = view.eval(include_str!("browser_capture.js"));
                let _ = view.eval(include_str!("browser_player_ui.js"));
                page_host.probe_extensions(&view);
                page_host.refresh_auth_state(view.app_handle(), false);
            }
        });
        let view = parent
            .add_child(
                builder,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1.0, 1.0),
            )
            .map_err(|_| unavailable())?;
        view.hide().map_err(|_| unavailable())?;
        if super::super::browser_video_ads::install(&view).is_err() {
            tracing::warn!("CHZZK video ad filter unavailable; requests remain unchanged");
        }
        attach_native(&view, self.clone())?;
        startup_blank.store(false, Ordering::Release);
        view.navigate(url).map_err(|_| unavailable())?;
        let viewport = {
            let mut s = self.inner.view.lock().map_err(|_| unavailable())?;
            s.open = true;
            s.viewport.clone()
        };
        self.set_viewport(app, viewport)?;
        // Profile extensions can persist, but UserAgent is per WebView and
        // resets to native Edge on recreation. Revalidate/re-align only after
        // a prior successful user opt-in, using the manual command's same path.
        if self
            .inner
            .view
            .lock()
            .map_err(|_| unavailable())?
            .extension_reconnect_enabled
        {
            if let Err(cause) = self.begin_extension_connection(app, false) {
                self.inner.view.lock().map_err(|_| unavailable())?.extension = cause.message;
            }
        }
        Ok(())
    }

    /// The production command and isolated probe share this explicit opt-in.
    /// Success schedules loading; snapshot reports its asynchronous outcome.
    pub fn connect_extension(&self, app: &AppHandle) -> Result<(), StreamError> {
        self.begin_extension_connection(app, true)
    }

    fn begin_extension_connection(
        &self,
        app: &AppHandle,
        remember: bool,
    ) -> Result<(), StreamError> {
        let view = app
            .get_webview(WINDOW_LABEL)
            .ok_or_else(|| error("BROWSER_NOT_OPEN", "먼저 공식 시청 창을 열어 주세요."))?;
        let generation = {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if self.inner.closing.load(Ordering::Acquire)
                || self.inner.reserved.load(Ordering::Acquire)
                || self.multiview_active()
            {
                return Err(unavailable());
            }
            if state.account_busy || self.account_window_open(app) {
                return Err(error(
                    "BROWSER_ACCOUNT_BUSY",
                    "로그인·계정 처리가 끝난 뒤 확장을 연결해 주세요.",
                ));
            }
            if state.recording.is_some() || state.arm.is_some() {
                return Err(error(
                    "RECORDING_ACTIVE",
                    "확장 연결 전에 녹화를 중지해 주세요.",
                ));
            }
            if state.extension_connecting {
                return Ok(());
            }
            state.extension_generation = state.extension_generation.wrapping_add(1);
            state.extension_connecting = true;
            state.extension = "연결 중".into();
            state.extension_generation
        };
        let host = self.clone();
        let handle = app.clone();
        let callback_view = view.clone();
        let callback = move |result: Result<
            super::super::browser_extension::ExtensionLoadReport,
            StreamError,
        >| {
            let mut state = host.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            if state.extension_generation != generation
                || host.inner.closing.load(Ordering::Acquire)
            {
                return;
            }
            match result {
                Ok(report) => {
                    // Keep the connecting reservation through the small setting
                    // commit; recording cannot overtake UA alignment/reload.
                    if remember {
                        match super::super::browser_extension::remember_connection(
                            &host.inner.data_dir,
                        ) {
                            Ok(()) => state.extension_reconnect_enabled = true,
                            Err(cause) => state.error = Some(cause.message),
                        }
                    }
                    state.loaded_extensions = report.loaded_ids;
                    state.extension = "확장 로드 완료 · 공식 페이지 감지 확인 중".into();
                    let reload = report.reload_required
                        && state.recording.is_none()
                        && state.arm.is_none()
                        && !state.account_busy
                        && !host.account_window_open(&handle)
                        && !host.inner.closing.load(Ordering::Acquire);
                    // Loading callbacks are deliberately delivered outside
                    // with_webview's dispatcher lock before any navigation.
                    if reload {
                        state.ready = false;
                    }
                    drop(state);
                    let mut navigation_failed = false;
                    if reload {
                        navigation_failed = callback_view
                            .url()
                            .ok()
                            .filter(|url| live_channel(url).is_some())
                            .is_none_or(|url| callback_view.navigate(url).is_err());
                    }
                    // Keep the reservation until navigation has been queued,
                    // without holding ViewState across a WebView callback.
                    let mut state = host.inner.view.lock().unwrap_or_else(|p| p.into_inner());
                    if state.extension_generation != generation {
                        return;
                    }
                    state.extension_connecting = false;
                    if reload {
                        state.ready = false;
                    }
                    if navigation_failed {
                        state.extension =
                            "확장 연결 후 새로고침하지 못했습니다 · 시청 영역을 다시 열어 주세요"
                                .into();
                    }
                    drop(state);
                    if !reload {
                        host.probe_extensions(&callback_view);
                    }
                }
                Err(cause) => {
                    state.extension_connecting = false;
                    state.extension = cause.message;
                }
            }
        };
        if let Err(cause) = super::super::browser_extension::connect(&view, callback) {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if state.extension_generation == generation {
                state.extension_connecting = false;
                state.extension = cause.message.clone();
            }
            return Err(cause);
        }
        Ok(())
    }

    pub fn set_viewport(
        &self,
        app: &AppHandle,
        mut viewport: BrowserViewport,
    ) -> Result<(), StreamError> {
        let view = app.get_webview(WINDOW_LABEL);
        let validation = viewport.validate();
        let (revision, validation) = {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            // Reject stale ingress without touching a newer accepted surface.
            // Atomic revision handles cancellation after ingress; this client
            // sequence also handles async commands which START out of order.
            viewport.request_sequence =
                viewport_sequence(&viewport, &state.viewport, state.viewport_epoch)?;
            // Hides are always safe, but an old document must not roll the
            // current epoch backwards while hiding its obsolete rectangle.
            viewport.epoch = state.viewport_epoch;
            if validation.is_err() {
                state.viewport.visible = false;
                state.viewport.occluded = false;
                state.viewport.preserve_background = false;
                state.viewport.occlusions.clear();
                state.viewport.request_sequence = viewport.request_sequence;
            } else {
                viewport.visible = viewport.visible
                    && viewport.width >= 1.0
                    && viewport.height >= 1.0
                    && (viewport.occluded
                        || !viewport
                            .clip
                            .as_ref()
                            .is_some_and(|c| c.width < 1.0 || c.height < 1.0));
                state.viewport = viewport.clone();
            }
            (
                self.inner
                    .viewport_revision
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1),
                validation,
            )
        };
        if validation.is_err() || !viewport.visible {
            // Privacy/detach and invalid requests preempt native work;
            // they never wait for its gate or a previous dispatch timeout.
            if let Some(view) = &view {
                view.hide().map_err(|_| unavailable())?;
            }
            return validation;
        }
        let Some(view) = view else {
            return Ok(());
        };
        // Never block the UI thread on a worker which is waiting for native UI
        // dispatch. Ordinary frontend writes are already single-in-flight.
        let result = if viewport.occluded {
            // A trusted modal must mask an in-flight show immediately, without
            // waiting for the ordinary visible writer's acknowledgement gate.
            apply_viewport(&view, &viewport, self.inner.clone(), revision)
        } else {
            match self.inner.viewport_writes.try_lock() {
                Ok(_write) => apply_viewport(&view, &viewport, self.inner.clone(), revision),
                Err(_) => Err(error(
                    "VIEWPORT_BUSY",
                    "시청 영역을 다시 배치하고 있습니다. 잠시 후 다시 시도해 주세요.",
                )),
            }
        };
        if result.is_err() {
            let mut state = self.inner.view.lock().map_err(|_| unavailable())?;
            if self.inner.viewport_revision.load(Ordering::Acquire) == revision {
                // A timeout is not cancellation. Invalidate its queued closure
                // before hiding so it cannot change or re-show a newer surface.
                self.inner.viewport_revision.fetch_add(1, Ordering::AcqRel);
                state.viewport.visible = false;
                state.viewport.occluded = false;
                state.viewport.preserve_background = false;
                state.viewport.occlusions.clear();
                let _ = view.hide();
            }
        }
        result
    }
    pub fn detach_viewport(&self, app: &AppHandle) {
        let mut viewport = {
            let mut state = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            state.viewport_epoch = state.viewport_epoch.wrapping_add(1);
            self.inner.viewport_revision.fetch_add(1, Ordering::AcqRel);
            state.viewport.request_sequence = None;
            let mut viewport = state.viewport.clone();
            viewport.epoch = state.viewport_epoch;
            viewport
        };
        viewport.visible = false;
        viewport.occluded = false;
        viewport.preserve_background = false;
        viewport.occlusions.clear();
        let _ = self.set_viewport(app, viewport);
    }
    pub fn probe_extensions(&self, view: &Webview) {
        let (ids, generation, channel, extension_generation) = {
            let s = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            if s.extension_connecting {
                return;
            }
            (
                s.loaded_extensions.clone(),
                s.page_generation,
                s.channel.clone(),
                s.extension_generation,
            )
        };
        if ids.is_empty() {
            return;
        }
        let host = self.clone();
        let callback = move |result: Result<
            super::super::browser_extension::ExtensionDetectionReport,
            StreamError,
        >| {
            let mut s = host.inner.view.lock().unwrap_or_else(|p| p.into_inner());
            if s.page_generation != generation
                || s.channel != channel
                || s.extension_connecting
                || s.extension_generation != extension_generation
            {
                return;
            }
            s.extension = match result {
                Ok(report) if !report.detected_ids.is_empty() => {
                    "공식 페이지 확장 감지됨 · 커넥터/고화질은 실제 재생으로 확인".into()
                }
                Ok(report) if !report.page_api_available => {
                    "확장 파일은 로드됐지만 페이지 통신 API를 사용할 수 없습니다".into()
                }
                Ok(_) => "확장 파일은 로드됐지만 공식 페이지의 확장 응답이 없습니다".into(),
                Err(e) => e.message,
            };
        };
        let _ = super::super::browser_extension::probe(view, &ids, callback);
    }

    pub fn account_window_open(&self, app: &AppHandle) -> bool {
        app.webview_windows()
            .keys()
            .any(|label| account_label(label))
    }
    pub(super) fn refresh_auth_state(&self, app: &AppHandle, force: bool) -> Option<u64> {
        if self.label() != WINDOW_LABEL
            || self.inner.closing.load(Ordering::Acquire)
            || self.account_window_open(app)
        {
            return None;
        }
        let generation = {
            let mut state = self.inner.view.lock().ok()?;
            state.begin_auth_probe(force, Instant::now())?
        };
        auth::probe_profile(self, app, generation);
        Some(generation)
    }
    /// Manual checks bypass the cache and await an actual result. Ordinary
    /// snapshot polling stays nonblocking and shares any in-flight request.
    pub async fn refresh_login_status(
        &self,
        app: &AppHandle,
        force: bool,
    ) -> Result<BrowserSnapshot, StreamError> {
        let started = self.refresh_auth_state(app, force);
        if force {
            let generation = started.or_else(|| {
                self.inner
                    .view
                    .lock()
                    .ok()
                    .and_then(|s| s.auth_checking.then_some(s.auth_generation))
            });
            if let Some(generation) = generation {
                let deadline = Instant::now() + auth::DEADLINE + Duration::from_secs(1);
                loop {
                    let (checking, current) = {
                        let state = self.inner.view.lock().map_err(|_| unavailable())?;
                        (state.auth_checking, state.auth_generation)
                    };
                    if !checking || current != generation {
                        break;
                    }
                    if Instant::now() >= deadline {
                        self.inner
                            .view
                            .lock()
                            .map_err(|_| unavailable())?
                            .finish_auth_probe(generation, auth::AuthStatus::Unknown);
                        auth::cancel(app);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        self.snapshot()
    }
    fn reserve_account_window(&self) -> Result<(), StreamError> {
        // All videos share this browser profile. Serialize against starting a
        // recording in ANY pane, not merely the single-view controller.
        let _gate = self.inner.contexts.gate.lock().map_err(|_| unavailable())?;
        if self.label() != WINDOW_LABEL
            || self.inner.contexts.reconfiguring.load(Ordering::Acquire) != 0
            || !self.active_ids().is_empty()
        {
            return Err(error(
                "BROWSER_ACCOUNT_BUSY",
                "모든 녹화의 저장과 화면 변경이 끝난 뒤 계정을 변경해 주세요.",
            ));
        }
        let mut s = self.inner.view.lock().map_err(|_| unavailable())?;
        if s.account_busy
            || s.recording.is_some()
            || s.arm.is_some()
            || self.inner.closing.load(Ordering::Acquire)
            || self.inner.reserved.load(Ordering::Acquire)
            || s.extension_connecting
        {
            return Err(error(
                "BROWSER_ACCOUNT_BUSY",
                "녹화와 로그인 정보 처리가 끝난 뒤 계정 창을 열어 주세요.",
            ));
        }
        s.account_busy = true;
        s.invalidate_auth();
        Ok(())
    }
    fn account_window_created(&self, opened: bool) {
        let mut s = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
        s.account_busy = false;
        if opened {
            s.invalidate_auth();
            s.auth_status = auth::AuthStatus::Checking;
            s.login_status = "로그인 창 열림 · 비밀번호·쿠키는 앱에서 추출하지 않습니다".into();
        }
    }
    pub fn login(&self, app: &AppHandle) -> Result<(), StreamError> {
        self.reserve_account_window()?;
        auth::cancel(app);
        if let Some((_, window)) = app
            .webview_windows()
            .into_iter()
            .find(|(label, _)| account_label(label))
        {
            self.account_window_created(true);
            let _ = window.show();
            let _ = window.set_focus();
            return Ok(());
        }
        let result = self.create_login_window(app);
        self.account_window_created(result.is_ok());
        result
    }
    fn create_login_window(&self, app: &AppHandle) -> Result<(), StreamError> {
        let profile = self.inner.data_dir.join("chzzk-browser-profile");
        std::fs::create_dir_all(&profile).map_err(|_| unavailable())?;
        let url = "https://nid.naver.com/nidlogin.login?url=https%3A%2F%2Fchzzk.naver.com%2F"
            .parse()
            .map_err(|_| unavailable())?;
        let return_app = app.clone();
        let window = WebviewWindowBuilder::new(app, LOGIN_LABEL, WebviewUrl::External(url))
            .title("CHZZK 로그인·계정 관리")
            .inner_size(700.0, 820.0)
            .data_directory(profile)
            .browser_extensions_enabled(true)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_navigation(move |url| {
                if url.host_str() == Some("chzzk.naver.com") && allowed_navigation(url) {
                    let app = return_app.clone();
                    // Destroyed is the single session-refresh path, whether the
                    // user closes the window or NAVER returns to CHZZK.
                    thread::spawn(move || {
                        let dispatch = app.clone();
                        let _ = dispatch.run_on_main_thread(move || {
                            if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
                                let _ = w.destroy();
                            }
                        });
                    });
                    return false;
                }
                allowed_navigation(url)
            })
            .build()
            .map_err(|_| unavailable())?;
        let host = self.clone();
        let handle = app.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                host.account_window_closed(&handle, LOGIN_LABEL);
            }
        });
        let _ = window.set_focus();
        Ok(())
    }
    fn account_window_closed(&self, app: &AppHandle, closed_label: &str) {
        let Ok(_gate) = self.inner.contexts.gate.lock() else {
            return;
        };
        if !self.active_ids().is_empty() {
            return;
        }
        let mut s = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
        if s.account_busy || self.inner.closing.load(Ordering::Acquire) {
            return;
        }
        // The destroyed window can still be present in Tauri's manager while
        // this event runs; only another account window keeps the session open.
        if app
            .webview_windows()
            .keys()
            .any(|label| label != closed_label && account_label(label))
        {
            return;
        }
        s.login_status = "저장된 브라우저 세션 반영 · 로그인 여부는 공식 화면에서 확인".into();
        s.invalidate_auth();
        s.account_busy = true;
        drop(s);
        drop(_gate);
        self.refresh_account_playback(app);
        self.inner
            .view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .account_busy = false;
    }
    pub fn logout(&self, app: &AppHandle) -> Result<(), StreamError> {
        self.reserve_account_window()?;
        auth::cancel(app);
        {
            let mut s = self.inner.view.lock().map_err(|_| unavailable())?;
            s.ready = false;
            s.auth_generation = s.auth_generation.wrapping_add(1);
            s.auth_status = auth::AuthStatus::Checking;
            s.login_status = "이 앱의 CHZZK 로그인 정보 삭제 중".into();
        }
        for (label, w) in app.webview_windows() {
            if account_label(&label) {
                let _ = w.destroy();
            }
        }
        let mut cleanup_label = None;
        let view = (|| {
            self.pause_multiview_for_account(app, true)?;
            if let Some(view) = app.get_webview(WINDOW_LABEL) {
                view.navigate("about:blank".parse().unwrap())
                    .map_err(|_| unavailable())?;
                return Ok(view);
            }
            // Login/logout must work before a channel is connected, and Mado
            // has no single-view window. Use a hidden blank profile owner,
            // never a fifth stream or the user's Chrome/Edge profile.
            let label = format!("{LOGIN_LABEL}-{}", uuid::Uuid::new_v4().simple());
            let profile = self.inner.data_dir.join("chzzk-browser-profile");
            std::fs::create_dir_all(&profile).map_err(|_| unavailable())?;
            let window = WebviewWindowBuilder::new(
                app,
                &label,
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .data_directory(profile)
            .browser_extensions_enabled(true)
            .visible(false)
            .focused(false)
            .skip_taskbar(true)
            .on_navigation(|url| url.as_str() == "about:blank")
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build()
            .map_err(|_| unavailable())?;
            cleanup_label = Some(label);
            let view: Webview = window.as_ref().clone();
            Ok(view)
        })();
        let view = match view {
            Ok(view) => view,
            Err(e) => {
                self.profile_clear_completed(app, false, cleanup_label.as_deref());
                return Err(e);
            }
        };
        let host = self.clone();
        let handle = app.clone();
        let result = clear_profile(&view, move |success| {
            host.profile_clear_completed(&handle, success, cleanup_label.as_deref())
        });
        if result
            .as_ref()
            .is_err_and(|e| e.code == "BROWSER_LOGOUT_PENDING")
        {
            let mut s = self.inner.view.lock().map_err(|_| unavailable())?;
            if s.account_busy {
                s.login_status =
                    "로그인 정보 삭제 응답 대기 중 · 완료 전까지 녹화와 계정 변경이 잠깁니다"
                        .into();
            }
        }
        result
    }
    fn refresh_account_playback(&self, app: &AppHandle) {
        if self.inner.closing.load(Ordering::Acquire) {
            return;
        }
        if let Err(cause) = self.pause_multiview_for_account(app, false) {
            self.inner
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .error = Some(cause.message);
        }
        if self.multiview_active() {
            return;
        }
        let channel = self
            .inner
            .view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .channel
            .clone();
        if let (Some(view), Some(channel)) = (app.get_webview(WINDOW_LABEL), channel) {
            self.inner
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .ready = false;
            if let Ok(url) = format!("https://chzzk.naver.com/live/{channel}").parse() {
                let _ = view.navigate(url);
            }
        }
    }
    fn profile_clear_completed(&self, app: &AppHandle, success: bool, cleanup_label: Option<&str>) {
        if let Some(window) = cleanup_label.and_then(|label| app.get_webview_window(label)) {
            let _ = window.destroy();
        }
        // Keep every recording entry point reserved until old authenticated
        // pages have been replaced, including when clearing fails or is late.
        self.refresh_account_playback(app);
        let mut s = self.inner.view.lock().unwrap_or_else(|p| p.into_inner());
        s.account_busy = false;
        s.ready = false;
        s.auth_generation = s.auth_generation.wrapping_add(1);
        s.auth_checking = false;
        s.auth_error = None;
        s.auth_status = if success {
            auth::AuthStatus::SignedOut
        } else {
            auth::AuthStatus::Unknown
        };
        s.auth_last_probe = if success { Some(Instant::now()) } else { None };
        s.login_status = if success {
            "이 앱의 CHZZK 로그인 정보를 삭제했습니다"
        } else {
            "로그인 정보 삭제에 실패했습니다. 공식 화면에서 계정 상태를 확인해 주세요."
        }
        .into();
    }
    pub fn open_installer(&self, browser: InstallerBrowser) -> Result<(), StreamError> {
        open_installer(browser)
    }
    fn install_from_page(&self, browser: InstallerBrowser) {
        if let Err(cause) = self.open_installer(browser) {
            self.inner
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .error = Some(cause.message);
        }
    }
}

fn viewport_stale() -> StreamError {
    error(
        "VIEWPORT_STALE",
        "시청 영역을 새 화면에 다시 연결하고 있습니다.",
    )
}
fn viewport_failed() -> StreamError {
    error(
        "VIEWPORT_CLIP_FAILED",
        "시청 영역을 안전하게 배치하지 못해 화면을 숨겼습니다.",
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PixelViewport {
    bounds: [i32; 4], // parent-client x, y, width, height
    clip: [i32; 4],   // child-local left, top, right, bottom
    occluded: bool,
    occlusions: PixelOcclusions,
}
#[derive(Clone, Copy)]
struct SurfaceState {
    bounds: [i32; 4],
    clip: Option<[i32; 4]>,
    visible: bool,
    enabled: bool,
}
trait ViewportSurface {
    fn inspect(&mut self) -> Result<SurfaceState, StreamError>;
    fn clip(&mut self, rect: [i32; 4]) -> Result<(), StreamError>;
    fn clip_occlusions(
        &mut self,
        rect: [i32; 4],
        occlusions: PixelOcclusions,
    ) -> Result<(), StreamError> {
        if !occlusions.is_empty() {
            return Err(viewport_failed());
        }
        self.clip(rect)
    }
    fn occlusions_match(&mut self, _: [i32; 4], _: PixelOcclusions) -> Result<bool, StreamError> {
        Ok(false)
    }
    fn bounds(&mut self, rect: [i32; 4]) -> Result<(), StreamError>;
    fn visible(&mut self, visible: bool) -> Result<(), StreamError>;
    fn input(&mut self, enabled: bool) -> Result<(), StreamError>;
    fn focus_main(&mut self) -> Result<(), StreamError>;
}
fn intersect_clips(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    [left, top, a[2].min(b[2]).max(left), a[3].min(b[3]).max(top)]
}

// The same small transaction drives the native HWND and fault-injection tests.
// The intermediate region is a subset of BOTH old and new local clips: safe
// before the move and after it. Never hide/show an already visible valid view.
fn paint_viewport(
    surface: &mut impl ViewportSurface,
    desired: PixelViewport,
    current: impl Fn() -> bool,
) -> Result<(), StreamError> {
    if !current() {
        return Err(viewport_stale());
    }
    let check = || {
        if current() {
            Ok(())
        } else {
            Err(viewport_stale())
        }
    };
    let result = (|| {
        let before = surface.inspect()?;
        let restricted = if desired.occluded {
            [0; 4]
        } else {
            intersect_clips(before.clip.unwrap_or([0; 4]), desired.clip)
        };
        if before.clip != Some(restricted) {
            surface.clip(restricted)?;
        }
        check()?;
        if desired.occluded {
            // Verify the actual HWND region before leaving IsVisible enabled.
            if surface.inspect()?.clip != Some([0; 4]) {
                return Err(viewport_failed());
            }
            surface.input(false)?;
            check()?;
            surface.focus_main()?;
            check()?;
        }
        if before.bounds != desired.bounds {
            surface.bounds(desired.bounds)?;
        }
        check()?;
        // Bounds must actually have reached the child, not merely been queued.
        if surface.inspect()?.bounds != desired.bounds {
            return Err(viewport_failed());
        }
        if restricted != desired.clip || !desired.occlusions.is_empty() {
            surface.clip_occlusions(desired.clip, desired.occlusions)?;
        }
        check()?;
        let applied = surface.inspect()?;
        let clip_matches = if desired.occlusions.is_empty() {
            applied.clip == Some(desired.clip)
        } else {
            surface.occlusions_match(desired.clip, desired.occlusions)?
        };
        if !clip_matches || (desired.occluded && applied.enabled) {
            return Err(viewport_failed());
        }
        if !desired.occluded && !applied.enabled {
            surface.input(true)?;
            check()?;
        }
        if !before.visible {
            surface.visible(true)?;
        }
        check()?;
        Ok(())
    })();
    if result.is_err() {
        // A request superseded during this UI transaction cannot leave newly
        // exposed pixels behind. A stale callback at entry did nothing above.
        let _ = surface.input(false);
        let _ = surface.visible(false);
    }
    result
}

fn apply_viewport(
    view: &Webview,
    viewport: &BrowserViewport,
    inner: Arc<Inner>,
    revision: u64,
) -> Result<(), StreamError> {
    apply_pane_viewport(view, viewport, inner.viewport_revision.clone(), revision)
}
#[cfg(windows)]
pub(super) fn apply_pane_viewport(
    view: &Webview,
    viewport: &BrowserViewport,
    revision: Arc<AtomicU64>,
    expected: u64,
) -> Result<(), StreamError> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, POINT, RECT},
            Graphics::Gdi::{
                GetWindowRgnBox, ScreenToClient, COMPLEXREGION, NULLREGION, SIMPLEREGION,
            },
            UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, SetFocus},
            UI::WindowsAndMessaging::{
                GetParent, GetWindowLongPtrW, GetWindowRect, IsChild, SetWindowPos, ShowWindow,
                GWL_STYLE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WS_VISIBLE,
            },
        },
    };
    struct NativeSurface {
        hwnd: HWND,
        parent: HWND,
        controller: ICoreWebView2Controller,
        trusted_main: Option<HWND>,
        return_focus: bool,
    }
    impl ViewportSurface for NativeSurface {
        fn inspect(&mut self) -> Result<SurfaceState, StreamError> {
            unsafe {
                let mut rect = RECT::default();
                GetWindowRect(self.hwnd, &mut rect).map_err(|_| viewport_failed())?;
                let mut point = POINT {
                    x: rect.left,
                    y: rect.top,
                };
                if !ScreenToClient(self.parent, &mut point).as_bool() {
                    return Err(viewport_failed());
                }
                let bounds = [
                    point.x,
                    point.y,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ];
                let mut region = RECT::default();
                let kind = GetWindowRgnBox(self.hwnd, &mut region);
                let clip = if kind == SIMPLEREGION {
                    Some([region.left, region.top, region.right, region.bottom])
                } else if kind == NULLREGION {
                    Some([0; 4])
                } else if kind.0 == 0 || kind == COMPLEXREGION {
                    // A complex popup region has no single safe visible rect.
                    // Transitions first restrict it; final holes are verified
                    // with EqualRgn rather than accepting the bounding box.
                    None
                } else {
                    return Err(viewport_failed());
                };
                let mut visible = BOOL::default();
                self.controller
                    .IsVisible(&mut visible)
                    .map_err(|_| viewport_failed())?;
                Ok(SurfaceState {
                    bounds,
                    clip,
                    visible: visible.as_bool()
                        && GetWindowLongPtrW(self.hwnd, GWL_STYLE) & WS_VISIBLE.0 as isize != 0,
                    enabled: IsWindowEnabled(self.hwnd).as_bool(),
                })
            }
        }
        fn clip(&mut self, rect: [i32; 4]) -> Result<(), StreamError> {
            native_region::Region::new(rect, PixelOcclusions::default())?.apply(self.hwnd)
        }
        fn clip_occlusions(
            &mut self,
            rect: [i32; 4],
            occlusions: PixelOcclusions,
        ) -> Result<(), StreamError> {
            native_region::Region::new(rect, occlusions)?.apply(self.hwnd)
        }
        fn occlusions_match(
            &mut self,
            rect: [i32; 4],
            occlusions: PixelOcclusions,
        ) -> Result<bool, StreamError> {
            native_region::Region::new(rect, occlusions)?.matches_window(self.hwnd)
        }
        fn bounds(&mut self, rect: [i32; 4]) -> Result<(), StreamError> {
            let old = self.inspect()?.bounds;
            unsafe {
                let same_size = old[2..] == rect[2..];
                // Wry 0.55.1 set_bounds_inner uses these exact native calls and
                // bounds() reads the HWND. No Wry position cache is bypassed.
                // A pure scroll move never resizes the official DOM/video.
                if !same_size {
                    self.controller
                        .SetBounds(RECT {
                            left: 0,
                            top: 0,
                            right: rect[2],
                            bottom: rect[3],
                        })
                        .map_err(|_| viewport_failed())?;
                }
                SetWindowPos(
                    self.hwnd,
                    None,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    SWP_NOACTIVATE
                        | SWP_NOZORDER
                        | if same_size {
                            SWP_NOSIZE
                        } else {
                            Default::default()
                        },
                )
                .map_err(|_| viewport_failed())?;
                Ok(())
            }
        }
        fn visible(&mut self, visible: bool) -> Result<(), StreamError> {
            unsafe {
                let _ = ShowWindow(self.hwnd, if visible { SW_SHOW } else { SW_HIDE });
                self.controller
                    .SetIsVisible(visible)
                    .map_err(|_| viewport_failed())
            }
        }
        fn input(&mut self, enabled: bool) -> Result<(), StreamError> {
            unsafe {
                if !enabled {
                    let focused = GetFocus();
                    self.return_focus |=
                        focused == self.hwnd || IsChild(self.hwnd, focused).as_bool();
                }
                let _ = EnableWindow(self.hwnd, enabled);
                if IsWindowEnabled(self.hwnd).as_bool() != enabled {
                    return Err(viewport_failed());
                }
                Ok(())
            }
        }
        fn focus_main(&mut self) -> Result<(), StreamError> {
            unsafe {
                let target = self.trusted_main.ok_or_else(viewport_failed)?;
                if target == self.hwnd
                    || target == self.parent
                    || GetParent(target).ok() != Some(self.parent)
                    || !IsWindowEnabled(target).as_bool()
                {
                    return Err(viewport_failed());
                }
                // Do not steal focus from an already-focused trusted modal or
                // account window. Only return focus formerly owned by this pane.
                if self.return_focus {
                    // Wry's child HWND WM_SETFOCUS handler calls the trusted
                    // main controller's MoveFocus(PROGRAMMATIC).
                    let _ = SetFocus(Some(target));
                    let focused = GetFocus();
                    if focused != target && !IsChild(target, focused).as_bool() {
                        return Err(viewport_failed());
                    }
                }
                let focused = GetFocus();
                if focused == self.hwnd || IsChild(self.hwnd, focused).as_bool() {
                    return Err(viewport_failed());
                }
                Ok(())
            }
        }
    }
    let scale = view.window().scale_factor().map_err(|_| unavailable())?;
    let desired = PixelViewport {
        bounds: [viewport.x, viewport.y, viewport.width, viewport.height]
            .map(|v| (v * scale).round() as i32),
        clip: clip_pixels(viewport, scale)?,
        occluded: viewport.occluded,
        occlusions: occlusion_pixels(viewport, scale)?,
    };
    // HWND is not Send; pass its address, never call Tauri from with_webview.
    let parent = view.window().hwnd().map_err(|_| unavailable())?.0 as usize;
    let main_hwnd = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    if viewport.occluded {
        // Resolve the trusted Tauri identity, not a page-supplied HWND, title or
        // class. Both callbacks run on the UI thread. If lookup is unavailable
        // or has not completed by paint time, the transaction fails closed.
        let main = view
            .app_handle()
            .get_webview("main")
            .ok_or_else(viewport_failed)?;
        if main.window().label() != view.window().label() || main.label() == view.label() {
            return Err(viewport_failed());
        }
        let slot = main_hwnd.clone();
        main.with_webview(move |platform| unsafe {
            let mut hwnd = HWND::default();
            if platform.controller().ParentWindow(&mut hwnd).is_ok() {
                slot.store(hwnd.0 as usize, Ordering::Release);
            }
        })
        .map_err(|_| viewport_failed())?;
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let current = || revision.load(Ordering::Acquire) == expected;
        let result = (|| {
            if !current() {
                return Err(viewport_stale());
            }
            let parent = HWND(parent as *mut std::ffi::c_void);
            let controller = platform.controller();
            let mut hwnd = HWND::default();
            controller
                .ParentWindow(&mut hwnd)
                .map_err(|_| viewport_failed())?;
            if hwnd == parent || GetParent(hwnd).ok() != Some(parent) {
                return Err(viewport_failed());
            }
            paint_viewport(
                &mut NativeSurface {
                    hwnd,
                    parent,
                    controller,
                    trusted_main: match main_hwnd.load(Ordering::Acquire) {
                        0 => None,
                        value => Some(HWND(value as *mut std::ffi::c_void)),
                    },
                    return_focus: false,
                },
                desired,
                current,
            )
        })();
        let _ = tx.try_send(result);
    })
    .map_err(|_| unavailable())?;
    rx.recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| Err(viewport_failed()))
}
#[cfg(not(windows))]
pub(super) fn apply_pane_viewport(
    _: &Webview,
    _: &BrowserViewport,
    _: Arc<AtomicU64>,
    _: u64,
) -> Result<(), StreamError> {
    Err(error(
        "VIEWPORT_UNSUPPORTED",
        "이 환경에서는 공식 시청 영역의 안전한 잘림을 지원하지 않습니다.",
    ))
}

type ProfileCompletion = Arc<Mutex<Option<Box<dyn FnOnce(bool) + Send>>>>;
fn complete_profile_clear(completion: &ProfileCompletion, success: bool) {
    let callback = completion.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(callback) = callback {
        callback(success);
    }
}

#[cfg(windows)]
fn clear_profile(
    view: &Webview,
    completed: impl FnOnce(bool) + Send + 'static,
) -> Result<(), StreamError> {
    use webview2_com::{
        ClearBrowsingDataCompletedHandler,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile2, ICoreWebView2_13},
    };
    use windows::core::Interface;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let completion: ProfileCompletion = Arc::new(Mutex::new(Some(Box::new(move |success| {
        completed(success);
        let _ = tx.try_send(success);
    }))));
    let dispatch_completion = completion.clone();
    let dispatch = view.with_webview(move |platform| unsafe {
        let callback_completion = dispatch_completion.clone();
        let callback = ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
            complete_profile_clear(&callback_completion, result.is_ok());
            Ok(())
        }));
        let result = (|| -> windows::core::Result<()> {
            platform
                .controller()
                .CoreWebView2()?
                .cast::<ICoreWebView2_13>()?
                .Profile()?
                .cast::<ICoreWebView2Profile2>()?
                .ClearBrowsingDataAll(&callback)
        })();
        if result.is_err() {
            complete_profile_clear(&dispatch_completion, false);
        }
    });
    if dispatch.is_err() {
        complete_profile_clear(&completion, false);
    }
    match rx.recv_timeout(Duration::from_secs(10)){
        Ok(true)=>Ok(()),
        Ok(false)=>Err(error("BROWSER_LOGOUT_FAILED","로그인 정보 삭제에 실패했습니다. 공식 화면에서 계정 상태를 확인해 주세요.")),
        // A timeout is not cancellation. Keep account_busy until the actual
        // completion callback, even if the operation outlives this command.
        Err(_)=>Err(StreamError::new("BROWSER_LOGOUT_PENDING","로그인 정보 삭제 완료 응답을 기다리고 있습니다. 완료 전에는 녹화와 계정 변경을 시작할 수 없습니다.",false)),
    }
}
#[cfg(not(windows))]
fn clear_profile(
    _: &Webview,
    completed: impl FnOnce(bool) + Send + 'static,
) -> Result<(), StreamError> {
    completed(false);
    Err(unavailable())
}

fn checked_browser_executable(root: &Path, browser: InstallerBrowser) -> Option<PathBuf> {
    if !root.is_absolute()
        || root.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    let executable = root.join(browser.relative_executable());
    for (index, path) in executable.ancestors().enumerate() {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return None;
            }
        }
        if metadata.file_type().is_symlink()
            || (index == 0 && !metadata.is_file())
            || (index != 0 && !metadata.is_dir())
        {
            return None;
        }
    }
    std::fs::canonicalize(executable).ok()
}

fn open_installer(browser: InstallerBrowser) -> Result<(), StreamError> {
    #[cfg(windows)]
    {
        // Do not use ShellExecute(URL): it sends an Edge-only listing to the
        // default Chrome (or vice versa). No PATH lookup, shell, profile flag,
        // registry edit, executable URL, or page-controlled argument is used.
        let executable = [
            "LOCALAPPDATA",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
        ]
        .into_iter()
        .filter_map(std::env::var_os)
        .find_map(|root| checked_browser_executable(Path::new(&root), browser))
        .ok_or_else(|| browser.missing())?;
        std::process::Command::new(executable)
            .arg(browser.url())
            .spawn()
            .map_err(|_| {
                error(
                    "INSTALL_PAGE_FAILED",
                    "선택한 브라우저에서 공식 네이버 확장 설치 페이지를 열지 못했습니다.",
                )
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(browser.missing())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeSurface {
        state: SurfaceState,
        occlusions: PixelOcclusions,
        operations: Vec<&'static str>,
        revision: Arc<AtomicU64>,
        cancel_after: Option<&'static str>,
        fail_at: Option<&'static str>,
        ignore_bounds: bool,
        ignore_clip: bool,
        ignore_input: bool,
        ignore_occlusions: bool,
    }
    impl FakeSurface {
        fn new(visible: bool) -> Self {
            Self {
                state: SurfaceState {
                    bounds: [0, 0, 100, 100],
                    clip: Some([0, 0, 100, 80]),
                    visible,
                    enabled: true,
                },
                operations: vec![],
                occlusions: PixelOcclusions::default(),
                revision: Arc::new(AtomicU64::new(1)),
                cancel_after: None,
                fail_at: None,
                ignore_bounds: false,
                ignore_clip: false,
                ignore_input: false,
                ignore_occlusions: false,
            }
        }
        fn operation(&mut self, name: &'static str) -> Result<(), StreamError> {
            self.operations.push(name);
            if self.cancel_after == Some(name) {
                self.revision.fetch_add(1, Ordering::AcqRel);
            }
            if self.fail_at == Some(name) {
                Err(viewport_failed())
            } else {
                Ok(())
            }
        }
    }
    impl ViewportSurface for FakeSurface {
        fn inspect(&mut self) -> Result<SurfaceState, StreamError> {
            self.operation("inspect")?;
            Ok(self.state)
        }
        fn clip(&mut self, rect: [i32; 4]) -> Result<(), StreamError> {
            self.operation(if rect == [0; 4] {
                "mask"
            } else if rect == [0, 10, 100, 80] {
                "restrict"
            } else {
                "final"
            })?;
            if !self.ignore_clip {
                self.state.clip = Some(rect);
                self.occlusions = PixelOcclusions::default();
            }
            Ok(())
        }
        fn clip_occlusions(
            &mut self,
            rect: [i32; 4],
            occlusions: PixelOcclusions,
        ) -> Result<(), StreamError> {
            self.clip(rect)?;
            if !self.ignore_clip && !self.ignore_occlusions {
                self.occlusions = occlusions;
            }
            Ok(())
        }
        fn occlusions_match(
            &mut self,
            rect: [i32; 4],
            occlusions: PixelOcclusions,
        ) -> Result<bool, StreamError> {
            Ok(self.state.clip == Some(rect) && self.occlusions == occlusions)
        }
        fn bounds(&mut self, bounds: [i32; 4]) -> Result<(), StreamError> {
            self.operation("bounds")?;
            if !self.ignore_bounds {
                self.state.bounds = bounds;
            }
            Ok(())
        }
        fn visible(&mut self, visible: bool) -> Result<(), StreamError> {
            self.operation(if visible { "show" } else { "hide" })?;
            self.state.visible = visible;
            Ok(())
        }
        fn input(&mut self, enabled: bool) -> Result<(), StreamError> {
            self.operation(if enabled { "enable" } else { "disable" })?;
            if !self.ignore_input {
                self.state.enabled = enabled;
            }
            Ok(())
        }
        fn focus_main(&mut self) -> Result<(), StreamError> {
            self.operation("focus")
        }
    }
    fn scrolled() -> PixelViewport {
        PixelViewport {
            bounds: [0, -10, 100, 100],
            clip: [0, 10, 100, 100],
            occluded: false,
            occlusions: PixelOcclusions::default(),
        }
    }
    #[test]
    fn visible_scroll_updates_in_place_without_hide_show_or_duplicate_paint() {
        let mut surface = FakeSurface::new(true);
        paint_viewport(&mut surface, scrolled(), || true).unwrap();
        assert_eq!(
            surface.operations,
            ["inspect", "restrict", "bounds", "inspect", "final", "inspect"]
        );
        assert!(surface.state.visible);
        assert_eq!(surface.state.clip, Some(scrolled().clip));
        surface.operations.clear();
        paint_viewport(&mut surface, scrolled(), || true).unwrap();
        assert_eq!(surface.operations, ["inspect", "inspect", "inspect"]);
    }
    #[test]
    fn hidden_surface_is_shown_only_after_confirmed_bounds_and_clip() {
        let mut surface = FakeSurface::new(false);
        paint_viewport(&mut surface, scrolled(), || true).unwrap();
        assert_eq!(surface.operations.last(), Some(&"show"));
        assert_eq!(surface.state.bounds, scrolled().bounds);
        assert_eq!(surface.state.clip, Some(scrolled().clip));
    }
    #[test]
    fn cancellation_during_each_native_phase_finishes_hidden_without_late_show() {
        for phase in ["restrict", "bounds", "final", "show"] {
            let mut surface = FakeSurface::new(false);
            surface.cancel_after = Some(phase);
            let revision = surface.revision.clone();
            let result = paint_viewport(&mut surface, scrolled(), || {
                revision.load(Ordering::Acquire) == 1
            });
            assert_eq!(result.unwrap_err().code, "VIEWPORT_STALE", "{phase}");
            assert!(!surface.state.visible, "{phase}");
            assert_eq!(surface.operations.last(), Some(&"hide"));
            if phase != "show" {
                assert!(!surface.operations.contains(&"show"));
            }
        }
    }
    #[test]
    fn timed_out_callback_at_entry_does_not_hide_or_modify_a_newer_surface() {
        let mut surface = FakeSurface::new(true);
        let result = paint_viewport(&mut surface, scrolled(), || false);
        assert_eq!(result.unwrap_err().code, "VIEWPORT_STALE");
        assert!(surface.operations.is_empty());
        assert!(surface.state.visible);
    }
    #[test]
    fn native_failure_or_unapplied_bounds_never_expands_or_leaves_visible() {
        for phase in ["inspect", "restrict", "bounds", "final", "show"] {
            let mut surface = FakeSurface::new(false);
            surface.fail_at = Some(phase);
            assert!(paint_viewport(&mut surface, scrolled(), || true).is_err());
            assert!(!surface.state.visible, "{phase}");
            assert_eq!(surface.operations.last(), Some(&"hide"));
        }
        let mut surface = FakeSurface::new(true);
        surface.ignore_bounds = true;
        assert!(paint_viewport(&mut surface, scrolled(), || true).is_err());
        assert!(!surface.operations.contains(&"final"));
        assert!(!surface.state.visible);
    }
    #[test]
    fn intermediate_clip_is_within_old_and_new_visible_regions() {
        for (old, new, expected) in [
            ([0, 0, 100, 80], [0, 10, 100, 100], [0, 10, 100, 80]),
            ([0, 10, 100, 100], [0, 0, 100, 80], [0, 10, 100, 80]),
            ([0, 0, 20, 20], [40, 40, 80, 80], [40, 40, 40, 40]),
        ] {
            assert_eq!(intersect_clips(old, new), expected);
        }
    }
    fn masked() -> PixelViewport {
        PixelViewport {
            bounds: [0, 0, 100, 100],
            clip: [0; 4],
            occluded: true,
            occlusions: PixelOcclusions::default(),
        }
    }
    #[test]
    fn modal_masks_pixels_and_input_without_hiding_or_resizing_media() {
        let mut surface = FakeSurface::new(true);
        paint_viewport(&mut surface, masked(), || true).unwrap();
        assert_eq!(
            surface.operations,
            ["inspect", "mask", "inspect", "disable", "focus", "inspect", "inspect"]
        );
        assert_eq!(surface.state.bounds, masked().bounds);
        assert_eq!(surface.state.clip, Some([0; 4]));
        assert!(surface.state.visible);
        assert!(!surface.state.enabled);
        surface.operations.clear();
        paint_viewport(&mut surface, scrolled(), || true).unwrap();
        assert!(surface.state.visible && surface.state.enabled);
        assert_eq!(surface.state.clip, Some(scrolled().clip));
        assert!(!surface.operations.contains(&"hide") && !surface.operations.contains(&"show"));
        assert!(
            surface
                .operations
                .iter()
                .position(|op| *op == "final")
                .unwrap()
                < surface
                    .operations
                    .iter()
                    .position(|op| *op == "enable")
                    .unwrap()
        );
    }
    #[test]
    fn trusted_sheet_retains_paint_above_it_but_disables_all_native_input() {
        let mut surface = FakeSurface::new(true);
        let desired = PixelViewport {
            clip: [0, 0, 100, 70],
            ..masked()
        };
        let original_bounds = surface.state.bounds;
        paint_viewport(&mut surface, desired, || true).unwrap();
        assert!(surface.state.visible);
        assert!(!surface.state.enabled);
        assert_eq!(surface.state.bounds, original_bounds);
        assert_eq!(surface.state.clip, Some(desired.clip));
        assert!(!surface.operations.contains(&"hide"));
        assert!(!surface.operations.contains(&"bounds"));
        let disable = surface
            .operations
            .iter()
            .position(|op| *op == "disable")
            .unwrap();
        let reveal = surface
            .operations
            .iter()
            .rposition(|op| *op == "final")
            .unwrap();
        assert!(disable < reveal);
    }
    fn popup_occlusions() -> PixelOcclusions {
        let mut value = PixelOcclusions::default();
        value.rectangles[0] = [30, 20, 70, 50];
        value.length = 1;
        value
    }
    #[test]
    fn popup_holes_preserve_bounds_and_background_but_reject_unapplied_holes() {
        let desired = PixelViewport {
            clip: [0, 0, 100, 100],
            occlusions: popup_occlusions(),
            ..masked()
        };
        let mut surface = FakeSurface::new(true);
        paint_viewport(&mut surface, desired, || true).unwrap();
        assert!(surface.state.visible && !surface.state.enabled);
        assert_eq!(surface.state.bounds, [0, 0, 100, 100]);
        assert_eq!(surface.state.clip, Some(desired.clip));
        assert_eq!(surface.occlusions, desired.occlusions);
        assert!(!surface
            .operations
            .iter()
            .any(|name| matches!(*name, "bounds" | "hide" | "show")));
        let mut incorrect = FakeSurface::new(true);
        incorrect.ignore_occlusions = true;
        assert_eq!(
            paint_viewport(&mut incorrect, desired, || true)
                .unwrap_err()
                .code,
            "VIEWPORT_CLIP_FAILED"
        );
        assert!(!incorrect.state.visible && !incorrect.state.enabled);
        // Dismissal removes the holes and enables input only after repaint.
        paint_viewport(&mut surface, scrolled(), || true).unwrap();
        assert!(surface.occlusions.is_empty());
        assert!(surface.state.visible && surface.state.enabled);
    }
    #[test]
    fn popup_rectangles_are_bounded_and_round_outward_at_fractional_dpi() {
        let mut viewport = BrowserViewport {
            visible: true,
            occluded: true,
            preserve_background: true,
            width: 800.0,
            height: 600.0,
            clip: Some(BrowserClip {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            }),
            occlusions: vec![BrowserClip {
                x: 10.25,
                y: 20.25,
                width: 30.25,
                height: 40.25,
            }],
            ..BrowserViewport::default()
        };
        assert_eq!(
            occlusion_pixels(&viewport, 1.25).unwrap().rectangles(),
            &[[12, 25, 51, 76]]
        );
        assert_eq!(clip_pixels(&viewport, 1.25).unwrap(), [0, 0, 1000, 750]);
        viewport.occlusions = vec![viewport.occlusions[0].clone(); 9];
        assert!(viewport.validate().is_err());
        viewport.occlusions.truncate(1);
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 1000.0] {
            viewport.occlusions[0].x = invalid;
            assert!(viewport.validate().is_err());
        }
        viewport.occlusions[0].x = 10.0;
        viewport.preserve_background = false;
        assert!(viewport.validate().is_err());
        viewport.occlusions.clear();
        assert!(occlusion_pixels(&viewport, 1.25).unwrap().is_empty());
        assert_eq!(clip_pixels(&viewport, 1.25).unwrap(), [0; 4]);
    }
    #[cfg(windows)]
    #[test]
    fn native_popup_region_preserves_sides_and_bottom_not_just_the_bounding_box() {
        use windows::Win32::Graphics::Gdi::{EqualRgn, PtInRegion};
        let mut holes = popup_occlusions();
        holes.rectangles[1] = [60, 40, 80, 70];
        holes.length = 2;
        let region = native_region::Region::new([0, 0, 100, 100], holes).unwrap();
        for point in [[10, 30], [90, 30], [50, 90], [50, 10]] {
            assert!(unsafe { PtInRegion(region.0, point[0], point[1]) }.as_bool());
        }
        for point in [[50, 30], [70, 60], [65, 45]] {
            assert!(!unsafe { PtInRegion(region.0, point[0], point[1]) }.as_bool());
        }
        let full =
            native_region::Region::new([0, 0, 100, 100], PixelOcclusions::default()).unwrap();
        assert!(!unsafe { EqualRgn(region.0, full.0) }.as_bool());
        let same = native_region::Region::new([0, 0, 100, 100], holes).unwrap();
        assert!(unsafe { EqualRgn(region.0, same.0) }.as_bool());
    }
    #[test]
    fn trusted_sheet_clip_requires_explicit_occlusion_and_fails_closed_at_reveal() {
        let mut viewport = BrowserViewport {
            visible: true,
            occluded: true,
            preserve_background: true,
            width: 800.0,
            height: 600.0,
            clip: Some(BrowserClip {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 450.0,
            }),
            ..BrowserViewport::default()
        };
        assert_eq!(clip_pixels(&viewport, 1.25).unwrap(), [0, 0, 1000, 562]);
        viewport.preserve_background = false;
        assert_eq!(clip_pixels(&viewport, 1.25).unwrap(), [0; 4]);
        viewport.preserve_background = true;
        viewport.occluded = false;
        assert!(viewport.validate().is_err());
        viewport.occluded = true;
        viewport.clip = None;
        assert!(viewport.validate().is_err());
        for cancel in [false, true] {
            let mut surface = FakeSurface::new(true);
            if cancel {
                surface.cancel_after = Some("final");
            } else {
                surface.fail_at = Some("final");
            }
            let revision = surface.revision.clone();
            assert!(paint_viewport(
                &mut surface,
                PixelViewport {
                    clip: [0, 0, 100, 70],
                    ..masked()
                },
                || revision.load(Ordering::Acquire) == 1
            )
            .is_err());
            assert!(!surface.state.visible && !surface.state.enabled);
        }
    }
    #[test]
    fn modal_failures_and_cancellation_never_leave_an_interactive_surface() {
        for phase in ["mask", "disable", "focus", "show"] {
            for cancel in [false, true] {
                let mut surface = FakeSurface::new(false);
                if cancel {
                    surface.cancel_after = Some(phase);
                } else {
                    surface.fail_at = Some(phase);
                }
                let revision = surface.revision.clone();
                assert!(
                    paint_viewport(&mut surface, masked(), || revision.load(Ordering::Acquire)
                        == 1)
                    .is_err(),
                    "{phase}/{cancel}"
                );
                assert!(!surface.state.visible, "{phase}/{cancel}");
            }
        }
        for ignore_clip in [false, true] {
            let mut surface = FakeSurface::new(true);
            surface.ignore_clip = ignore_clip;
            surface.ignore_input = !ignore_clip;
            assert!(paint_viewport(&mut surface, masked(), || true).is_err());
            assert!(!surface.state.visible);
            assert!(!surface.operations.contains(&"show"));
        }
    }
    #[test]
    fn late_show_cannot_unmask_a_newer_modal_or_accept_equal_sequence_replacement() {
        let mut surface = FakeSurface::new(true);
        paint_viewport(&mut surface, masked(), || true).unwrap();
        surface.operations.clear();
        assert_eq!(
            paint_viewport(&mut surface, scrolled(), || false)
                .unwrap_err()
                .code,
            "VIEWPORT_STALE"
        );
        assert!(surface.operations.is_empty());
        assert!(surface.state.visible && !surface.state.enabled);
        assert_eq!(surface.state.clip, Some([0; 4]));
        let modal = BrowserViewport {
            epoch: 2,
            request_sequence: Some(9),
            visible: true,
            occluded: true,
            width: 800.0,
            height: 600.0,
            ..BrowserViewport::default()
        };
        let old_show = BrowserViewport {
            request_sequence: Some(8),
            occluded: false,
            ..modal.clone()
        };
        assert!(viewport_sequence(&old_show, &modal, 2).is_err());
        let same_sequence_show = BrowserViewport {
            occluded: false,
            ..modal.clone()
        };
        assert!(viewport_sequence(&same_sequence_show, &modal, 2).is_err());
        let restore = BrowserViewport {
            request_sequence: Some(10),
            ..same_sequence_show
        };
        assert_eq!(viewport_sequence(&restore, &modal, 2).unwrap(), Some(10));
    }
    #[test]
    fn modal_clip_is_always_empty_but_invalid_inputs_are_not_ignored() {
        let mut viewport = BrowserViewport {
            visible: true,
            occluded: true,
            width: 800.0,
            height: 600.0,
            ..BrowserViewport::default()
        };
        for clip in [
            None,
            Some(BrowserClip {
                x: 5.0,
                y: 5.0,
                width: 700.0,
                height: 500.0,
            }),
            Some(BrowserClip {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            }),
        ] {
            viewport.clip = clip;
            assert_eq!(clip_pixels(&viewport, 1.25).unwrap(), [0; 4]);
        }
        viewport.clip = Some(BrowserClip {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 600.0,
        });
        assert!(viewport.validate().is_err());
        viewport.clip = None;
        viewport.visible = false;
        assert!(viewport.validate().is_err());
        viewport.visible = true;
        viewport.height = 0.0;
        assert!(viewport.validate().is_err());
        assert!(serde_json::from_str::<BrowserViewport>(
            r#"{"x":0,"y":0,"width":800,"height":600,"visible":true,"occluded":"true"}"#
        )
        .is_err());
        let legacy: BrowserViewport =
            serde_json::from_str(r#"{"x":0,"y":0,"width":800,"height":600,"visible":true}"#)
                .unwrap();
        assert!(!legacy.occluded);
    }
    #[test]
    fn client_sequence_blocks_late_ingress_without_changing_the_current_intent() {
        let current = BrowserViewport {
            epoch: 3,
            request_sequence: Some(12),
            visible: false,
            ..BrowserViewport::default()
        };
        for visible in [false, true] {
            let old = BrowserViewport {
                request_sequence: Some(11),
                visible,
                ..current.clone()
            };
            assert_eq!(
                viewport_sequence(&old, &current, 3).unwrap_err().code,
                "VIEWPORT_STALE"
            );
        }
        assert_eq!(viewport_sequence(&current, &current, 3).unwrap(), Some(12));
        let changed_equal = BrowserViewport {
            visible: true,
            ..current.clone()
        };
        assert!(viewport_sequence(&changed_equal, &current, 3).is_err());
        let latest = BrowserViewport {
            request_sequence: Some(13),
            visible: true,
            ..current.clone()
        };
        assert_eq!(viewport_sequence(&latest, &current, 3).unwrap(), Some(13));
    }
    #[test]
    fn old_epoch_hide_and_native_replay_do_not_poison_the_new_epoch_sequence() {
        let current = BrowserViewport {
            epoch: 4,
            request_sequence: Some(2),
            visible: true,
            ..BrowserViewport::default()
        };
        let old_hide = BrowserViewport {
            epoch: 3,
            request_sequence: Some(9999),
            visible: false,
            ..current.clone()
        };
        assert_eq!(
            viewport_sequence(&old_hide, &current, 4).unwrap_err().code,
            "VIEWPORT_STALE"
        );
        let old_show = BrowserViewport {
            visible: true,
            ..old_hide.clone()
        };
        assert!(viewport_sequence(&old_show, &current, 4).is_err());
        let internal = BrowserViewport {
            request_sequence: None,
            ..current.clone()
        };
        assert_eq!(viewport_sequence(&internal, &current, 4).unwrap(), Some(2));
        let reset = BrowserViewport {
            request_sequence: None,
            ..current.clone()
        };
        let fresh_document = BrowserViewport {
            request_sequence: Some(1),
            ..current
        };
        assert_eq!(viewport_sequence(&old_hide, &reset, 4).unwrap(), None);
        assert_eq!(
            viewport_sequence(&fresh_document, &reset, 4).unwrap(),
            Some(1)
        );
        for invalid in [0, 9_007_199_254_740_992] {
            assert!(BrowserViewport {
                request_sequence: Some(invalid),
                ..BrowserViewport::default()
            }
            .validate()
            .is_err());
        }
    }
    #[test]
    fn native_gate_is_nonblocking_while_revision_can_cancel_a_pending_writer() {
        let root = tempfile::tempdir().unwrap();
        let host = OfficialBrowser::new(root.path().to_owned()).unwrap();
        let _writer = host.inner.viewport_writes.try_lock().unwrap();
        assert!(host.inner.viewport_writes.try_lock().is_err());
        host.inner.viewport_revision.store(5, Ordering::Release);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let inner = host.inner.clone();
        let worker = std::thread::spawn(move || {
            inner.viewport_revision.fetch_add(1, Ordering::AcqRel);
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
        assert_eq!(host.inner.viewport_revision.load(Ordering::Acquire), 6);
    }
    #[test]
    fn fresh_host_restores_only_the_explicit_reconnect_choice() {
        let root = tempfile::tempdir().unwrap();
        let fresh = OfficialBrowser::new(root.path().to_owned()).unwrap();
        let initial = fresh.inner.view.lock().unwrap();
        assert!(!initial.extension_reconnect_enabled);
        assert!(initial.loaded_extensions.is_empty());
        drop(initial);
        super::super::super::browser_extension::remember_connection(root.path()).unwrap();
        let reopened = OfficialBrowser::new(root.path().to_owned()).unwrap();
        let state = reopened.inner.view.lock().unwrap();
        assert!(state.extension_reconnect_enabled);
        assert!(state.loaded_extensions.is_empty());
        assert!(!state.extension_connecting);
        assert_eq!(state.extension, "not_connected");
    }
    #[test]
    fn viewport_preserves_negative_origin_but_rejects_invalid_clip() {
        let mut v = BrowserViewport {
            epoch: 0,
            request_sequence: None,
            x: 20.0,
            y: -100.0,
            width: 1000.0,
            height: 700.0,
            visible: true,
            occluded: false,
            preserve_background: false,
            occlusions: vec![],
            clip: Some(BrowserClip {
                x: 0.0,
                y: 100.0,
                width: 1000.0,
                height: 600.0,
            }),
        };
        assert!(v.validate().is_ok());
        v.clip.as_mut().unwrap().height = 701.0;
        assert!(v.validate().is_err());
        v.clip = None;
        v.x = f64::NAN;
        assert!(v.validate().is_err());
    }
    #[test]
    fn installer_navigation_only_accepts_the_two_exact_public_products() {
        assert_eq!(
            installation_browser(&INSTALL_URL.parse().unwrap()),
            Some(InstallerBrowser::Chrome)
        );
        assert_eq!(installation_browser(
            &"https://microsoftedge.microsoft.com/addons/detail/jedbgfnhnpbfcbplibkacnmiafbojobk"
                .parse()
                .unwrap()
        ), Some(InstallerBrowser::Edge));
        for url in [
            "https://chromewebstore.google.com/detail/unknown",
            "https://evil.test/ooadnieabchijkibjpeieeliohjidnjj",
            "file:///install.exe",
            "https://chromewebstore.google.com/not-detail/ooadnieabchijkibjpeieeliohjidnjj",
            "https://microsoftedge.microsoft.com/addons/detail/ooadnieabchijkibjpeieeliohjidnjj",
            "https://user@chromewebstore.google.com/detail/ooadnieabchijkibjpeieeliohjidnjj",
            "https://chromewebstore.google.com/detail/ooadnieabchijkibjpeieeliohjidnjj#other",
            "https://chromewebstore.google.com/detail/a/b/ooadnieabchijkibjpeieeliohjidnjj",
        ] {
            assert_eq!(installation_browser(&url.parse().unwrap()), None);
        }
    }
    #[test]
    fn store_browser_selection_is_explicit_and_drops_page_parameters() {
        let edge: InstallerBrowser = serde_json::from_str("\"edge\"").unwrap();
        let chrome: InstallerBrowser = serde_json::from_str("\"chrome\"").unwrap();
        assert_eq!(InstallerBrowser::default(), chrome);
        assert!(serde_json::from_str::<InstallerBrowser>("\"firefox\"").is_err());
        let url = "https://microsoftedge.microsoft.com/addons/detail/naver/jedbgfnhnpbfcbplibkacnmiafbojobk?hl=ko".parse().unwrap();
        assert_eq!(installation_browser(&url), Some(edge));
        assert_eq!(edge.url(), EDGE_INSTALL_URL);
        assert!(!edge.url().contains('?'));
        assert!(edge.relative_executable().ends_with("msedge.exe"));
        assert!(chrome.relative_executable().ends_with("chrome.exe"));
        // Store slugs are cosmetic and may be Korean/percent-encoded. Their
        // content never reaches the executable or its canonical URL argument.
        let korean = "https://microsoftedge.microsoft.com/addons/detail/네이버-동영상-플러그인/jedbgfnhnpbfcbplibkacnmiafbojobk?hl=ko".parse().unwrap();
        assert_eq!(installation_browser(&korean), Some(edge));
        let encoded = "https://chromewebstore.google.com/detail/%EB%84%A4%EC%9D%B4%EB%B2%84/ooadnieabchijkibjpeieeliohjidnjj".parse().unwrap();
        assert_eq!(installation_browser(&encoded), Some(chrome));
    }
    #[test]
    fn installer_executable_uses_only_the_selected_known_product_path() {
        let root = tempfile::tempdir().unwrap();
        let chrome = root
            .path()
            .join(InstallerBrowser::Chrome.relative_executable());
        std::fs::create_dir_all(chrome.parent().unwrap()).unwrap();
        std::fs::write(&chrome, b"fixture only, never executed").unwrap();
        assert_eq!(
            checked_browser_executable(root.path(), InstallerBrowser::Chrome),
            Some(std::fs::canonicalize(&chrome).unwrap())
        );
        assert!(checked_browser_executable(root.path(), InstallerBrowser::Edge).is_none());
        assert!(
            checked_browser_executable(Path::new("relative"), InstallerBrowser::Chrome).is_none()
        );
        assert!(
            checked_browser_executable(&root.path().join(".."), InstallerBrowser::Chrome).is_none()
        );
    }
    #[test]
    fn clip_rounds_inward_and_clamps_tolerated_css_rounding() {
        let v = BrowserViewport {
            width: 100.0,
            height: 50.0,
            clip: Some(BrowserClip {
                x: 0.3,
                y: 1.1,
                width: 100.0,
                height: 49.0,
            }),
            ..BrowserViewport::default()
        };
        assert_eq!(clip_pixels(&v, 1.25).unwrap(), [1, 2, 125, 62]);
        for scale in [f64::NAN, f64::INFINITY, 0.0, -1.0, 17.0] {
            assert!(clip_pixels(&v, scale).is_err());
        }
    }
    #[test]
    fn account_window_labels_do_not_match_unrelated_windows() {
        assert!(account_label(LOGIN_LABEL));
        assert!(account_label(
            "chzzk-login-0123456789abcdef0123456789abcdef"
        ));
        for label in [
            "main",
            "chzzk-login-other",
            "chzzk-login",
            "chzzk-login-../main",
        ] {
            if label != LOGIN_LABEL {
                assert!(!account_label(label));
            }
        }
    }
    #[test]
    fn profile_completion_remains_reserved_until_callback_and_fires_once() {
        let busy = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (observed_busy, observed_calls) = (busy.clone(), calls.clone());
        let completion: ProfileCompletion = Arc::new(Mutex::new(Some(Box::new(move |success| {
            assert!(success);
            observed_busy.store(false, Ordering::Release);
            observed_calls.fetch_add(1, Ordering::AcqRel);
        }))));
        // The caller returning/timing out does not consume native completion.
        let late_callback = completion.clone();
        drop(completion);
        assert!(busy.load(Ordering::Acquire));
        complete_profile_clear(&late_callback, true);
        complete_profile_clear(&late_callback, false);
        assert!(!busy.load(Ordering::Acquire));
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn account_actions_work_before_playback_and_during_idle_multiview() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        assert!(!root.snapshot().unwrap().window_open);
        root.reserve_account_window().unwrap();
        assert!(root.snapshot().unwrap().account_busy);
        assert!(root.reserve_account_window().is_err());
        root.account_window_created(false);
        root.set_multiview_active_for_test();
        root.reserve_account_window().unwrap();
        root.account_window_created(false);
        root.inner
            .contexts
            .reconfiguring
            .store(1, Ordering::Release);
        assert!(root.reserve_account_window().is_err());
    }

    #[test]
    fn account_actions_preserve_a_recording_in_any_pane_and_block_pending_start() {
        let dir = tempfile::tempdir().unwrap();
        let root = OfficialBrowser::new(dir.path().into()).unwrap();
        let child = root
            .pane_controller(
                &format!("chzzk-mado-{}", uuid::Uuid::new_v4().simple()),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap();
        root.set_multiview_active_for_test();
        child.inner.view.lock().unwrap().recording = Some("synthetic-recording".into());
        assert!(root.reserve_account_window().is_err());
        assert!(!root.snapshot().unwrap().account_busy);
        assert_eq!(
            child.inner.view.lock().unwrap().recording.as_deref(),
            Some("synthetic-recording")
        );
        {
            let mut state = child.inner.view.lock().unwrap();
            state.recording = None;
            state.ready = true;
        }
        child.request_control(ControlAction::RecordStart).unwrap();
        let pending = child.snapshot().unwrap().pending_control.unwrap();
        root.reserve_account_window().unwrap();
        assert!(child.primary_profile_busy());
        assert!(child.take_control(&pending.id, true, true).is_err());
        assert!(child
            .inner
            .view
            .lock()
            .unwrap()
            .confirming_control
            .is_none());
        assert!(root.active_ids().is_empty());
    }
}
