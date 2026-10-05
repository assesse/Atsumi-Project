//! Local, bounded application flight recorder. Never persist general tracing
//! messages, Debug values, IPC payloads, URLs, paths, SQL or content identifiers.
use serde_json::{json, Map, Value};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id},
    Event, Subscriber,
};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

const SLOTS: usize = 4;
const SLOT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECORD: usize = 8192;
const QUEUE_SIZE: usize = 256;
static RECORDER: OnceLock<Recorder> = OnceLock::new();
struct Recorder {
    sender: mpsc::SyncSender<Message>,
    session: String,
    dropped: AtomicU64,
}
enum Message {
    Event(Value),
    Flush(mpsc::SyncSender<()>),
}

#[derive(Clone, Copy)]
pub(crate) enum Provider {
    Hitomi,
    Danbooru,
    Community,
}
#[derive(Default)]
struct HttpWindow {
    requests: u64,
    failures: u64,
    bytes: u64,
    total_ms: u64,
    max_ms: u64,
}
static HTTP: [Mutex<HttpWindow>; 3] = [const {
    Mutex::new(HttpWindow {
        requests: 0,
        failures: 0,
        bytes: 0,
        total_ms: 0,
        max_ms: 0,
    })
}; 3];
pub(crate) fn http_result(
    provider: Provider,
    success: bool,
    status: u16,
    duration: Duration,
    bytes: usize,
) {
    let elapsed = duration.as_millis() as u64;
    {
        let mut window = HTTP[provider as usize]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        window.requests = window.requests.saturating_add(1);
        window.failures += u64::from(!success);
        window.bytes = window.bytes.saturating_add(bytes as u64);
        window.total_ms = window.total_ms.saturating_add(elapsed);
        window.max_ms = window.max_ms.max(elapsed);
    }
    if !success {
        let stage = match provider {
            Provider::Hitomi => "hitomi_http_failed",
            Provider::Danbooru => "danbooru_http_failed",
            Provider::Community => "community_http_failed",
        };
        tracing::warn!(
            diag_stage = stage,
            status = status as u64,
            elapsed_ms = elapsed
        );
    }
}
fn http_snapshots() -> Vec<Value> {
    HTTP.iter().zip(["hitomi","danbooru","community"]).filter_map(|(lock, provider)| {
        let window = std::mem::take(&mut *lock.lock().unwrap_or_else(|e|e.into_inner()));
        (window.requests>0).then(|| json!({"kind":"http_summary", "provider":provider, "requests":window.requests,
            "failures":window.failures,"bytes":window.bytes,"totalMs":window.total_ms,"maxMs":window.max_ms}))
    }).collect()
}
pub(crate) fn http_error(error: &reqwest::Error) {
    let code = if error.is_timeout() {
        "HTTP_TIMEOUT"
    } else if error.is_connect() {
        "HTTP_CONNECT"
    } else if error.is_redirect() {
        "HTTP_REDIRECT"
    } else if error.is_body() {
        "HTTP_BODY"
    } else if error.is_decode() {
        "HTTP_DECODE"
    } else {
        "HTTP_TRANSPORT"
    };
    tracing::warn!(diag_stage = "http_transport_error", error_code = code);
}

pub(crate) fn init() {
    if RECORDER.get().is_some() {
        return;
    }
    RECORDER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_SIZE);
        let session = uuid::Uuid::new_v4().to_string();
        std::thread::Builder::new()
            .name("atsumi-diagnostics".into())
            .spawn(move || {
                let Some(base) = std::env::var_os("LOCALAPPDATA") else {
                    return;
                };
                let directory = PathBuf::from(base).join("Atsumi/Logs/diagnostics");
                let Ok(mut writer) = RollingWriter::open(&directory, SLOTS, SLOT_BYTES) else {
                    return;
                };
                let mut synced = Instant::now();
                loop {
                    match receiver.recv_timeout(Duration::from_secs(1)) {
                        Ok(Message::Event(value)) => {
                            if writer.append(&value).is_err() {
                                break;
                            }
                        }
                        Ok(Message::Flush(done)) => {
                            let _ = writer.sync();
                            let _ = done.try_send(());
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            let _ = writer.sync();
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if synced.elapsed() >= Duration::from_secs(5) {
                        for event in http_snapshots() {
                            record(event);
                        }
                        let _ = writer.sync();
                        synced = Instant::now();
                    }
                }
            })
            .ok();
        Recorder {
            sender,
            session,
            dropped: AtomicU64::new(0),
        }
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic payloads can include SQL, tokens and filenames. Location alone
        // identifies the source in this build without copying that data.
        record(
            json!({"kind":"panic", "file":info.location().and_then(|l|l.file().rsplit(['/', '\\']).next()),
            "line":info.location().map(|l|l.line())}),
        );
        flush();
        previous(info);
    }));
    record(json!({"kind":"process_started", "debug":cfg!(debug_assertions)}));
}

fn record(mut value: Value) {
    let Some(recorder) = RECORDER.get() else {
        return;
    };
    let object = value
        .as_object_mut()
        .expect("diagnostic records are objects");
    object.insert(
        "timeUnixMs".into(),
        json!(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64),
    );
    object.insert("session".into(), json!(recorder.session));
    object.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    object.insert(
        "dropped".into(),
        json!(recorder.dropped.load(Ordering::Relaxed)),
    );
    if recorder.sender.try_send(Message::Event(value)).is_err() {
        recorder.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn flush() {
    if let Some(recorder) = RECORDER.get() {
        for event in http_snapshots() {
            record(event);
        }
        let (done, wait) = mpsc::sync_channel(1);
        if recorder.sender.try_send(Message::Flush(done)).is_ok() {
            // Only shutdown/panic use this bounded wait, never UI/worker logging.
            let _ = wait.recv_timeout(Duration::from_millis(500));
        }
    }
}

pub(crate) fn startup(stage: &'static str, elapsed_ms: u64) {
    record(json!({"kind":"startup", "stage":stage,"elapsedMs":elapsed_ms}));
}

pub(crate) fn operation(name: &'static str, work_id: Option<&str>) -> tracing::Span {
    use std::hash::BuildHasher;
    static HASHER: OnceLock<std::collections::hash_map::RandomState> = OnceLock::new();
    // Per-process keyed correlation, never a persisted album/channel/user ID.
    let work = work_id
        .map(|id| format!("{:016x}", HASHER.get_or_init(Default::default).hash_one(id)))
        .unwrap_or_default();
    tracing::info_span!(
        "diagnostic_operation",
        diag_operation = name,
        diag_work = work.as_str()
    )
}

pub(crate) fn database_profile(_: &str, duration: Duration) {
    // Do not copy or format SQL (including expanded bound values).
    if duration >= Duration::from_millis(50) {
        tracing::info!(
            diag_stage = "database_slow_statement",
            elapsed_ms = duration.as_millis() as u64
        );
    }
}

/// Normal events across all app modules are retained by callsite. Data is an
/// explicit allowlist, not a best-effort blacklist for arbitrary log messages.
#[derive(Default)]
struct SafeFields(Map<String, Value>);
impl SafeFields {
    fn number(&mut self, field: &Field, value: Value) {
        if matches!(
            field.name(),
            "elapsed_ms"
                | "duration_ms"
                | "wait_ms"
                | "bytes"
                | "pages"
                | "attempt"
                | "status"
                | "schema_version"
                | "count"
                | "active_count"
                | "resumed_jobs"
                | "recovered_entries"
                | "recovered_auto_find_runs"
                | "recovered_duplicate_runs"
                | "recovered_internal_runs"
                | "startup_recovery_issues"
                | "lag_ms"
                | "js_heap_bytes"
                | "long_tasks"
                | "renderer_private_bytes"
                | "line"
                | "column"
                | "eligible"
                | "metadata_requests"
        ) && self.0.len() < 24
        {
            self.0.insert(field.name().into(), value);
        }
    }
}
impl Visit for SafeFields {
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.number(f, json!(v));
    }
    fn record_i64(&mut self, f: &Field, v: i64) {
        self.number(f, json!(v));
    }
    fn record_f64(&mut self, f: &Field, v: f64) {
        if v.is_finite() {
            self.number(f, json!(v));
        }
    }
    fn record_bool(&mut self, f: &Field, v: bool) {
        if matches!(
            f.name(),
            "success"
                | "retry"
                | "retryable"
                | "cancelled"
                | "panicked"
                | "foreground"
                | "allowed"
                | "incremental"
        ) {
            self.0.insert(f.name().into(), json!(v));
        }
    }
    fn record_str(&mut self, f: &Field, v: &str) {
        let safe = match f.name() {
            "diag_operation" | "diag_stage" => {
                !v.is_empty()
                    && v.len() <= 64
                    && v.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            }
            "error_code" => {
                !v.is_empty()
                    && v.len() <= 64
                    && v.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            }
            "diag_work" => v.len() == 16 && v.bytes().all(|b| b.is_ascii_hexdigit()),
            _ => false,
        };
        if safe {
            self.0.insert(f.name().into(), json!(v));
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

#[derive(Default)]
pub(crate) struct DiagnosticLayer {
    #[cfg(test)]
    captured: Option<std::sync::Arc<Mutex<Vec<Value>>>>,
}
impl DiagnosticLayer {
    fn write(&self, value: Value) {
        #[cfg(test)]
        if let Some(captured) = &self.captured {
            captured.lock().unwrap().push(value);
            return;
        }
        record(value);
    }
}
struct Operation {
    started: Instant,
    fields: Map<String, Value>,
    parent: Option<u64>,
}
impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for DiagnosticLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().name() != "diagnostic_operation" {
            return;
        }
        let mut fields = SafeFields::default();
        attrs.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            let parent = span.parent().map(|s| s.id().into_u64());
            self.write(
                json!({"kind":"operation_started", "traceId":id.into_u64(), "parentTraceId":parent,"fields":fields.0}),
            );
            span.extensions_mut().insert(Operation {
                started: Instant::now(),
                fields: fields.0,
                parent,
            });
        }
    }
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        if !meta.target().starts_with("atsumi") || *meta.level() > tracing::Level::INFO {
            return;
        }
        let mut fields = SafeFields::default();
        event.record(&mut fields);
        let trace = ctx.event_scope(event).and_then(|scope| {
            scope
                .filter(|s| s.metadata().name() == "diagnostic_operation")
                .map(|s| s.id().into_u64())
                .next()
        });
        self.write(
            json!({"kind":"event", "level":meta.level().as_str(),"module":meta.module_path(),"line":meta.line(),"traceId":trace,"fields":fields.0}),
        );
    }
    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            if let Some(op) = span.extensions().get::<Operation>() {
                // Closed is not success: the outcome is a separate event.
                self.write(
                    json!({"kind":"operation_closed", "traceId":id.into_u64(),"parentTraceId":op.parent,
                    "elapsedMs":op.started.elapsed().as_millis() as u64,"fields":op.fields}),
                );
            }
        }
    }
}

/// Own only application-N.jsonl and the lock in this directory. A second
/// process must not truncate a ring being written by the first process.
struct RollingWriter {
    directory: PathBuf,
    slots: usize,
    limit: u64,
    index: usize,
    size: u64,
    file: File,
    _lock: File,
}
impl RollingWriter {
    fn open(directory: &Path, slots: usize, limit: u64) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(directory.join("writer.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        let index = (0..slots)
            .max_by_key(|i| {
                std::fs::metadata(directory.join(format!("application-{i}.jsonl")))
                    .and_then(|m| m.modified())
                    .ok()
            })
            .unwrap_or(0);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(directory.join(format!("application-{index}.jsonl")))?;
        let size = file.metadata()?.len();
        let mut writer = Self {
            directory: directory.into(),
            slots,
            limit,
            index,
            size,
            file,
            _lock: lock,
        };
        // After abrupt process death, finish an incomplete record on its own
        // line, so it cannot corrupt the first complete event of the next run.
        if size > 0 {
            use std::io::{Read, Seek, SeekFrom};
            writer.file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            writer.file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                writer.file.write_all(b"\n")?;
                writer.size += 1;
            }
        }
        Ok(writer)
    }
    fn append(&mut self, event: &Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(event)?;
        if line.len() >= MAX_RECORD || line.len() as u64 >= self.limit {
            return Ok(());
        }
        line.push(b'\n');
        if self.size + line.len() as u64 > self.limit {
            self.sync()?;
            self.index = (self.index + 1) % self.slots;
            self.file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(
                    self.directory
                        .join(format!("application-{}.jsonl", self.index)),
                )?;
            self.size = 0;
        }
        self.file.write_all(&line)?;
        self.size += line.len() as u64;
        Ok(())
    }
    fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;
    #[test]
    fn operations_correlate_nested_events_without_work_ids_or_result_payloads() {
        let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
        let local = DiagnosticLayer {
            captured: Some(captured.clone()),
        };
        let console = tracing_subscriber::fmt::layer()
            .with_filter(tracing_subscriber::filter::LevelFilter::OFF);
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(console).with(local),
            || {
                let _parent = operation("download_receive", Some("sensitive-entry-id")).entered();
                {
                    let _child =
                        operation("download_finalize", Some("sensitive-entry-id")).entered();
                    tracing::info!(
                        diag_stage = "verify_files",
                        success = true,
                        gallery_id = 98765u64,
                        title = "sensitive-album"
                    );
                }
            },
        );
        let rows = captured.lock().unwrap();
        let starts: Vec<_> = rows
            .iter()
            .filter(|r| r["kind"] == "operation_started")
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[1]["parentTraceId"], starts[0]["traceId"]);
        assert_eq!(
            starts[0]["fields"]["diag_work"],
            starts[1]["fields"]["diag_work"]
        );
        assert_eq!(starts[0]["fields"]["diag_work"].as_str().unwrap().len(), 16);
        let event = rows.iter().find(|r| r["kind"] == "event").unwrap();
        assert_eq!(event["traceId"], starts[1]["traceId"]);
        assert_eq!(
            event["fields"],
            json!({"diag_stage":"verify_files","success":true})
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r["kind"] == "operation_closed")
                .count(),
            2
        );
        assert!(!serde_json::to_string(&*rows).unwrap().contains("sensitive"));
        assert!(!serde_json::to_string(&*rows).unwrap().contains("98765"));
    }
    #[test]
    fn ring_is_bounded_and_appends_across_restart_without_touching_other_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("user.txt"), "preserve").unwrap();
        {
            let mut w = RollingWriter::open(temp.path(), 3, 180).unwrap();
            for i in 0..100 {
                w.append(&json!({"sequence":i})).unwrap();
            }
        }
        let bytes = (0..3)
            .map(|i| {
                std::fs::metadata(temp.path().join(format!("application-{i}.jsonl")))
                    .unwrap()
                    .len()
            })
            .sum::<u64>();
        assert!(bytes <= 540);
        {
            let mut w = RollingWriter::open(temp.path(), 3, 180).unwrap();
            w.append(&json!({"nextSession":true})).unwrap();
            w.sync().unwrap();
        }
        let all = (0..3)
            .map(|i| {
                std::fs::read_to_string(temp.path().join(format!("application-{i}.jsonl"))).unwrap()
            })
            .collect::<String>();
        assert!(all.contains("99"));
        assert!(all.contains("nextSession"));
        assert!(!all.contains("\"sequence\":0}"));
        assert_eq!(
            std::fs::read_to_string(temp.path().join("user.txt")).unwrap(),
            "preserve"
        );
        for line in all.lines() {
            serde_json::from_str::<Value>(line).unwrap();
        }
    }
    #[test]
    fn second_writer_cannot_truncate_logs_and_partial_line_is_separated() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("application-0.jsonl"), b"{partial").unwrap();
        let mut writer = RollingWriter::open(temp.path(), 3, 1024).unwrap();
        assert!(RollingWriter::open(temp.path(), 3, 1024).is_err());
        writer.append(&json!({"complete":true})).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("application-0.jsonl")).unwrap(),
            "{partial\n{\"complete\":true}\n"
        );
    }
    #[test]
    fn field_allowlist_never_formats_private_messages_or_debug_payloads() {
        struct MustNotFormat;
        impl std::fmt::Debug for MustNotFormat {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("private Debug data was formatted")
            }
        }
        struct Capture(std::sync::Arc<std::sync::Mutex<Value>>);
        impl<S: Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
                let mut fields = SafeFields::default();
                event.record(&mut fields);
                *self.0.lock().unwrap() = json!(fields.0);
            }
        }
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Value::Null));
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(Capture(captured.clone())),
            || {
                tracing::warn!(diag_stage="download", error_code="HTTP_TIMEOUT", elapsed_ms=42u64, gallery_id=12345u64,
                path="C:/Users/private", cookie="secret", query="private", error=?MustNotFormat, "secret message");
            },
        );
        assert_eq!(
            *captured.lock().unwrap(),
            json!({"diag_stage":"download","error_code":"HTTP_TIMEOUT","elapsed_ms":42})
        );
    }
    #[test]
    fn blocked_disk_queue_is_bounded_and_nonblocking() {
        let (tx, _rx) = mpsc::sync_channel::<Message>(2);
        assert!(tx.try_send(Message::Event(json!({}))).is_ok());
        assert!(tx.try_send(Message::Event(json!({}))).is_ok());
        assert!(matches!(
            tx.try_send(Message::Event(json!({}))),
            Err(mpsc::TrySendError::Full(_))
        ));
    }
}
