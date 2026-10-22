//! Explicit consent-gated in-app reverse search. Provider workers receive only
//! a frozen media selection, never an X profile, cookie or account credential.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};
#[path = "x_media_search_provider.rs"]
mod provider;

const MAX_FRAME: usize = 4 * 1024 * 1024;
const MAX_PAYLOAD: usize = 5_594_512;
const MAX_SIDE: u32 = 2048;
const CONSENT_TTL: Duration = Duration::from_secs(120);
const PROVIDER_TTL: Duration = Duration::from_secs(30);
// The provider owns its 30-second work budget. This outer watchdog also covers
// its bounded close acknowledgement (2s) and dispatch/result publication (3s),
// so a result obtained within the work budget survives native window cleanup.
// It does not extend provider work, retry a request or delay explicit cancel.
const PROVIDER_WATCHDOG_TTL: Duration = PROVIDER_TTL.saturating_add(Duration::from_secs(5));
const VERIFICATION_TTL: Duration = Duration::from_secs(600);
const INVALID: &str = "선택한 이미지나 현재 장면을 확인하지 못했습니다. 다시 선택해 주세요.";
const STALE_SEARCH: &str = "검색을 준비하는 동안 계정이나 화면이 바뀌었습니다. 다시 선택해 주세요.";
const BUSY: &str = "이미 검색을 준비하거나 진행 중입니다. 현재 검색을 닫은 뒤 다시 선택해 주세요.";
const SETTINGS_ERROR: &str =
    "검색 동의 설정을 저장하지 못했습니다. 다시 묻지 않기를 해제하고 다시 검색해 주세요.";
static EXECUTING: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
pub(super) fn available() -> bool {
    // Menu/bridge availability is independent of provider result readiness.
    // The per-kind capture gate below and provider.run still fail closed.
    true
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub(super) struct Capabilities {
    image: bool,
    video: bool,
}
impl Capabilities {
    fn allows(self, request: &Request) -> bool {
        match (request.kind, request.provider) {
            (Kind::Image, Provider::Saucenao) => self.image,
            (Kind::Video, Provider::Lens) => self.video,
            _ => false,
        }
    }
}
pub(super) fn capabilities() -> Capabilities {
    Capabilities {
        image: provider::image_available(),
        video: provider::video_available(),
    }
}
pub(crate) fn busy() -> bool {
    EXECUTING.load(Ordering::Acquire) || provider::busy()
}
/// Called by the existing blocking graceful drain, never waits on the GUI.
pub(crate) fn shutdown(app: &AppHandle) {
    SHUTTING_DOWN.store(true, Ordering::Release);
    let removed = {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(job) = data.current.take() {
            job.cancel.store(true, Ordering::Release);
            true
        } else {
            false
        }
    };
    if removed {
        publish(app, None);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Image,
    Video,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Saucenao,
    Lens,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    id: String,
    kind: Kind,
    provider: Provider,
    path: String,
}
impl Request {
    pub(super) fn parse(value: &str) -> Option<Self> {
        if value.len() > 1024 {
            return None;
        }
        let request: Self = serde_json::from_str(value).ok()?;
        (actions::canonical_request_id(&request.id)
            && actions::base_post_path(&request.path)
            && matches!(
                (request.kind, request.provider),
                (Kind::Image, Provider::Saucenao) | (Kind::Video, Provider::Lens)
            ))
        .then_some(request)
    }
}
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Selection {
    Image {
        id: String,
        provider: Provider,
        path: String,
        source_url: String,
        image_url: String,
    },
    Video {
        id: String,
        provider: Provider,
        path: String,
        source_url: String,
        data_url: String,
        width: u32,
        height: u32,
        time: f64,
    },
}
#[derive(Debug)]
enum Prepared {
    Image(String),
    Video { jpeg: Vec<u8> },
}
pub(super) enum ProviderInput {
    ImageUrl(String),
    Jpeg(Vec<u8>),
}
pub(super) struct ProviderRequest {
    pub job_id: String,
    pub input: ProviderInput,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchLink {
    pub label: String,
    pub url: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Candidate {
    pub title: String,
    pub creator: Option<String>,
    pub similarity: Option<f64>,
    pub links: Vec<SearchLink>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XSearchSnapshot {
    id: String,
    generation: u64,
    account_id: String,
    feed: String,
    pane: String,
    kind: Kind,
    provider: Provider,
    phase: String,
    image_url: Option<String>,
    preview_data: Option<String>,
    message: String,
    candidates: Vec<Candidate>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Capturing,
    Consent,
    Running,
    Verification,
    Finished,
}
struct Job {
    snapshot: XSearchSnapshot,
    pane: Arc<Pane>,
    source: Url,
    revision: u64,
    stage: Stage,
    since: Instant,
    cancel: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    prepared: Option<Prepared>,
}
#[derive(Clone)]
struct Fence {
    id: String,
    generation: u64,
    account_id: String,
    feed: String,
    pane_name: String,
    pane: Arc<Pane>,
    source: Url,
    revision: u64,
    cancel: Arc<AtomicBool>,
}
impl Fence {
    fn of(job: &Job) -> Self {
        Self {
            id: job.snapshot.id.clone(),
            generation: job.snapshot.generation,
            account_id: job.snapshot.account_id.clone(),
            feed: job.snapshot.feed.clone(),
            pane_name: job.snapshot.pane.clone(),
            pane: job.pane.clone(),
            source: job.source.clone(),
            revision: job.revision,
            cancel: job.cancel.clone(),
        }
    }
    fn matches(&self, job: &Job) -> bool {
        self.id == job.snapshot.id && Arc::ptr_eq(&self.cancel, &job.cancel)
    }
}
fn current_fence(id: &str) -> Option<Fence> {
    read_fence(controller(), id)
}
fn read_fence(state: &Mutex<Controller>, id: &str) -> Option<Fence> {
    state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .current
        .as_ref()
        .filter(|job| job.snapshot.id == id)
        .map(Fence::of)
}
#[derive(Default)]
struct Controller {
    current: Option<Job>,
    seen: VecDeque<(String, String, Instant)>,
}
fn controller() -> &'static Mutex<Controller> {
    static STATE: OnceLock<Mutex<Controller>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}
struct WorkerSlot;
impl WorkerSlot {
    fn claim() -> Option<Self> {
        EXECUTING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}
impl Drop for WorkerSlot {
    fn drop(&mut self) {
        EXECUTING.store(false, Ordering::Release);
    }
}

fn image_url(value: &str) -> Option<Url> {
    if value.is_empty()
        || value.len() > 2048
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
        || value.contains(['\\', '%'])
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.as_str() != value
        || url.scheme() != "https"
        || url.host_str() != Some("pbs.twimg.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let leaf = url.path().strip_prefix("/media/")?;
    let (id, extension) = leaf
        .rsplit_once('.')
        .map_or((leaf, None), |(id, ext)| (id, Some(ext)));
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || extension.is_some_and(|ext| !matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "gif"))
    {
        return None;
    }
    let mut format = false;
    let mut name = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "format"
                if !format && matches!(value.as_ref(), "jpg" | "jpeg" | "png" | "webp" | "gif") =>
            {
                if extension.is_some_and(|ext| ext != value) {
                    return None;
                }
                format = true;
            }
            "name"
                if !name
                    && !value.is_empty()
                    && value.len() <= 24
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) =>
            {
                name = true
            }
            _ => return None,
        }
    }
    (extension.is_some() || format).then_some(url)
}
fn prepare(raw: &str, request: &Request, source: &Url) -> Result<Prepared, &'static str> {
    if raw.len() > MAX_PAYLOAD {
        return Err(INVALID);
    }
    let selection: Selection = serde_json::from_str(raw).map_err(|_| INVALID)?;
    match selection {
        Selection::Image {
            id,
            provider,
            path,
            source_url,
            image_url: image,
        } => {
            if request.kind != Kind::Image
                || request.provider != provider
                || request.id != id
                || request.path != path
                || source.as_str() != source_url
            {
                return Err(INVALID);
            }
            Ok(Prepared::Image(
                image_url(&image).ok_or(INVALID)?.to_string(),
            ))
        }
        Selection::Video {
            id,
            provider,
            path,
            source_url,
            data_url,
            width,
            height,
            time,
        } => {
            if request.kind != Kind::Video
                || request.provider != provider
                || request.id != id
                || request.path != path
                || source.as_str() != source_url
                || !time.is_finite()
                || !(0.0..=864_000.0).contains(&time)
                || width == 0
                || height == 0
                || width > MAX_SIDE
                || height > MAX_SIDE
            {
                return Err(INVALID);
            }
            let encoded = data_url
                .strip_prefix("data:image/jpeg;base64,")
                .ok_or(INVALID)?;
            if encoded.len() > MAX_FRAME.div_ceil(3) * 4 {
                return Err(INVALID);
            }
            let jpeg = STANDARD.decode(encoded).map_err(|_| INVALID)?;
            if jpeg.len() > MAX_FRAME || !jpeg.starts_with(&[0xff, 0xd8, 0xff]) {
                return Err(INVALID);
            }
            let mut reader = image::ImageReader::with_format(
                std::io::Cursor::new(&jpeg),
                image::ImageFormat::Jpeg,
            );
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_SIDE);
            limits.max_image_height = Some(MAX_SIDE);
            limits.max_alloc = Some(32 * 1024 * 1024);
            reader.limits(limits);
            let decoded = reader.decode().map_err(|_| INVALID)?;
            if decoded.width() != width || decoded.height() != height {
                return Err(INVALID);
            }
            // Export decoded pixels only; metadata and trailing bytes stay behind.
            let mut clean = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut clean, 90)
                .encode_image(&decoded)
                .map_err(|_| INVALID)?;
            if clean.len() > MAX_FRAME {
                return Err(INVALID);
            }
            Ok(Prepared::Video { jpeg: clean })
        }
    }
}
fn capture_current(
    session: &XSession,
    app: &AppHandle,
    pane: &Pane,
    generation: u64,
    source: &Url,
) -> bool {
    capture_pane_current(session, pane, generation, &pane.profile)
        && app.get_webview(&pane.label).is_some_and(|view| {
            view.url().is_ok_and(|url| &url == source)
                && view.window().is_visible().unwrap_or(false)
                && !view.window().is_minimized().unwrap_or(true)
        })
}
// Never call while holding controller(): the WebView URL getter synchronously
// dispatches to the GUI thread, whose IPC handler may need the same mutex.
fn owned_current(session: &XSession, app: &AppHandle, fence: &Fence) -> bool {
    fence_context_current(session, fence)
        && app
            .get_webview(&fence.pane.label)
            .is_some_and(|view| view.url().is_ok_and(|url| url == fence.source))
}
fn owned_context_current(session: &XSession, job: &Job) -> bool {
    fence_context_current(session, &Fence::of(job))
}
fn fence_context_current(session: &XSession, fence: &Fence) -> bool {
    if fence.revision != session.revision.load(Ordering::Acquire)
        || pane_state(&fence.pane) != "ready"
    {
        return false;
    }
    let account = session
        .registry
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|registry| {
            registry.active == fence.account_id
                && registry.accounts.iter().any(|entry| {
                    entry.id == fence.account_id
                        && entry.handle.eq_ignore_ascii_case(&fence.pane.owner)
                })
        });
    if !account {
        return false;
    }
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    let selected = if fence.pane_name == "preview" {
        data.preview
            .as_ref()
            .is_some_and(|pane| pane.label == fence.pane.label)
    } else {
        data.feeds
            .get(&data.active)
            .is_some_and(|pane| pane.label == fence.pane.label)
    };
    // Our aria-modal dialog makes viewport.visible and pane.visible false.
    // Frozen selections survive that presentation-only concealment. Privacy,
    // account and workspace changes revoke the generation/registered selection.
    data.generation == fence.generation && data.active == fence.feed && selected
}
fn publish(app: &AppHandle, value: Option<XSearchSnapshot>) {
    publish_with_consent_focus(app, value, None);
}
fn publish_initial_selection(app: &AppHandle, session: &XSession, value: XSearchSnapshot, fence: Fence) {
    let focus = (value.phase == "consent").then(|| (session.clone(), fence));
    publish_with_consent_focus(app, Some(value), focus);
}
fn publish_with_consent_focus(
    app: &AppHandle,
    value: Option<XSearchSnapshot>,
    initial_consent: Option<(XSession, Fence)>,
) {
    // Marshal publication to the GUI queue without any controller lock. Check
    // freshness there, then release the mutex before calling the event runtime.
    let owned = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some((session, fence)) = initial_consent {
            focus_initial_consent(&owned, &session, &fence);
        }
        // Native focus may pump a cancellation or account-change message.
        // Recheck publication after that call, never while holding its lock.
        let current = {
            let data = controller().lock().unwrap_or_else(|p| p.into_inner());
            match &value {
                Some(snapshot) => data.current.as_ref().is_some_and(|job| {
                    job.snapshot.id == snapshot.id && job.snapshot.phase == snapshot.phase
                }),
                None => data.current.is_none(),
            }
        };
        if current {
            let _ = owned.emit_to("main", "x:media-search", value);
        }
    });
}
fn consent_focus_job(job: &Job, fence: &Fence) -> bool {
    fence.matches(job)
        && job.stage == Stage::Consent
        && job.snapshot.phase == "consent"
        && job.prepared.is_some()
        && !job.cancel.load(Ordering::Acquire)
        && job.since.elapsed() < CONSENT_TTL
}
fn consent_focus_current(session: &XSession, fence: &Fence) -> bool {
    let current = controller()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .current
        .as_ref()
        .is_some_and(|job| consent_focus_job(job, fence));
    current && fence_context_current(session, fence)
}
#[derive(Clone, Copy)]
struct ConsentFocusScope {
    root: usize,
    main: usize,
    source: usize,
    main_parent: usize,
    source_parent: usize,
    foreground: bool,
    visible: bool,
    enabled: bool,
    source_has_focus: bool,
}
fn consent_focus_allowed(scope: ConsentFocusScope) -> bool {
    scope.root != 0
        && scope.main != 0
        && scope.source != 0
        && scope.main != scope.root
        && scope.source != scope.root
        && scope.main != scope.source
        && scope.main_parent == scope.root
        && scope.source_parent == scope.root
        && scope.foreground
        && scope.visible
        && scope.enabled
        && scope.source_has_focus
}
#[cfg(windows)]
fn focus_initial_consent(app: &AppHandle, session: &XSession, fence: &Fence) {
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC;
    use windows::{core::BOOL, Win32::{
        Foundation::HWND,
        UI::{Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled},
            WindowsAndMessaging::{GetForegroundWindow, GetParent, IsChild, IsIconic, IsWindowVisible}},
    }};
    // Called only on the GUI thread, before the first consent event conceals
    // the source pane. All Tauri calls are outside controller()/COM callbacks.
    if !consent_focus_current(session, fence) || !owned_current(session, app, fence) {
        return;
    }
    let Some(main) = app.get_webview("main").filter(|view| trusted(view).is_ok()) else { return };
    let Some(source) = app.get_webview(&fence.pane.label) else { return };
    if main.window().label() != "main" || source.window().label() != "main"
        || main.label() == source.label() { return; }
    let Ok(root) = main.window().hwnd().map(|hwnd| hwnd.0 as usize) else { return };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if source.with_webview(move |platform| unsafe {
        let mut hwnd = HWND::default();
        let address = platform.controller().ParentWindow(&mut hwnd).ok().map(|_| hwnd.0 as usize);
        let _ = tx.send(address);
    }).is_err() { return; }
    // Wry dispatches with_webview inline on its GUI thread. Never wait for a
    // callback here: an unavailable/non-inline runtime simply skips repair.
    let Ok(Some(source_address)) = rx.try_recv() else { return };
    if !owned_current(session, app, fence) { return; }
    let session = session.clone();
    let fence = fence.clone();
    let _ = main.with_webview(move |platform| unsafe {
        let root = HWND(root as *mut std::ffi::c_void);
        let source = HWND(source_address as *mut std::ffi::c_void);
        let controller = platform.controller();
        let mut main = HWND::default();
        let mut visible = BOOL::default();
        if controller.ParentWindow(&mut main).is_err()
            || controller.IsVisible(&mut visible).is_err() { return; }
        // COM can pump input/teardown. Revalidate the exact request and native
        // ownership afterwards; no locks are held across MoveFocus. Do not
        // activate a background window or steal another app/modal's focus.
        if !consent_focus_current(&session, &fence) { return; }
        let focused = GetFocus();
        let scope = ConsentFocusScope {
            root: root.0 as usize,
            main: main.0 as usize,
            source: source.0 as usize,
            main_parent: GetParent(main).ok().map_or(0, |hwnd| hwnd.0 as usize),
            source_parent: GetParent(source).ok().map_or(0, |hwnd| hwnd.0 as usize),
            foreground: GetForegroundWindow() == root,
            visible: visible.as_bool() && IsWindowVisible(root).as_bool()
                && !IsIconic(root).as_bool() && IsWindowVisible(main).as_bool()
                && IsWindowVisible(source).as_bool(),
            enabled: IsWindowEnabled(root).as_bool() && IsWindowEnabled(main).as_bool()
                && IsWindowEnabled(source).as_bool(),
            source_has_focus: focused == source || IsChild(source, focused).as_bool(),
        };
        if consent_focus_allowed(scope) {
            // Same as Wry WebView::focus; never Window::set_focus or OS
            // activation. The upcoming React dialog owns its final element.
            let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    });
}
#[cfg(not(windows))]
fn focus_initial_consent(_app: &AppHandle, _session: &XSession, _fence: &Fence) {}
fn notify_source(
    app: &AppHandle,
    session: &XSession,
    pane: &Pane,
    source: &Url,
    id: &str,
    message: &str,
    failed: bool,
) {
    if !is_current(session, pane) {
        return;
    }
    let Some(view) = app
        .get_webview(&pane.label)
        .filter(|view| view.url().is_ok_and(|url| &url == source))
    else {
        return;
    };
    let args =
        serde_json::json!([id,{"state":if failed{"failed"}else{"opened"},"message":message}]);
    let _ = view.eval(format!("window.__atsumiXActions?.searchResult(...{args})"));
}
fn first_request(data: &mut Controller, pane: &Pane, id: &str) -> bool {
    data.seen
        .retain(|(_, _, time)| time.elapsed() < CONSENT_TTL);
    if data.seen.len() >= 512
        || data
            .seen
            .iter()
            .any(|(nonce, request, _)| nonce == &pane.nonce && request == id)
    {
        return false;
    }
    data.seen
        .push_back((pane.nonce.clone(), id.into(), Instant::now()));
    true
}
fn clear_job(data: &mut Controller, id: &str) -> bool {
    if data
        .current
        .as_ref()
        .is_none_or(|job| job.snapshot.id != id)
    {
        return false;
    }
    if let Some(job) = data.current.take() {
        job.cancel.store(true, Ordering::Release);
    }
    true
}
fn fail(job: &mut Job, message: &str, drop_preview: bool) {
    job.cancel.store(true, Ordering::Release);
    job.prepared = None;
    job.stage = Stage::Finished;
    job.snapshot.phase = "failed".into();
    job.snapshot.message = message.into();
    job.snapshot.candidates.clear();
    if drop_preview {
        job.snapshot.image_url = None;
        job.snapshot.preview_data = None;
    }
}
fn expire(job: &mut Job, now: Instant) -> bool {
    let age = now.saturating_duration_since(job.since);
    let message = match job.stage {
        Stage::Consent if age >= CONSENT_TTL => {
            "검색 확인 시간이 만료되었습니다. 미디어를 다시 선택해 주세요."
        }
        Stage::Running if age >= PROVIDER_WATCHDOG_TTL => {
            "검색 서비스의 응답 시간이 초과되었습니다. 자동으로 다시 요청하지 않았습니다."
        }
        Stage::Verification if age >= VERIFICATION_TTL => {
            "사람 확인 대기 시간이 만료되었습니다. 검색을 다시 선택해 주세요."
        }
        _ => return false,
    };
    fail(job, message, true);
    true
}
fn maintain(session: &XSession, app: &AppHandle, id: &str) {
    let Some(fence) = current_fence(id) else {
        return;
    };
    let owned = owned_current(session, app, &fence);
    let event = {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let Some(job) = data.current.as_mut().filter(|job| fence.matches(job)) else {
            return;
        };
        if !owned
            || !owned_context_current(session, job)
            || (job.cancel.load(Ordering::Acquire) && job.stage != Stage::Finished)
        {
            clear_job(&mut data, id);
            Some(None)
        } else if expire(job, Instant::now()) {
            Some(Some(job.snapshot.clone()))
        } else {
            None
        }
    };
    if let Some(value) = event {
        publish(app, value);
    }
}
/// Invoke outside the surfaces lock after account/privacy/surface invalidation.
pub(super) fn invalidate(session: &XSession, app: &AppHandle) {
    let id = controller()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .current
        .as_ref()
        .map(|job| job.snapshot.id.clone());
    if let Some(id) = id {
        maintain(session, app, &id);
    }
}
pub(super) fn start(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, value: &str) {
    if !available() || SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let Some(request) = Request::parse(value) else {
        return;
    };
    // A verified provider must not enable capture for an unrelated provider.
    // Recheck natively before reading the source document or selected pixels.
    if !capabilities().allows(&request) {
        return;
    }
    if !interaction_allowed(session, pane) {
        return;
    }
    let Some(view) = app.get_webview(&pane.label) else {
        return;
    };
    let Ok(source) = view.url() else {
        return;
    };
    if !allowed(&source)
        || source.as_str().len() > 2048
        || source.path().starts_with("/__atsumi_surface__/")
    {
        return;
    }
    let (generation, feed) = {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        (data.generation, data.active.clone())
    };
    if !capture_current(session, app, pane, generation, &source) {
        return;
    }
    let Some(slot) = WorkerSlot::claim() else {
        notify_source(app, session, pane, &source, &request.id, BUSY, true);
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            return;
        }
        if !first_request(&mut data, pane, &request.id) {
            return;
        }
        if let Some(old) = data.current.take() {
            old.cancel.store(true, Ordering::Release);
        }
        data.current = Some(Job {
            snapshot: XSearchSnapshot {
                id: request.id.clone(),
                generation,
                account_id: pane.profile.clone(),
                feed,
                pane: if pane.feed == "preview" {
                    "preview"
                } else {
                    "feed"
                }
                .into(),
                kind: request.kind,
                provider: request.provider,
                phase: "searching".into(),
                image_url: None,
                preview_data: None,
                message: "선택한 미디어 확인 중…".into(),
                candidates: vec![],
            },
            pane: pane.clone(),
            source: source.clone(),
            revision: pane.revision,
            stage: Stage::Capturing,
            since: Instant::now(),
            cancel: cancel.clone(),
            resume: Arc::new(AtomicBool::new(false)),
            prepared: None,
        });
    }
    let host = session.clone();
    let owned = app.clone();
    let child = pane.clone();
    let watch_id = request.id.clone();
    // Cancellation/timeout does not detach the provider. Always await its worker;
    // the slot is returned only after provider cleanup has actually completed.
    tauri::async_runtime::spawn(async move {
        let worker_host = host.clone();
        let worker_app = owned.clone();
        let worker = tauri::async_runtime::spawn_blocking(move || {
            let _slot = slot;
            let result = run_selection(
                &worker_host,
                &worker_app,
                &child,
                &source,
                &request,
                generation,
                &cancel,
            );
            if let Err(message) = result {
                let fence = current_fence(&request.id);
                let owned = fence
                    .as_ref()
                    .is_some_and(|fence| owned_current(&worker_host, &worker_app, fence));
                let event = {
                    let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(job) = data.current.as_mut().filter(|job| {
                        fence.as_ref().is_some_and(|fence| fence.matches(job))
                            && !job.cancel.load(Ordering::Acquire)
                    }) {
                        if !owned || !owned_context_current(&worker_host, job) {
                            clear_job(&mut data, &request.id);
                            Some(None)
                        } else {
                            fail(job, &message, false);
                            Some(Some(job.snapshot.clone()))
                        }
                    } else {
                        None
                    }
                };
                if let Some(snapshot) = event {
                    publish(&worker_app, snapshot);
                    notify_source(
                        &worker_app,
                        &worker_host,
                        &child,
                        &source,
                        &request.id,
                        &message,
                        true,
                    );
                }
            }
        });
        while !worker.inner().is_finished() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            maintain(&host, &owned, &watch_id);
        }
        if worker.await.is_err() {
            let event = {
                let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
                data.current
                    .as_mut()
                    .filter(|job| {
                        job.snapshot.id == watch_id && !job.cancel.load(Ordering::Acquire)
                    })
                    .map(|job| {
                        fail(
                            job,
                            "검색 처리 중 오류가 발생했습니다. 자동으로 다시 요청하지 않았습니다.",
                            true,
                        );
                        job.snapshot.clone()
                    })
            };
            if let Some(snapshot) = event {
                publish(&owned, Some(snapshot));
            }
        }
    });
}
fn run_selection(
    session: &XSession,
    app: &AppHandle,
    pane: &Pane,
    source: &Url,
    request: &Request,
    generation: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    if !capture_current(session, app, pane, generation, source) {
        return Err(STALE_SEARCH.into());
    }
    let view = app.get_webview(&pane.label).ok_or(INVALID)?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let id = serde_json::to_string(&request.id).map_err(|_| INVALID)?;
    view.eval_with_callback(
        format!("window.__atsumiXActions?.takeSearch({id}) ?? null"),
        move |raw| {
            if raw.len() <= MAX_PAYLOAD {
                let _ = tx.send(raw);
            }
        },
    )
    .map_err(|_| INVALID)?;
    let raw = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| INVALID)?;
    if cancel.load(Ordering::Acquire) || !capture_current(session, app, pane, generation, source) {
        return Err(STALE_SEARCH.into());
    }
    let prepared = prepare(&raw, request, source)?;
    drop(raw);
    let remembered = load_consent(app)
        .map(|value| value.permits(request.provider))
        .unwrap_or(false);
    let fence = current_fence(&request.id).ok_or(STALE_SEARCH)?;
    if !owned_current(session, app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    let event = {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let job = data
            .current
            .as_mut()
            .filter(|job| fence.matches(job) && !job.cancel.load(Ordering::Acquire))
            .ok_or(STALE_SEARCH)?;
        if !owned_context_current(session, job) {
            return Err(STALE_SEARCH.into());
        }
        match &prepared {
            Prepared::Image(url) => job.snapshot.image_url = Some(url.clone()),
            Prepared::Video { jpeg } => {
                job.snapshot.preview_data =
                    Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)))
            }
        }
        job.prepared = Some(prepared);
        job.since = Instant::now();
        job.stage = if remembered {
            Stage::Running
        } else {
            Stage::Consent
        };
        job.snapshot.phase = if remembered { "searching" } else { "consent" }.into();
        job.snapshot.message=if remembered{"검색 중…"}else{"선택한 이미지 주소는 SauceNAO로, 현재 영상 장면 한 장은 Google Lens로 전송됩니다. X 계정·로그인 정보는 보내지 않습니다."}.into();
        job.snapshot.clone()
    };
    publish_initial_selection(app, session, event, fence);
    notify_source(
        app,
        session,
        pane,
        source,
        &request.id,
        "앱에서 검색 내용을 확인해 주세요.",
        false,
    );
    let input = loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let input = {
            let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
            let Some(job) = data
                .current
                .as_mut()
                .filter(|job| job.snapshot.id == request.id)
            else {
                return Ok(());
            };
            if job.stage == Stage::Finished {
                return Ok(());
            }
            if job.stage == Stage::Running {
                job.prepared.take()
            } else {
                None
            }
        };
        if let Some(value) = input {
            break value;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    maintain(session, app, &request.id);
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let input = match input {
        Prepared::Image(url) => ProviderInput::ImageUrl(url),
        Prepared::Video { jpeg } => ProviderInput::Jpeg(jpeg),
    };
    let resume = {
        let data = controller().lock().unwrap_or_else(|p| p.into_inner());
        data.current
            .as_ref()
            .filter(|job| job.snapshot.id == request.id)
            .ok_or(STALE_SEARCH)?
            .resume
            .clone()
    };
    let host = session.clone();
    let owned = app.clone();
    let verification_fence = current_fence(&request.id).ok_or(STALE_SEARCH)?;
    let handoff: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |message| {
        // A provider cannot resurrect a cancelled/replaced selection. GUI URL
        // reads stay outside the controller mutex, then the exact token and
        // source account/generation are rechecked before publishing the handoff.
        let still_owned = owned_current(&host, &owned, &verification_fence);
        let event = {
            let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
            let Some(job) = data
                .current
                .as_mut()
                .filter(|job| verification_fence.matches(job))
            else {
                return;
            };
            if !still_owned || !owned_context_current(&host, job) {
                clear_job(&mut data, &verification_fence.id);
                Some(None)
            } else if enter_verification(job, &message) {
                Some(Some(job.snapshot.clone()))
            } else {
                None
            }
        };
        if let Some(event) = event {
            publish(&owned, event);
        }
    });
    let outcome = provider::run(
        app.clone(),
        ProviderRequest {
            job_id: request.id.clone(),
            input,
        },
        cancel.clone(),
        resume,
        handoff,
    );
    maintain(session, app, &request.id);
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let candidates = validate_candidates(outcome.map_err(|message| {
        if valid_text(&message, 2048, false) {
            message
        } else {
            "검색 서비스의 응답을 확인하지 못했습니다.".into()
        }
    })?)?;
    let fence = current_fence(&request.id).ok_or(STALE_SEARCH)?;
    if !owned_current(session, app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    let event = {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let job = data
            .current
            .as_mut()
            .filter(|job| fence.matches(job) && !job.cancel.load(Ordering::Acquire))
            .ok_or(STALE_SEARCH)?;
        if !owned_context_current(session, job) {
            return Err(STALE_SEARCH.into());
        }
        job.stage = Stage::Finished;
        job.snapshot.phase = "complete".into();
        job.snapshot.message = if candidates.is_empty() {
            "검색 서비스에서 일치 후보를 찾지 못했습니다."
        } else {
            "검색 후보입니다. 유사도가 높아도 원작자나 원본임을 보장하지 않습니다."
        }
        .into();
        job.snapshot.candidates = candidates;
        job.snapshot.clone()
    };
    publish(app, Some(event));
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Consent {
    version: u8,
    providers: Vec<Provider>,
}
impl Consent {
    fn permits(&self, provider: Provider) -> bool {
        self.validate() && self.providers.contains(&provider)
    }
    fn validate(&self) -> bool {
        self.version == 1
            && self.providers.len() <= 2
            && (self.providers.len() != 2 || self.providers[0] != self.providers[1])
    }
}
fn read_consent(path: &Path) -> Result<Consent, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Consent {
                version: 1,
                providers: vec![],
            })
        }
        Err(_) => return Err(SETTINGS_ERROR.into()),
    };
    let mut bytes = Vec::new();
    file.take(1025)
        .read_to_end(&mut bytes)
        .map_err(|_| SETTINGS_ERROR)?;
    if bytes.len() > 1024 {
        return Err(SETTINGS_ERROR.into());
    }
    let value: Consent = serde_json::from_slice(&bytes).map_err(|_| SETTINGS_ERROR)?;
    if !value.validate() {
        return Err(SETTINGS_ERROR.into());
    }
    Ok(value)
}
fn load_consent(app: &AppHandle) -> Result<Consent, String> {
    read_consent(
        &app.path()
            .app_config_dir()
            .map_err(|_| SETTINGS_ERROR)?
            .join("x-media-search-consent.json"),
    )
}
fn write_consent(path: &Path, value: &Consent) -> Result<(), String> {
    if !value.validate() {
        return Err(SETTINGS_ERROR.into());
    }
    let parent = path.parent().ok_or(SETTINGS_ERROR)?;
    std::fs::create_dir_all(parent).map_err(|_| SETTINGS_ERROR)?;
    let temporary_path = parent.join(format!(
        "x-media-search-consent-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut temporary = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|_| SETTINGS_ERROR)?;
    let cleanup = TemporaryConsent(temporary_path);
    let written = temporary
        .write_all(&serde_json::to_vec(value).map_err(|_| SETTINGS_ERROR)?)
        .and_then(|_| temporary.sync_all());
    drop(temporary);
    written.map_err(|_| SETTINGS_ERROR)?;
    accounts::replace(&cleanup.0, path).map_err(|_| SETTINGS_ERROR.into())
}
// Only owns a fresh create_new temporary file, never an existing preference.
// Failed writes/replacements do not erase the previous consent selection.
struct TemporaryConsent(PathBuf);
impl Drop for TemporaryConsent {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn save_consent(app: &AppHandle, remember: bool) -> Result<(), String> {
    write_consent(
        &app.path()
            .app_config_dir()
            .map_err(|_| SETTINGS_ERROR)?
            .join("x-media-search-consent.json"),
        &Consent {
            version: 1,
            providers: if remember {
                vec![Provider::Saucenao, Provider::Lens]
            } else {
                vec![]
            },
        },
    )
}
fn valid_text(value: &str, limit: usize, empty: bool) -> bool {
    // Match JavaScript String.length and provider extraction exactly. Never
    // truncate a Unicode scalar to satisfy the UTF-16 code-unit transport cap.
    (empty || !value.trim().is_empty())
        && value.encode_utf16().count() <= limit
        && !value.chars().any(|c| c.is_control())
}
fn result_url(value: &str) -> Option<Url> {
    if value.is_empty()
        || value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return None;
    }
    // URL serialization expands Unicode and normalizes host casing/default
    // ports/dot segments. Apply the final transport bound after that expansion.
    (url.as_str().len() <= 2048).then_some(url)
}
fn validate_candidates(mut values: Vec<Candidate>) -> Result<Vec<Candidate>, String> {
    if values.len() > 10
        || values.iter().any(|item| {
            !valid_text(&item.title, 512, false)
                || item
                    .creator
                    .as_ref()
                    .is_some_and(|value| !valid_text(value, 256, false))
                || item
                    .similarity
                    .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
                || item.links.is_empty()
                || item.links.len() > 3
                || item.links.iter().any(|link| {
                    !valid_text(&link.label, 256, false) || result_url(&link.url).is_none()
                })
        })
    {
        return Err("검색 후보의 형식을 안전하게 확인하지 못했습니다.".into());
    }
    for candidate in &mut values {
        for link in &mut candidate.links {
            // The browser validator requires new URL(value).href === value.
            // Emit this canonical serialization, not an unnormalized source.
            link.url = result_url(&link.url)
                .ok_or("검색 후보의 링크를 안전하게 확인하지 못했습니다.")?
                .into();
        }
    }
    Ok(values)
}

#[tauri::command]
pub fn x_media_search_status(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
) -> Result<Option<XSearchSnapshot>, String> {
    trusted(&window)?;
    invalidate(&session, &app);
    Ok(controller()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .current
        .as_ref()
        .filter(|job| job.stage != Stage::Capturing)
        .map(|job| job.snapshot.clone()))
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_media_search_confirm(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
    remember: bool,
) -> Result<Option<XSearchSnapshot>, String> {
    trusted(&window)?;
    maintain(&session, &app, &id);
    let Some(fence) = current_fence(&id) else {
        return Ok(None);
    };
    if !owned_current(&session, &app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    let event = {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let Some(job) = data.current.as_mut().filter(|job| fence.matches(job)) else {
            return Ok(None);
        };
        if job.stage != Stage::Consent || job.cancel.load(Ordering::Acquire) {
            return Err("이미 확인했거나 만료된 검색입니다.".into());
        }
        if !owned_context_current(&session, job) {
            return Err(STALE_SEARCH.into());
        }
        if remember {
            save_consent(&app, true)?;
        }
        job.stage = Stage::Running;
        job.since = Instant::now();
        job.snapshot.phase = "searching".into();
        job.snapshot.message = "검색 중…".into();
        job.snapshot.clone()
    };
    publish(&app, Some(event.clone()));
    Ok(Some(event))
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_media_search_resume(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
) -> Result<Option<XSearchSnapshot>, String> {
    trusted(&window)?;
    maintain(&session, &app, &id);
    let Some(fence) = current_fence(&id) else {
        return Ok(None);
    };
    if !owned_current(&session, &app, &fence) {
        return Err(STALE_SEARCH.into());
    }
    let event = {
        let mut data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let Some(job) = data.current.as_mut().filter(|job| fence.matches(job)) else {
            return Ok(None);
        };
        if !owned_context_current(&session, job) {
            return Err(STALE_SEARCH.into());
        }
        if !resume_verification(job) {
            return Err("사람 확인을 기다리는 검색이 아닙니다.".into());
        }
        job.snapshot.clone()
    };
    publish(&app, Some(event.clone()));
    Ok(Some(event))
}
fn enter_verification(job: &mut Job, message: &str) -> bool {
    if job.stage != Stage::Running || job.cancel.load(Ordering::Acquire) {
        return false;
    }
    job.resume.store(false, Ordering::Release);
    job.stage = Stage::Verification;
    job.since = Instant::now();
    job.snapshot.phase = "verification".into();
    job.snapshot.message = if valid_text(message, 2048, false) {
        message
    } else {
        "검색 서비스 창에서 직접 확인한 뒤 계속해 주세요."
    }
    .into();
    true
}
fn resume_verification(job: &mut Job) -> bool {
    if job.stage != Stage::Verification || job.cancel.load(Ordering::Acquire) {
        return false;
    }
    job.stage = Stage::Running;
    job.since = Instant::now();
    job.snapshot.phase = "searching".into();
    job.snapshot.message = "확인한 페이지에서 검색 결과를 읽는 중…".into();
    // The worker reads the same document. This flag never authorizes another
    // upload and is consumed once; it is not a reusable verification token.
    job.resume.store(true, Ordering::Release);
    true
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_media_search_cancel(window: Webview, app: AppHandle, id: String) -> Result<(), String> {
    trusted(&window)?;
    let cleared = clear_job(
        &mut controller().lock().unwrap_or_else(|p| p.into_inner()),
        &id,
    );
    if cleared {
        publish(&app, None);
    }
    Ok(())
}
#[tauri::command]
pub fn x_media_search_reset_consent(window: Webview, app: AppHandle) -> Result<(), String> {
    trusted(&window)?;
    save_consent(&app, false)
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_media_search_open_result(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    id: String,
    candidate_index: usize,
    link_index: usize,
) -> Result<(), String> {
    trusted(&window)?;
    maintain(&session, &app, &id);
    let url = {
        let data = controller().lock().unwrap_or_else(|p| p.into_inner());
        let job = data
            .current
            .as_ref()
            .filter(|job| job.snapshot.id == id && job.snapshot.phase == "complete")
            .ok_or(STALE_SEARCH)?;
        let link = job
            .snapshot
            .candidates
            .get(candidate_index)
            .and_then(|item| item.links.get(link_index))
            .ok_or(INVALID)?;
        result_url(&link.url).ok_or(INVALID)?
    };
    open_result_browser(url.as_str())
}
fn open_result_browser(value: &str) -> Result<(), String> {
    if result_url(value).is_none() {
        return Err(INVALID.into());
    }
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let wide = value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err("외부 브라우저를 열지 못했습니다.".into())
        }
    }
    #[cfg(not(windows))]
    {
        Err("이 환경에서 외부 브라우저를 열 수 없습니다.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "30178d3a-9b74-4c38-9d12-777a97afff11";
    #[test]
    fn per_media_capabilities_do_not_enable_an_unverified_provider() {
        let image_request = request("image", "saucenao");
        let video_request = request("video", "lens");
        for image in [false, true] {
            for video in [false, true] {
                let capabilities = Capabilities { image, video };
                assert_eq!(capabilities.allows(&image_request), image);
                assert_eq!(capabilities.allows(&video_request), video);
                assert_eq!(
                    serde_json::to_value(capabilities).unwrap(),
                    serde_json::json!({"image": image, "video": video})
                );
                let mut mismatch = image_request.clone();
                mismatch.provider = Provider::Lens;
                assert!(!capabilities.allows(&mismatch));
                let mut mismatch = video_request.clone();
                mismatch.provider = Provider::Saucenao;
                assert!(!capabilities.allows(&mismatch));
            }
        }
    }
    #[test]
    fn native_capability_guard_precedes_document_access_and_capture() {
        let source = include_str!("x_surface_search.rs");
        let start = source.split("pub(super) fn start(").nth(1).unwrap();
        let guard = start.find("!capabilities().allows(&request)").unwrap();
        assert!(guard < start.find("app.get_webview(&pane.label)").unwrap());
        assert!(guard < start.find("capture_current(").unwrap());
        assert!(guard < start.find("WorkerSlot::claim()").unwrap());
        assert!(include_str!("x_surface.rs").contains("\"searchCapabilities\":search::capabilities()"));
    }
    fn job_fixture() -> (XSession, Job) {
        let session = XSession::default();
        let revision = session.revision.load(Ordering::Acquire);
        let pane = Arc::new(Pane {
            label: "search-fixture".into(),
            nonce: "fixture".into(),
            feed: "foryou".into(),
            owner: "alice".into(),
            profile: "legacy".into(),
            revision,
            state: Mutex::new("ready".into()),
            loading_since: Mutex::new(Instant::now()),
            document_load: Mutex::new(DocumentLoad::default()),
            root_recovery: Mutex::new(RootRecovery::default()),
            visible: AtomicBool::new(true),
            fullscreen: AtomicBool::new(false),
            media: Arc::new(surface_media::Observations::new(revision)),
        });
        *session.registry.lock().unwrap() = Some(accounts::Registry {
            active: "legacy".into(),
            accounts: vec![accounts::Account {
                id: "legacy".into(),
                handle: "alice".into(),
            }],
        });
        {
            let mut data = session.surfaces.lock().unwrap();
            data.active = "foryou".into();
            data.feeds.insert("foryou".into(), pane.clone());
        }
        let job = Job {
            snapshot: XSearchSnapshot {
                id: ID.into(),
                generation: 0,
                account_id: "legacy".into(),
                feed: "foryou".into(),
                pane: "feed".into(),
                kind: Kind::Image,
                provider: Provider::Saucenao,
                phase: "consent".into(),
                image_url: Some("https://pbs.twimg.com/media/fixture.jpg".into()),
                preview_data: None,
                message: "확인".into(),
                candidates: vec![],
            },
            pane,
            source: "https://x.com/home".parse().unwrap(),
            revision,
            stage: Stage::Consent,
            since: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            resume: Arc::new(AtomicBool::new(false)),
            prepared: Some(Prepared::Image(
                "https://pbs.twimg.com/media/fixture.jpg".into(),
            )),
        };
        (session, job)
    }
    #[test]
    fn search_consent_focus_requires_current_initial_consent_lease() {
        let (_, mut job) = job_fixture();
        let fence = Fence::of(&job);
        assert!(consent_focus_job(&job, &fence));
        for stage in [Stage::Capturing, Stage::Running, Stage::Verification, Stage::Finished] {
            job.stage = stage;
            assert!(!consent_focus_job(&job, &fence));
        }
        job.stage = Stage::Consent;
        for phase in ["searching", "verification", "complete", "failed"] {
            job.snapshot.phase = phase.into();
            assert!(!consent_focus_job(&job, &fence));
        }
        job.snapshot.phase = "consent".into();
        job.cancel.store(true, Ordering::Release);
        assert!(!consent_focus_job(&job, &fence));
        job.cancel.store(false, Ordering::Release);
        job.snapshot.id = "replacement-id".into();
        assert!(!consent_focus_job(&job, &fence));
        job.snapshot.id = ID.into();
        job.cancel = Arc::new(AtomicBool::new(false));
        assert!(!consent_focus_job(&job, &fence));
        job.cancel = fence.cancel.clone();
        job.since = Instant::now() - CONSENT_TTL;
        assert!(!consent_focus_job(&job, &fence));
        job.since = Instant::now();
        job.prepared = None;
        assert!(!consent_focus_job(&job, &fence));
    }
    #[test]
    fn search_consent_focus_does_not_activate_or_steal_another_surface() {
        let allowed = ConsentFocusScope {
            root: 1, main: 2, source: 3, main_parent: 1, source_parent: 1,
            foreground: true, visible: true, enabled: true, source_has_focus: true,
        };
        assert!(consent_focus_allowed(allowed));
        for denied in [
            ConsentFocusScope { root: 0, ..allowed },
            ConsentFocusScope { main: 0, ..allowed },
            ConsentFocusScope { source: 0, ..allowed },
            ConsentFocusScope { main: 1, ..allowed },
            ConsentFocusScope { source: 1, ..allowed },
            ConsentFocusScope { source: 2, ..allowed },
            ConsentFocusScope { main_parent: 4, ..allowed },
            ConsentFocusScope { source_parent: 4, ..allowed },
            ConsentFocusScope { foreground: false, ..allowed },
            ConsentFocusScope { visible: false, ..allowed },
            ConsentFocusScope { enabled: false, ..allowed },
            ConsentFocusScope { source_has_focus: false, ..allowed },
        ] {
            assert!(!consent_focus_allowed(denied));
        }
    }
    #[test]
    fn search_consent_focus_is_only_wired_to_first_selection_publication() {
        let source = include_str!("x_surface_search.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert_eq!(production.matches("publish_initial_selection(app, session, event, fence)").count(), 1);
        assert!(production.contains("publish_with_consent_focus(app, value, None)"));
        assert!(production.contains("(value.phase == \"consent\").then(|| (session.clone(), fence))"));
        let focus = production.split("fn focus_initial_consent(").nth(1).unwrap()
            .split("#[cfg(not(windows))]").next().unwrap();
        assert!(!focus.contains(".set_focus("));
        assert!(!focus.contains("SetForegroundWindow("));
        assert!(!focus.contains("recv_timeout("));
        assert!(focus.contains("MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)"));
    }
    #[test]
    fn search_frozen_context_survives_own_modal_not_account_or_surface_changes() {
        let (session, job) = job_fixture();
        assert!(owned_context_current(&session, &job));
        job.pane.visible.store(false, Ordering::Release);
        {
            let mut data = session.surfaces.lock().unwrap();
            data.viewport = Some(NativeViewport {
                sequence: 1,
                player: None,
                chat: None,
                visible: false,
                muted: true,
                occlusions: vec![],
            });
        }
        assert!(owned_context_current(&session, &job));
        session.surfaces.lock().unwrap().generation += 1;
        assert!(!owned_context_current(&session, &job));
        session.surfaces.lock().unwrap().generation = 0;
        session.surfaces.lock().unwrap().active = "likes".into();
        assert!(!owned_context_current(&session, &job));
        session.surfaces.lock().unwrap().active = "foryou".into();
        session.registry.lock().unwrap().as_mut().unwrap().accounts[0].handle = "bob".into();
        assert!(!owned_context_current(&session, &job));
        session.registry.lock().unwrap().as_mut().unwrap().accounts[0].handle = "alice".into();
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!owned_context_current(&session, &job));
    }
    #[test]
    fn search_cancellation_is_exact_and_releases_frozen_payload() {
        let (_, job) = job_fixture();
        let cancelled = job.cancel.clone();
        let mut data = Controller {
            current: Some(job),
            seen: VecDeque::new(),
        };
        assert!(!clear_job(&mut data, "old-id"));
        assert!(!cancelled.load(Ordering::Acquire));
        assert!(data.current.as_ref().unwrap().prepared.is_some());
        assert!(clear_job(&mut data, ID));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(data.current.is_none());
        assert!(!clear_job(&mut data, ID));
    }
    #[test]
    fn search_consent_and_provider_watchdog_deadlines_cancel_without_retry() {
        let (_, mut job) = job_fixture();
        let since = job.since;
        assert!(!expire(
            &mut job,
            since + CONSENT_TTL - Duration::from_millis(1)
        ));
        assert!(expire(&mut job, since + CONSENT_TTL));
        assert!(job.cancel.load(Ordering::Acquire));
        assert!(job.prepared.is_none());
        assert!(job.snapshot.image_url.is_none());
        assert!(job.snapshot.preview_data.is_none());
        assert_eq!(job.snapshot.phase, "failed");
        assert!(!expire(
            &mut job,
            since + CONSENT_TTL + Duration::from_secs(10)
        ));
        let (_, mut job) = job_fixture();
        job.stage = Stage::Running;
        job.snapshot.phase = "searching".into();
        let since = job.since;
        assert!(!expire(
            &mut job,
            since + PROVIDER_WATCHDOG_TTL - Duration::from_millis(1)
        ));
        assert!(expire(&mut job, since + PROVIDER_WATCHDOG_TTL));
        assert!(job.cancel.load(Ordering::Acquire));
        assert!(job.prepared.is_none());
        assert!(job.snapshot.image_url.is_none());
        assert!(job.snapshot.preview_data.is_none());
        assert_eq!(job.snapshot.phase, "failed");
    }
    #[test]
    fn search_provider_cleanup_grace_preserves_on_time_results() {
        assert_eq!(PROVIDER_TTL, Duration::from_secs(30));
        assert_eq!(PROVIDER_WATCHDOG_TTL, Duration::from_secs(35));
        let (_, mut job) = job_fixture();
        job.stage = Stage::Running;
        job.snapshot.phase = "searching".into();
        let since = job.since;
        for elapsed in [
            PROVIDER_TTL,
            PROVIDER_TTL + Duration::from_secs(2),
            PROVIDER_WATCHDOG_TTL - Duration::from_millis(1),
        ] {
            assert!(!expire(&mut job, since + elapsed));
            assert!(!job.cancel.load(Ordering::Acquire));
            assert_eq!(job.snapshot.phase, "searching");
        }
        // A provider result can be published only after its close ACK. Once
        // published, the outer watchdog must not expire this completed result.
        job.stage = Stage::Finished;
        job.snapshot.phase = "complete".into();
        job.snapshot.candidates = vec![Candidate {
            title: "Observed result".into(),
            creator: None,
            similarity: None,
            links: vec![SearchLink {
                label: "Source".into(),
                url: "https://example.org/observed".into(),
            }],
        }];
        assert!(!expire(
            &mut job,
            since + PROVIDER_WATCHDOG_TTL + Duration::from_secs(1)
        ));
        assert!(!job.cancel.load(Ordering::Acquire));
        assert_eq!(job.snapshot.phase, "complete");
        assert_eq!(job.snapshot.candidates.len(), 1);
    }
    #[test]
    fn search_provider_cleanup_grace_does_not_defer_explicit_cancel() {
        let (_, mut job) = job_fixture();
        job.stage = Stage::Running;
        job.snapshot.phase = "searching".into();
        let since = job.since;
        assert!(!expire(
            &mut job,
            since + PROVIDER_TTL + Duration::from_secs(2)
        ));
        let cancelled = job.cancel.clone();
        let mut data = Controller {
            current: Some(job),
            seen: VecDeque::new(),
        };
        assert!(clear_job(&mut data, ID));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(data.current.is_none());
    }
    #[test]
    fn search_manual_verification_requires_explicit_current_resume() {
        let (_, mut job) = job_fixture();
        assert!(!enter_verification(&mut job, "직접 확인"));
        assert!(!resume_verification(&mut job));
        job.stage = Stage::Running;
        job.prepared = None; // Already submitted once; resume cannot resubmit.
        assert!(enter_verification(&mut job, "직접 확인"));
        assert_eq!(job.snapshot.phase, "verification");
        assert!(!job.resume.load(Ordering::Acquire));
        assert!(!enter_verification(&mut job, "중복"));
        assert!(resume_verification(&mut job));
        assert!(job.resume.load(Ordering::Acquire));
        assert_eq!(job.snapshot.phase, "searching");
        assert!(job.prepared.is_none());
        assert!(!resume_verification(&mut job));
        assert!(enter_verification(&mut job, "아직 확인 필요"));
        assert!(!job.resume.load(Ordering::Acquire));
        job.cancel.store(true, Ordering::Release);
        assert!(!resume_verification(&mut job));
    }
    #[test]
    fn search_manual_verification_has_its_own_bounded_deadline() {
        let (_, mut job) = job_fixture();
        job.stage = Stage::Running;
        assert!(enter_verification(&mut job, "직접 확인"));
        let since = job.since;
        assert!(!expire(&mut job, since + PROVIDER_TTL));
        assert!(!expire(
            &mut job,
            since + VERIFICATION_TTL - Duration::from_millis(1)
        ));
        assert!(expire(&mut job, since + VERIFICATION_TTL));
        assert!(job.cancel.load(Ordering::Acquire));
        assert!(job.snapshot.image_url.is_none());
        assert!(!resume_verification(&mut job));
    }
    #[test]
    fn search_manual_verification_keeps_only_plain_bounded_messages() {
        let (_, mut job) = job_fixture();
        job.stage = Stage::Running;
        assert!(enter_verification(&mut job, "bad\nmessage"));
        assert!(!job.snapshot.message.contains('\n'));
        assert!(job.snapshot.message.contains("직접 확인"));
    }
    #[test]
    fn search_request_replay_is_bounded_and_document_nonce_scoped() {
        let (_, job) = job_fixture();
        let mut data = Controller::default();
        assert!(first_request(&mut data, &job.pane, ID));
        assert!(!first_request(&mut data, &job.pane, ID));
        data.seen[0].2 = Instant::now() - CONSENT_TTL;
        assert!(first_request(&mut data, &job.pane, ID));
        for index in 1..512 {
            data.seen
                .push_back(("other".into(), index.to_string(), Instant::now()));
        }
        assert!(!first_request(&mut data, &job.pane, "new-id"));
        assert_eq!(data.seen.len(), 512);
    }
    fn request(kind: &str, provider: &str) -> Request {
        Request::parse(&format!(r#"{{"id":"{ID}","kind":"{kind}","provider":"{provider}","path":"/artist/status/123456789"}}"#)).unwrap()
    }
    fn frame() -> Vec<u8> {
        let image = image::RgbImage::from_fn(160, 100, |x, y| {
            image::Rgb([(x * 255 / 160) as u8, (y * 255 / 100) as u8, 96])
        });
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85)
            .encode_image(&image)
            .unwrap();
        bytes
    }
    #[test]
    fn search_intent_is_closed_small_and_provider_bound() {
        assert!(Request::parse(&format!(
            r#"{{"id":"{ID}","kind":"video","provider":"saucenao","path":"/a/status/1"}}"#
        ))
        .is_none());
        for extra in [
            r#", "url":"https://evil.test""#,
            r#", "dataUrl":"data:image/png;base64,AA==""#,
        ] {
            assert!(Request::parse(&format!(r#"{{"id":"{ID}","kind":"image","provider":"saucenao","path":"/a/status/1"{extra}}}"#)).is_none());
        }
        assert!(Request::parse(&"a".repeat(1025)).is_none());
        assert!(Request::parse(
            r#"{"id":"bad","kind":"image","provider":"saucenao","path":"/a/status/1"}"#
        )
        .is_none());
    }
    #[test]
    fn search_image_url_is_exact_public_body_media() {
        for valid in [
            "https://pbs.twimg.com/media/AbC_-12.jpg",
            "https://pbs.twimg.com/media/AbC_-12?format=jpg&name=small",
        ] {
            assert!(image_url(valid).is_some());
        }
        for invalid in [
            "https://pbs.twimg.com/profile_images/123/a.jpg",
            "http://pbs.twimg.com/media/a.jpg",
            "https://evil.test/media/a.jpg",
            "https://user@pbs.twimg.com/media/a.jpg",
            "https://pbs.twimg.com:444/media/a.jpg",
            "https://pbs.twimg.com/media/a.jpg?token=secret",
            "https://pbs.twimg.com/media/a.jpg?format=png",
            "https://pbs.twimg.com/media/a?format=jpg&format=jpg",
            "https://pbs.twimg.com/media/a%2ejpg",
            "https://pbs.twimg.com/media/../a.jpg",
            "https://pbs.twimg.com/media/a.svg",
            "https://pbs.twimg.com/media/a.jpg#fragment",
        ] {
            assert!(image_url(invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn search_frame_requires_matching_request_source_and_decoded_jpeg() {
        let source: Url = "https://x.com/home".parse().unwrap();
        let req = request("video", "lens");
        let good = serde_json::json!({"id":ID,"kind":"video","provider":"lens","path":req.path,"sourceUrl":source.as_str(),"dataUrl":format!("data:image/jpeg;base64,{}",STANDARD.encode(frame())),"width":160,"height":100,"time":1.5});
        assert!(prepare(&good.to_string(), &req, &source).is_ok());
        for (key, value) in [
            ("width", serde_json::json!(2049)),
            ("height", serde_json::json!(99)),
            ("time", serde_json::json!(-1)),
            ("sourceUrl", serde_json::json!("https://x.com/bookmarks")),
            ("id", serde_json::json!(uuid::Uuid::new_v4().to_string())),
            ("provider", serde_json::json!("saucenao")),
            (
                "dataUrl",
                serde_json::json!("data:image/jpeg;base64,aGVsbG8="),
            ),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(prepare(&bad.to_string(), &req, &source).is_err(), "{key}");
        }
        let mut extra = good.clone();
        extra["cookie"] = serde_json::json!("secret");
        assert!(prepare(&extra.to_string(), &req, &source).is_err());
        assert!(prepare(&"a".repeat(MAX_PAYLOAD + 1), &req, &source).is_err());
        let marker = b"DO_NOT_EXPORT_TRAILING_SOURCE_DATA";
        let mut dirty = frame();
        dirty.extend_from_slice(marker);
        let mut trailing = good.clone();
        trailing["dataUrl"] =
            serde_json::json!(format!("data:image/jpeg;base64,{}", STANDARD.encode(dirty)));
        let Prepared::Video { jpeg } = prepare(&trailing.to_string(), &req, &source).unwrap()
        else {
            panic!()
        };
        assert!(!jpeg.windows(marker.len()).any(|part| part == marker));
    }
    #[test]
    fn search_image_payload_does_not_promote_to_unobserved_image() {
        let source: Url = "https://x.com/home".parse().unwrap();
        let req = request("image", "saucenao");
        let raw = serde_json::json!({"id":ID,"kind":"image","provider":"saucenao","path":req.path,"sourceUrl":source.as_str(),"imageUrl":"https://pbs.twimg.com/media/exact?format=jpg&name=small"});
        let Prepared::Image(url) = prepare(&raw.to_string(), &req, &source).unwrap() else {
            panic!()
        };
        assert!(url.ends_with("name=small"));
    }
    #[test]
    fn search_consent_has_no_implicit_grant_or_data_fields() {
        for raw in [
            r#"{"version":2,"providers":["lens"]}"#,
            r#"{"version":1,"providers":["lens","lens"]}"#,
            r#"{"version":1,"providers":["lens"],"image":"secret"}"#,
            r#"{"version":1,"providers":["other"]}"#,
        ] {
            assert!(!serde_json::from_str::<Consent>(raw)
                .map(|value| value.validate())
                .unwrap_or(false));
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("consent.json");
        assert!(!read_consent(&path).unwrap().permits(Provider::Lens));
        write_consent(
            &path,
            &Consent {
                version: 1,
                providers: vec![Provider::Lens, Provider::Saucenao],
            },
        )
        .unwrap();
        assert!(read_consent(&path).unwrap().permits(Provider::Lens));
        write_consent(
            &path,
            &Consent {
                version: 1,
                providers: vec![],
            },
        )
        .unwrap();
        assert!(!read_consent(&path).unwrap().permits(Provider::Lens));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    #[test]
    fn search_candidates_are_bounded_and_links_are_not_commands() {
        let good = Candidate {
            title: "원본 후보".into(),
            creator: None,
            similarity: Some(90.0),
            links: vec![SearchLink {
                label: "출처".into(),
                url: "https://www.pixiv.net/artworks/123".into(),
            }],
        };
        assert!(validate_candidates(vec![good.clone()]).is_ok());
        assert!(validate_candidates(vec![]).is_ok());
        assert!(validate_candidates(vec![good.clone(); 11]).is_err());
        for url in [
            "javascript:alert(1)",
            "file:///C:/secret",
            "https://user:pass@example.org/",
            "https://example.org/ bad",
            "https://example.org/\\other",
        ] {
            let mut bad = good.clone();
            bad.links[0].url = url.into();
            assert!(validate_candidates(vec![bad]).is_err());
        }
        for value in [f64::NAN, f64::INFINITY, -1.0, 101.0] {
            let mut bad = good.clone();
            bad.similarity = Some(value);
            assert!(validate_candidates(vec![bad]).is_err());
        }
        let mut bad = good;
        bad.links.clear();
        assert!(validate_candidates(vec![bad]).is_err());
    }
    #[test]
    fn search_slot_is_exactly_one_until_worker_drop() {
        let first = WorkerSlot::claim().unwrap();
        assert!(WorkerSlot::claim().is_none());
        drop(first);
        assert!(WorkerSlot::claim().is_some());
    }
    #[test]
    fn search_fence_releases_controller_before_runtime_probe_and_rejects_replacement() {
        let (_, job) = job_fixture();
        let state = Mutex::new(Controller {
            current: Some(job),
            seen: VecDeque::new(),
        });
        let fence = read_fence(&state, ID).unwrap();
        let mut data = state
            .try_lock()
            .expect("runtime probe must not inherit the controller guard");
        assert!(fence.matches(data.current.as_ref().unwrap()));
        let (_, replacement) = job_fixture();
        data.current = Some(replacement);
        assert!(
            !fence.matches(data.current.as_ref().unwrap()),
            "same request text must not authorize a replacement lease"
        );
    }
    #[test]
    fn search_result_urls_are_browser_canonical_and_bounded_after_expansion() {
        for (raw, expected) in [
            ("https://EXAMPLE.org:443", "https://example.org/"),
            ("http://example.org:80/a/../b", "http://example.org/b"),
            (
                "https://example.org/作品?q=한#구",
                "https://example.org/%E4%BD%9C%E5%93%81?q=%ED%95%9C#%EA%B5%AC",
            ),
        ] {
            let input = Candidate {
                title: "후보".into(),
                creator: None,
                similarity: None,
                links: vec![SearchLink {
                    label: "출처".into(),
                    url: raw.into(),
                }],
            };
            let output = validate_candidates(vec![input]).unwrap();
            assert_eq!(output[0].links[0].url, expected);
            assert_eq!(
                Url::parse(&output[0].links[0].url).unwrap().as_str(),
                output[0].links[0].url
            );
        }
        let unicode = format!("https://example.org/{}", "한".repeat(226));
        assert!(unicode.len() < 2048);
        assert!(result_url(&unicode).is_none());
    }
    #[test]
    fn search_text_caps_match_frontend_utf16_without_truncating_scalars() {
        for (limit, unit, count) in [
            (512, "a", 512),
            (512, "あ", 512),
            (512, "😀", 256),
            (256, "한", 256),
            (256, "😀", 128),
        ] {
            let value = unit.repeat(count);
            assert!(valid_text(&value, limit, false));
            assert!(value.encode_utf16().count() <= limit);
            assert!(!valid_text(&(value + unit), limit, false));
        }
        assert!(!valid_text(&("a".repeat(511) + "😀"), 512, false));
        assert!(!valid_text("line\nnext", 512, false));
        assert!(!valid_text(" ", 512, false));
    }
    #[test]
    fn search_consent_failed_replace_preserves_target_and_cleans_only_own_temp() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("target-directory");
        std::fs::create_dir(&path).unwrap();
        assert!(write_consent(
            &path,
            &Consent {
                version: 1,
                providers: vec![]
            }
        )
        .is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
