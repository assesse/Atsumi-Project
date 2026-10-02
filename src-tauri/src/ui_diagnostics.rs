//! Small, bounded SSD flight recorder. Never log image bodies, keystrokes,
//! search text, cookies, or whole IPC arguments. Logging cannot block the UI.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State};

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPulse {
    pub epoch: String,
    pub visible: bool,
    pub lag_ms: u64,
    pub js_heap_bytes: Option<u64>,
    pub input_count: u64,
    pub long_tasks: u64,
    pub longest_task_ms: u64,
    pub thumbnail_received: u64,
    pub thumbnail_live_bytes: u64,
    pub thumbnail_live_count: u64,
    pub download_received: u64,
    pub download_notifications: u64,
}

struct Health {
    epoch: String,
    started: Instant,
    last_pulse: Option<Instant>,
    foreground_since: Option<Instant>,
    last_check: Instant,
    native_unresponsive: bool,
    pulse: UiPulse,
    reloads: VecDeque<Instant>,
    reload_pending: bool,
}
impl Health {
    fn new(now: Instant) -> Self {
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            started: now,
            last_pulse: None,
            foreground_since: None,
            last_check: now,
            native_unresponsive: false,
            pulse: UiPulse::default(),
            reloads: VecDeque::new(),
            reload_pending: false,
        }
    }
    fn stalled(&mut self, now: Instant, foreground: bool) -> bool {
        // A suspended PC / delayed native monitor is not renderer evidence.
        if now.duration_since(self.last_check) > Duration::from_secs(15) {
            self.foreground_since = None;
        }
        self.last_check = now;
        if !foreground {
            self.foreground_since = None;
            return false;
        }
        let foreground_since = *self.foreground_since.get_or_insert(now);
        // Native focus and visibility are authoritative; a throttled background
        // timer is never a failure. Allow time to resume and to finish startup.
        if now.duration_since(self.started) < Duration::from_secs(60)
            || now.duration_since(foreground_since) < Duration::from_secs(40)
            || self.last_pulse.is_none()
            || self.reload_pending
        {
            return false;
        }
        let silence = now.duration_since(self.last_pulse.unwrap());
        silence >= Duration::from_secs(if self.native_unresponsive { 30 } else { 60 })
    }
    fn claim(&mut self, now: Instant, epoch: &str) -> bool {
        while self
            .reloads
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(300))
        {
            self.reloads.pop_front();
        }
        if epoch != self.epoch || self.reload_pending || self.reloads.len() >= 2 {
            return false;
        }
        self.reloads.push_back(now);
        self.reload_pending = true;
        true
    }
}

pub struct UiDiagnostics {
    health: Mutex<Health>,
    log: SyncSender<Value>,
    sample_pending: AtomicBool,
    renderer_private_bytes: AtomicU64,
    dropped_logs: AtomicU64,
    last_capture: Mutex<Option<Instant>>,
}
impl UiDiagnostics {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        let (tx, rx) = mpsc::sync_channel::<Value>(128);
        std::thread::spawn(move || {
            if std::fs::create_dir_all(&directory).is_err() {
                return;
            }
            // Append across restarts. Only the next (oldest) ring slot is
            // truncated when the size budget is exhausted.
            let mut index = (0..3)
                .max_by_key(|i| {
                    std::fs::metadata(directory.join(format!("ui-health-{i}.jsonl")))
                        .and_then(|m| m.modified())
                        .ok()
                })
                .unwrap_or(0);
            let mut size = std::fs::metadata(directory.join(format!("ui-health-{index}.jsonl")))
                .map(|m| m.len() as usize)
                .unwrap_or(0);
            let open = |index| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(directory.join(format!("ui-health-{index}.jsonl")))
            };
            let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join(format!("ui-health-{index}.jsonl")))
            else {
                return;
            };
            while let Ok(value) = rx.recv() {
                let Ok(mut line) = serde_json::to_vec(&value) else {
                    continue;
                };
                if line.len() > 32 * 1024 {
                    continue;
                }
                line.push(b'\n');
                if size + line.len() > 2 * 1024 * 1024 {
                    index = (index + 1) % 3;
                    let Ok(next) = open(index) else {
                        break;
                    };
                    file = next;
                    size = 0;
                }
                if file.write_all(&line).is_err() {
                    break;
                }
                size += line.len();
            }
        });
        Arc::new(Self {
            health: Mutex::new(Health::new(Instant::now())),
            log: tx,
            sample_pending: AtomicBool::new(false),
            renderer_private_bytes: AtomicU64::new(0),
            dropped_logs: AtomicU64::new(0),
            last_capture: Mutex::new(None),
        })
    }
    pub fn record(&self, kind: &'static str, data: Value) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if self.log.try_send(json!({"timeUnixMs":timestamp,"kind":kind,"droppedLogs":self.dropped_logs.load(Ordering::Relaxed),"data":data})).is_err() { self.dropped_logs.fetch_add(1,Ordering::Relaxed); }
    }
    pub fn epoch(&self) -> String {
        self.health
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .epoch
            .clone()
    }
    pub fn snapshot(&self) -> Value {
        let health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        json!({"epoch":health.epoch,"pulseAgeMs":health.last_pulse.map(|t|t.elapsed().as_millis()),
            "nativeUnresponsive":health.native_unresponsive,"reloadPending":health.reload_pending,
            "rendererPrivateBytes":self.renderer_private_bytes.load(Ordering::Relaxed),
            "droppedLogs":self.dropped_logs.load(Ordering::Relaxed),"pulse":health.pulse})
    }
    pub fn document_started(&self) {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        let reloads = std::mem::take(&mut health.reloads);
        *health = Health::new(Instant::now());
        health.reloads = reloads;
        self.record("document_started", json!({"epoch":health.epoch}));
    }
    pub fn pulse(&self, pulse: UiPulse) -> bool {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if pulse.epoch != health.epoch {
            return false;
        }
        health.last_pulse = Some(Instant::now());
        health.native_unresponsive = false;
        health.pulse = pulse;
        true
    }
    pub fn failed(&self, kind: i32, reason: i32, exit_code: i32, unresponsive: bool) {
        self.health
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .native_unresponsive = unresponsive;
        self.record(
            "process_failed",
            json!({"epoch":self.epoch(),"kind":kind,"reason":reason,"exitCode":exit_code}),
        );
    }
    pub fn claim_reload(&self, epoch: &str, reason: &'static str) -> bool {
        let allowed = self
            .health
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .claim(Instant::now(), epoch);
        self.record(
            "reload_decision",
            json!({"epoch":epoch,"reason":reason,"allowed":allowed}),
        );
        allowed
    }
    pub fn cancel_reload(&self, epoch: &str, reason: &'static str) {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if health.epoch == epoch {
            health.reload_pending = false;
        }
        self.record("reload_cancelled", json!({"epoch":epoch,"reason":reason}));
    }
    pub fn may_reload(&self, epoch: &str, hung: bool) -> bool {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if health.epoch != epoch {
            return false;
        }
        if hung
            && health
                .last_pulse
                .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
        {
            health.reload_pending = false;
            return false;
        }
        true
    }
}

#[tauri::command]
pub fn ui_diagnostics_session(state: State<'_, Arc<UiDiagnostics>>) -> String {
    state.epoch()
}
#[tauri::command]
pub fn ui_diagnostics_pulse(state: State<'_, Arc<UiDiagnostics>>, pulse: UiPulse) -> bool {
    state.pulse(pulse)
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiMark {
    F5Results,
    FrontendReady,
    ManualCapture,
}
#[tauri::command]
pub fn ui_diagnostics_mark(state: State<'_, Arc<UiDiagnostics>>, mark: UiMark) {
    let kind = match mark {
        UiMark::F5Results => "f5_results_refresh",
        UiMark::FrontendReady => "frontend_ready",
        UiMark::ManualCapture => "manual_capture",
    };
    state.record(kind, json!({"epoch":state.epoch()}));
}

pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let recorder = UiDiagnostics::new(app.path().app_data_dir()?.join("diagnostics"));
    recorder.record(
        "process_started",
        json!({"pid":std::process::id(),"version":env!("CARGO_PKG_VERSION")}),
    );
    app.manage(Arc::clone(&recorder));
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let Some(view) = app.get_webview_window("main") else {
            break;
        };
        if app
            .try_state::<crate::interface::AppState>()
            .is_some_and(|s| s.is_quitting())
        {
            break;
        }
        let foreground = view.is_visible().unwrap_or(false)
            && view.is_focused().unwrap_or(false)
            && !view.is_minimized().unwrap_or(true);
        let (stalled, epoch, pulse, silence) = {
            let mut health = recorder.health.lock().unwrap_or_else(|e| e.into_inner());
            (
                health.stalled(Instant::now(), foreground),
                health.epoch.clone(),
                health.pulse.clone(),
                health.last_pulse.map(|t| t.elapsed().as_millis()),
            )
        };
        let transport = app
            .try_state::<crate::interface::AppState>()
            .map(|s| s.thumbnail_transport.stats());
        let downloads = app
            .try_state::<Arc<crate::ui_download_events::DownloadEvents>>()
            .map(|s| s.stats());
        recorder.record("health",json!({"epoch":epoch,"foreground":foreground,"silenceMs":silence,"frontend":pulse,"images":transport,"downloads":downloads}));
        sample_native(&view, Arc::clone(&recorder));
        if pulse.js_heap_bytes.unwrap_or(0) > 256 * 1024 * 1024
            || recorder.renderer_private_bytes.load(Ordering::Relaxed) > 768 * 1024 * 1024
        {
            // Evidence before a freeze, not memory-triggered automatic reload.
            capture(&app, "memory_pressure");
        }
        if stalled {
            crate::renderer_recovery::request_reload(&app, "foreground_hang", epoch, true);
        }
    });
    Ok(())
}

#[cfg(windows)]
fn sample_native(view: &tauri::WebviewWindow, recorder: Arc<UiDiagnostics>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment8, ICoreWebView2_2, COREWEBVIEW2_PROCESS_KIND,
    };
    use windows::{
        core::Interface,
        Win32::{
            Foundation::CloseHandle,
            System::{
                ProcessStatus::{
                    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
                },
                Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
            },
        },
    };
    if recorder.sample_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let pending = Arc::clone(&recorder);
    if view.with_webview(move |platform| unsafe {
        let result=(||->windows::core::Result<Vec<Value>> {
            let core=platform.controller().CoreWebView2()?.cast::<ICoreWebView2_2>()?;
            let processes=core.Environment()?.cast::<ICoreWebView2Environment8>()?.GetProcessInfos()?;
            let mut count=0; processes.Count(&mut count)?; let mut rows=Vec::new();
            for i in 0..count.min(24) {
                let info=processes.GetValueAtIndex(i)?; let mut pid=0; let mut kind=COREWEBVIEW2_PROCESS_KIND::default();
                info.ProcessId(&mut pid)?; info.Kind(&mut kind)?;
                let Ok(handle)=OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ,false,pid as u32) else { continue; };
                let mut memory=PROCESS_MEMORY_COUNTERS_EX::default(); memory.cb=std::mem::size_of_val(&memory) as u32;
                let read=GetProcessMemoryInfo(handle,&mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS,memory.cb);
                let _=CloseHandle(handle);
                if read.is_ok() { rows.push(json!({"pid":pid,"kind":kind.0,"privateBytes":memory.PrivateUsage,"workingSetBytes":memory.WorkingSetSize})); }
            }
            Ok(rows)
        })();
        if let Ok(rows)=result {
            let renderer_bytes=rows.iter().filter(|row|row["kind"].as_i64()==Some(1)).filter_map(|row|row["privateBytes"].as_u64()).sum();
            recorder.renderer_private_bytes.store(renderer_bytes,Ordering::Relaxed);
            recorder.record("webview_memory",json!({"epoch":recorder.epoch(),"processes":rows}));
        }
        recorder.sample_pending.store(false,Ordering::Release);
    }).is_err() { pending.sample_pending.store(false,Ordering::Release); }
}
#[cfg(not(windows))]
fn sample_native(_: &tauri::WebviewWindow, _: Arc<UiDiagnostics>) {}

/// CDP remains private to the native app; no remote debugging port is opened.
/// Only aggregate counts and capped sampled allocation call sites are written.
#[cfg(windows)]
pub fn capture(app: &tauri::AppHandle, reason: &'static str) {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    let Some(view) = app.get_webview_window("main") else {
        return;
    };
    let Some(recorder) = app
        .try_state::<Arc<UiDiagnostics>>()
        .map(|s| s.inner().clone())
    else {
        return;
    };
    {
        let mut last = recorder
            .last_capture
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let epoch = recorder.epoch();
    recorder.record(
        "capture_requested",
        json!({"reason":reason,"epoch":recorder.epoch()}),
    );
    let _=view.with_webview(move |platform| unsafe {
        let Ok(core)=platform.controller().CoreWebView2() else { return; };
        for method in ["Runtime.getHeapUsage","Memory.getDOMCounters","HeapProfiler.getSamplingProfile"] {
            let recorder=Arc::clone(&recorder);
            let epoch=epoch.clone();
            let handler=CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status,text| {
                if status.is_ok() {
                    if let Ok(value)=serde_json::from_str::<Value>(&text) {
                        let data=if method=="HeapProfiler.getSamplingProfile" { sampled_sites(&value) } else {
                            Value::Object(value.as_object().into_iter().flatten().filter(|(_,v)|v.is_number()).map(|(k,v)|(k.clone(),v.clone())).collect())
                        };
                        recorder.record("cdp_capture",json!({"epoch":epoch,"method":method,"reason":reason,"result":data}));
                    }
                } else { recorder.record("cdp_unavailable",json!({"method":method,"reason":reason})); }
                Ok(())
            }));
            let _=core.CallDevToolsProtocolMethod(&HSTRING::from(method),&HSTRING::from("{}"),&handler);
        }
    });
}
#[cfg(not(windows))]
pub fn capture(_: &tauri::AppHandle, _: &'static str) {}

fn sampled_sites(value: &Value) -> Value {
    let mut pending = vec![&value["profile"]["head"]];
    let mut rows = Vec::new();
    let mut seen = 0;
    while let Some(node) = pending.pop() {
        seen += 1;
        if seen > 20_000 {
            break;
        }
        if let Some(children) = node["children"].as_array() {
            pending.extend(children.iter());
        }
        let size = node["selfSize"].as_u64().unwrap_or(0);
        if size == 0 {
            continue;
        }
        let frame = &node["callFrame"];
        let name: String = frame["functionName"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(100)
            .collect();
        let file: String = frame["url"]
            .as_str()
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .chars()
            .take(100)
            .collect();
        rows.push(json!({"bytes":size,"function":name,"file":file,"line":frame["lineNumber"]}));
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row["bytes"].as_u64().unwrap_or(0)));
    rows.truncate(48);
    json!({"topSites":rows,"sampled":true})
}

#[cfg(windows)]
pub fn start_sampling(view: &tauri::WebviewWindow) {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    // Debug diagnosis (or explicit release opt-in), not a release-wide profiler.
    if !cfg!(debug_assertions) && std::env::var("ATSUMI_UI_PROFILE").as_deref() != Ok("1") {
        return;
    }
    let _ = view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            let handler =
                CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
            let _ = core.CallDevToolsProtocolMethod(
                &HSTRING::from("HeapProfiler.startSampling"),
                &HSTRING::from(r#"{"samplingInterval":131072}"#),
                &handler,
            );
        }
    });
}
#[cfg(not(windows))]
pub fn start_sampling(_: &tauri::WebviewWindow) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_budget_survives_document_replacement_and_expires() {
        let now = Instant::now();
        let mut health = Health::new(now);
        let epoch = health.epoch.clone();
        assert!(health.claim(now, &epoch));
        health.reload_pending = false;
        assert!(health.claim(now + Duration::from_secs(1), &epoch));
        health.reload_pending = false;
        let reloads = std::mem::take(&mut health.reloads);
        health = Health::new(now + Duration::from_secs(2));
        health.reloads = reloads;
        let new_epoch = health.epoch.clone();
        assert!(!health.claim(now + Duration::from_secs(3), &epoch));
        assert!(!health.claim(now + Duration::from_secs(3), &new_epoch));
        assert!(health.claim(now + Duration::from_secs(300), &new_epoch));
    }
    #[test]
    fn hidden_starting_or_temporarily_throttled_documents_never_recover() {
        let now = Instant::now();
        let mut health = Health::new(now);
        health.last_pulse = Some(now);
        assert!(!health.stalled(now + Duration::from_secs(100), false));
        assert!(!health.stalled(now + Duration::from_secs(101), true));
        for seconds in [110, 120, 130, 140] {
            assert!(!health.stalled(now + Duration::from_secs(seconds), true));
        }
        assert!(health.stalled(now + Duration::from_secs(145), true));
        let epoch = health.epoch.clone();
        assert!(health.claim(now + Duration::from_secs(145), &epoch));
        assert!(!health.claim(now + Duration::from_secs(146), &epoch));
        assert!(!health.claim(now + Duration::from_secs(146), "old-document"));
    }
    #[test]
    fn sleep_resume_receives_a_new_foreground_grace_period() {
        let now = Instant::now();
        let mut health = Health::new(now);
        health.last_pulse = Some(now);
        for seconds in (0..50).step_by(5) {
            assert!(!health.stalled(now + Duration::from_secs(seconds), true));
        }
        assert!(!health.stalled(now + Duration::from_secs(3600), true));
        assert!(!health.stalled(now + Duration::from_secs(3605), true));
    }
    #[test]
    fn sampling_output_does_not_contain_remote_queries_or_heap_objects() {
        let output = sampled_sites(
            &json!({"profile":{"head":{"selfSize":10,"callFrame":{"functionName":"render","url":"https://example/a.js?secret=123","lineNumber":2},"children":[]}}}),
        );
        assert!(!output.to_string().contains("secret"));
        assert!(!output.to_string().contains("example"));
        assert_eq!(output["topSites"][0]["file"], "a.js");
    }
}
