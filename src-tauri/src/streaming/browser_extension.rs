//! Loads only an already-installed NAVER extension into the official window's
//! isolated WebView2 profile. No browser cookies/profile data are copied, and no
//! installer, network request, registry edit, or extension-source write is used.
//!
//! AddBrowserExtension retains a reference to the existing extension directory:
//! browser updates/removal can invalidate it. A successful callback establishes
//! extension loading only, not Native Messaging or NAVER Grid compatibility.
//! The verified extension identity is aligned with this WebView's User Agent
//! before reporting success, so the official page addresses the loaded product.

use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::model::StreamError;

pub const CHROME_EXTENSION_ID: &str = "ooadnieabchijkibjpeieeliohjidnjj";
pub const EDGE_EXTENSION_ID: &str = "jedbgfnhnpbfcbplibkacnmiafbojobk";
const EXTENSION_IDS: [&str; 2] = [CHROME_EXTENSION_ID, EDGE_EXTENSION_ID];
const MAX_DIRECTORY_ENTRIES: usize = 512;
const MAX_PROFILES: usize = 64;
const MAX_VERSIONS: usize = 64;
const MAX_CANDIDATES: usize = 256;
const MAX_TREE_ENTRIES: usize = 4096;
const MAX_TREE_DEPTH: usize = 16;
const MAX_TREE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST: usize = 256 * 1024;
const MAX_KEY: usize = 16 * 1024;
const CHOICE_FILE: &str = "chzzk-extension-choice.json";

// App-owned, non-sensitive opt-in only. Never persist an extension source path,
// browser identity/version, account, cookie, or a claim that playback works.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReconnectChoice {
    version: u8,
    auto_connect: bool,
}

fn choice_error() -> StreamError {
    StreamError::new("BROWSER_EXTENSION_CHOICE_FAILED", "확장은 연결됐지만 다음 실행의 자동 연결 선택을 저장하지 못했습니다. 다음 실행에서는 네이버 확장 연결을 다시 눌러 주세요.", true)
}

pub(crate) fn reconnect_choice(data_dir: &Path) -> Result<bool, StreamError> {
    let root = checked_directory(data_dir).map_err(|_| choice_error())?;
    let path = root.join(CHOICE_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        result => result.map_err(|_| choice_error())?,
    };
    if !metadata.is_file() || is_reparse(&metadata) || metadata.len() > 256 {
        return Err(choice_error());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| choice_error())?
        .take(257)
        .read_to_end(&mut bytes)
        .map_err(|_| choice_error())?;
    if bytes.len() > 256 {
        return Err(choice_error());
    }
    let choice: ReconnectChoice = serde_json::from_slice(&bytes).map_err(|_| choice_error())?;
    if choice.version != 1 {
        return Err(choice_error());
    }
    Ok(choice.auto_connect)
}

pub(crate) fn remember_connection(data_dir: &Path) -> Result<(), StreamError> {
    // An existing malformed/unrecognized file is preserved, not overwritten.
    if reconnect_choice(data_dir)? {
        return Ok(());
    }
    let root = checked_directory(data_dir).map_err(|_| choice_error())?;
    let target = root.join(CHOICE_FILE);
    let temporary = root.join(format!(
        ".chzzk-extension-choice-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| choice_error())?;
        file.write_all(br#"{"version":1,"autoConnect":true}"#)
            .map_err(|_| choice_error())?;
        file.sync_all().map_err(|_| choice_error())?;
        drop(file);
        // Revalidate the exact target before an atomic replacement. Only our
        // recognized small setting can be replaced; no profile data is opened.
        reconnect_choice(data_dir)?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                core::PCWSTR,
                Win32::Storage::FileSystem::{
                    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
                },
            };
            let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let destination: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(destination.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(|_| choice_error())?;
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, &target).map_err(|_| choice_error())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionLoadReport {
    pub loaded_ids: Vec<String>,
    pub reload_required: bool,
}

/// An official `{svc:"install"}` reply proves page-to-extension messaging only.
/// It does not prove Native Messaging, connector readiness, or high-quality playback.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionDetectionReport {
    pub detected_ids: Vec<String>,
    pub page_api_available: bool,
    pub timed_out: bool,
}

#[derive(Debug)]
struct InstalledExtension {
    id: &'static str,
    directory: PathBuf,
}

fn identity_error() -> StreamError {
    StreamError::new("BROWSER_EXTENSION_IDENTITY_FAILED", "확장 파일은 로드됐지만 공식 페이지의 브라우저 식별을 안전하게 맞추지 못했습니다. 시청 영역을 다시 열어 주세요.", true)
}

// Primary source, inspected 2026-09-11:
// https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/player-vendor-BYg0wCyN.js
// K.plugout.browser.isNewEdge() checks UA.toLowerCase().indexOf("edg") > -1;
// K.extension.EXT_ID then selects Edge's jedbg... rather than Chrome's ooad....
// Both installation and playback sendMessage calls use that selected ID.
// Keep the real runtime/version; never manufacture a Chrome/Edge version.
pub(crate) fn aligned_user_agent(
    native: &str,
    loaded_ids: &[String],
) -> Result<String, StreamError> {
    let ids = checked_probe_ids(loaded_ids).map_err(|_| identity_error())?;
    if native.is_empty()
        || native.len() > 1024
        || !native.is_ascii()
        || native.bytes().any(|b| b.is_ascii_control())
        || !native.starts_with("Mozilla/5.0 ")
        || !native.contains("AppleWebKit/")
        || !native.contains("Safari/")
    {
        return Err(identity_error());
    }
    let parts: Vec<&str> = native.split_ascii_whitespace().collect();
    let valid_version =
        |version: &str| version.split('.').count() == 4 && numeric_version(version).is_some();
    let chrome: Vec<_> = parts
        .iter()
        .filter_map(|part| part.strip_prefix("Chrome/"))
        .collect();
    let edge: Vec<_> = parts
        .iter()
        .filter_map(|part| part.strip_prefix("Edg/"))
        .collect();
    if chrome.len() != 1
        || !valid_version(chrome[0])
        || edge.len() > 1
        || edge.first().is_some_and(|version| !valid_version(version))
    {
        return Err(identity_error());
    }
    let chrome_identity = parts
        .iter()
        .filter(|part| !part.starts_with("Edg/"))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let lower = chrome_identity.to_ascii_lowercase();
    if ["edg", "whale", "opera", "opr/", "firefox"]
        .iter()
        .any(|token| lower.contains(token))
    {
        return Err(identity_error());
    }
    if ids.iter().any(|id| id == EDGE_EXTENSION_ID) {
        // Native WebView2 is Edge. An unknown/previously replaced origin cannot
        // safely supply its original Edge version: fail instead of inventing it.
        if edge.len() != 1 {
            return Err(identity_error());
        }
        Ok(native.to_string())
    } else {
        Ok(chrome_identity)
    }
}

// Only this module changes the official WebView identity. Retain its first
// validated native UA in RAM so a later Chrome-only -> Edge reconnect restores
// the true value. SetUserAgent("") is explicitly a no-op in WebView2, not reset.
#[derive(Default)]
struct NativeUserAgents(std::collections::VecDeque<(usize, String)>);
impl NativeUserAgents {
    fn selected(
        &mut self,
        identity: usize,
        current: &str,
        ids: &[String],
    ) -> Result<String, StreamError> {
        if let Some((_, original)) = self.0.iter_mut().find(|(id, _)| *id == identity) {
            let prior_chrome = aligned_user_agent(original, &[CHROME_EXTENSION_ID.into()])?;
            if current == original || current == prior_chrome {
                return aligned_user_agent(original, ids);
            }
            // COM addresses can be reused after a view closes. A newly observed
            // native Edge UA replaces the stale entry; unexpected overrides
            // fail closed rather than reapplying an old browser version.
            let selected = aligned_user_agent(current, ids)?;
            if !current
                .split_ascii_whitespace()
                .any(|part| part.starts_with("Edg/"))
            {
                return Err(identity_error());
            }
            *original = current.to_string();
            return Ok(selected);
        }
        let selected = aligned_user_agent(current, ids)?;
        if self.0.len() == 8 {
            self.0.pop_front();
        }
        self.0.push_back((identity, current.to_string()));
        Ok(selected)
    }
}

fn checked_probe_ids(ids: &[String]) -> Result<Vec<String>, StreamError> {
    if ids.is_empty()
        || ids.len() > EXTENSION_IDS.len()
        || ids.iter().any(|id| !EXTENSION_IDS.contains(&id.as_str()))
    {
        return Err(invalid());
    }
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn detection_script(ids: &[String]) -> Result<String, StreamError> {
    let ids = checked_probe_ids(ids)?;
    let ids = serde_json::to_string(&ids).map_err(|_| invalid())?;
    Ok(r#"(() => new Promise(resolve => {
      const ids=__IDS__;
      const report={detectedIds:[],pageApiAvailable:false,timedOut:false};
      const runtime=window.chrome && window.chrome.runtime;
      if(!runtime || typeof runtime.sendMessage!=='function'){resolve(report);return;}
      report.pageApiAvailable=true;
      let remaining=ids.length,finished=false;
      const finish=()=>{if(!finished){finished=true;clearTimeout(timer);report.detectedIds.sort();resolve(report);}};
      const timer=setTimeout(()=>{report.timedOut=true;finish();},2500);
      const done=()=>{remaining--;if(remaining===0)finish();};
      for(const id of ids){
        try{runtime.sendMessage(id,{svc:'install'},response=>{
          const failed=!!runtime.lastError;
          if(finished)return;
          if(!failed && response==='install')report.detectedIds.push(id);
          done();
        });}catch{done();}
      }
    }))()"#.replace("__IDS__", &ids))
}

fn parse_detection_response(
    body: &str,
    expected_ids: &[String],
) -> Result<ExtensionDetectionReport, StreamError> {
    if body.len() > 16 * 1024 {
        return Err(probe_error());
    }
    let result: Value = serde_json::from_str(body).map_err(|_| probe_error())?;
    if result.get("exceptionDetails").is_some() {
        return Err(probe_error());
    }
    let value = result
        .get("result")
        .and_then(|value| value.get("value"))
        .ok_or_else(probe_error)?;
    let detected = value
        .get("detectedIds")
        .and_then(Value::as_array)
        .ok_or_else(probe_error)?;
    if detected.len() > EXTENSION_IDS.len() {
        return Err(probe_error());
    }
    let mut detected_ids = Vec::new();
    for id in detected {
        let id = id.as_str().ok_or_else(probe_error)?;
        if !EXTENSION_IDS.contains(&id)
            || !expected_ids.iter().any(|expected| expected == id)
            || detected_ids.iter().any(|seen| seen == id)
        {
            return Err(probe_error());
        }
        detected_ids.push(id.to_string());
    }
    let page_api_available = value
        .get("pageApiAvailable")
        .and_then(Value::as_bool)
        .ok_or_else(probe_error)?;
    let timed_out = value
        .get("timedOut")
        .and_then(Value::as_bool)
        .ok_or_else(probe_error)?;
    if !page_api_available && (!detected_ids.is_empty() || timed_out) {
        return Err(probe_error());
    }
    detected_ids.sort();
    Ok(ExtensionDetectionReport {
        detected_ids,
        page_api_available,
        timed_out,
    })
}

/// Probe only the official extension's inert installation check on the current
/// CHZZK document. This does not launch the native connector or start playback.
#[cfg(windows)]
pub fn probe(
    view: &tauri::Webview,
    loaded_ids: &[String],
    on_complete: impl FnOnce(Result<ExtensionDetectionReport, StreamError>) + Send + 'static,
) -> Result<(), StreamError> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;

    static RUNNING: AtomicBool = AtomicBool::new(false);
    let url = view.url().map_err(|_| probe_error())?;
    if view.label() != super::browser::WINDOW_LABEL
        || url.scheme() != "https"
        || url.host_str() != Some("chzzk.naver.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(probe_error());
    }
    let ids = checked_probe_ids(loaded_ids)?;
    let script = detection_script(&ids)?;
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err(StreamError::new(
            "BROWSER_EXTENSION_PROBE_BUSY",
            "확장 응답을 확인 중입니다.",
            true,
        ));
    }
    let completion: Completion<ExtensionDetectionReport> =
        Arc::new(Mutex::new(Some(Box::new(move |result| {
            RUNNING.store(false, Ordering::Release);
            on_complete(result);
        }))));
    let timeout = completion.clone();
    if std::thread::Builder::new()
        .name("chzzk-extension-probe-deadline".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            complete(&timeout, Err(probe_error()));
        })
        .is_err()
    {
        RUNNING.store(false, Ordering::Release);
        return Err(probe_error());
    }
    let dispatched = completion.clone();
    let result = view.with_webview(move |platform| unsafe {
        let outcome = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let callback = dispatched.clone();
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |status, json| {
                    let result = status
                        .map_err(|_| probe_error())
                        .and_then(|_| parse_detection_response(&json, &ids));
                    complete(&callback, result);
                    Ok(())
                },
            ));
            let params =
                serde_json::json!({"expression":script,"awaitPromise":true,"returnByValue":true})
                    .to_string();
            core.CallDevToolsProtocolMethod(
                &HSTRING::from("Runtime.evaluate"),
                &HSTRING::from(params),
                &handler,
            )
        })();
        if outcome.is_err() {
            complete(&dispatched, Err(probe_error()));
        }
    });
    if result.is_err() {
        complete(&completion, Err(probe_error()));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn probe(
    _: &tauri::Webview,
    _: &[String],
    _: impl FnOnce(Result<ExtensionDetectionReport, StreamError>) + Send + 'static,
) -> Result<(), StreamError> {
    Err(probe_error())
}

fn probe_error() -> StreamError {
    StreamError::new("BROWSER_EXTENSION_PROBE_FAILED", "현재 공식 페이지에서 네이버 확장 응답을 확인하지 못했습니다. 확장 로드와 커넥터 연결은 별도로 확인해야 합니다.", true)
}

/// Schedules bounded discovery off the UI thread, then uses the WebView2 UI
/// thread for COM. Immediate Err means the callback was not scheduled. After Ok,
/// discovery, dispatch, and COM completion errors are sent to `on_complete`.
/// The caller must require the main-window command and prohibit active capture.
#[cfg(windows)]
pub fn connect(
    window: &tauri::Webview,
    on_complete: impl FnOnce(Result<ExtensionLoadReport, StreamError>) + Send + 'static,
) -> Result<(), StreamError> {
    use std::sync::{Arc, Mutex};
    if window.label() != super::browser::WINDOW_LABEL {
        return Err(load_error());
    }
    let completion: Completion<ExtensionLoadReport> =
        Arc::new(Mutex::new(Some(Box::new(on_complete))));
    let window = window.clone();
    std::thread::Builder::new()
        .name("chzzk-extension-connect".into())
        .spawn(move || {
            let result = std::env::var_os("LOCALAPPDATA")
                .ok_or_else(not_found)
                .and_then(|root| discover(&PathBuf::from(root)));
            match result {
                Ok(extensions) => add_to_profile(&window, extensions, completion),
                Err(cause) => complete(&completion, Err(cause)),
            }
        })
        .map_err(|_| load_error())?;
    Ok(())
}

#[cfg(not(windows))]
pub fn connect(
    _: &tauri::Webview,
    _: impl FnOnce(Result<ExtensionLoadReport, StreamError>) + Send + 'static,
) -> Result<(), StreamError> {
    Err(StreamError::new(
        "BROWSER_EXTENSION_UNSUPPORTED",
        "네이버 확장 연결은 Windows WebView2 환경에서 지원합니다.",
        false,
    ))
}

#[cfg(windows)]
type Completion<T> = std::sync::Arc<
    std::sync::Mutex<Option<Box<dyn FnOnce(Result<T, StreamError>) + Send + 'static>>>,
>;

#[cfg(windows)]
fn complete<T>(completion: &Completion<T>, result: Result<T, StreamError>) {
    let callback = completion
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(callback) = callback {
        callback(result);
    }
}

#[cfg(windows)]
fn deferred_completion<T: Send + 'static>(
    completion: Completion<T>,
    post: impl FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), ()> + Send + 'static,
) -> Result<std::sync::mpsc::SyncSender<Result<T, StreamError>>, StreamError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("chzzk-extension-identity-completion".into())
        .spawn(move || {
            let result = receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap_or_else(|_| Err(identity_error()));
            let queued = completion.clone();
            if post(Box::new(move || complete(&queued, result))).is_err() {
                complete(&completion, Err(identity_error()));
            }
        })
        .map_err(|_| identity_error())?;
    Ok(sender)
}

#[cfg(windows)]
fn align_and_complete(
    view: &tauri::Webview,
    report: ExtensionLoadReport,
    completion: Completion<ExtensionLoadReport>,
) {
    use tauri::Manager;
    use webview2_com::{CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2};
    use windows::core::{Interface, HSTRING, PWSTR};
    static ORIGINALS: std::sync::OnceLock<std::sync::Mutex<NativeUserAgents>> =
        std::sync::OnceLock::new();
    if view.label() != super::browser::WINDOW_LABEL {
        complete(&completion, Err(identity_error()));
        return;
    }
    let app = view.app_handle().clone();
    let sender = match deferred_completion(completion.clone(), move |task| {
        app.run_on_main_thread(task).map_err(|_| ())
    }) {
        Ok(sender) => sender,
        Err(error) => {
            complete(&completion, Err(error));
            return;
        }
    };
    let failure = sender.clone();
    let result = view.with_webview(move |platform| unsafe {
        let result = (|| -> Result<ExtensionLoadReport, StreamError> {
            let core = platform
                .controller()
                .CoreWebView2()
                .map_err(|_| identity_error())?;
            let settings = core
                .Settings()
                .and_then(|settings| settings.cast::<ICoreWebView2Settings2>())
                .map_err(|_| identity_error())?;
            let mut raw = PWSTR::null();
            settings.UserAgent(&mut raw).map_err(|_| identity_error())?;
            let current = CoTaskMemPWSTR::from(raw).to_string();
            let identity = core
                .cast::<windows::core::IUnknown>()
                .map_err(|_| identity_error())?
                .as_raw() as usize;
            let selected = ORIGINALS
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| identity_error())?
                .selected(identity, &current, &report.loaded_ids)?;
            if current != selected {
                settings
                    .SetUserAgent(&HSTRING::from(&selected))
                    .map_err(|_| identity_error())?;
            }
            let mut raw = PWSTR::null();
            settings.UserAgent(&mut raw).map_err(|_| identity_error())?;
            if CoTaskMemPWSTR::from(raw).to_string() != selected {
                return Err(identity_error());
            }
            Ok(report)
        })();
        // Never invoke the caller here: Tauri's synchronous UI dispatcher holds
        // its window_id mutex throughout with_webview. A caller's reload/url
        // would re-enter that mutex and deadlock. Send only the native result;
        // the worker posts a separate UI task after this dispatcher unwinds.
        // Setter/readback still finish before any success callback or reload.
        let _ = sender.try_send(result);
    });
    if result.is_err() {
        let _ = failure.try_send(Err(identity_error()));
    }
}

#[cfg(windows)]
fn add_to_profile(
    window: &tauri::Webview,
    extensions: Vec<InstalledExtension>,
    completion: Completion<ExtensionLoadReport>,
) {
    use std::os::windows::ffi::OsStrExt;
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile7, ICoreWebView2_13},
        ProfileAddBrowserExtensionCompletedHandler,
    };
    use windows::core::{Interface, PCWSTR, PWSTR};

    struct Pending {
        remaining: usize,
        loaded: Vec<String>,
    }
    let pending = std::sync::Arc::new(std::sync::Mutex::new(Pending {
        remaining: extensions.len(),
        loaded: Vec::new(),
    }));
    let identity_view = window.clone();
    let finish_one = std::sync::Arc::new(move |id: Option<String>| {
        let report = {
            let mut state = pending.lock().unwrap_or_else(|p| p.into_inner());
            if state.remaining == 0 {
                return;
            }
            if let Some(id) = id {
                state.loaded.push(id);
            }
            state.remaining -= 1;
            if state.remaining == 0 {
                state.loaded.sort();
                Some(if state.loaded.is_empty() {
                    Err(load_error())
                } else {
                    Ok(ExtensionLoadReport {
                        loaded_ids: state.loaded.clone(),
                        reload_required: true,
                    })
                })
            } else {
                None
            }
        };
        if let Some(report) = report {
            match report {
                Ok(report) => align_and_complete(&identity_view, report, completion.clone()),
                Err(error) => complete(&completion, Err(error)),
            }
        }
    });
    let fallback = finish_one.clone();
    let count = extensions.len();
    let result = window.with_webview(move |platform| {
        let outcome = (|| -> windows::core::Result<()> {
            // COM objects remain on the WebView2 thread. The only cross-thread
            // state is the Send callback and a validated filesystem path.
            unsafe {
                let core = platform.controller().CoreWebView2()?;
                let profile: ICoreWebView2Profile7 =
                    core.cast::<ICoreWebView2_13>()?.Profile()?.cast()?;
                for extension in extensions {
                    let expected_id = extension.id;
                    let callback = finish_one.clone();
                    let handler = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(
                        move |status, extension| {
                            let result = (|| -> Result<String, StreamError> {
                                status.map_err(|_| load_error())?;
                                let extension = extension.ok_or_else(load_error)?;
                                let mut id = PWSTR::null();
                                extension.Id(&mut id).map_err(|_| load_error())?;
                                if CoTaskMemPWSTR::from(id).to_string() != expected_id {
                                    return Err(invalid());
                                }
                                let mut enabled = windows::core::BOOL::default();
                                extension
                                    .IsEnabled(&mut enabled)
                                    .map_err(|_| load_error())?;
                                if !enabled.as_bool() {
                                    return Err(load_error());
                                }
                                Ok(expected_id.to_string())
                            })();
                            callback(result.ok());
                            Ok(())
                        },
                    ));
                    let path: Vec<u16> = extension
                        .directory
                        .as_os_str()
                        .encode_wide()
                        .chain(Some(0))
                        .collect();
                    if profile
                        .AddBrowserExtension(PCWSTR(path.as_ptr()), &handler)
                        .is_err()
                    {
                        finish_one(None);
                    }
                }
                Ok(())
            }
        })();
        if outcome.is_err() {
            for _ in 0..count {
                finish_one(None);
            }
        }
    });
    if result.is_err() {
        for _ in 0..count {
            fallback(None);
        }
    }
}

/// Only Default/Profile n directories and the two official extension IDs
/// are examined. Other profile files (including Preferences/Login Data/Cookies)
/// are never opened. The newest valid version of each installed identity wins.
fn discover(local_app_data: &Path) -> Result<Vec<InstalledExtension>, StreamError> {
    let local_app_data = checked_directory(local_app_data)?;
    let mut candidates = Vec::<(&'static str, [u32; 4], PathBuf)>::new();
    let mut found_installation = false;
    for browser in [
        ["Google", "Chrome", "User Data"],
        ["Microsoft", "Edge", "User Data"],
    ] {
        let user_data = browser
            .iter()
            .fold(local_app_data.clone(), |path, name| path.join(name));
        let Ok(user_data) = checked_directory(&user_data) else {
            continue;
        };
        if !user_data.starts_with(&local_app_data) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&user_data) else {
            continue;
        };
        let mut profiles = 0;
        for entry in entries.take(MAX_DIRECTORY_ENTRIES).flatten() {
            let name = entry.file_name();
            if !name.to_str().is_some_and(valid_profile) {
                continue;
            }
            profiles += 1;
            if profiles > MAX_PROFILES {
                break;
            }
            for extension_id in EXTENSION_IDS {
                let installed = entry.path().join("Extensions").join(extension_id);
                if fs::symlink_metadata(&installed).is_err() {
                    continue;
                }
                found_installation = true;
                let Ok(installed) = checked_directory(&installed) else {
                    continue;
                };
                if !installed.starts_with(&user_data) {
                    continue;
                }
                let Ok(versions) = fs::read_dir(installed) else {
                    continue;
                };
                for version in versions.take(MAX_VERSIONS).flatten() {
                    let Some(number) = version.file_name().to_str().and_then(directory_version)
                    else {
                        continue;
                    };
                    if candidates
                        .iter()
                        .filter(|item| item.0 == extension_id)
                        .count()
                        < MAX_CANDIDATES / EXTENSION_IDS.len()
                    {
                        candidates.push((extension_id, number, version.path()));
                    }
                }
            }
        }
    }
    candidates.sort_by(|left, right| {
        left.0
            .cmp(right.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let mut installed = Vec::<InstalledExtension>::new();
    for (id, version, path) in candidates {
        if installed.iter().any(|extension| extension.id == id) {
            continue;
        }
        if let Ok(directory) = validate_extension(&local_app_data, &path, version, id) {
            installed.push(InstalledExtension { id, directory });
        }
    }
    if !installed.is_empty() {
        return Ok(installed);
    }
    Err(if found_installation {
        invalid()
    } else {
        not_found()
    })
}

fn valid_profile(name: &str) -> bool {
    name == "Default"
        || name.strip_prefix("Profile ").is_some_and(|number| {
            !number.is_empty()
                && number.len() <= 10
                && number.bytes().all(|byte| byte.is_ascii_digit())
                && number.parse::<u32>().is_ok()
        })
}

fn numeric_version(value: &str) -> Option<[u32; 4]> {
    if value.is_empty() || value.len() > 23 {
        return None;
    }
    let mut version = [0; 4];
    for (index, part) in value.split('.').enumerate() {
        if index >= 4 || part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let number = part.parse::<u32>().ok()?;
        if number > u16::MAX as u32 {
            return None;
        }
        version[index] = number;
    }
    Some(version)
}

fn directory_version(value: &str) -> Option<[u32; 4]> {
    let version = if let Some((version, installation)) = value.split_once('_') {
        if installation.is_empty()
            || installation.len() > 5
            || !installation.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        installation.parse::<u16>().ok()?;
        version
    } else {
        value
    };
    numeric_version(version)
}

fn validate_extension(
    anchor: &Path,
    path: &Path,
    version: [u32; 4],
    expected_id: &str,
) -> Result<PathBuf, StreamError> {
    if !EXTENSION_IDS.contains(&expected_id)
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some(expected_id)
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(directory_version)
            != Some(version)
    {
        return Err(invalid());
    }
    let path = checked_directory(path)?;
    if !path.starts_with(anchor) {
        return Err(invalid());
    }
    validate_tree(&path)?;
    let manifest_path = path.join("manifest.json");
    let metadata = fs::symlink_metadata(&manifest_path).map_err(|_| invalid())?;
    if !metadata.is_file() || is_reparse(&metadata) || metadata.len() > MAX_MANIFEST as u64 {
        return Err(invalid());
    }
    let mut data = Vec::new();
    File::open(&manifest_path)
        .map_err(|_| invalid())?
        .take((MAX_MANIFEST + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| invalid())?;
    if data.len() > MAX_MANIFEST {
        return Err(invalid());
    }
    let manifest: Value = serde_json::from_slice(&data).map_err(|_| invalid())?;
    validate_manifest(&manifest, version, expected_id)?;
    Ok(path)
}

fn validate_manifest(
    manifest: &Value,
    version: [u32; 4],
    expected_id: &str,
) -> Result<(), StreamError> {
    if !EXTENSION_IDS.contains(&expected_id)
        || !matches!(
            manifest.get("manifest_version").and_then(Value::as_u64),
            Some(2 | 3)
        )
        || manifest
            .get("version")
            .and_then(Value::as_str)
            .and_then(numeric_version)
            != Some(version)
    {
        return Err(invalid());
    }
    // An unpacked extension without its public key gets a path-derived ID,
    // which cannot preserve NAVER's allowlisted Native Messaging identity.
    let key = manifest
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if extension_id(key).as_deref() != Some(expected_id) {
        return Err(invalid());
    }
    Ok(())
}

fn extension_id(key: &str) -> Option<String> {
    if key.is_empty() || key.len() > MAX_KEY || !key.is_ascii() {
        return None;
    }
    let compact: String = key
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    let decoded = STANDARD.decode(compact).ok()?;
    if decoded.is_empty() {
        return None;
    }
    // Chromium components/crx_file/id_util.cc: first 16 SHA-256 bytes,
    // hexadecimal nibbles translated to the extension alphabet a..p.
    let digest = Sha256::digest(&decoded);
    let mut id = String::with_capacity(32);
    for byte in &digest[..16] {
        id.push((b'a' + (byte >> 4)) as char);
        id.push((b'a' + (byte & 15)) as char);
    }
    Some(id)
}

fn checked_directory(path: &Path) -> Result<PathBuf, StreamError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| invalid())?;
        if !metadata.is_dir() || is_reparse(&metadata) {
            return Err(invalid());
        }
    }
    fs::canonicalize(path).map_err(|_| invalid())
}

fn is_reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn validate_tree(root: &Path) -> Result<(), StreamError> {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut count = 0usize;
    let mut bytes = 0u64;
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_TREE_DEPTH {
            return Err(invalid());
        }
        for entry in fs::read_dir(&directory).map_err(|_| invalid())? {
            count += 1;
            if count > MAX_TREE_ENTRIES {
                return Err(invalid());
            }
            let path = entry.map_err(|_| invalid())?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| invalid())?;
            if is_reparse(&metadata)
                || !fs::canonicalize(&path)
                    .map_err(|_| invalid())?
                    .starts_with(root)
            {
                return Err(invalid());
            }
            if metadata.is_dir() {
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                bytes = bytes.checked_add(metadata.len()).ok_or_else(invalid)?;
                if bytes > MAX_TREE_BYTES {
                    return Err(invalid());
                }
            } else {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn not_found() -> StreamError {
    StreamError::new("BROWSER_EXTENSION_NOT_FOUND", "Chrome 또는 Edge의 기본 설치 위치에서 네이버 동영상 플러그인을 찾지 못했습니다. 해당 브라우저에 공식 확장이 설치되어 있는지 확인해 주세요.", false)
}
fn invalid() -> StreamError {
    StreamError::new("BROWSER_EXTENSION_INVALID", "설치된 네이버 확장의 경로·버전·공개키 ID를 안전하게 확인하지 못했습니다. 원본 확장과 브라우저 프로필은 변경하지 않았습니다.", false)
}
fn load_error() -> StreamError {
    StreamError::new("BROWSER_EXTENSION_LOAD_FAILED", "네이버 확장을 공식 시청 창에 연결하지 못했습니다. WebView2 확장 지원과 설치 폴더 호환성을 확인해 주세요. 그리드 연결 프로그램은 별도로 필요할 수 있습니다.", true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_is_opt_in_and_persists_no_profile_or_user_agent_data() {
        let root = tempfile::tempdir().unwrap();
        assert!(!reconnect_choice(root.path()).unwrap());
        remember_connection(root.path()).unwrap();
        assert!(reconnect_choice(root.path()).unwrap());
        remember_connection(root.path()).unwrap();
        let bytes = fs::read(root.path().join(CHOICE_FILE)).unwrap();
        assert_eq!(bytes, br#"{"version":1,"autoConnect":true}"#);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn malformed_or_oversized_reconnect_choice_is_not_trusted_or_overwritten() {
        let root = tempfile::tempdir().unwrap();
        for bytes in [
            b"{".to_vec(),
            br#"{"version":2,"autoConnect":true}"#.to_vec(),
            br#"{"version":1,"autoConnect":true,"path":"other"}"#.to_vec(),
            vec![b' '; 257],
        ] {
            fs::write(root.path().join(CHOICE_FILE), &bytes).unwrap();
            assert!(reconnect_choice(root.path()).is_err());
            assert!(remember_connection(root.path()).is_err());
            assert_eq!(fs::read(root.path().join(CHOICE_FILE)).unwrap(), bytes);
        }
    }
    #[test]
    fn explicit_connection_can_atomically_update_a_recognized_disabled_choice() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(CHOICE_FILE),
            br#"{"version":1,"autoConnect":false}"#,
        )
        .unwrap();
        assert!(!reconnect_choice(root.path()).unwrap());
        remember_connection(root.path()).unwrap();
        assert!(reconnect_choice(root.path()).unwrap());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn reconnect_choice_rejects_a_directory_or_relative_data_root() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(CHOICE_FILE)).unwrap();
        assert!(reconnect_choice(root.path()).is_err());
        assert!(remember_connection(root.path()).is_err());
        assert!(reconnect_choice(Path::new("relative")).is_err());
    }

    const EDGE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36 Edg/152.0.0.0";

    #[test]
    fn chrome_only_identity_removes_only_edge_product_and_keeps_real_version() {
        let chrome = aligned_user_agent(EDGE_UA, &[CHROME_EXTENSION_ID.into()]).unwrap();
        assert_eq!(chrome, EDGE_UA.strip_suffix(" Edg/152.0.0.0").unwrap());
        assert_eq!(
            aligned_user_agent(&chrome, &[CHROME_EXTENSION_ID.into()]).unwrap(),
            chrome
        );
        let future = EDGE_UA.replace("152.0.0.0", "163.7.42.19");
        assert!(aligned_user_agent(&future, &[CHROME_EXTENSION_ID.into()])
            .unwrap()
            .contains("Chrome/163.7.42.19"));
    }

    #[test]
    fn edge_only_and_both_identities_preserve_native_edge_user_agent() {
        for ids in [
            vec![EDGE_EXTENSION_ID.into()],
            vec![CHROME_EXTENSION_ID.into(), EDGE_EXTENSION_ID.into()],
        ] {
            assert_eq!(aligned_user_agent(EDGE_UA, &ids).unwrap(), EDGE_UA);
        }
    }

    #[test]
    fn unknown_or_ambiguous_identity_is_an_error_not_a_false_connected_report() {
        for ua in [
            "",
            "unknown browser",
            "Mozilla/5.0 Chrome/1 Safari/1",
            &EDGE_UA.replace("Edg/152.0.0.0", "Whale/152.0.0.0"),
            &format!("{EDGE_UA} Edg/152.0.0.0"),
            &format!("{EDGE_UA}\r\n"),
            &"A".repeat(1025),
        ] {
            assert_eq!(
                aligned_user_agent(ua, &[CHROME_EXTENSION_ID.into()])
                    .unwrap_err()
                    .code,
                "BROWSER_EXTENSION_IDENTITY_FAILED"
            );
        }
        let chrome = EDGE_UA.strip_suffix(" Edg/152.0.0.0").unwrap();
        assert!(aligned_user_agent(chrome, &[EDGE_EXTENSION_ID.into()]).is_err());
        assert!(aligned_user_agent(EDGE_UA, &[]).is_err());
        assert!(aligned_user_agent(EDGE_UA, &["unknown".into()]).is_err());
    }

    #[test]
    fn reconnect_restores_observed_native_identity_and_cache_is_bounded() {
        let mut cache = NativeUserAgents::default();
        let chrome = cache
            .selected(1, EDGE_UA, &[CHROME_EXTENSION_ID.into()])
            .unwrap();
        assert_eq!(
            cache
                .selected(1, &chrome, &[EDGE_EXTENSION_ID.into()])
                .unwrap(),
            EDGE_UA
        );
        assert!(cache
            .selected(1, "unknown override", &[CHROME_EXTENSION_ID.into()])
            .is_err());
        let updated_runtime = EDGE_UA.replace("152.0.0.0", "153.0.0.0");
        let updated_chrome = cache
            .selected(1, &updated_runtime, &[CHROME_EXTENSION_ID.into()])
            .unwrap();
        assert_eq!(
            cache
                .selected(1, &updated_chrome, &[EDGE_EXTENSION_ID.into()])
                .unwrap(),
            updated_runtime
        );
        for identity in 2..=20 {
            cache
                .selected(identity, EDGE_UA, &[CHROME_EXTENSION_ID.into()])
                .unwrap();
        }
        assert_eq!(cache.0.len(), 8);
        assert!(!cache.0.iter().any(|(identity, _)| *identity == 1));
    }

    #[cfg(windows)]
    #[test]
    fn completion_is_consumed_once_even_after_late_failure() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let completion: Completion<()> = Arc::new(Mutex::new(Some(Box::new(move |result| {
            assert!(result.is_ok());
            observed.fetch_add(1, Ordering::AcqRel);
        }))));
        complete(&completion, Ok(()));
        complete(&completion, Err(identity_error()));
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[cfg(windows)]
    #[test]
    fn identity_callback_is_posted_outside_the_native_dispatcher_lock() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        };
        let dispatcher_lock = Arc::new(Mutex::new(()));
        let callback_lock = dispatcher_lock.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let completion: Completion<()> = Arc::new(Mutex::new(Some(Box::new(move |result| {
            assert!(result.is_ok());
            assert!(
                callback_lock.try_lock().is_ok(),
                "callback must not re-enter a held UI dispatcher"
            );
            observed.fetch_add(1, Ordering::AcqRel);
        }))));
        let (post, tasks) = mpsc::sync_channel(1);
        let results =
            deferred_completion(completion, move |task| post.send(task).map_err(|_| ())).unwrap();
        let held = dispatcher_lock.lock().unwrap();
        results.send(Ok(())).unwrap();
        let task = tasks
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert_eq!(calls.load(Ordering::Acquire), 0);
        drop(held);
        task();
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn probe_ids_are_exact_bounded_and_not_javascript_input() {
        assert_eq!(
            checked_probe_ids(&[CHROME_EXTENSION_ID.into(), EDGE_EXTENSION_ID.into()])
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            checked_probe_ids(&[CHROME_EXTENSION_ID.into(), CHROME_EXTENSION_ID.into()])
                .unwrap()
                .len(),
            1
        );
        assert!(checked_probe_ids(&[]).is_err());
        assert!(checked_probe_ids(&vec![CHROME_EXTENSION_ID.into(); 3]).is_err());
        assert!(checked_probe_ids(&["';throw 1;//".into()]).is_err());
        assert!(checked_probe_ids(&["jielicmbabjkjaeffodlffnaiiflnphp".into()]).is_err());
        let script = detection_script(&[CHROME_EXTENSION_ID.into()]).unwrap();
        assert!(script.contains("{svc:'install'}"));
        assert!(!script.contains("sendNativeMessage"));
        assert!(!script.contains("connectNative"));
    }

    #[test]
    fn detection_requires_real_install_reply_shape_and_allowed_ids() {
        let ids = vec![CHROME_EXTENSION_ID.into(), EDGE_EXTENSION_ID.into()];
        let response = |detected: Value, api: bool, timeout: bool| {
            serde_json::json!({
            "result":{"type":"object","value":{"detectedIds":detected,"pageApiAvailable":api,"timedOut":timeout}}
        }).to_string()
        };
        let detected = parse_detection_response(
            &response(serde_json::json!([EDGE_EXTENSION_ID]), true, false),
            &ids,
        )
        .unwrap();
        assert_eq!(detected.detected_ids, vec![EDGE_EXTENSION_ID.to_string()]);
        assert!(detected.page_api_available);
        let absent =
            parse_detection_response(&response(serde_json::json!([]), false, false), &ids).unwrap();
        assert!(!absent.page_api_available);
        assert!(parse_detection_response(
            &response(serde_json::json!([CHROME_EXTENSION_ID]), false, false),
            &ids
        )
        .is_err());
        assert!(parse_detection_response(
            &response(
                serde_json::json!([CHROME_EXTENSION_ID, CHROME_EXTENSION_ID]),
                true,
                false
            ),
            &ids
        )
        .is_err());
        assert!(parse_detection_response(
            &response(serde_json::json!(["other"]), true, false),
            &ids
        )
        .is_err());
        assert!(parse_detection_response(
            &response(serde_json::json!([EDGE_EXTENSION_ID]), true, false),
            &[CHROME_EXTENSION_ID.into()]
        )
        .is_err());
        assert!(parse_detection_response("{\"exceptionDetails\":{}}", &ids).is_err());
        assert!(parse_detection_response(&" ".repeat(16 * 1024 + 1), &ids).is_err());
    }

    #[test]
    fn edge_identity_is_discovered_but_unverified_key_is_not_trusted() {
        let directory = tempfile::tempdir().unwrap();
        let extension = directory
            .path()
            .join("Microsoft/Edge/User Data/Default/Extensions")
            .join(EDGE_EXTENSION_ID)
            .join("1.0_0");
        fs::create_dir_all(&extension).unwrap();
        fs::write(
            extension.join("manifest.json"),
            br#"{"manifest_version":3,"version":"1.0","key":"YWJj"}"#,
        )
        .unwrap();
        assert_eq!(
            discover(directory.path()).unwrap_err().code,
            "BROWSER_EXTENSION_INVALID"
        );
        assert!(validate_manifest(
            &serde_json::json!({"manifest_version":3,"version":"1.0","key":"YWJj"}),
            [1, 0, 0, 0],
            "lkhibglpipabmpokebebeanofnkocccd"
        )
        .is_err());
    }

    #[test]
    fn profile_and_version_names_are_bounded_and_not_paths() {
        for name in ["Default", "Profile 1", "Profile 4294967295"] {
            assert!(valid_profile(name));
        }
        for name in [
            "Guest Profile",
            "Profile ",
            "Profile -1",
            "Profile 1/../Default",
            "Profile 4294967296",
        ] {
            assert!(!valid_profile(name));
        }
        assert_eq!(directory_version("1.2.3.4_0"), Some([1, 2, 3, 4]));
        assert_eq!(directory_version("2.10_12"), Some([2, 10, 0, 0]));
        for name in [
            "../1.0",
            "1.0/evil",
            "1.2.3.4.5",
            "65536.0",
            "1..2",
            "1.0_",
            "1.0_0_1",
            "1.0_65536",
        ] {
            assert!(directory_version(name).is_none());
        }
    }

    #[test]
    fn public_key_hash_uses_chromium_id_alphabet_and_rejects_other_identity() {
        assert_eq!(
            extension_id("YWJj").as_deref(),
            Some("lkhibglpipabmpokebebeanofnkocccd")
        );
        assert_eq!(extension_id(" YWJj\n"), extension_id("YWJj"));
        assert!(extension_id("%%%").is_none());
        assert!(extension_id("").is_none());
        assert!(extension_id(&"A".repeat(MAX_KEY + 1)).is_none());
        let manifest = serde_json::json!({"manifest_version":3,"version":"1.2","key":"YWJj"});
        assert!(validate_manifest(&manifest, [1, 2, 0, 0], CHROME_EXTENSION_ID).is_err());
        assert!(validate_manifest(
            &serde_json::json!({"manifest_version":3,"version":"1.2"}),
            [1, 2, 0, 0],
            CHROME_EXTENSION_ID
        )
        .is_err());
    }

    #[test]
    fn discovery_does_not_use_custom_profiles_or_unrelated_extensions() {
        let directory = tempfile::tempdir().unwrap();
        let custom = directory
            .path()
            .join("Google/Chrome/User Data/Guest Profile/Extensions")
            .join(CHROME_EXTENSION_ID)
            .join("1.0_0");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("manifest.json"), b"{}").unwrap();
        let unrelated = directory.path().join(
            "Microsoft/Edge/User Data/Default/Extensions/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/1.0_0",
        );
        fs::create_dir_all(&unrelated).unwrap();
        assert_eq!(
            discover(directory.path()).unwrap_err().code,
            "BROWSER_EXTENSION_NOT_FOUND"
        );
    }

    #[test]
    fn exact_id_installation_with_wrong_manifest_is_not_loaded() {
        let directory = tempfile::tempdir().unwrap();
        let extension = directory
            .path()
            .join("Google/Chrome/User Data/Profile 2/Extensions")
            .join(CHROME_EXTENSION_ID)
            .join("1.0_0");
        fs::create_dir_all(&extension).unwrap();
        fs::write(
            extension.join("manifest.json"),
            br#"{"manifest_version":3,"version":"1.0","key":"YWJj"}"#,
        )
        .unwrap();
        assert_eq!(
            discover(directory.path()).unwrap_err().code,
            "BROWSER_EXTENSION_INVALID"
        );
    }

    #[test]
    fn manifest_and_resource_sizes_and_tree_depth_are_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let large = File::create(root.join("large.js")).unwrap();
        large.set_len(MAX_TREE_BYTES + 1).unwrap();
        assert!(validate_tree(&root).is_err());
        drop(large);
        fs::remove_file(root.join("large.js")).unwrap();
        let mut nested = root.clone();
        for _ in 0..MAX_TREE_DEPTH + 1 {
            nested = nested.join("nested");
            fs::create_dir(&nested).unwrap();
        }
        assert!(validate_tree(&root).is_err());
    }

    #[test]
    fn traversal_and_unexpected_extension_parent_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        assert!(checked_directory(Path::new("relative")).is_err());
        // Windows canonical paths use the verbatim prefix. PathBuf::join may
        // normalize a pushed ".." before checked_directory ever sees it, so
        // construct the untrusted literal without PathBuf's push semantics.
        let mut traversal = root.as_os_str().to_os_string();
        traversal.push(std::path::MAIN_SEPARATOR.to_string());
        traversal.push("..");
        assert!(checked_directory(&PathBuf::from(traversal)).is_err());
        let path = root.join("other-id/1.0_0");
        fs::create_dir_all(&path).unwrap();
        assert!(validate_extension(&root, &path, [1, 0, 0, 0], CHROME_EXTENSION_ID).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn reparse_resource_is_rejected_when_symlink_creation_is_available() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = outside.path().join("outside.js");
        fs::write(&source, b"external").unwrap();
        let link = root.join("background.js");
        if std::os::windows::fs::symlink_file(&source, &link).is_ok() {
            assert!(validate_tree(&root).is_err());
        }
    }
}
