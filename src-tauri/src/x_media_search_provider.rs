//! Consented public-form lookup with isolated provider-owned persistent profiles.
//! No X profile, credential copying, clearance-token extraction or provider IPC.
//! Called only after the controller admits the frozen, bounded selection.
use super::{Candidate, ProviderInput, ProviderRequest, SearchLink, SearchMetadata};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder, WebviewUrl,
    WindowBuilder,
};

const DEADLINE: Duration = Duration::from_secs(30);
const HUMAN_DEADLINE: Duration = Duration::from_secs(10 * 60);
const MAX_REPLY: usize = 2 * 1024 * 1024;
const ORIGINAL_TTL: Duration = Duration::from_secs(30 * 60);
const INVALID: &str = "검색할 이미지나 장면을 확인하지 못했습니다.";
const CANCELLED: &str = "검색이 취소되었습니다.";
const TIMEOUT: &str = "검색 서비스의 응답을 제한 시간 안에 확인하지 못했습니다.";
const ISOLATION: &str =
    "검색 전용 브라우저 또는 분리된 저장 공간의 상태를 확인하지 못해 검색을 중단했습니다.";
const HUMAN_TIMEOUT: &str = "사람 확인 대기 시간이 끝났습니다. 자료를 다시 전송하지 않았습니다.";
const UNKNOWN: &str =
    "검색 결과의 화면 구조를 확인하지 못했습니다. 검색 완료로 처리하지 않았습니다.";
const CLEANUP: &str = "검색 전용 브라우저의 종료를 확인하지 못했습니다. 종료 확인 전까지 새 검색을 시작하지 않습니다.";
const CLOSE_DEADLINE: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResultNotice {
    LowSimilarityHidden,
}
#[derive(Debug)]
pub(super) struct Results {
    pub candidates: Vec<Candidate>,
    pub notice: Option<ResultNotice>,
}
impl From<Vec<Candidate>> for Results {
    fn from(candidates: Vec<Candidate>) -> Self {
        Self {
            candidates,
            notice: None,
        }
    }
}
// This reservation outlives a timed-out controller operation. Only a native
// close acknowledged on the UI thread may permit another provider worker.
static CHILD_RESERVED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
static PROFILE_LOCK: [Mutex<Option<std::fs::File>>; 2] = [Mutex::new(None), Mutex::new(None)];
static LENS_REFRESHING: AtomicBool = AtomicBool::new(false);
#[cfg(debug_assertions)]
static THUMBNAIL_PROBE: Mutex<Option<(String, Vec<Value>)>> = Mutex::new(None);
fn thumbnail_probe_job(request: &ProviderRequest) -> Option<String> {
    #[cfg(debug_assertions)]
    {
        use sha2::{Digest, Sha256};
        if std::env::var("ATSUMI_SYNTHETIC_RETAIN_ORIGINAL").as_deref() != Ok("1") {
            return None;
        }
        let bytes = match &request.input {
            ProviderInput::ImageFile(bytes) | ProviderInput::Jpeg(bytes) => bytes,
            _ => return None,
        };
        // Same fixed neutral fixture gate as the dedicated example. Never
        // collect provider diagnostics for an arbitrary user image or URL.
        if bytes.len() != 11793
            || format!("{:X}", Sha256::digest(bytes))
                != "4D2FEB4CF3F6A87DBF830A583B43108A21A25E246AC14DCDA8C68A2C110390FF"
        {
            return None;
        }
        *THUMBNAIL_PROBE.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((request.job_id.clone(), Vec::new()));
        return Some(request.job_id.clone());
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = request;
        None
    }
}
fn thumbnail_observation(
    job: Option<&str>,
    index: usize,
    outcome: &'static str,
    mime: &'static str,
    status: Option<u16>,
) {
    #[cfg(debug_assertions)]
    {
        let Some(job) = job else { return };
        let mut stored = THUMBNAIL_PROBE.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((id, items)) = stored.as_mut().filter(|(id, _)| id == job) {
            let _ = id;
            if items.len() < 10 {
                items.push(json!({"index":index,"outcome":outcome,"mime":mime,"status":status}));
            }
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (job, index, outcome, mime, status);
    }
}
#[cfg(debug_assertions)]
pub(super) fn take_thumbnail_probe(job_id: &str) -> Vec<Value> {
    let mut stored = THUMBNAIL_PROBE.lock().unwrap_or_else(|p| p.into_inner());
    if stored.as_ref().is_some_and(|(id, _)| id == job_id) {
        stored.take().map(|(_, items)| items).unwrap_or_default()
    } else {
        Vec::new()
    }
}
pub(super) fn busy() -> bool {
    CHILD_RESERVED
        .iter()
        .any(|slot| slot.load(Ordering::Acquire))
}
// Fixed synthetic fixture proved real rendered result DTOs, not bootstrap HTTP:
// Lens: lens-persistent-probe-1792619668276.json (9 candidates, 3.177s close ACK).
// SauceNAO: saucenao-persistent-probe-1792620130992.json (2, 2.298s close ACK).
// Each provider remains independently gated until its own evidence exists.
const LENS_LAYOUT_VERIFIED: bool = true;
const SAUCENAO_LAYOUT_VERIFIED: bool = true;
pub(super) fn available() -> bool {
    image_available() || video_available()
}
pub(super) fn image_available() -> bool {
    SAUCENAO_LAYOUT_VERIFIED
}
pub(super) fn video_available() -> bool {
    LENS_LAYOUT_VERIFIED
}

struct ProbeTrace {
    enabled: bool,
    started: Instant,
    count: AtomicUsize,
}
impl ProbeTrace {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            started: Instant::now(),
            count: AtomicUsize::new(0),
        }
    }
    fn phase(&self, stage: &'static str) {
        if !self.enabled
            || self
                .count
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < 32).then_some(n + 1)
                })
                .is_err()
        {
            return;
        }
        // Synthetic probe only: fixed stage names and elapsed time, never a URL,
        // search title, account, cookie, provider response or selected payload.
        eprintln!(
            "{}",
            json!({"searchProbeStage":stage,"elapsedMs":self.started.elapsed().as_millis()})
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Lens,
    Saucenao,
}
impl Provider {
    fn index(self) -> usize {
        match self {
            Self::Saucenao => 0,
            Self::Lens => 1,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Lens => "lens",
            Self::Saucenao => "saucenao",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Lens => "Google Lens · 직접 확인",
            Self::Saucenao => "SauceNAO · 직접 확인",
        }
    }
}

fn profile_directory(root: &Path, provider: Provider) -> PathBuf {
    // Stable, provider-only paths. Never derive a directory from account IDs,
    // source URLs, the default browser, or the X profile registry.
    root.join("x-media-search-profiles-v1")
        .join(provider.name())
}
fn lock_profile(directory: &Path) -> Result<std::fs::File, String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".atsumi-provider.lock"))
        .map_err(|_| ISOLATION)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
        "다른 앱에서 이 검색 서비스 창을 사용 중입니다. 자료를 전송하지 않았습니다."
    })?;
    Ok(file)
}

struct Budget {
    automatic: Duration,
    human: Duration,
}
impl Budget {
    fn new() -> Self {
        Self {
            automatic: DEADLINE,
            human: HUMAN_DEADLINE,
        }
    }
    fn charge_automatic(&mut self, elapsed: Duration) -> Result<(), String> {
        self.automatic = self.automatic.saturating_sub(elapsed);
        if self.automatic.is_zero() {
            Err(TIMEOUT.into())
        } else {
            Ok(())
        }
    }
    fn charge_human(&mut self, elapsed: Duration) -> Result<(), String> {
        self.human = self.human.saturating_sub(elapsed);
        if self.human.is_zero() {
            Err(HUMAN_TIMEOUT.into())
        } else {
            Ok(())
        }
    }
}

fn provider(input: &ProviderInput) -> Provider {
    match input {
        ProviderInput::ImageUrl(_) | ProviderInput::ImageFile(_) => Provider::Saucenao,
        ProviderInput::Jpeg(_) | ProviderInput::LensResults { .. } => Provider::Lens,
    }
}

fn basic_url(value: &str) -> Option<Url> {
    if value.is_empty()
        || value.len() > 2048
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains('\\')
    {
        return None;
    }
    let parsed = Url::parse(value).ok()?;
    (matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none())
    .then_some(parsed)
}

fn navigation(provider: Provider, url: &Url) -> bool {
    if url.as_str() == "about:blank" {
        return true;
    }
    if basic_url(url.as_str()).is_none() || url.scheme() != "https" || url.port().is_some() {
        return false;
    }
    match provider {
        Provider::Saucenao => {
            url.host_str() == Some("saucenao.com") && matches!(url.path(), "/" | "/search.php")
        }
        Provider::Lens => match url.host_str() {
            Some("lens.google.com") => matches!(url.path(), "/v3/upload" | "/upload" | "/search"),
            Some("www.google.com") => {
                url.path() == "/search"
                    || url.path().starts_with("/sorry/")
                    || lens_settings_url(url)
            }
            // The provider's own consent document is exposed for the user,
            // never clicked or accepted by automation. Account sign-in remains denied.
            Some("consent.google.com") => matches!(url.path(), "/m" | "/ml" | "/d" | "/dl"),
            _ => false,
        },
    }
}

fn lens_settings_url(url: &Url) -> bool {
    basic_url(url.as_str()).is_some()
        && url.scheme() == "https"
        && url.host_str() == Some("www.google.com")
        && url.port().is_none()
        && matches!(url.path(), "/safesearch" | "/setprefs")
}

fn lens_results_url(url: &Url) -> bool {
    basic_url(url.as_str()).is_some()
        && url.scheme() == "https"
        && url.port().is_none()
        && matches!(url.host_str(), Some("www.google.com" | "lens.google.com"))
        && url.path() == "/search"
        && url.query().is_some()
}

fn lens_manual_settings_url(url: &Url) -> bool {
    if basic_url(url.as_str()).is_none() || url.scheme() != "https" || url.port().is_some() {
        return false;
    }
    lens_settings_url(url)
        || match url.host_str() {
            Some("accounts.google.com") => true,
            Some("ageverification.google.co.kr") => {
                url.path() == "/ageverification" || url.path().starts_with("/ageverification/")
            }
            Some("myaccount.google.com") => {
                url.path() == "/age-verification" || url.path().starts_with("/age-verification/")
            }
            _ => false,
        }
}

fn denied_navigation(url: &Url) -> &'static str {
    match url.host_str() {
        Some("accounts.google.com") => {
            "검색 서비스에서 로그인을 요구합니다. 자동 로그인하지 않았습니다."
        }
        Some("consent.google.com") => {
            "검색 서비스의 개인정보 동의가 필요합니다. 자동 동의하지 않았습니다."
        }
        _ => "검색 서비스가 허용되지 않은 주소로 이동했습니다. 이동과 검색을 중단했습니다.",
    }
}

fn submission(input: &ProviderInput) -> Result<String, String> {
    submission_for(input, provider(input))
}
fn submission_for(input: &ProviderInput, service: Provider) -> Result<String, String> {
    if let ProviderInput::LensResults { url, expires } = input {
        if service != Provider::Lens || !lens_results_url(url) || Instant::now() >= *expires {
            return Err(INVALID.into());
        }
        let url = serde_json::to_string(url.as_str()).map_err(|_| INVALID)?;
        // This is a GET of the native-observed result for the same frozen job.
        // It never reconstructs the upload form or retains/resends image bytes.
        return Ok(format!("(()=>{{if(location.href!=='about:blank'||window.__atsumiSearchSubmitted)return false;window.__atsumiSearchSubmitted=true;location.replace({url});return true;}})()"));
    }
    let payload = match input {
        ProviderInput::ImageUrl(value) => {
            if service != Provider::Saucenao {
                return Err(INVALID.into());
            }
            let url = basic_url(value).ok_or(INVALID)?;
            if url.scheme() != "https"
                || url.host_str() != Some("pbs.twimg.com")
                || !url.path().starts_with("/media/")
                || url.port().is_some()
            {
                return Err(INVALID.into());
            }
            json!({"kind":"url", "value":value})
        }
        ProviderInput::Jpeg(bytes) | ProviderInput::ImageFile(bytes) => {
            if bytes.len() > 4 * 1024 * 1024 || bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8])
            {
                return Err(INVALID.into());
            }
            json!({"kind":"file", "value":STANDARD.encode(bytes),"provider":service.name()})
        }
        ProviderInput::LensResults { .. } => return Err(INVALID.into()),
    };
    // A fresh about:blank document alone receives the frozen data. The exact
    // public form uses a native browser POST; no fetch, private API or cookies
    // are copied from the source document. Never submit twice after navigation.
    Ok(format!(
        r#"(()=>{{
      if(location.href!=='about:blank'||window.__atsumiSearchSubmitted)return false;
      const selection={payload}; window.__atsumiSearchSubmitted=true;
      const root=document.body||document.documentElement.appendChild(document.createElement('body'));
      const ref=document.createElement('meta');ref.name='referrer';ref.content='no-referrer';(document.head||root).appendChild(ref);
      const form=document.createElement('form');form.method='POST';form.enctype='multipart/form-data';
      const input=document.createElement('input');
      if(selection.kind==='url'){{form.action='https://saucenao.com/search.php';input.type='hidden';input.name='url';input.value=selection.value;}}
      else{{form.action=selection.provider==='saucenao'?'https://saucenao.com/search.php':'https://lens.google.com/v3/upload?hl=ko&re=df';input.type='file';input.name=selection.provider==='saucenao'?'file':'encoded_image';
        const raw=atob(selection.value),bytes=new Uint8Array(raw.length);for(let i=0;i<raw.length;i++)bytes[i]=raw.charCodeAt(i);
        const transfer=new DataTransfer();transfer.items.add(new File([bytes],'selected-frame.jpg',{{type:'image/jpeg'}}));input.files=transfer.files;}}
      form.appendChild(input);root.appendChild(form);HTMLFormElement.prototype.submit.call(form);return true;
    }})()"#
    ))
}

struct Child {
    app: AppHandle,
    label: String,
    live: Arc<AtomicBool>,
    cleanup: Arc<Cleanup>,
    close_requested: AtomicBool,
}
struct PageControl {
    retained: AtomicBool,
    shown: AtomicBool,
    settings: AtomicBool,
    closed: Mutex<bool>,
    changed: Condvar,
    deadline: Mutex<Instant>,
    results_url: Mutex<Option<Url>>,
    result_intent: Mutex<Option<ResultIntent>>,
}
struct ResultIntent {
    id: String,
    initial: Url,
    steps: usize,
    expires: Instant,
    reply: std::sync::mpsc::SyncSender<Result<(), String>>,
}
enum ResultNavigation {
    Allow,
    Stop,
    Open(Url, std::sync::mpsc::SyncSender<Result<(), String>>),
}
fn result_navigation(control: &PageControl, url: &Url) -> Option<ResultNavigation> {
    let current = control.retained.load(Ordering::Acquire)
        && control.shown.load(Ordering::Acquire)
        && !*control.closed.lock().unwrap_or_else(|p| p.into_inner())
        && Instant::now() < *control.deadline.lock().unwrap_or_else(|p| p.into_inner());
    let mut pending = control
        .result_intent
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let intent = pending.as_mut()?;
    if !current
        || Instant::now() >= intent.expires
        || intent.steps >= 4 && google_result_redirect(url)
        || intent.steps == 0 && url != &intent.initial
    {
        if let Some(intent) = pending.take() {
            let _ = intent.reply.send(Err(REDIRECT_UNRESOLVED.into()));
        }
        return Some(ResultNavigation::Stop);
    }
    if google_result_redirect(url) {
        intent.steps += 1;
        return Some(ResultNavigation::Allow);
    }
    let google = url.host_str().is_some_and(|host| {
        host == "google.com"
            || host.ends_with(".google.com")
            || host == "google.co.kr"
            || host.ends_with(".google.co.kr")
    });
    let intent = pending.take()?;
    if google || super::result_url(url.as_str()).is_none() {
        let _ = intent.reply.send(Err(REDIRECT_UNRESOLVED.into()));
        Some(ResultNavigation::Stop)
    } else {
        Some(ResultNavigation::Open(url.clone(), intent.reply))
    }
}
fn manual_lens_settings(control: &PageControl, service: Provider) -> bool {
    service == Provider::Lens
        && control.settings.load(Ordering::Acquire)
        && control.retained.load(Ordering::Acquire)
        && control.shown.load(Ordering::Acquire)
        && !*control.closed.lock().unwrap_or_else(|p| p.into_inner())
        && Instant::now() < *control.deadline.lock().unwrap_or_else(|p| p.into_inner())
}
impl PageControl {
    fn dismiss(&self) {
        *self.closed.lock().unwrap_or_else(|p| p.into_inner()) = true;
        if let Some(intent) = self
            .result_intent
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            let _ = intent.reply.send(Err(CANCELLED.into()));
        }
        self.changed.notify_all();
    }
}
#[derive(Clone)]
struct RetainedPage {
    job_id: String,
    child: Arc<Child>,
    control: Arc<PageControl>,
    expires: Instant,
}
static ORIGINAL_PAGE: [Mutex<Option<RetainedPage>>; 2] = [Mutex::new(None), Mutex::new(None)];
fn original_page(job_id: &str) -> Option<RetainedPage> {
    ORIGINAL_PAGE.iter().find_map(|slot| {
        slot.lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|p| p.job_id == job_id)
            .cloned()
    })
}
pub(super) fn page_state(job_id: &str) -> &'static str {
    let Some(page) = original_page(job_id) else {
        return "unavailable";
    };
    if Instant::now() >= page.expires {
        "expired"
    } else if *page
        .control
        .closed
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        || !page.child.live.load(Ordering::Acquire)
    {
        "closed"
    } else {
        "available"
    }
}
pub(super) fn close_original(job_id: &str) {
    // Serialize close with terminal publication, independently for each service.
    for slot in &ORIGINAL_PAGE {
        if let Some(page) = slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|p| p.job_id == job_id)
        {
            page.control.dismiss();
        }
    }
}
pub(super) fn close_all() {
    for slot in &ORIGINAL_PAGE {
        if let Some(page) = slot.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            page.control.dismiss();
        }
    }
}
fn await_prior_close(
    service: Provider,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    if !CHILD_RESERVED[service.index()].load(Ordering::Acquire) {
        return Ok(());
    }
    let cleanup = {
        let stored = ORIGINAL_PAGE[service.index()]
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let page = stored.as_ref().ok_or(CLEANUP)?;
        if !*page
            .control
            .closed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            return Err(CLEANUP.into());
        }
        page.child.cleanup.clone()
    };
    let until = deadline.min(Instant::now() + CLOSE_DEADLINE + Duration::from_millis(250));
    let mut result = cleanup.result.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        if let Some(result) = result.as_ref() {
            return result.clone().and_then(|()| {
                if CHILD_RESERVED[service.index()].load(Ordering::Acquire) {
                    Err(CLEANUP.into())
                } else {
                    Ok(())
                }
            });
        }
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CLEANUP.into());
        }
        result = cleanup
            .changed
            .wait_timeout(result, remaining.min(Duration::from_millis(50)))
            .unwrap_or_else(|p| p.into_inner())
            .0;
    }
}
// Invoked by a trusted main-window command worker, never by the provider page.
// Reuses the exact native page; no GET, reload, navigation or duplicate upload.
pub(super) fn show_original(app: &AppHandle, job_id: &str) -> Result<(), String> {
    let page = original_page(job_id)
        .ok_or("검색 원본 창이 남아 있지 않습니다. 자료를 다시 전송하지 않았습니다.")?;
    let (child, control, expires) = (page.child, page.control, page.expires);
    if Instant::now() >= expires {
        return Err(
            "검색 원본 창의 보관 시간이 끝났습니다. 자료를 다시 전송하지 않았습니다.".into(),
        );
    }
    if !child.live.load(Ordering::Acquire)
        || *control.closed.lock().unwrap_or_else(|p| p.into_inner())
    {
        return Err("검색 원본 창이 닫혔습니다. 자료를 다시 전송하지 않았습니다.".into());
    }
    let label = child.label.clone();
    let owned = app.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = (|| {
            if !child.live.load(Ordering::Acquire)
                || Instant::now() >= expires
                || *control.closed.lock().unwrap_or_else(|p| p.into_inner())
            {
                return Err(CANCELLED.to_string());
            }
            let window = owned
                .get_window(&label)
                .ok_or_else(|| ISOLATION.to_string())?;
            window
                .show()
                .and_then(|_| window.set_focus())
                .map_err(|_| ISOLATION.to_string())?;
            control.shown.store(true, Ordering::Release);
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| ISOLATION)?;
    rx.recv_timeout(CLOSE_DEADLINE)
        .map_err(|_| ISOLATION.to_string())?
}

fn retained_lens(job_id: &str) -> Result<RetainedPage, String> {
    let page = original_page(job_id).ok_or("남아 있는 Google Lens 검색이 없습니다.")?;
    if page.child.cleanup.service_index != Provider::Lens.index() || Instant::now() >= page.expires
    {
        return Err(
            "Google Lens 검색 보관 시간이 끝났습니다. 미디어를 다시 전송하지 않았습니다.".into(),
        );
    }
    Ok(page)
}

pub(super) fn show_lens_settings(app: &AppHandle, job_id: &str) -> Result<(), String> {
    let page = retained_lens(job_id)?;
    let owned = app.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = (|| {
            if !page.child.live.load(Ordering::Acquire)
                || Instant::now() >= page.expires
                || *page
                    .control
                    .closed
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
            {
                return Err("Google Lens 검색 창이 닫혔습니다.".to_string());
            }
            let window = owned
                .get_window(&page.child.label)
                .ok_or_else(|| ISOLATION.to_string())?;
            window
                .show()
                .and_then(|_| window.set_focus())
                .map_err(|_| ISOLATION.to_string())?;
            page.control.shown.store(true, Ordering::Release);
            page.control.settings.store(true, Ordering::Release);
            owned
                .get_webview(&page.child.label)
                .ok_or_else(|| ISOLATION.to_string())?
                .navigate(
                    Url::parse("https://www.google.com/safesearch?hl=ko")
                        .map_err(|_| ISOLATION.to_string())?,
                )
                .map_err(|_| ISOLATION.to_string())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| ISOLATION.to_string())?;
    rx.recv_timeout(CLOSE_DEADLINE)
        .map_err(|_| ISOLATION.to_string())?
}

struct RefreshStop {
    child: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}
impl RefreshStop {
    fn new(parent: Arc<AtomicBool>) -> Result<Self, String> {
        if parent.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        let child = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let stopped = child.clone();
        let finished = done.clone();
        std::thread::Builder::new()
            .name("lens-refresh-cancel".into())
            .spawn(move || {
                while !finished.load(Ordering::Acquire) && !stopped.load(Ordering::Acquire) {
                    if parent.load(Ordering::Acquire) {
                        stopped.store(true, Ordering::Release);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            })
            .map_err(|_| ISOLATION.to_string())?;
        Ok(Self { child, done })
    }
}
impl Drop for RefreshStop {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
    }
}

pub(super) fn refresh_lens_results(
    app: &AppHandle,
    job_id: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<Candidate>, String> {
    let page = retained_lens(job_id)?;
    let url = page
        .control
        .results_url
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .filter(lens_results_url)
        .ok_or("기존 Lens 검색 주소를 확인하지 못했습니다. 새 검색이 필요합니다.")?;
    if LENS_REFRESHING.swap(true, Ordering::AcqRel) {
        return Err("Google Lens 결과를 갱신 중입니다.".into());
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            LENS_REFRESHING.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    // Closing this provider window must not cancel the parent job or the
    // other provider's retained results. Context cancellation still flows down.
    let stop = RefreshStop::new(cancel.clone())?;
    close_original(job_id);
    let result = run_inner(
        app.clone(),
        ProviderRequest {
            job_id: job_id.into(),
            input: ProviderInput::LensResults {
                url,
                expires: page.expires,
            },
        },
        stop.child.clone(),
        Arc::new(AtomicBool::new(false)),
        Arc::new(|_| {}),
        None,
        None,
    );
    if cancel.load(Ordering::Acquire) {
        close_original(job_id);
        return Err(CANCELLED.into());
    }
    result.map(|result| result.candidates)
}
fn retain_original(
    job_id: String,
    child: Arc<Child>,
    control: Arc<PageControl>,
    cancel: &AtomicBool,
    expires: Option<Instant>,
) -> bool {
    // Serialize publication with close_original/close_all. A job invalidated
    // just as its terminal response arrives must never resurrect a browser.
    let mut stored = ORIGINAL_PAGE[child.cleanup.service_index]
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if cancel.load(Ordering::Acquire) {
        return false;
    }
    let expires = expires.unwrap_or_else(|| Instant::now() + ORIGINAL_TTL);
    *control.deadline.lock().unwrap_or_else(|p| p.into_inner()) = expires;
    control.retained.store(true, Ordering::Release);
    *stored = Some(RetainedPage {
        job_id,
        child: child.clone(),
        control: control.clone(),
        expires,
    });
    drop(stored);
    std::thread::spawn(move || {
        let mut closed = control.closed.lock().unwrap_or_else(|p| p.into_inner());
        while !*closed {
            let remaining = expires.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            closed = control
                .changed
                .wait_timeout(closed, remaining)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        drop(closed);
        let _ = child.close(); // ACK releases provider lease; failure remains busy.
    });
    true
}
#[derive(Default)]
struct Cleanup {
    service_index: usize,
    creation_finished: AtomicBool,
    result: Mutex<Option<Result<(), String>>>,
    changed: Condvar,
}
impl Cleanup {
    fn finish(&self, result: Result<(), String>) {
        let mut stored = self.result.lock().unwrap_or_else(|p| p.into_inner());
        if stored.is_some() {
            return;
        }
        if result.is_ok() {
            // Keep the cross-process lease after a failed/unacknowledged close.
            // No other app/probe may reuse the profile while this view survives.
            PROFILE_LOCK[self.service_index]
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            CHILD_RESERVED[self.service_index].store(false, Ordering::Release);
        }
        *stored = Some(result);
        self.changed.notify_all();
    }
}
// Must be called on Tauri's UI thread. Here Webview::close synchronously
// dispatches removal and WebView2 Controller.Close; a worker-thread call would
// only enqueue that operation and prematurely remove the manager entry.
fn close_on_ui(app: &AppHandle, label: &str, cleanup: &Cleanup) {
    if !cleanup.creation_finished.load(Ordering::Acquire)
        || cleanup
            .result
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    {
        return;
    }
    let result = app
        .get_webview(label)
        .map_or(Ok(()), |view| view.close().map_err(|_| CLEANUP.to_string()))
        .and_then(|()| {
            app.get_window(label).map_or(Ok(()), |window| {
                window.destroy().map_err(|_| CLEANUP.to_string())
            })
        });
    cleanup.finish(result);
}
impl Child {
    fn close(&self) -> Result<(), String> {
        self.close_requested.store(true, Ordering::Release);
        self.live.store(false, Ordering::Release);
        if let Some(result) = self
            .cleanup
            .result
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            return result;
        }
        let app = self.app.clone();
        let label = self.label.clone();
        let cleanup = self.cleanup.clone();
        if self
            .app
            .run_on_main_thread(move || close_on_ui(&app, &label, &cleanup))
            .is_err()
        {
            self.cleanup.finish(Err(CLEANUP.into()));
        }
        let deadline = Instant::now() + CLOSE_DEADLINE;
        let mut result = self
            .cleanup
            .result
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(result) = result.as_ref() {
                return result.clone();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(CLEANUP.into());
            }
            result = self
                .cleanup
                .changed
                .wait_timeout(result, remaining)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        // Fallback for unwinding/early failures. Normal return awaits close()
        // explicitly and propagates an unconfirmed cleanup as a failure.
        if !self.close_requested.load(Ordering::Acquire) {
            let _ = self.close();
        }
    }
}

fn check(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELLED.into())
    } else if Instant::now() >= deadline {
        Err(TIMEOUT.into())
    } else {
        Ok(())
    }
}

fn receive<T>(
    rx: &std::sync::mpsc::Receiver<T>,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<T, String> {
    loop {
        check(cancel, deadline)?;
        match rx.recv_timeout(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        ) {
            Ok(value) => return Ok(value),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(ISOLATION.into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn read_url(
    app: &AppHandle,
    label: &str,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<Url, String> {
    let owned = app.clone();
    let name = label.to_string();
    let stop = cancel.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = (|| {
            check(&stop, deadline)?;
            owned
                .get_webview(&name)
                .ok_or(ISOLATION)?
                .url()
                .map_err(|_| ISOLATION.to_string())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| ISOLATION)?;
    receive(&rx, cancel, deadline)?
}

fn evaluate(
    app: &AppHandle,
    label: &str,
    script: String,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<String, String> {
    let owned = app.clone();
    let name = label.to_string();
    let stop = cancel.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let on_result = tx.clone();
        let admitted = (|| {
            check(&stop, deadline)?;
            owned
                .get_webview(&name)
                .ok_or(ISOLATION)?
                .eval_with_callback(script, move |raw| {
                    let _ = on_result.send(Ok(raw));
                })
                .map_err(|_| UNKNOWN.to_string())
        })();
        if let Err(error) = admitted {
            let _ = tx.send(Err(error));
        }
    })
    .map_err(|_| ISOLATION)?;
    receive(&rx, cancel, deadline)?
}

fn dispatch_submission(
    app: &AppHandle,
    label: &str,
    script: String,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<(), String> {
    let owned = app.clone();
    let name = label.to_string();
    let stop = cancel.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let admitted = (|| {
            check(&stop, deadline)?;
            owned
                .get_webview(&name)
                .ok_or(ISOLATION)?
                .eval(script)
                .map_err(|_| ISOLATION.to_string())
        })();
        let _ = tx.send(admitted);
    })
    .map_err(|_| ISOLATION)?;
    receive(&rx, cancel, deadline)?
}

#[cfg(windows)]
fn verify_profile_and_mute(
    view: &Webview,
    directory: PathBuf,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Environment7, ICoreWebView2_13, ICoreWebView2_2, ICoreWebView2_8,
        },
    };
    use windows::core::{Interface, BOOL, PWSTR};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<bool> {
            unsafe {
                let core = platform.controller().CoreWebView2()?;
                let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
                let mut private = BOOL::default();
                profile.IsInPrivateModeEnabled(&mut private)?;
                let environment = core
                    .cast::<ICoreWebView2_2>()?
                    .Environment()?
                    .cast::<ICoreWebView2Environment7>()?;
                let mut raw = PWSTR::null();
                environment.UserDataFolder(&mut raw)?;
                let actual = PathBuf::from(CoTaskMemPWSTR::from(raw).to_string());
                let isolated = std::fs::canonicalize(&directory)
                    .ok()
                    .zip(std::fs::canonicalize(actual).ok())
                    .is_some_and(|(expected, actual)| expected == actual);
                core.cast::<ICoreWebView2_8>()?.SetIsMuted(true)?;
                Ok(!private.as_bool() && isolated)
            }
        })();
        let _ = tx.send(result.unwrap_or(false));
    })
    .map_err(|_| ISOLATION)?;
    if receive(&rx, cancel, deadline)? {
        Ok(())
    } else {
        Err(ISOLATION.into())
    }
}
#[cfg(not(windows))]
fn verify_profile_and_mute(
    _: &Webview,
    _: PathBuf,
    _: &AtomicBool,
    _: Instant,
) -> Result<(), String> {
    Err(ISOLATION.into())
}

fn window_visibility(
    app: &AppHandle,
    label: &str,
    visible: bool,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<(), String> {
    let owned = app.clone();
    let name = label.to_string();
    let stopped = cancel.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = (|| -> Result<(), String> {
            check(&stopped, deadline)?;
            let window = owned.get_window(&name).ok_or(ISOLATION)?;
            if visible {
                window.show().map_err(|_| ISOLATION)?;
                window.set_focus().map_err(|_| ISOLATION)?;
            } else {
                window.hide().map_err(|_| ISOLATION)?;
            }
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| ISOLATION)?;
    receive(&rx, cancel, deadline)?
}

fn original_external_link(
    control: &PageControl,
    service: Provider,
    value: &str,
    user: bool,
    redirected: bool,
) -> Option<Url> {
    if !user
        || redirected
        || !control.retained.load(Ordering::Acquire)
        || !control.shown.load(Ordering::Acquire)
        || *control.closed.lock().unwrap_or_else(|p| p.into_inner())
        || Instant::now() >= *control.deadline.lock().unwrap_or_else(|p| p.into_inner())
    {
        return None;
    }
    let url = super::result_url(value)?;
    if manual_lens_settings(control, service) && lens_manual_settings_url(&url) {
        return None;
    }
    (!navigation(service, &url)).then_some(url)
}

const REDIRECT_UNRESOLVED: &str = "SEARCH_RESULT_REDIRECT_UNRESOLVED";
static REDIRECT_RESOLVERS: AtomicUsize = AtomicUsize::new(0);
fn google_result_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("www.google.com")
        && url.port().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "/goto" | "/url")
        && url.query().is_some()
}
fn resolve_redirect_with<F>(
    start: Url,
    alive: &dyn Fn() -> bool,
    mut request: F,
) -> Result<Url, String>
where
    F: FnMut(&Url) -> Result<Option<String>, String>,
{
    let mut current = start;
    let mut visited = Vec::new();
    while google_result_redirect(&current) {
        if !alive() {
            return Err(CANCELLED.into());
        }
        if visited.contains(&current) || visited.len() >= 4 {
            return Err(REDIRECT_UNRESOLVED.into());
        }
        visited.push(current.clone());
        let location = request(&current)?
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 2048
                    && !value.chars().any(|c| c.is_control() || c == '\\')
            })
            .ok_or(REDIRECT_UNRESOLVED)?;
        if !alive() {
            return Err(CANCELLED.into());
        }
        current = current
            .join(&location)
            .ok()
            .and_then(|url| basic_url(url.as_str()))
            .ok_or(REDIRECT_UNRESOLVED)?;
        // A login/challenge or another Google UI is not a resolved source.
        if current
            .host_str()
            .is_some_and(|host| host == "google.com" || host.ends_with(".google.com"))
            && !google_result_redirect(&current)
        {
            return Err(REDIRECT_UNRESOLVED.into());
        }
    }
    if !alive() {
        return Err(CANCELLED.into());
    }
    Ok(current)
}
pub(super) fn resolve_result_url(value: &str, alive: &dyn Fn() -> bool) -> Result<String, String> {
    resolve_result_url_until(value, alive, Instant::now() + Duration::from_secs(8))
}
pub(super) fn resolve_result_url_until(
    value: &str,
    alive: &dyn Fn() -> bool,
    deadline: Instant,
) -> Result<String, String> {
    let start = basic_url(value).ok_or(INVALID)?;
    if !google_result_redirect(&start) {
        return if alive() {
            Ok(start.to_string())
        } else {
            Err(CANCELLED.into())
        };
    }
    REDIRECT_RESOLVERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < 4).then_some(count + 1)
        })
        .map_err(|_| REDIRECT_UNRESOLVED.to_string())?;
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) {
            REDIRECT_RESOLVERS.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let _permit = Permit;
    let deadline = deadline.min(Instant::now() + Duration::from_secs(8));
    if Instant::now() >= deadline {
        return Err(REDIRECT_UNRESOLVED.into());
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| REDIRECT_UNRESOLVED)?;
    resolve_redirect_with(start, alive, |url| {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(REDIRECT_UNRESOLVED.into());
        }
        let response = client
            .get(url.clone())
            .timeout(remaining.min(Duration::from_secs(5)))
            .send()
            .map_err(|_| REDIRECT_UNRESOLVED)?;
        if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(None);
        }
        let locations = response.headers().get_all(reqwest::header::LOCATION);
        if locations.iter().count() != 1 {
            return Ok(None);
        }
        Ok(locations
            .iter()
            .next()
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned))
    })
    .map(|url| url.to_string())
}
fn prepare_direct_links<F>(
    candidates: &mut [Candidate],
    cancel: &AtomicBool,
    deadline: Instant,
    resolve: F,
) -> Result<(), String>
where
    F: Fn(&str, Instant) -> Result<String, String> + Sync,
{
    // At most four workers share one deadline. Resolve each observed wrapper
    // once, regardless of how many candidates contain it. A failed/expired
    // lookup keeps the retained original link usable instead of hiding results.
    let jobs = candidates
        .iter()
        .flat_map(|candidate| &candidate.links)
        .filter_map(|link| basic_url(&link.url))
        .filter(google_result_redirect)
        .map(|url| url.to_string())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let next = AtomicUsize::new(0);
    let destinations = Mutex::new(std::collections::BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.len().min(4) {
            scope.spawn(|| {
                while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                    let Some(source) = jobs.get(next.fetch_add(1, Ordering::AcqRel)) else {
                        break;
                    };
                    let Ok(target) = resolve(source, deadline) else {
                        continue;
                    };
                    let Some(url) = basic_url(&target).filter(|url| {
                        !url.host_str().is_some_and(|host| {
                            host == "google.com" || host.ends_with(".google.com")
                        })
                    }) else {
                        continue;
                    };
                    if !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
                        destinations
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(source.clone(), url);
                    }
                }
            });
        }
    });
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    let destinations = destinations.into_inner().unwrap_or_else(|p| p.into_inner());
    for link in candidates
        .iter_mut()
        .flat_map(|candidate| &mut candidate.links)
    {
        if let Some(url) = destinations.get(&link.url) {
            link.label = url.host_str().unwrap_or_default().to_owned();
            link.url = url.to_string();
        }
    }
    Ok(())
}
fn dispatch_original_result(
    app: &AppHandle,
    control: Arc<PageControl>,
    service: Provider,
    value: String,
) {
    let owned = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let alive = || original_external_link(&control, service, &value, true, false).is_some();
        match resolve_result_url(&value, &alive) {
            Ok(target) if alive() => {
                let _ = super::open_provider_result(&owned, &target);
            }
            Err(_) if alive() => {
                let page = ORIGINAL_PAGE[service.index()]
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                if let Some(page) = page.filter(|page| Arc::ptr_eq(&page.control, &control)) {
                    let _ = open_wrapped_result(
                        &owned,
                        &page.job_id,
                        &value,
                        Arc::new(AtomicBool::new(false)),
                    );
                }
            }
            _ => {}
        }
    });
}

pub(super) fn open_wrapped_result(
    app: &AppHandle,
    job_id: &str,
    value: &str,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let url = basic_url(value)
        .filter(google_result_redirect)
        .ok_or(REDIRECT_UNRESOLVED)?;
    let page = original_page(job_id).ok_or(REDIRECT_UNRESOLVED)?;
    if Instant::now() >= page.expires || !page.child.live.load(Ordering::Acquire) {
        return Err(REDIRECT_UNRESOLVED.into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let deadline = (Instant::now() + Duration::from_secs(8)).min(page.expires);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    {
        let mut intent = page
            .control
            .result_intent
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if intent.is_some() {
            return Err(REDIRECT_UNRESOLVED.into());
        }
        *intent = Some(ResultIntent {
            id: id.clone(),
            initial: url.clone(),
            steps: 0,
            expires: deadline,
            reply: tx,
        });
    }
    let host = app.clone();
    let child = page.child.clone();
    let control = page.control.clone();
    let stopped = cancel.clone();
    let scheduled = app.run_on_main_thread(move || {
        let result = (|| {
            check(&stopped, deadline)?;
            if *control.closed.lock().unwrap_or_else(|p| p.into_inner()) {
                return Err(CANCELLED.into());
            }
            host.get_window(&child.label)
                .ok_or(ISOLATION)?
                .show()
                .map_err(|_| ISOLATION)?;
            control.shown.store(true, Ordering::Release);
            host.get_webview(&child.label)
                .ok_or(ISOLATION)?
                .navigate(url)
                .map_err(|_| ISOLATION)?;
            Ok::<(), String>(())
        })();
        if result.is_err() {
            if let Some(intent) = control
                .result_intent
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                let _ = intent.reply.send(result);
            }
        }
    });
    let result = if scheduled.is_err() {
        Err(REDIRECT_UNRESOLVED.into())
    } else {
        loop {
            match rx.try_recv() {
                Ok(result) => break result,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    break Err(REDIRECT_UNRESOLVED.into())
                }
                _ => {}
            }
            if cancel.load(Ordering::Acquire) {
                break Err(CANCELLED.into());
            }
            if Instant::now() >= deadline {
                break Err(REDIRECT_UNRESOLVED.into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    let mut intent = page
        .control
        .result_intent
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if intent.as_ref().is_some_and(|intent| intent.id == id) {
        *intent = None;
    }
    drop(intent);
    if result.is_err()
        && !cancel.load(Ordering::Acquire)
        && !*page
            .control
            .closed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    {
        let original = page
            .control
            .results_url
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(original) = original {
            let _ = navigate_results_tab(
                app,
                &page.child.label,
                original,
                &cancel,
                (Instant::now() + Duration::from_secs(2)).min(page.expires),
            );
        }
    }
    result
}
#[cfg(windows)]
fn install_original_links(
    view: &Webview,
    service: Provider,
    control: Arc<PageControl>,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    use webview2_com::{
        CoTaskMemPWSTR, NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    };
    use windows::core::{BOOL, PWSTR};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let owned_app = view.app_handle().clone();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            unsafe {
                let core = platform.controller().CoreWebView2()?;
                let nav_control = control.clone();
                let nav_app = owned_app.clone();
                let mut token = 0;
                core.add_NavigationStarting(
                    &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                        let Some(args) = args else { return Ok(()) };
                        if nav_control
                            .result_intent
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .is_some()
                        {
                            return Ok(());
                        }
                        let mut user = BOOL::default();
                        let mut redirected = BOOL::default();
                        let mut raw = PWSTR::null();
                        args.IsUserInitiated(&mut user)?;
                        args.IsRedirected(&mut redirected)?;
                        args.Uri(&mut raw)?;
                        let value = CoTaskMemPWSTR::from(raw).to_string();
                        if let Some(url) = original_external_link(
                            &nav_control,
                            service,
                            &value,
                            user.as_bool(),
                            redirected.as_bool(),
                        ) {
                            args.SetCancel(true)?;
                            dispatch_original_result(
                                &nav_app,
                                nav_control.clone(),
                                service,
                                url.to_string(),
                            );
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
                core.add_NewWindowRequested(
                    &NewWindowRequestedEventHandler::create(Box::new(move |sender, args| {
                        let Some(args) = args else { return Ok(()) };
                        args.SetHandled(true)?;
                        let mut user = BOOL::default();
                        let mut raw = PWSTR::null();
                        args.IsUserInitiated(&mut user)?;
                        args.Uri(&mut raw)?;
                        let value = CoTaskMemPWSTR::from(raw).to_string();
                        if user.as_bool()
                            && manual_lens_settings(&control, service)
                            && Url::parse(&value)
                                .ok()
                                .is_some_and(|url| lens_manual_settings_url(&url))
                        {
                            // A human-opened Google account/age page stays in
                            // this exact Lens profile. No form inspection or
                            // automatic authentication is performed.
                            if let Some(sender) = sender {
                                let wide = value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
                                sender.Navigate(windows::core::PCWSTR(wide.as_ptr()))?;
                            }
                            return Ok(());
                        }
                        if let Some(url) =
                            original_external_link(&control, service, &value, user.as_bool(), false)
                        {
                            dispatch_original_result(
                                &owned_app,
                                control.clone(),
                                service,
                                url.to_string(),
                            );
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
                Ok(())
            }
        })();
        let _ = tx.send(result.is_ok());
    })
    .map_err(|_| ISOLATION)?;
    if receive(&rx, cancel, deadline)? {
        Ok(())
    } else {
        Err(ISOLATION.into())
    }
}
#[cfg(not(windows))]
fn install_original_links(
    _: &Webview,
    _: Provider,
    _: Arc<PageControl>,
    _: &AtomicBool,
    _: Instant,
) -> Result<(), String> {
    Err(ISOLATION.into())
}

fn wait_for_human(
    resume: &AtomicBool,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    // No browser/DOM inspection, requests, clicks, token reads or automated
    // challenge handling while the user owns this exact document.
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        if Instant::now() >= deadline {
            return Err(HUMAN_TIMEOUT.into());
        }
        if resume.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        std::thread::sleep(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LensNavigation {
    mode: String,
    exact_url: Option<String>,
    visual_url: Option<String>,
    all_url: Option<String>,
}

struct LensPlan {
    desired: &'static str,
    sent: bool,
    visual: Option<Url>,
    all: Option<Url>,
    visited: Vec<Url>,
}
impl Default for LensPlan {
    fn default() -> Self {
        Self {
            desired: "exact",
            sent: false,
            visual: None,
            all: None,
            visited: vec![],
        }
    }
}
#[derive(Debug, PartialEq)]
enum LensDecision {
    Read,
    Wait,
    Navigate(Url),
}
impl LensPlan {
    fn observe(
        &mut self,
        document: Option<&LensNavigation>,
        current: &Url,
        state: &str,
    ) -> Result<LensDecision, String> {
        if matches!(state, "challenge" | "consent" | "quota") {
            return Ok(LensDecision::Read);
        }
        let Some(document) = document else {
            return Ok(LensDecision::Wait);
        };
        let tab = |value: &Option<String>| {
            value
                .as_deref()
                .and_then(basic_url)
                .filter(|url| lens_results_url(url) && url.fragment().is_none())
        };
        if let Some(url) = tab(&document.visual_url) {
            self.visual = Some(url);
        }
        if let Some(url) = tab(&document.all_url) {
            self.all = Some(url);
        }
        if document.mode == "visual" {
            self.visual = Some(current.clone());
        }
        if document.mode == "all" {
            self.all = Some(current.clone());
        }
        let next = if document.mode != self.desired {
            if self.desired == "exact" && !self.sent {
                let Some(url) = tab(&document.exact_url) else {
                    return Ok(LensDecision::Wait);
                };
                Some(("exact", url))
            } else {
                return Ok(LensDecision::Wait);
            }
        } else if state == "empty" {
            match self.desired {
                "exact" => self
                    .visual
                    .clone()
                    .map(|url| ("visual", url))
                    .or_else(|| self.all.clone().map(|url| ("all", url))),
                "visual" => self.all.clone().map(|url| ("all", url)),
                _ => None,
            }
        } else {
            None
        };
        let Some((desired, url)) = next else {
            return Ok(LensDecision::Read);
        };
        if url == *current || self.visited.contains(&url) || self.visited.len() >= 3 {
            return Err(UNKNOWN.into());
        }
        self.desired = desired;
        self.sent = true;
        self.visited.push(url.clone());
        Ok(LensDecision::Navigate(url))
    }
}

fn navigate_results_tab(
    app: &AppHandle,
    label: &str,
    url: Url,
    cancel: &Arc<AtomicBool>,
    deadline: Instant,
) -> Result<(), String> {
    if !lens_results_url(&url) {
        return Err(INVALID.into());
    }
    let owned = app.clone();
    let label = label.to_owned();
    let stopped = cancel.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result = (|| {
            check(&stopped, deadline)?;
            owned
                .get_webview(&label)
                .ok_or(ISOLATION)?
                .navigate(url)
                .map_err(|_| ISOLATION.to_string())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| ISOLATION.to_string())?;
    receive(&rx, cancel, deadline)?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    nonce: String,
    state: String,
    candidates: Vec<RawCandidate>,
    #[serde(default)]
    probe: Option<Value>,
    #[serde(default)]
    lens: Option<LensNavigation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCandidate {
    title: String,
    creator: Option<String>,
    similarity: Option<f64>,
    links: Vec<RawLink>,
    #[serde(default, rename = "thumbnailSource")]
    thumbnail_source: Option<String>,
    #[serde(default, rename = "sourceMetadata")]
    source_metadata: Vec<SearchMetadata>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLink {
    label: String,
    url: String,
}
fn text_valid(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.encode_utf16().count() <= max
        && !value.chars().any(|c| c.is_control())
}
fn thumbnail_url(service: Provider, value: &str) -> Option<Url> {
    let url = basic_url(value)?;
    if url.scheme() != "https" || url.port().is_some() || url.fragment().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let allowed = match service {
        Provider::Saucenao => {
            host.strip_prefix("img")
                .and_then(|v| v.strip_suffix(".saucenao.com"))
                .is_some_and(|v| v.len() == 1 && matches!(v.as_bytes()[0], b'1'..=b'9'))
                && (url.path().starts_with("/res/") || artstation_thumbnail_path(url.path()))
        }
        Provider::Lens => {
            host.strip_prefix("encrypted-tbn")
                .and_then(|v| v.strip_suffix(".gstatic.com"))
                .is_some_and(|v| v.len() == 1 && v.as_bytes()[0].is_ascii_digit())
                && url.path() == "/images"
        }
    };
    allowed.then_some(url)
}
fn artstation_thumbnail_path(path: &str) -> bool {
    let numeric = |v: &str| !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit());
    let Some((group, file)) = path
        .strip_prefix("/artstation/")
        .and_then(|v| v.split_once('/'))
    else {
        return false;
    };
    let Some((work, image)) = file.strip_suffix(".jpg").and_then(|v| v.split_once('_')) else {
        return false;
    };
    numeric(group) && numeric(work) && numeric(image)
}
fn thumbnail_pixels(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    if !matches!(
        format,
        image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP
    ) {
        return None;
    }
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 8_000_000
    {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let image = if width > 640 || height > 640 {
        decoded.thumbnail(640, 640).to_rgb8()
    } else {
        decoded.to_rgb8()
    };
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
        .encode_image(&image)
        .ok()?;
    (bytes.len() <= 256 * 1024)
        .then(|| format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))
}
fn projected_candidates(
    values: Vec<RawCandidate>,
    service: Provider,
    cancel: &AtomicBool,
    deadline: Instant,
    probe_job: Option<&str>,
) -> Result<Vec<Candidate>, String> {
    use std::io::Read;
    let sources = values
        .iter()
        .map(|v| v.thumbnail_source.clone())
        .collect::<Vec<_>>();
    let mut output = candidates(values)?;
    if service == Provider::Lens {
        prepare_direct_links(
            &mut output,
            cancel,
            deadline.min(Instant::now() + Duration::from_secs(3)),
            |value, stop| {
                resolve_result_url_until(value, &|| !cancel.load(Ordering::Acquire), stop)
            },
        )?;
    }
    let stop = deadline.min(Instant::now() + Duration::from_secs(4));
    // Only exact already-rendered provider CDN thumbnails, with no account jar,
    // authorization, referrer or redirects. Missing previews do not erase results.
    let Ok(client) = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_millis(700))
        .build()
    else {
        return Ok(output);
    };
    for (index, (candidate, source)) in output.iter_mut().zip(sources).enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
        let remaining = stop.saturating_duration_since(Instant::now());
        if remaining < Duration::from_millis(100) {
            thumbnail_observation(probe_job, index, "budget_exhausted", "unknown", None);
            continue;
        }
        if let Some(data) = source
            .as_deref()
            .filter(|_| service == Provider::Lens)
            .filter(|v| v.len() <= 128 * 1024)
        {
            if let Some((mime, payload)) = data.split_once(";base64,") {
                if matches!(
                    mime,
                    "data:image/jpeg" | "data:image/png" | "data:image/webp"
                ) {
                    candidate.thumbnail_data = STANDARD
                        .decode(payload)
                        .ok()
                        .and_then(|bytes| thumbnail_pixels(&bytes));
                    thumbnail_observation(
                        probe_job,
                        index,
                        if candidate.thumbnail_data.is_some() {
                            "data_decoded"
                        } else {
                            "data_rejected"
                        },
                        "data",
                        None,
                    );
                    continue;
                }
            }
        }
        let Some(url) = source.as_deref().and_then(|v| thumbnail_url(service, v)) else {
            thumbnail_observation(
                probe_job,
                index,
                if source.is_none() {
                    "source_absent"
                } else {
                    "source_rejected"
                },
                "unknown",
                None,
            );
            continue;
        };
        let response = match client
            .get(url.clone())
            .timeout(remaining.min(Duration::from_secs(1)))
            .header(reqwest::header::ACCEPT, "image/jpeg,image/png,image/webp")
            .send()
        {
            Ok(response) => response,
            Err(error) => {
                thumbnail_observation(
                    probe_job,
                    index,
                    if error.is_timeout() {
                        "request_timeout"
                    } else {
                        "request_failed"
                    },
                    "unknown",
                    None,
                );
                continue;
            }
        };
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let mime_code = match mime {
            "image/jpeg" => "jpeg",
            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "text/html" => "html",
            _ => "other",
        };
        let status = response.status().as_u16();
        if response.status() != reqwest::StatusCode::OK
            || response.url() != &url
            || !matches!(mime, "image/jpeg" | "image/png" | "image/webp")
            || response
                .content_length()
                .is_some_and(|n| n == 0 || n > 2 * 1024 * 1024)
        {
            thumbnail_observation(
                probe_job,
                index,
                "response_rejected",
                mime_code,
                Some(status),
            );
            continue;
        }
        let mut bytes = Vec::new();
        if response
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_ok()
        {
            candidate.thumbnail_data = thumbnail_pixels(&bytes);
            thumbnail_observation(
                probe_job,
                index,
                if candidate.thumbnail_data.is_some() {
                    "decoded"
                } else {
                    "pixels_rejected"
                },
                mime_code,
                Some(status),
            );
        } else {
            thumbnail_observation(
                probe_job,
                index,
                "body_read_failed",
                mime_code,
                Some(status),
            );
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    Ok(output)
}
fn candidates(values: Vec<RawCandidate>) -> Result<Vec<Candidate>, String> {
    if values.len() > 10 {
        return Err(UNKNOWN.into());
    }
    values
        .into_iter()
        .map(|value| {
            if (!value.title.is_empty() && !text_valid(&value.title, 512))
                || value.creator.as_ref().is_some_and(|v| !text_valid(v, 256))
                || value
                    .similarity
                    .is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v))
                || value.links.is_empty()
                || value.links.len() > 3
                || value.source_metadata.len() > 10
                || value
                    .source_metadata
                    .iter()
                    .any(|v| !text_valid(&v.label, 128) || !text_valid(&v.value, 1024))
                || value
                    .thumbnail_source
                    .as_ref()
                    .is_some_and(|v| v.len() > 128 * 1024)
            {
                return Err(UNKNOWN.into());
            }
            let links = value
                .links
                .into_iter()
                .map(|link| {
                    if !text_valid(&link.label, 256) || basic_url(&link.url).is_none() {
                        return Err(UNKNOWN.to_string());
                    }
                    let url = basic_url(&link.url).ok_or(UNKNOWN)?.to_string();
                    if url.len() > 2048 {
                        return Err(UNKNOWN.to_string());
                    }
                    Ok(SearchLink {
                        label: link.label,
                        url,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Candidate {
                title: value.title,
                creator: value.creator,
                similarity: value.similarity,
                links,
                thumbnail_data: None,
                source_metadata: value.source_metadata,
            })
        })
        .collect()
}

fn state_error(state: &str) -> &'static str {
    match state {
        "challenge" => {
            "검색 서비스에서 사람 확인을 요구합니다. 자동으로 우회하거나 재시도하지 않았습니다."
        }
        "login" => "검색 서비스에서 로그인을 요구합니다. 자동 로그인하지 않았습니다.",
        "consent" => "검색 서비스의 개인정보 동의가 필요합니다. 자동 동의하지 않았습니다.",
        "quota" => "검색 서비스의 사용 한도에 도달했습니다. 자동으로 재시도하지 않았습니다.",
        "unavailable" => "검색 서비스 서버에 연결할 수 없습니다. 잠시 후 다시 확인해 주세요.",
        _ => UNKNOWN,
    }
}

pub(super) fn run(
    app: AppHandle,
    request: ProviderRequest,
    cancel: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    on_handoff: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<Vec<Candidate>, String> {
    // Result layouts are independently proven with the approved synthetic
    // fixture. Retain the fail-closed gate if a provider later becomes unsupported.
    let verified = match &request.input {
        ProviderInput::ImageUrl(_) | ProviderInput::ImageFile(_) => SAUCENAO_LAYOUT_VERIFIED,
        ProviderInput::Jpeg(_) | ProviderInput::LensResults { .. } => LENS_LAYOUT_VERIFIED,
    };
    if !verified {
        return Err(
            "검색 서비스의 자동 연결을 아직 검증하지 못했습니다. 자료를 전송하지 않았습니다."
                .into(),
        );
    }
    run_inner(app, request, cancel, resume, on_handoff, None, None).map(|result| result.candidates)
}

pub(super) fn run_for(
    app: AppHandle,
    request: ProviderRequest,
    cancel: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    on_handoff: Arc<dyn Fn(String) + Send + Sync>,
    service: &str,
) -> Result<Results, String> {
    let selected = match service {
        "saucenao" if SAUCENAO_LAYOUT_VERIFIED => Provider::Saucenao,
        "lens" if LENS_LAYOUT_VERIFIED => Provider::Lens,
        _ => return Err("이 검색 서비스는 현재 사용할 수 없습니다.".into()),
    };
    run_inner(
        app,
        request,
        cancel,
        resume,
        on_handoff,
        Some(selected),
        None,
    )
}

// Probe only: the dedicated example supplies agent-created neutral bytes and
// owns the evidence sink. Production run never records provider page content.
#[cfg(debug_assertions)]
pub(super) fn probe(
    app: AppHandle,
    request: ProviderRequest,
    cancel: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    on_handoff: Arc<dyn Fn(String) + Send + Sync>,
    service: &str,
    evidence: Arc<Mutex<Option<Value>>>,
) -> Result<Vec<Candidate>, String> {
    let service = match service {
        "lens" => Provider::Lens,
        "saucenao" => Provider::Saucenao,
        _ => return Err(INVALID.into()),
    };
    // Only the opt-in fixed-hash synthetic example calls this evidence path.
    // Production run_for selects a closed destination after dual consent.
    run_inner(
        app,
        request,
        cancel,
        resume,
        on_handoff,
        Some(service),
        Some(evidence),
    )
    .map(|result| result.candidates)
}

fn run_inner(
    app: AppHandle,
    request: ProviderRequest,
    cancel: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    on_handoff: Arc<dyn Fn(String) + Send + Sync>,
    probe_service: Option<Provider>,
    evidence: Option<Arc<Mutex<Option<Value>>>>,
) -> Result<Results, String> {
    let retained_expiry = match &request.input {
        ProviderInput::LensResults { expires, .. } => Some(*expires),
        _ => None,
    };
    let revisiting = retained_expiry.is_some();
    let trace = Arc::new(ProbeTrace::new(evidence.is_some()));
    let thumbnail_probe = thumbnail_probe_job(&request);
    trace.phase("started");
    let mut budget = Budget::new();
    let mut automatic_started = Instant::now();
    let mut deadline = automatic_started + budget.automatic;
    let overall_deadline = (automatic_started + DEADLINE + HUMAN_DEADLINE)
        .min(retained_expiry.unwrap_or(automatic_started + DEADLINE + HUMAN_DEADLINE));
    deadline = deadline.min(overall_deadline);
    check(&cancel, deadline)?;
    if uuid::Uuid::parse_str(&request.job_id).is_err() {
        return Err(INVALID.into());
    }
    let service = probe_service.unwrap_or_else(|| provider(&request.input));
    let submit = if probe_service.is_some() {
        submission_for(&request.input, service)?
    } else {
        submission(&request.input)?
    };
    // The controller has invalidated the old owner and requested its close.
    // Await the native ACK before acquiring its profile lease for the new job.
    await_prior_close(service, &cancel, deadline)?;
    let directory = profile_directory(
        &app.path().app_local_data_dir().map_err(|_| ISOLATION)?,
        service,
    );
    std::fs::create_dir_all(&directory).map_err(|_| ISOLATION)?;
    let profile_lock = lock_profile(&directory)?;
    let label = format!("x-media-search-{}", uuid::Uuid::new_v4().simple());
    if CHILD_RESERVED[service.index()]
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(CLEANUP.into());
    }
    *PROFILE_LOCK[service.index()]
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(profile_lock);
    let live = Arc::new(AtomicBool::new(true));
    let cleanup = Arc::new(Cleanup {
        service_index: service.index(),
        ..Cleanup::default()
    });
    let page_control = Arc::new(PageControl {
        retained: AtomicBool::new(false),
        shown: AtomicBool::new(false),
        settings: AtomicBool::new(false),
        closed: Mutex::new(false),
        changed: Condvar::new(),
        deadline: Mutex::new(overall_deadline),
        results_url: Mutex::new(match &request.input {
            ProviderInput::LensResults { url, .. } => Some(url.clone()),
            _ => None,
        }),
        result_intent: Mutex::new(None),
    });
    let child = Arc::new(Child {
        app: app.clone(),
        label: label.clone(),
        live: live.clone(),
        cleanup: cleanup.clone(),
        close_requested: AtomicBool::new(false),
    });
    let mut verified_page = false;
    let operation = (|| {
        let blocked: Arc<Mutex<Option<&'static str>>> = Arc::default();
        let blocked_on_navigation = blocked.clone();
        let owned = app.clone();
        let name = label.clone();
        let wanted = live.clone();
        let stop = cancel.clone();
        let close_trace = trace.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let on_created = cleanup.clone();
        let create_control = page_control.clone();
        let scheduled = app.run_on_main_thread(move || {
            let built = (|| -> Result<(), String> {
                if !wanted.load(Ordering::Acquire)
                    || stop.load(Ordering::Acquire)
                    || Instant::now() >= deadline
                {
                    return Err(CANCELLED.into());
                }
                let parent = WindowBuilder::new(&owned, &name)
                    .title(service.title())
                    .visible(false)
                    .focused(false)
                    .inner_size(1100., 900.)
                    .min_inner_size(500., 500.)
                    .build()
                    .map_err(|_| ISOLATION)?;
                let close_cancel = stop.clone();
                let close_live = wanted.clone();
                let close_control = create_control.clone();
                parent.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        close_trace.phase("provider_close_requested");
                        close_cancel.store(true, Ordering::Release);
                        close_control.dismiss();
                    }
                    tauri::WindowEvent::Destroyed if close_live.load(Ordering::Acquire) => {
                        close_cancel.store(true, Ordering::Release);
                        close_control.dismiss();
                    }
                    _ => {}
                });
                let nav_live = wanted.clone();
                let nav_cancel = stop.clone();
                let nav_control = create_control.clone();
                let route_app = owned.clone();
                let builder = WebviewBuilder::new(
                    &name,
                    WebviewUrl::External("about:blank".parse().unwrap()),
                )
                .incognito(false)
                .data_directory(directory.clone())
                .focused(false)
                .browser_extensions_enabled(false)
                .additional_browser_args(
                    "--disable-extensions --autoplay-policy=user-gesture-required",
                )
                .on_navigation(move |url| {
                    if !nav_live.load(Ordering::Acquire)
                        || (!nav_control.retained.load(Ordering::Acquire)
                            && nav_cancel.load(Ordering::Acquire))
                        || *nav_control.closed.lock().unwrap_or_else(|p| p.into_inner())
                        || Instant::now()
                            >= *nav_control
                                .deadline
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                    {
                        return false;
                    }
                    if let Some(action) = result_navigation(&nav_control, url) {
                        return match action {
                            ResultNavigation::Allow => true,
                            ResultNavigation::Stop => false,
                            ResultNavigation::Open(target, reply) => {
                                let result =
                                    super::open_provider_result(&route_app, &target.to_string());
                                let _ = reply.send(result);
                                false
                            }
                        };
                    }
                    if navigation(service, url) && !lens_settings_url(url)
                        || manual_lens_settings(&nav_control, service)
                            && lens_manual_settings_url(url)
                    {
                        true
                    } else {
                        *blocked_on_navigation
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = Some(denied_navigation(url));
                        false
                    }
                })
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .on_download(|_, _| false);
                let view = parent
                    .add_child(
                        builder,
                        LogicalPosition::new(0., 0.),
                        LogicalSize::new(1100., 900.),
                    )
                    .map_err(|_| ISOLATION)?;
                view.set_auto_resize(true).map_err(|_| ISOLATION)?;
                if !wanted.load(Ordering::Acquire)
                    || stop.load(Ordering::Acquire)
                    || Instant::now() >= deadline
                {
                    return Err(CANCELLED.into());
                }
                Ok(())
            })();
            if built.is_err() {
                wanted.store(false, Ordering::Release);
            }
            on_created.creation_finished.store(true, Ordering::Release);
            // If the controller timed out while creation was queued/in progress,
            // the late-created native view is closed before the reservation clears.
            if !wanted.load(Ordering::Acquire) {
                close_on_ui(&owned, &name, &on_created);
            }
            let _ = tx.send(built);
        });
        if scheduled.is_err() {
            // Scheduling failed, so no child creation was admitted.
            cleanup.creation_finished.store(true, Ordering::Release);
            cleanup.finish(Ok(()));
            return Err(ISOLATION.into());
        }
        receive(&rx, &cancel, deadline)??;
        trace.phase("provider_created");
        let view = app.get_webview(&label).ok_or(ISOLATION)?;
        let directory = profile_directory(
            &app.path().app_local_data_dir().map_err(|_| ISOLATION)?,
            service,
        );
        verify_profile_and_mute(&view, directory, &cancel, deadline)?;
        install_original_links(&view, service, page_control.clone(), &cancel, deadline)?;
        verified_page = true;
        trace.phase("profile_verified");
        check(&cancel, deadline)?;
        if read_url(
            &app,
            &label,
            &cancel,
            deadline.min(Instant::now() + Duration::from_secs(2)),
        )?
        .as_str()
            != "about:blank"
        {
            return Err(ISOLATION.into());
        }
        dispatch_submission(
            &app,
            &label,
            submit,
            &cancel,
            deadline.min(Instant::now() + Duration::from_secs(2)),
        )?;
        trace.phase("submitted_once");
        let nonce = uuid::Uuid::new_v4().to_string();
        let config = json!({"nonce":nonce,"provider":service.name(),"probe":evidence.is_some(),"preferExact":service==Provider::Lens,"lensVerified":LENS_LAYOUT_VERIFIED,"saucenaoVerified":SAUCENAO_LAYOUT_VERIFIED});
        let mut extractor = include_str!("x_media_search_extract.js").to_owned();
        #[cfg(debug_assertions)]
        if evidence.is_some()
            && std::env::var("ATSUMI_SYNTHETIC_CURRENT_EXTRACTOR").as_deref() == Ok("1")
        {
            use sha2::{Digest, Sha256};
            let bytes = match &request.input {
                ProviderInput::ImageFile(bytes) | ProviderInput::Jpeg(bytes) => bytes,
                _ => return Err(INVALID.into()),
            };
            if bytes.len() != 11793
                || format!("{:X}", Sha256::digest(bytes))
                    != "4D2FEB4CF3F6A87DBF830A583B43108A21A25E246AC14DCDA8C68A2C110390FF"
            {
                return Err(INVALID.into());
            }
            // Fixed-fixture diagnostic only. No arbitrary path, production
            // command, account profile or runtime page can select this source.
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/x_media_search_extract.js");
            let bytes = std::fs::read(path).map_err(|_| INVALID)?;
            if bytes.len() > 64 * 1024 {
                return Err(INVALID.into());
            }
            extractor = String::from_utf8(bytes).map_err(|_| INVALID)?;
            trace.phase("fixed_workspace_extractor_loaded");
        }
        let mut script = extractor.replace("__ATSUMI_MEDIA_SEARCH__", &config.to_string());
        if evidence.is_some() {
            let unknown = json!({"nonce":nonce,"state":"unknown","candidates":[],"probe":{"classification":"extractor_exception"}});
            script = format!("(()=>{{try{{return ({script});}}catch(error){{const reply={unknown};reply.probe.errorKind=['TypeError','ReferenceError','SyntaxError','RangeError'].includes(error?.name)?error.name:'Other';return reply;}}}})()");
        }
        let mut last_unknown = false;
        let mut document_observed = false;
        let mut reply_observed = false;
        let mut lens_plan = LensPlan::default();
        loop {
            check(&cancel, deadline)?;
            if let Some(error) = *blocked.lock().unwrap_or_else(|p| p.into_inner()) {
                return Err(error.into());
            }
            let before = read_url(
                &app,
                &label,
                &cancel,
                deadline.min(Instant::now() + Duration::from_secs(2)),
            )?;
            if before.as_str() == "about:blank" {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if !navigation(service, &before) {
                return Err(UNKNOWN.into());
            }
            if service == Provider::Lens && lens_results_url(&before) {
                let mut result = page_control
                    .results_url
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if result.is_none() {
                    *result = Some(before.clone());
                }
            }
            if !document_observed {
                trace.phase("provider_document_reached");
                document_observed = true;
            }
            let reply = if before.host_str() == Some("consent.google.com") {
                // Do not inject a result extractor into a service-consent page.
                Reply {
                    nonce: nonce.clone(),
                    state: "consent".into(),
                    candidates: vec![],
                    probe: None,
                    lens: None,
                }
            } else {
                let raw = evaluate(
                    &app,
                    &label,
                    script.clone(),
                    &cancel,
                    deadline.min(Instant::now() + Duration::from_secs(2)),
                )?;
                check(&cancel, deadline)?;
                if read_url(
                    &app,
                    &label,
                    &cancel,
                    deadline.min(Instant::now() + Duration::from_secs(2)),
                )? != before
                {
                    continue;
                }
                if raw.len()
                    > if evidence.is_some() {
                        4 * 1024 * 1024
                    } else {
                        MAX_REPLY
                    }
                {
                    return Err(UNKNOWN.into());
                }
                serde_json::from_str::<Reply>(&raw).map_err(|_| {
                    trace.phase(if raw == "null" {
                        "extractor_null_reply"
                    } else {
                        "extractor_invalid_reply"
                    });
                    UNKNOWN
                })?
            };
            if reply.nonce != nonce {
                return Err(UNKNOWN.into());
            }
            if !reply_observed {
                trace.phase("extractor_reply_observed");
                reply_observed = true;
            }
            let extractor_exception = reply.probe.as_ref().is_some_and(|probe| {
                probe.get("classification").and_then(Value::as_str) == Some("extractor_exception")
            });
            if let Some(sink) = &evidence {
                let probe = reply
                    .probe
                    .unwrap_or_else(|| json!({"classification":"projection_absent"}));
                *sink.lock().unwrap_or_else(|p| p.into_inner()) = Some(
                    json!({"provider":service.name(),"origin":before.origin().ascii_serialization(),"path":before.path(),"state":reply.state,"document":probe}),
                );
            }
            if extractor_exception {
                return Err(UNKNOWN.into());
            }
            if service == Provider::Lens {
                match lens_plan.observe(reply.lens.as_ref(), &before, &reply.state)? {
                    LensDecision::Navigate(url) => {
                        navigate_results_tab(
                            &app,
                            &label,
                            url,
                            &cancel,
                            deadline.min(Instant::now() + Duration::from_secs(2)),
                        )?;
                        continue;
                    }
                    LensDecision::Wait => {
                        last_unknown = true;
                        std::thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    LensDecision::Read => {
                        if lens_results_url(&before)
                            && matches!(reply.state.as_str(), "complete" | "empty")
                        {
                            *page_control
                                .results_url
                                .lock()
                                .unwrap_or_else(|p| p.into_inner()) = Some(before.clone());
                        }
                    }
                }
            }
            match reply.state.as_str() {
                "complete" if !reply.candidates.is_empty() => {
                    return projected_candidates(
                        reply.candidates,
                        service,
                        &cancel,
                        deadline,
                        thumbnail_probe.as_deref(),
                    )
                    .map(Results::from)
                }
                "empty" if reply.candidates.is_empty() => return Ok(Vec::new().into()),
                "lowSimilarityHidden"
                    if service == Provider::Saucenao && reply.candidates.is_empty() =>
                {
                    return Ok(Results {
                        candidates: Vec::new(),
                        notice: Some(ResultNotice::LowSimilarityHidden),
                    });
                }
                "pending" => {}
                "unknown" => {
                    last_unknown = true;
                }
                "challenge" | "consent" => {
                    if revisiting {
                        window_visibility(
                            &app,
                            &label,
                            true,
                            &cancel,
                            deadline.min(Instant::now() + CLOSE_DEADLINE),
                        )?;
                        return Err("Google 검색 화면에서 직접 확인한 뒤 결과 새로고침을 눌러 주세요. 미디어를 다시 전송하지 않았습니다.".into());
                    }
                    trace.phase("verification_required");
                    budget.charge_automatic(automatic_started.elapsed())?;
                    let human_started = Instant::now();
                    let human_deadline = human_started + budget.human;
                    resume.store(false, Ordering::Release);
                    window_visibility(
                        &app,
                        &label,
                        true,
                        &cancel,
                        human_deadline.min(Instant::now() + Duration::from_secs(2)),
                    )?;
                    check(&cancel, human_deadline)?;
                    on_handoff("검색 서비스 창에서 직접 확인한 뒤 앱의 ‘계속’을 눌러 주세요. 자료를 다시 전송하지 않습니다.".into());
                    wait_for_human(&resume, &cancel, human_deadline)?;
                    trace.phase("explicit_resume_received");
                    budget.charge_human(human_started.elapsed())?;
                    automatic_started = Instant::now();
                    deadline = automatic_started + budget.automatic;
                    window_visibility(
                        &app,
                        &label,
                        false,
                        &cancel,
                        deadline.min(Instant::now() + Duration::from_secs(2)),
                    )?;
                    // Explicit resume observes the same document only. `submit`
                    // is never called here, even if verification remains pending.
                    last_unknown = false;
                    continue;
                }
                state => return Err(state_error(state).into()),
            }
            if deadline.saturating_duration_since(Instant::now()) < Duration::from_millis(300) {
                return Err(if last_unknown { UNKNOWN } else { TIMEOUT }.into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    })();
    trace.phase(if cancel.load(Ordering::Acquire) {
        "cancel_observed"
    } else {
        "operation_finished"
    });
    // Terminal errors still have useful genuine provider pages. The controller
    // may set its job cancellation flag after this return; retained ownership is
    // separately closed by explicit cancel/stale/account/privacy/shutdown hooks.
    if evidence.is_none()
        && verified_page
        && !cancel.load(Ordering::Acquire)
        && live.load(Ordering::Acquire)
        && cleanup.creation_finished.load(Ordering::Acquire)
    {
        if retain_original(
            request.job_id,
            child.clone(),
            page_control,
            &cancel,
            retained_expiry,
        ) {
            return operation;
        }
    }
    trace.phase("closing");
    if let Err(error) = child.close() {
        trace.phase("close_unconfirmed");
        return Err(error);
    }
    trace.phase("close_acknowledged");
    operation
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result_control() -> (PageControl, std::sync::mpsc::Receiver<Result<(), String>>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        (
            PageControl {
                retained: AtomicBool::new(true),
                shown: AtomicBool::new(true),
                settings: AtomicBool::new(false),
                closed: Mutex::new(false),
                changed: Condvar::new(),
                deadline: Mutex::new(Instant::now() + ORIGINAL_TTL),
                results_url: Mutex::new(None),
                result_intent: Mutex::new(Some(ResultIntent {
                    id: "test".into(),
                    initial: Url::parse("https://www.google.com/goto?url=observed").unwrap(),
                    steps: 0,
                    expires: Instant::now() + Duration::from_secs(8),
                    reply: tx,
                })),
            },
            rx,
        )
    }
    #[test]
    fn retained_browser_resolves_only_the_selected_wrapper_and_observed_target() {
        let (control, rx) = result_control();
        let initial = Url::parse("https://www.google.com/goto?url=observed").unwrap();
        assert!(matches!(
            result_navigation(&control, &initial),
            Some(ResultNavigation::Allow)
        ));
        let target = Url::parse("https://x.com/example/status/12345").unwrap();
        let Some(ResultNavigation::Open(observed, receipt)) = result_navigation(&control, &target)
        else {
            panic!("missing observed target")
        };
        assert_eq!(observed, target);
        receipt.send(Ok(())).unwrap();
        assert!(rx.recv().unwrap().is_ok());
        assert!(result_navigation(&control, &target).is_none());
        let (control, rx) = result_control();
        assert!(matches!(
            result_navigation(&control, &target),
            Some(ResultNavigation::Stop)
        ));
        assert!(rx.recv().unwrap().is_err());
    }
    #[test]
    fn browser_redirect_expiry_close_security_and_hop_limit_fail_without_target() {
        let initial = Url::parse("https://www.google.com/goto?url=observed").unwrap();
        for kind in [
            "closed",
            "expired",
            "hidden",
            "unretained",
            "security",
            "hops",
        ] {
            let (control, rx) = result_control();
            match kind {
                "closed" => control.dismiss(),
                "expired" => {
                    control
                        .result_intent
                        .lock()
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .expires = Instant::now()
                }
                "hidden" => control.shown.store(false, Ordering::Release),
                "unretained" => control.retained.store(false, Ordering::Release),
                _ => {}
            }
            if kind == "closed" {
                assert!(rx.recv().unwrap().is_err());
                continue;
            }
            if kind == "security" {
                assert!(matches!(
                    result_navigation(&control, &initial),
                    Some(ResultNavigation::Allow)
                ));
                assert!(matches!(
                    result_navigation(
                        &control,
                        &Url::parse("https://accounts.google.com/ServiceLogin").unwrap()
                    ),
                    Some(ResultNavigation::Stop)
                ));
            } else if kind == "hops" {
                for _ in 0..4 {
                    assert!(matches!(
                        result_navigation(&control, &initial),
                        Some(ResultNavigation::Allow)
                    ));
                }
                assert!(matches!(
                    result_navigation(&control, &initial),
                    Some(ResultNavigation::Stop)
                ));
            } else {
                assert!(matches!(
                    result_navigation(&control, &initial),
                    Some(ResultNavigation::Stop)
                ));
            }
            assert!(rx.recv().unwrap().is_err(), "{kind}");
        }
    }
    fn lens_document(mode: &str) -> LensNavigation {
        LensNavigation {
            mode: mode.into(),
            exact_url: Some("https://www.google.com/search?fixture=exact".into()),
            visual_url: Some("https://www.google.com/search?fixture=visual".into()),
            all_url: Some("https://www.google.com/search?fixture=all".into()),
        }
    }
    #[test]
    fn exact_results_are_required_before_any_other_mode_and_only_empty_unlocks_fallback() {
        let all = Url::parse("https://www.google.com/search?fixture=all").unwrap();
        let exact = Url::parse("https://www.google.com/search?fixture=exact").unwrap();
        let visual = Url::parse("https://www.google.com/search?fixture=visual").unwrap();
        let mut plan = LensPlan::default();
        assert_eq!(
            plan.observe(None, &all, "complete").unwrap(),
            LensDecision::Wait
        );
        assert_eq!(
            plan.observe(Some(&lens_document("all")), &all, "complete")
                .unwrap(),
            LensDecision::Navigate(exact.clone())
        );
        assert_eq!(
            plan.observe(Some(&lens_document("all")), &all, "complete")
                .unwrap(),
            LensDecision::Wait
        );
        for state in ["unknown", "pending", "complete"] {
            assert_eq!(
                plan.observe(Some(&lens_document("exact")), &exact, state)
                    .unwrap(),
                LensDecision::Read
            );
            assert_eq!(plan.desired, "exact");
        }
        assert_eq!(
            plan.observe(Some(&lens_document("exact")), &exact, "empty")
                .unwrap(),
            LensDecision::Navigate(visual.clone())
        );
        assert_eq!(
            plan.observe(Some(&lens_document("visual")), &visual, "complete")
                .unwrap(),
            LensDecision::Read
        );
        assert_eq!(
            plan.observe(Some(&lens_document("visual")), &visual, "empty")
                .unwrap(),
            LensDecision::Navigate(all.clone())
        );
        assert_eq!(
            plan.observe(Some(&lens_document("all")), &all, "empty")
                .unwrap(),
            LensDecision::Read
        );
    }
    #[test]
    fn unsafe_missing_ambiguous_or_challenged_exact_navigation_does_not_become_zero() {
        let current = Url::parse("https://www.google.com/search?fixture=all").unwrap();
        let mut document = lens_document("unknown");
        for url in [
            "https://evil.example/search?x=1",
            "https://www.google.com/search",
            "https://user@www.google.com/search?x=1",
            "https://www.google.com/search?x=1#extra",
        ] {
            document.exact_url = Some(url.into());
            let mut plan = LensPlan::default();
            assert_eq!(
                plan.observe(Some(&document), &current, "empty").unwrap(),
                LensDecision::Wait
            );
        }
        document.exact_url = None;
        assert_eq!(
            LensPlan::default()
                .observe(Some(&document), &current, "complete")
                .unwrap(),
            LensDecision::Wait
        );
        assert_eq!(
            LensPlan::default()
                .observe(None, &current, "challenge")
                .unwrap(),
            LensDecision::Read
        );
    }
    #[test]
    fn google_redirect_resolution_observes_locations_and_never_requests_the_destination() {
        let start = Url::parse("https://www.google.com/goto?url=OPAQUE").unwrap();
        let mut requested = vec![];
        let found = resolve_redirect_with(start.clone(), &|| true, |url| {
            requested.push(url.clone());
            Ok(Some(
                if requested.len() == 1 {
                    "/url?q=still-opaque"
                } else {
                    "https://twitter.com/example/status/12345/photo/1"
                }
                .into(),
            ))
        })
        .unwrap();
        assert_eq!(
            found.as_str(),
            "https://twitter.com/example/status/12345/photo/1"
        );
        assert_eq!(requested.len(), 2);
        assert!(requested.iter().all(google_result_redirect));
        assert!(resolve_redirect_with(start, &|| true, |_| Ok(None)).is_err());
    }
    #[test]
    fn redirect_unknown_loop_unsafe_and_cancelled_results_cannot_open_a_target() {
        let start =
            Url::parse("https://www.google.com/goto?url=https%3A%2F%2Fx.com%2Fguessed").unwrap();
        for location in [
            "file:///C:/secret",
            "https://user:pass@x.com/example",
            "https://x.com\\@evil.example/",
            "https://www.google.com/sorry/index",
            "https://accounts.google.com/",
            "https://x.com/example\r\nX: injected",
        ] {
            assert!(
                resolve_redirect_with(start.clone(), &|| true, |_| Ok(Some(location.into())))
                    .is_err()
            );
        }
        assert!(
            resolve_redirect_with(start.clone(), &|| true, |_| Ok(Some(start.to_string())))
                .is_err()
        );
        let cancelled = AtomicBool::new(false);
        assert!(
            resolve_redirect_with(start, &|| !cancelled.load(Ordering::Acquire), |_| {
                cancelled.store(true, Ordering::Release);
                Ok(Some("https://x.com/example".into()))
            })
            .is_err()
        );
    }
    fn linked_candidate(url: &str) -> Candidate {
        Candidate {
            title: "Public illustration".into(),
            creator: None,
            similarity: None,
            links: vec![SearchLink {
                label: "Original label".into(),
                url: url.into(),
            }],
            thumbnail_data: None,
            source_metadata: vec![],
        }
    }
    #[test]
    fn prepared_lens_links_use_observed_destinations_once_and_retain_failures() {
        let observed = "https://www.google.com/goto?url=observed";
        let unresolved = "https://www.google.com/url?q=unresolved";
        let direct = "https://example.org/already-direct";
        let mut values = vec![
            linked_candidate(observed),
            linked_candidate(observed),
            linked_candidate(unresolved),
            linked_candidate(direct),
        ];
        let cancel = AtomicBool::new(false);
        let requests = Mutex::new(vec![]);
        let deadline = Instant::now() + Duration::from_secs(1);
        prepare_direct_links(&mut values, &cancel, deadline, |source, stop| {
            assert_eq!(stop, deadline);
            requests.lock().unwrap().push(source.to_owned());
            if source == observed {
                Ok("https://www.pixiv.net/artworks/12345".into())
            } else {
                Err(REDIRECT_UNRESOLVED.into())
            }
        })
        .unwrap();
        let requests = requests.into_inner().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests.contains(&direct.to_owned()));
        for value in &values[..2] {
            assert_eq!(value.links[0].url, "https://www.pixiv.net/artworks/12345");
            assert_eq!(value.links[0].label, "www.pixiv.net");
        }
        assert_eq!(values[2].links[0].url, unresolved);
        assert_eq!(values[3].links[0].url, direct);
        assert_eq!(values[3].links[0].label, "Original label");
    }
    #[test]
    fn link_preparation_honors_shared_deadline_and_cancellation_without_late_mutation() {
        let original = linked_candidate("https://www.google.com/goto?url=observed");
        let cancel = AtomicBool::new(false);
        let mut values = vec![original.clone()];
        prepare_direct_links(&mut values, &cancel, Instant::now(), |_, _| {
            panic!("expired lookup must not run")
        })
        .unwrap();
        assert_eq!(values[0].links[0].url, original.links[0].url);
        let result = prepare_direct_links(
            &mut values,
            &cancel,
            Instant::now() + Duration::from_secs(1),
            |_, _| {
                cancel.store(true, Ordering::Release);
                Ok("https://example.org/late".into())
            },
        );
        assert_eq!(result, Err(CANCELLED.into()));
        assert_eq!(values[0].links[0].url, original.links[0].url);
    }
    #[test]
    fn unsafe_or_still_google_preparation_cannot_replace_an_unresolved_link() {
        for destination in [
            "file:///C:/secret",
            "https://user:secret@example.org/",
            "https://accounts.google.com/",
            "https://www.google.com/goto?url=still-opaque",
        ] {
            let mut values = vec![linked_candidate("https://www.google.com/goto?url=observed")];
            let original = values[0].links[0].url.clone();
            prepare_direct_links(
                &mut values,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1),
                |_, _| Ok(destination.into()),
            )
            .unwrap();
            assert_eq!(values[0].links[0].url, original);
        }
    }
    #[test]
    fn closing_refreshed_provider_cannot_cancel_parent_but_parent_cancellation_reaches_refresh() {
        let parent = Arc::new(AtomicBool::new(false));
        let child = RefreshStop::new(parent.clone()).unwrap();
        child.child.store(true, Ordering::Release);
        assert!(!parent.load(Ordering::Acquire));
        drop(child);
        let child = RefreshStop::new(parent.clone()).unwrap();
        parent.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !child.child.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.child.load(Ordering::Acquire));
        assert!(RefreshStop::new(parent).is_err());
    }
    #[test]
    fn lens_refresh_is_a_bounded_results_get_and_never_an_upload() {
        let expires = Instant::now() + Duration::from_secs(30);
        let input = ProviderInput::LensResults {
            url: Url::parse("https://www.google.com/search?vsrid=observed&udm=26").unwrap(),
            expires,
        };
        let script = submission_for(&input, Provider::Lens).unwrap();
        assert!(script.contains("location.replace"));
        for forbidden in [
            "FormData",
            "File(",
            "fetch(",
            "/upload",
            "base64",
            "input.files",
        ] {
            assert!(!script.contains(forbidden));
        }
        assert!(submission_for(&input, Provider::Saucenao).is_err());
        for value in [
            "https://evil.test/search?q=x",
            "https://www.google.com/v3/upload?q=x",
            "https://user@www.google.com/search?q=x",
            "http://www.google.com/search?q=x",
            "https://www.google.com/search",
        ] {
            assert!(submission_for(
                &ProviderInput::LensResults {
                    url: Url::parse(value).unwrap(),
                    expires
                },
                Provider::Lens
            )
            .is_err());
        }
        assert!(submission_for(
            &ProviderInput::LensResults {
                url: Url::parse("https://lens.google.com/search?vsrid=x").unwrap(),
                expires: Instant::now()
            },
            Provider::Lens
        )
        .is_err());
        assert!(lens_settings_url(
            &Url::parse("https://www.google.com/safesearch?hl=ko").unwrap()
        ));
        for value in [
            "https://accounts.google.com/safesearch",
            "https://www.google.com/account",
            "https://evil.test/setprefs",
            "https://user@www.google.com/safesearch",
        ] {
            assert!(!lens_settings_url(&Url::parse(value).unwrap()));
        }
    }
    #[test]
    fn one_provider_cleanup_does_not_release_the_other_provider_reservation() {
        assert_ne!(Provider::Saucenao.index(), Provider::Lens.index());
        let a = Provider::Saucenao.index();
        let b = Provider::Lens.index();
        assert!(!CHILD_RESERVED[a].swap(true, Ordering::AcqRel));
        assert!(!CHILD_RESERVED[b].swap(true, Ordering::AcqRel));
        let first = Cleanup {
            service_index: a,
            ..Cleanup::default()
        };
        let second = Cleanup {
            service_index: b,
            ..Cleanup::default()
        };
        first.finish(Ok(()));
        assert!(!CHILD_RESERVED[a].load(Ordering::Acquire));
        assert!(CHILD_RESERVED[b].load(Ordering::Acquire));
        assert!(busy());
        second.finish(Ok(()));
        assert!(!busy());
    }
    #[test]
    fn local_image_pixels_use_saucenao_without_changing_video_frame_provider() {
        let image = ProviderInput::ImageFile(vec![0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(provider(&image), Provider::Saucenao);
        assert!(submission(&image)
            .unwrap()
            .contains("\"provider\":\"saucenao\""));
        assert!(submission_for(&image, Provider::Lens).is_ok());
        assert_eq!(
            provider(&ProviderInput::Jpeg(vec![0xff, 0xd8, 0xff, 0xd9])),
            Provider::Lens
        );
    }
    #[test]
    fn provider_thumbnails_are_bounded_clean_pixels_from_exact_service_hosts() {
        assert!(thumbnail_url(
            Provider::Saucenao,
            "https://img3.saucenao.com/artstation/533/12345_67890.jpg?auth=neutral&exp=2000000000"
        )
        .is_some());
        for path in [
            "/artstation/a/123_456.jpg",
            "/artstation/533/123_456.svg",
            "/artstation/533/123_456/other.jpg",
            "/artstation/533/123_456_789.jpg",
        ] {
            assert!(thumbnail_url(
                Provider::Saucenao,
                &format!("https://img3.saucenao.com{path}")
            )
            .is_none());
        }
        assert!(thumbnail_url(
            Provider::Saucenao,
            "https://img1.saucenao.com/res/pixiv/a.jpg?auth=fixture"
        )
        .is_some());
        assert!(thumbnail_url(
            Provider::Lens,
            "https://encrypted-tbn0.gstatic.com/images?q=fixture"
        )
        .is_some());
        for url in [
            "https://evil.test/a.jpg",
            "http://img1.saucenao.com/res/a.jpg",
            "https://img10.saucenao.com/res/a.jpg",
            "https://user@img1.saucenao.com/res/a.jpg",
            "https://img1.saucenao.com/profile/a.jpg",
        ] {
            assert!(thumbnail_url(Provider::Saucenao, url).is_none());
        }
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1200,
            600,
            image::Rgb([25, 100, 200]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let mut input = bytes.into_inner();
        let marker = b"PRIVATE_TRAILING_METADATA";
        input.extend_from_slice(marker);
        let data = thumbnail_pixels(&input).unwrap();
        let encoded = data.strip_prefix("data:image/jpeg;base64,").unwrap();
        let pixels = STANDARD.decode(encoded).unwrap();
        assert!(pixels.len() <= 256 * 1024);
        assert!(!pixels.windows(marker.len()).any(|v| v == marker));
        let image = image::load_from_memory(&pixels).unwrap();
        assert_eq!((image.width(), image.height()), (640, 320));
        assert!(thumbnail_pixels(b"<svg></svg>").is_none());
        assert!(thumbnail_pixels(&vec![0; 2 * 1024 * 1024 + 1]).is_none());
    }
    #[test]
    fn small_provider_preview_keeps_its_original_pixel_dimensions() {
        let source = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            48,
            32,
            image::Rgb([30, 80, 130]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let data = thumbnail_pixels(bytes.get_ref()).unwrap();
        let pixels = STANDARD
            .decode(data.strip_prefix("data:image/jpeg;base64,").unwrap())
            .unwrap();
        let decoded = image::load_from_memory(&pixels).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (48, 32));
    }
    #[test]
    fn maximum_admitted_thumbnail_and_metadata_reply_fits_transport_bound() {
        // Ten individually valid data thumbnails plus maximal UTF-8 metadata
        // must not turn valid text results into UNKNOWN at the reply boundary.
        let prefix = "data:image/jpeg;base64,";
        let source = format!("{prefix}{}", "A".repeat(128 * 1024 - prefix.len()));
        let metadata = (0..10)
            .map(|_| json!({"label":"가".repeat(128),"value":"가".repeat(1024)}))
            .collect::<Vec<_>>();
        let item = json!({"title":"가".repeat(512),"creator":"가".repeat(256),"similarity":50.12345,"thumbnailSource":source,"sourceMetadata":metadata,"links":[{"label":"가".repeat(256),"url":"https://example.org/a"}]});
        let raw =
            json!({"nonce":"fixture","state":"complete","candidates":vec![item;10]}).to_string();
        assert!(raw.len() > 128 * 1024);
        assert!(raw.len() <= MAX_REPLY);
        let reply: Reply = serde_json::from_str(&raw).unwrap();
        assert_eq!(candidates(reply.candidates).unwrap().len(), 10);
    }
    #[test]
    fn original_links_require_visible_retained_page_and_real_nonredirected_user_action() {
        let control = PageControl {
            retained: AtomicBool::new(false),
            shown: AtomicBool::new(false),
            settings: AtomicBool::new(false),
            closed: Mutex::new(false),
            changed: Condvar::new(),
            deadline: Mutex::new(Instant::now() + ORIGINAL_TTL),
            results_url: Mutex::new(None),
            result_intent: Mutex::new(None),
        };
        let url = "https://example.org/observed-result";
        assert!(original_external_link(&control, Provider::Lens, url, true, false).is_none());
        control.retained.store(true, Ordering::Release);
        control.shown.store(true, Ordering::Release);
        let account = Url::parse("https://accounts.google.com/ServiceLogin?hl=ko").unwrap();
        assert!(!navigation(Provider::Lens, &account));
        assert!(!manual_lens_settings(&control, Provider::Lens));
        control.settings.store(true, Ordering::Release);
        assert!(manual_lens_settings(&control, Provider::Lens));
        assert!(!manual_lens_settings(&control, Provider::Saucenao));
        assert!(lens_manual_settings_url(&account));
        assert!(
            original_external_link(&control, Provider::Lens, account.as_str(), true, false)
                .is_none()
        );
        for value in [
            "https://accounts.google.com.evil.test/",
            "http://accounts.google.com/",
            "https://user@accounts.google.com/",
            "https://www.google.com/account",
            "https://myaccount.google.com/payments",
            "https://ageverification.google.co.kr/other",
        ] {
            assert!(!lens_manual_settings_url(&Url::parse(value).unwrap()));
        }
        assert!(original_external_link(&control, Provider::Lens, url, true, false).is_some());
        assert!(original_external_link(&control, Provider::Lens, url, false, false).is_none());
        assert!(original_external_link(&control, Provider::Lens, url, true, true).is_none());
        assert!(
            original_external_link(&control, Provider::Lens, "file:///C:/secret", true, false)
                .is_none()
        );
        assert!(original_external_link(
            &control,
            Provider::Lens,
            "https://www.google.com/search?q=filter",
            true,
            false
        )
        .is_none());
        *control.deadline.lock().unwrap() = Instant::now();
        assert!(!manual_lens_settings(&control, Provider::Lens));
        assert!(original_external_link(&control, Provider::Lens, url, true, false).is_none());
        *control.deadline.lock().unwrap() = Instant::now() + ORIGINAL_TTL;
        control.dismiss();
        assert!(!manual_lens_settings(&control, Provider::Lens));
        assert!(original_external_link(&control, Provider::Lens, url, true, false).is_none());
    }
    #[test]
    fn navigation_is_closed_and_never_admits_account_or_arbitrary_hosts() {
        for value in [
            "https://lens.google.com/v3/upload?hl=ko",
            "https://www.google.com/search?q=test",
            "https://www.google.com/sorry/index",
            "https://consent.google.com/m?continue=https%3A%2F%2Fwww.google.com%2Fsearch",
        ] {
            assert!(navigation(Provider::Lens, &Url::parse(value).unwrap()));
        }
        for value in [
            "https://accounts.google.com/",
            "https://www.google.com.evil.test/search",
            "http://www.google.com/search",
            "https://user@www.google.com/search",
            "https://www.google.com:444/search",
            "https://x.com/home",
            "https://www.google.com/url?q=https://example.org",
        ] {
            assert!(!navigation(Provider::Lens, &Url::parse(value).unwrap()));
        }
    }
    #[test]
    fn forms_are_fixed_public_destinations_without_source_account_state() {
        let script = submission(&ProviderInput::ImageUrl(
            "https://pbs.twimg.com/media/fixture.jpg".into(),
        ))
        .unwrap();
        assert!(script.contains("https://saucenao.com/search.php"));
        assert!(script.contains("HTMLFormElement.prototype.submit.call(form)"));
        for denied in [
            "fetch(",
            "XMLHttpRequest",
            "document.cookie",
            "localStorage",
            "window.open",
        ] {
            assert!(!script.contains(denied));
        }
        for url in [
            "file:///C:/secret",
            "https://x.com/home",
            "https://user@pbs.twimg.com/media/a.jpg",
        ] {
            assert!(submission(&ProviderInput::ImageUrl(url.into())).is_err());
        }
    }
    #[test]
    fn result_urls_and_text_remain_bounded_plain_data() {
        let make = |url: &str| RawCandidate {
            title: "Observed title".into(),
            thumbnail_source: None,
            source_metadata: Vec::new(),
            creator: None,
            similarity: None,
            links: vec![RawLink {
                label: "Source".into(),
                url: url.into(),
            }],
        };
        assert!(candidates(vec![make("https://example.org/art")]).is_ok());
        for url in [
            "javascript:alert(1)",
            "file:///C:/secret",
            "https://user:password@example.org/",
            "https://example.org/\nnext",
        ] {
            assert!(candidates(vec![make(url)]).is_err());
        }
        let mut value = make("https://example.org");
        value.similarity = Some(f64::NAN);
        assert!(candidates(vec![value]).is_err());
        assert_eq!(state_error("unrecognized"), UNKNOWN);
        // This shared failure can occur after the one native form submission.
        assert!(!ISOLATION.contains("전송하지 않았습니다"));
        assert!(text_valid(&"가".repeat(512), 512));
        assert!(text_valid(&"😀".repeat(256), 512));
        assert!(!text_valid(&"😀".repeat(257), 512));
    }
    #[test]
    fn missing_result_title_stays_empty_without_weakening_other_text_validation() {
        let make = || RawCandidate {
            title: String::new(),
            thumbnail_source: None,
            source_metadata: Vec::new(),
            creator: Some("Fixture Artist".into()),
            similarity: Some(91.25),
            links: vec![RawLink {
                label: "example.org".into(),
                url: "https://example.org/source".into(),
            }],
        };
        let output = candidates(vec![make()]).unwrap();
        assert_eq!(output[0].title, "");
        assert_eq!(output[0].creator.as_deref(), Some("Fixture Artist"));
        for title in [
            " ".into(),
            "\n".into(),
            "name\0".into(),
            "a".repeat(513),
            "😀".repeat(257),
        ] {
            let mut value = make();
            value.title = title;
            assert!(candidates(vec![value]).is_err());
        }
        let mut value = make();
        value.creator = Some(String::new());
        assert!(candidates(vec![value]).is_err());
        let mut value = make();
        value.links[0].label.clear();
        assert!(candidates(vec![value]).is_err());
        let mut value = make();
        value.links.clear();
        assert!(candidates(vec![value]).is_err());
    }
    #[test]
    fn persistent_profiles_are_provider_only_and_hold_exclusive_local_lease() {
        let root = tempfile::tempdir().unwrap();
        let lens = profile_directory(root.path(), Provider::Lens);
        let sauce = profile_directory(root.path(), Provider::Saucenao);
        assert_ne!(lens, sauce);
        assert_eq!(lens, root.path().join("x-media-search-profiles-v1/lens"));
        assert_eq!(
            sauce,
            root.path().join("x-media-search-profiles-v1/saucenao")
        );
        std::fs::create_dir_all(&lens).unwrap();
        let lease = lock_profile(&lens).unwrap();
        assert!(lock_profile(&lens).is_err());
        drop(lease);
        assert!(lock_profile(&lens).is_ok());
        assert!(
            lens.is_dir(),
            "normal lease cleanup preserves provider session files"
        );
    }
    #[test]
    fn repeated_handoffs_cannot_extend_automatic_or_human_budget() {
        let mut budget = Budget::new();
        budget.charge_automatic(Duration::from_secs(7)).unwrap();
        budget.charge_human(Duration::from_secs(290)).unwrap();
        budget.charge_automatic(Duration::from_secs(8)).unwrap();
        budget.charge_human(Duration::from_secs(290)).unwrap();
        assert_eq!(budget.automatic, Duration::from_secs(15));
        assert_eq!(budget.human, Duration::from_secs(20));
        assert_eq!(
            budget.charge_human(Duration::from_secs(20)).unwrap_err(),
            HUMAN_TIMEOUT
        );
        assert_eq!(
            budget
                .charge_automatic(Duration::from_secs(15))
                .unwrap_err(),
            TIMEOUT
        );
    }
    #[test]
    fn human_wait_requires_explicit_resume_and_cancellation_wins() {
        let resume = AtomicBool::new(true);
        let cancel = AtomicBool::new(false);
        wait_for_human(&resume, &cancel, Instant::now() + Duration::from_secs(1)).unwrap();
        assert!(!resume.load(Ordering::Acquire), "resume is one-shot");
        assert_eq!(
            wait_for_human(&resume, &cancel, Instant::now()).unwrap_err(),
            HUMAN_TIMEOUT
        );
        resume.store(true, Ordering::Release);
        cancel.store(true, Ordering::Release);
        assert_eq!(
            wait_for_human(&resume, &cancel, Instant::now() + Duration::from_secs(1)).unwrap_err(),
            CANCELLED
        );
        assert!(
            resume.load(Ordering::Acquire),
            "cancellation does not authorize continuing"
        );
    }
    #[test]
    fn synthetic_saucenao_form_is_separate_from_production_input_mapping() {
        let input = ProviderInput::Jpeg(vec![0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(provider(&input), Provider::Lens);
        let script = submission_for(&input, Provider::Saucenao).unwrap();
        assert!(script.contains("\"provider\":\"saucenao\""));
        assert!(script.contains("?'file':'encoded_image'"));
        assert!(submission_for(
            &ProviderInput::ImageUrl("https://pbs.twimg.com/media/a.jpg".into()),
            Provider::Lens
        )
        .is_err());
    }
    #[test]
    fn handoff_wait_does_not_access_browser_or_resubmit_payload() {
        let source = include_str!("x_media_search_provider.rs");
        let wait = source
            .split("fn wait_for_human(")
            .nth(1)
            .unwrap()
            .split("#[derive(Deserialize)]")
            .next()
            .unwrap();
        for forbidden in [
            ".eval(",
            ".eval_with_callback(",
            ".url(",
            "submission(",
            ".navigate(",
        ] {
            assert!(!wait.contains(forbidden));
        }
        let implementation = source
            .split("fn run_inner(")
            .nth(1)
            .unwrap()
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert_eq!(implementation.matches("dispatch_submission(").count(), 1);
        assert!(!implementation.contains(".incognito(true)"));
    }
    #[test]
    fn provider_capabilities_are_independent_and_production_flags_remain_explicit() {
        assert_eq!(image_available(), SAUCENAO_LAYOUT_VERIFIED);
        assert_eq!(video_available(), LENS_LAYOUT_VERIFIED);
        assert_eq!(available(), image_available() || video_available());
    }
    #[test]
    fn gui_accesses_use_cancelled_bounded_channels_without_worker_getters() {
        let source = include_str!("x_media_search_provider.rs");
        let implementation = source
            .split("fn run_inner(")
            .nth(1)
            .unwrap()
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!implementation.contains("view.url()"));
        assert!(!implementation.contains("view.eval_with_callback"));
        assert!(implementation.contains("read_url("));
        assert!(implementation.contains("evaluate("));
    }
}
