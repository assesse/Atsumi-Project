//! Startup coordination is independent of database state and never waits on the
//! native UI thread. IPC waits asynchronously until recovery has safely finished.
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Invoke, Manager};

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Starting,
    Ready,
    Failed,
    Cancelling,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    phase: Phase,
    elapsed_ms: u64,
}

pub(crate) struct Startup {
    phase: Mutex<Phase>,
    changes: tokio::sync::watch::Sender<Phase>,
    pending: AtomicUsize,
    frontend_connected: AtomicBool,
}

impl Default for Startup {
    fn default() -> Self {
        let (changes, _) = tokio::sync::watch::channel(Phase::Starting);
        Self {
            phase: Mutex::new(Phase::Starting),
            changes,
            pending: AtomicUsize::new(0),
            frontend_connected: AtomicBool::new(false),
        }
    }
}

impl Startup {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: *self.phase.lock().unwrap_or_else(|e| e.into_inner()),
            elapsed_ms: elapsed_ms(),
        }
    }

    pub fn ready(&self) -> bool {
        self.snapshot().phase == Phase::Ready
    }
    pub fn cancelling(&self) -> bool {
        self.snapshot().phase == Phase::Cancelling
    }
    pub fn frontend_connected(&self) -> bool {
        self.frontend_connected.load(Ordering::Acquire)
    }

    pub fn cancel(&self) -> bool {
        let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
        if *phase == Phase::Ready {
            return false;
        }
        *phase = Phase::Cancelling;
        self.changes.send_replace(*phase);
        true
    }

    /// Only the short resume/publication step runs under this mutex. Slow disk
    /// recovery has already completed. A cancellation either wins before this
    /// commit (no worker resumes), or observes Ready and uses normal safe quit.
    pub fn commit_ready(&self, publish: impl FnOnce()) -> bool {
        let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
        if *phase != Phase::Starting {
            return false;
        }
        publish();
        *phase = Phase::Ready;
        self.changes.send_replace(*phase);
        mark("backend_ready");
        true
    }

    pub fn fail(&self) {
        let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
        if *phase == Phase::Starting {
            *phase = Phase::Failed;
            mark("backend_failed");
        }
        self.changes.send_replace(*phase);
    }

    async fn wait_ready(&self) -> bool {
        let mut changes = self.changes.subscribe();
        let result = changes.wait_for(|phase| *phase != Phase::Starting).await;
        result.is_ok_and(|phase| *phase == Phase::Ready)
    }
}

fn startup_error() -> serde_json::Value {
    serde_json::json!({"code":"APP_STARTUP_UNAVAILABLE", "message":"앱 데이터를 준비하지 못했거나 시작을 취소했습니다. 시작 상태를 확인해 주세요.", "retryable":true})
}

pub(crate) fn gate<R: tauri::Runtime>(
    handler: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    let handler = Arc::new(handler);
    move |invoke: Invoke<R>| {
        let app = invoke.message.webview().app_handle().clone();
        let startup = app.state::<Arc<Startup>>().inner().clone();
        let command = invoke.message.command();
        if startup.ready()
            || matches!(
                command,
                "app_startup_snapshot"
                    | "app_startup_frame"
                    | "app_startup_cancel"
                    | "app_minimize_to_tray"
            )
        {
            return handler(invoke);
        }
        if command == "app_active_work_snapshot" {
            invoke
                .resolver
                .resolve(serde_json::json!({"ok":false,"error":startup_error()}));
            return true;
        }
        if command == "app_quit" {
            // The existing exit dialog has already required explicit choice.
            let failed = startup.snapshot().phase == Phase::Failed;
            if startup.cancel() {
                invoke
                    .resolver
                    .resolve(serde_json::json!({"ok":true,"data":{"accepted":true}}));
                if failed {
                    app.exit(0);
                }
            } else {
                return handler(invoke);
            }
            return true;
        }
        // A bounded wait set avoids unbounded requests from a broken renderer.
        if startup
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 256).then_some(n + 1)
            })
            .is_err()
        {
            invoke.resolver.reject(startup_error());
            return true;
        }
        let handler = handler.clone();
        tauri::async_runtime::spawn(async move {
            let ready = startup.wait_ready().await;
            startup.pending.fetch_sub(1, Ordering::AcqRel);
            if ready {
                let resolver = invoke.resolver.clone();
                if !handler(invoke) {
                    resolver.reject("Unknown command");
                }
            } else {
                invoke.resolver.reject(startup_error());
            }
        });
        true
    }
}

#[tauri::command]
pub(crate) fn app_startup_snapshot(state: tauri::State<'_, Arc<Startup>>) -> Snapshot {
    state.snapshot()
}

#[tauri::command]
pub(crate) fn app_startup_cancel(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Startup>>,
) -> bool {
    let failed = state.snapshot().phase == Phase::Failed;
    let cancelled = state.cancel();
    if cancelled && failed {
        app.exit(0);
    }
    cancelled
}

#[tauri::command]
pub(crate) async fn app_startup_frame(
    app: tauri::AppHandle,
    settings_ready: bool,
) -> Result<Snapshot, String> {
    let startup = app.state::<Arc<Startup>>().inner().clone();
    startup.frontend_connected.store(true, Ordering::Release);
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        mark(if settings_ready {
            "interactive_frame"
        } else {
            "shell_frame"
        });
        let _ = tx.send(startup.snapshot());
    })
    .map_err(|_| "The native event loop is unavailable".to_string())?;
    rx.await
        .map_err(|_| "The native event loop was closed".to_string())
}

struct MetricEvent {
    stage: &'static str,
    elapsed_ms: u64,
    unix_ms: u64,
}
struct Metrics {
    entry: Instant,
    before_entry_ms: u64,
    events: mpsc::SyncSender<MetricEvent>,
}
static METRICS: OnceLock<Metrics> = OnceLock::new();

pub(crate) fn init_metrics() {
    METRICS.get_or_init(|| {
        let entry = Instant::now();
        let before_entry_ms = process_age_ms();
        let (events, receiver) = mpsc::sync_channel::<MetricEvent>(64);
        // Capture this before creating even the logger thread: thread creation
        // can itself invoke slow DLL callbacks on Windows. Do not attribute that
        // time to the loader before main().
        let _ = events.try_send(MetricEvent {
            stage: "main_entered",
            elapsed_ms: before_entry_ms,
            unix_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        });
        std::thread::Builder::new().name("atsumi-startup-metrics".into()).spawn(move || {
            let Some(base) = std::env::var_os("LOCALAPPDATA") else { return; };
            let directory = std::path::PathBuf::from(base).join("Atsumi/Logs");
            if std::fs::create_dir_all(&directory).is_err() { return; }
            let path = directory.join("startup-timings.jsonl");
            // This is a dedicated bounded timing log, never a user-data file.
            if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
                let _ = std::fs::rename(&path, directory.join("startup-timings.previous.jsonl"));
            }
            let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else { return; };
            for event in receiver {
                let record = serde_json::json!({"stage":event.stage, "elapsedMs":event.elapsed_ms,
                    "unixMs":event.unix_ms,
                    "pid":std::process::id(), "version":env!("CARGO_PKG_VERSION"), "debug":cfg!(debug_assertions)});
                let _ = writeln!(file, "{record}");
                let _ = file.flush();
            }
        }).ok();
        Metrics { entry, before_entry_ms, events }
    });
}

pub(crate) fn mark(stage: &'static str) {
    if let Some(metrics) = METRICS.get() {
        let _ = metrics.events.try_send(MetricEvent {
            stage,
            elapsed_ms: elapsed_ms(),
            unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });
    }
}
fn elapsed_ms() -> u64 {
    METRICS.get().map_or(0, |m| {
        m.before_entry_ms + m.entry.elapsed().as_millis() as u64
    })
}

#[cfg(windows)]
fn process_age_ms() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let (mut creation, mut exit, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    // SAFETY: pseudo-handle is valid and all four output structures live through the call.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return 0;
    }
    let ticks = (u64::from(creation.high) << 32) | u64::from(creation.low);
    let created = (ticks / 10_000).saturating_sub(11_644_473_600_000);
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64)
        .saturating_sub(created)
}
#[cfg(not(windows))]
fn process_age_ms() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_prevents_resume_and_publication() {
        let startup = Startup::default();
        assert!(startup.cancel());
        assert!(!startup.commit_ready(|| panic!("cancelled startup resumed work")));
        assert!(startup.cancelling());
    }
    #[test]
    fn ready_hands_shutdown_back_to_normal_managed_work_gate() {
        let startup = Startup::default();
        assert!(startup.commit_ready(|| {}));
        assert!(!startup.cancel());
        assert!(startup.ready());
    }
    #[test]
    fn failure_wakes_existing_and_late_waiters() {
        let startup = Startup::default();
        startup.fail();
        assert!(!tauri::async_runtime::block_on(startup.wait_ready()));
        assert!(startup.cancel());
    }

    #[test]
    fn late_optional_initialization_failure_cannot_bypass_normal_shutdown() {
        let startup = Startup::default();
        assert!(startup.commit_ready(|| {}));
        startup.fail();
        assert!(startup.ready());
        assert!(!startup.cancel());
    }

    #[test]
    fn ipc_is_parked_without_blocking_dispatch_until_recovery_commits() {
        use tauri::{
            ipc::{CallbackFn, InvokeBody},
            test::{get_ipc_response, mock_builder, mock_context, noop_assets},
            webview::InvokeRequest,
            WebviewWindowBuilder,
        };
        let startup = Arc::new(Startup::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let app = mock_builder()
            .manage(startup.clone())
            .invoke_handler(gate(move |invoke| {
                handler_calls.fetch_add(1, Ordering::SeqCst);
                invoke.resolver.resolve("ready");
                true
            }))
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let request = InvokeRequest {
            cmd: "settings_get".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: app.handle().invoke_key().to_string(),
        };
        let pending = std::thread::spawn(move || get_ipc_response(&window, request));
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while startup.pending.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(startup.pending.load(Ordering::Acquire), 1);
        assert_eq!(calls.load(Ordering::Acquire), 0);
        assert!(startup.commit_ready(|| {}));
        assert_eq!(
            pending
                .join()
                .unwrap()
                .unwrap()
                .deserialize::<String>()
                .unwrap(),
            "ready"
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert_eq!(startup.pending.load(Ordering::Acquire), 0);
    }
}
