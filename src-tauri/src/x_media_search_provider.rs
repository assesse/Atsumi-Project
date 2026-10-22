//! Consented public-form lookup with isolated provider-owned persistent profiles.
//! No X profile, credential copying, clearance-token extraction or provider IPC.
//! Called only after the controller admits the frozen, bounded selection.
use super::{Candidate, ProviderInput, ProviderRequest, SearchLink};
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
const MAX_REPLY: usize = 64 * 1024;
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
// This reservation outlives a timed-out controller operation. Only a native
// close acknowledged on the UI thread may permit another provider worker.
static CHILD_RESERVED: AtomicBool = AtomicBool::new(false);
static PROFILE_LOCK: Mutex<Option<std::fs::File>> = Mutex::new(None);
pub(super) fn busy() -> bool {
    CHILD_RESERVED.load(Ordering::Acquire)
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
        ProviderInput::ImageUrl(_) => Provider::Saucenao,
        ProviderInput::Jpeg(_) => Provider::Lens,
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
            Some("www.google.com") => url.path() == "/search" || url.path().starts_with("/sorry/"),
            // The provider's own consent document is exposed for the user,
            // never clicked or accepted by automation. Account sign-in remains denied.
            Some("consent.google.com") => matches!(url.path(), "/m" | "/ml" | "/d" | "/dl"),
            _ => false,
        },
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
        ProviderInput::Jpeg(bytes) => {
            if bytes.len() > 4 * 1024 * 1024 || bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8])
            {
                return Err(INVALID.into());
            }
            json!({"kind":"file", "value":STANDARD.encode(bytes),"provider":service.name()})
        }
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
#[derive(Default)]
struct Cleanup {
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
            PROFILE_LOCK
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            CHILD_RESERVED.store(false, Ordering::Release);
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
#[serde(deny_unknown_fields)]
struct Reply {
    nonce: String,
    state: String,
    candidates: Vec<RawCandidate>,
    #[serde(default)]
    probe: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCandidate {
    title: String,
    creator: Option<String>,
    similarity: Option<f64>,
    links: Vec<RawLink>,
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
fn candidates(values: Vec<RawCandidate>) -> Result<Vec<Candidate>, String> {
    if values.len() > 10 {
        return Err(UNKNOWN.into());
    }
    values
        .into_iter()
        .map(|value| {
            if !text_valid(&value.title, 512)
                || value.creator.as_ref().is_some_and(|v| !text_valid(v, 256))
                || value
                    .similarity
                    .is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v))
                || value.links.is_empty()
                || value.links.len() > 3
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
        ProviderInput::ImageUrl(_) => SAUCENAO_LAYOUT_VERIFIED,
        ProviderInput::Jpeg(_) => LENS_LAYOUT_VERIFIED,
    };
    if !verified {
        return Err(
            "검색 서비스의 자동 연결을 아직 검증하지 못했습니다. 자료를 전송하지 않았습니다."
                .into(),
        );
    }
    run_inner(app, request, cancel, resume, on_handoff, None, None)
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
    // Only the opt-in fixed-hash synthetic example calls this path. Production
    // derives its provider solely from the closed image/frame input pair.
    run_inner(
        app,
        request,
        cancel,
        resume,
        on_handoff,
        Some(service),
        Some(evidence),
    )
}

fn run_inner(
    app: AppHandle,
    request: ProviderRequest,
    cancel: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
    on_handoff: Arc<dyn Fn(String) + Send + Sync>,
    probe_service: Option<Provider>,
    evidence: Option<Arc<Mutex<Option<Value>>>>,
) -> Result<Vec<Candidate>, String> {
    let trace = Arc::new(ProbeTrace::new(evidence.is_some()));
    trace.phase("started");
    let mut budget = Budget::new();
    let mut automatic_started = Instant::now();
    let mut deadline = automatic_started + budget.automatic;
    let overall_deadline = automatic_started + DEADLINE + HUMAN_DEADLINE;
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
    let directory = profile_directory(
        &app.path().app_local_data_dir().map_err(|_| ISOLATION)?,
        service,
    );
    std::fs::create_dir_all(&directory).map_err(|_| ISOLATION)?;
    let profile_lock = lock_profile(&directory)?;
    let label = format!("x-media-search-{}", uuid::Uuid::new_v4().simple());
    if CHILD_RESERVED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(CLEANUP.into());
    }
    *PROFILE_LOCK.lock().unwrap_or_else(|p| p.into_inner()) = Some(profile_lock);
    let live = Arc::new(AtomicBool::new(true));
    let cleanup = Arc::new(Cleanup::default());
    let child = Child {
        app: app.clone(),
        label: label.clone(),
        live: live.clone(),
        cleanup: cleanup.clone(),
        close_requested: AtomicBool::new(false),
    };
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
                parent.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        close_trace.phase("provider_close_requested");
                        close_cancel.store(true, Ordering::Release);
                    }
                    tauri::WindowEvent::Destroyed if close_live.load(Ordering::Acquire) => {
                        close_cancel.store(true, Ordering::Release);
                    }
                    _ => {}
                });
                let nav_live = wanted.clone();
                let nav_cancel = stop.clone();
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
                        || nav_cancel.load(Ordering::Acquire)
                        || Instant::now() >= overall_deadline
                    {
                        return false;
                    }
                    if navigation(service, url) {
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
        let config = json!({"nonce":nonce,"provider":service.name(),"probe":evidence.is_some(),"lensVerified":LENS_LAYOUT_VERIFIED,"saucenaoVerified":SAUCENAO_LAYOUT_VERIFIED});
        let script = include_str!("x_media_search_extract.js")
            .replace("__ATSUMI_MEDIA_SEARCH__", &config.to_string());
        let mut last_unknown = false;
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
            let reply = if before.host_str() == Some("consent.google.com") {
                // Do not inject a result extractor into a service-consent page.
                Reply {
                    nonce: nonce.clone(),
                    state: "consent".into(),
                    candidates: vec![],
                    probe: None,
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
                        512 * 1024
                    } else {
                        MAX_REPLY
                    }
                {
                    return Err(UNKNOWN.into());
                }
                serde_json::from_str::<Reply>(&raw).map_err(|_| UNKNOWN)?
            };
            if reply.nonce != nonce {
                return Err(UNKNOWN.into());
            }
            if let (Some(sink), Some(probe)) = (&evidence, reply.probe) {
                *sink.lock().unwrap_or_else(|p| p.into_inner()) = Some(
                    json!({"provider":service.name(),"origin":before.origin().ascii_serialization(),"path":before.path(),"state":reply.state,"document":probe}),
                );
            }
            match reply.state.as_str() {
                "complete" if !reply.candidates.is_empty() => return candidates(reply.candidates),
                "empty" if reply.candidates.is_empty() => return Ok(Vec::new()),
                "pending" => {}
                "unknown" => {
                    last_unknown = true;
                }
                "challenge" | "consent" => {
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
