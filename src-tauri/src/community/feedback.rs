//! Explicit, private diagnostic reports. Preparing a preview is local-only;
//! the renderer can submit a native snapshot ID, never arbitrary files/JSON.
use super::{
    ensure_session, now_seconds, require_main, vault, Identity, IdentityStore, SupabaseTransport,
    Transport,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tauri::Manager;
use uuid::Uuid;

// Keep reports bounded without losing the incident to routine health samples.
// The server allows 1.25 MiB for jsonb's whitespace-expanded representation.
const MAX_PAYLOAD: usize = 1024 * 1024;
const FILE_TAIL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LINE: usize = 8192;
const MAX_EVENTS: usize = 10_000;
const WINDOW_MS: u64 = 2 * 60 * 60 * 1000;
const DRAFT_LIFETIME_MS: u64 = 30 * 60 * 1000;
const MAX_DRAFTS: usize = 4;
const PROJECT_FILE: &str = "yfpgshvflnawmrimyfzo.v1.dpapi";
static DRAFTS: OnceLock<Mutex<DraftStore>> = OnceLock::new();
static IDENTITY_REQUESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackPrepared {
    id: String,
    event_count: usize,
    bytes: usize,
    created_at: u64,
    payload: Value,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReceipt {
    receipt_id: String,
}
#[derive(Clone)]
struct Draft {
    prepared: FeedbackPrepared,
    source: String,
    comment: Option<String>,
    sending: bool,
    receipt: Option<FeedbackReceipt>,
}
#[derive(Default)]
struct DraftStore(HashMap<Uuid, Draft>);
enum Claim {
    Send(Draft),
    Complete(FeedbackReceipt),
}
impl DraftStore {
    fn insert(
        &mut self,
        prepared: FeedbackPrepared,
        source: String,
        now: u64,
    ) -> Result<(), String> {
        self.0.retain(|_, draft| {
            draft.sending || now.saturating_sub(draft.prepared.created_at) < DRAFT_LIFETIME_MS
        });
        if self.0.len() >= MAX_DRAFTS {
            let oldest = self
                .0
                .iter()
                .filter(|(_, draft)| !draft.sending)
                .min_by_key(|(_, draft)| draft.prepared.created_at)
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else {
                return Err("진단 제보를 전송 중입니다. 잠시 후 다시 준비해 주세요.".into());
            };
            self.0.remove(&oldest);
        }
        let id = Uuid::parse_str(&prepared.id).map_err(|_| "진단 식별자를 만들지 못했습니다.")?;
        self.0.insert(
            id,
            Draft {
                prepared,
                source,
                comment: None,
                sending: false,
                receipt: None,
            },
        );
        Ok(())
    }
    fn claim(&mut self, id: Uuid, comment: &str, now: u64) -> Result<Claim, String> {
        let draft = self
            .0
            .get_mut(&id)
            .ok_or("진단 미리보기가 만료되었습니다. 다시 열어 주세요.")?;
        if now.saturating_sub(draft.prepared.created_at) >= DRAFT_LIFETIME_MS {
            return Err("진단 미리보기가 만료되었습니다. 다시 열어 주세요.".into());
        }
        if draft.sending {
            return Err("이 진단 제보를 이미 전송 중입니다.".into());
        }
        if let Some(receipt) = &draft.receipt {
            return Ok(Claim::Complete(receipt.clone()));
        }
        if draft
            .comment
            .as_ref()
            .is_some_and(|previous| previous != comment)
        {
            return Err("전송 결과를 확인하기 위한 재시도는 같은 설명으로 진행합니다. 설명을 바꾸려면 진단 창을 다시 열어 주세요.".into());
        }
        draft.comment = Some(comment.into());
        draft.sending = true;
        Ok(Claim::Send(draft.clone()))
    }
    fn finish(&mut self, id: Uuid, receipt: Option<FeedbackReceipt>) {
        if let Some(draft) = self.0.get_mut(&id) {
            draft.sending = false;
            draft.receipt = receipt;
        }
    }
}
fn drafts() -> &'static Mutex<DraftStore> {
    DRAFTS.get_or_init(|| Mutex::new(DraftStore::default()))
}
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
struct SendLease(Option<Uuid>);
impl SendLease {
    fn finish(&mut self, receipt: Option<FeedbackReceipt>) {
        if let Some(id) = self.0.take() {
            // Disarm this attempt before making the draft available to a retry.
            // Its later Drop must not clear a newer attempt's sending flag.
            drafts()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .finish(id, receipt);
        }
    }
}
impl Drop for SendLease {
    fn drop(&mut self) {
        let Some(id) = self.0 else { return };
        if let Some(draft) = drafts()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .0
            .get_mut(&id)
        {
            // A closing webview / failed worker must not permanently strand a draft.
            draft.sending = false;
        }
    }
}

fn validate_source(source: &str) -> Result<(), String> {
    if matches!(
        source,
        "hitomi" | "danbooru" | "chzzk" | "youtube" | "soop" | "pinterest" | "pixiv"
    ) {
        Ok(())
    } else {
        Err("올바른 제보 화면을 선택해 주세요.".into())
    }
}
fn validate_comment(comment: &str) -> Result<String, String> {
    let comment = comment.trim();
    if comment.chars().count() > 1000
        || comment
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("설명은 1,000자 이하로 입력해 주세요.".into());
    }
    Ok(comment.into())
}
fn symbol(value: &str, uppercase: bool) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| {
            if uppercase {
                b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
            } else {
                b.is_ascii_lowercase() || b == b'_'
            }
        })
}
fn version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}
fn number(value: &Value, maximum: u64) -> bool {
    value
        .as_f64()
        .is_some_and(|number| number.is_finite() && number >= 0.0 && number <= maximum as f64)
}
fn safe_fields(value: &Value) -> Value {
    let mut output = Map::new();
    for (key, value) in value.as_object().into_iter().flatten() {
        let allowed = match key.as_str() {
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
            | "metadata_requests" => number(value, 1u64 << 53),
            "width" | "height" => value.as_u64().is_some_and(|v| v <= 16384),
            "track_index" => value.as_u64().is_some_and(|v| v <= 1),
            "init_bytes" => value.as_u64().is_some_and(|v| v <= 65536),
            "success" | "retry" | "retryable" | "cancelled" | "panicked" | "foreground"
            | "allowed" | "incremental" => value.is_boolean(),
            "diag_operation" | "diag_stage" => value.as_str().is_some_and(|v| symbol(v, false)),
            "error_code" => value.as_str().is_some_and(|v| symbol(v, true)),
            "capture_reason" => value
                .as_str()
                .is_some_and(crate::diagnostics::safe_capture_reason),
            "capture_mime" => value
                .as_str()
                .is_some_and(crate::diagnostics::safe_capture_mime),
            _ => false,
        };
        if allowed {
            output.insert(key.clone(), value.clone());
        }
    }
    Value::Object(output)
}
fn safe_event(value: &Value, now: u64, sessions: &mut HashMap<String, usize>) -> Option<Value> {
    let time = value.get("timeUnixMs")?.as_u64()?;
    if time > now || now.saturating_sub(time) > WINDOW_MS {
        return None;
    }
    let kind = value.get("kind")?.as_str()?;
    if !matches!(
        kind,
        "event"
            | "operation_started"
            | "operation_closed"
            | "startup"
            | "process_started"
            | "panic"
            | "http_summary"
    ) {
        return None;
    }
    let mut output = json!({"kind":kind, "timeUnixMs":time});
    let object = output.as_object_mut()?;
    if let Some(session) = value
        .get("session")
        .and_then(Value::as_str)
        .filter(|s| Uuid::parse_str(s).is_ok())
    {
        let next = sessions.len() + 1;
        let index = sessions.entry(session.into()).or_insert(next);
        object.insert("session".into(), json!(index));
    }
    for key in [
        "line",
        "dropped",
        "traceId",
        "parentTraceId",
        "elapsedMs",
        "requests",
        "failures",
        "bytes",
        "totalMs",
        "maxMs",
    ] {
        if let Some(value) = value.get(key).filter(|v| number(v, 1u64 << 53)) {
            object.insert(key.into(), value.clone());
        }
    }
    if let Some(fields) = value.get("fields") {
        object.insert("fields".into(), safe_fields(fields));
    }
    if let Some(v) = value
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| version(v))
    {
        object.insert("version".into(), json!(v));
    }
    if let Some(v) = value
        .get("level")
        .and_then(Value::as_str)
        .filter(|v| matches!(*v, "INFO" | "WARN" | "ERROR"))
    {
        object.insert("level".into(), json!(v));
    }
    if let Some(v) = value
        .get("provider")
        .and_then(Value::as_str)
        .filter(|v| matches!(*v, "hitomi" | "danbooru" | "community"))
    {
        object.insert("provider".into(), json!(v));
    }
    if let Some(v) = value
        .get("stage")
        .and_then(Value::as_str)
        .filter(|v| symbol(v, false))
    {
        object.insert("stage".into(), json!(v));
    }
    if let Some(v) = value.get("debug").filter(|v| v.is_boolean()) {
        object.insert("debug".into(), v.clone());
    }
    if let Some(v) = value.get("module").and_then(Value::as_str).filter(|v| {
        v.starts_with("atsumi_lib::")
            && v.len() <= 160
            && v.bytes()
                .all(|b| b.is_ascii_lowercase() || matches!(b, b'_' | b':'))
    }) {
        object.insert("module".into(), json!(v));
    }
    Some(output)
}
fn regular_unlinked(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}
fn report_priority(event: &Value) -> u8 {
    let fields = &event["fields"];
    let stage = fields["diag_stage"].as_str().unwrap_or_default();
    if event["kind"] == "panic"
        || matches!(event["level"].as_str(), Some("WARN" | "ERROR"))
        || fields.get("error_code").is_some()
        || fields["success"] == false
        || event["failures"].as_u64().is_some_and(|n| n > 0)
        || stage.starts_with("chzzk_auth_")
        || stage.starts_with("chzzk_account_")
    {
        0
    } else if matches!(stage, "health" | "ui_health" | "webview_memory") {
        2
    } else {
        1
    }
}

fn collect_payload(directory: &Path, source: &str, now: u64) -> Result<Value, String> {
    validate_source(source)?;
    match std::fs::symlink_metadata(directory) {
        Ok(metadata) if !metadata.is_dir() || !regular_unlinked(&metadata) => {
            return Err("진단 로그 위치를 안전하게 확인하지 못했습니다.".into())
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err("진단 로그를 읽지 못했습니다.".into())
        }
        _ => {}
    }
    let mut events = Vec::new();
    let mut sessions = HashMap::new();
    let mut truncated = false;
    for slot in 0..4 {
        let path = directory.join(format!("application-{slot}.jsonl"));
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err("진단 로그 일부를 읽지 못했습니다. 잠시 후 다시 열어 주세요.".into())
            }
        };
        if !metadata.is_file() || !regular_unlinked(&metadata) {
            continue;
        }
        let mut file = File::open(&path).map_err(|_| "진단 로그를 열지 못했습니다.")?;
        let start = metadata.len().saturating_sub(FILE_TAIL_BYTES);
        truncated |= start > 0;
        file.seek(SeekFrom::Start(start))
            .map_err(|_| "진단 로그를 읽지 못했습니다.")?;
        let mut bytes = Vec::new();
        file.take(FILE_TAIL_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|_| "진단 로그를 읽지 못했습니다.")?;
        for (index, line) in bytes.split(|b| *b == b'\n').enumerate() {
            if (start > 0 && index == 0) || line.len() > MAX_LINE {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            if let Some(event) = safe_event(&value, now, &mut sessions) {
                events.push(event);
            }
        }
    }
    events.sort_by_key(|event| {
        (
            report_priority(event),
            std::cmp::Reverse(event["timeUnixMs"].as_u64().unwrap_or_default()),
        )
    });
    let mut selected = Vec::new();
    let mut size = 512; // Reserve the payload envelope and array punctuation.
    for event in events {
        let event_bytes = serde_json::to_vec(&event)
            .map_err(|_| "진단 정보를 정리하지 못했습니다.")?
            .len()
            + 1;
        if selected.len() >= MAX_EVENTS {
            truncated = true;
            break;
        }
        if size + event_bytes > MAX_PAYLOAD {
            truncated = true;
            continue;
        }
        selected.push(event);
        size += event_bytes;
    }
    selected.sort_by_key(|event| event["timeUnixMs"].as_u64().unwrap_or_default());
    let payload = json!({"schemaVersion":1,"appVersion":env!("CARGO_PKG_VERSION"),"source":source,"createdAt":now,
        "windowMinutes":120,"eventCount":selected.len(),"truncated":truncated,"events":selected});
    if serde_json::to_vec(&payload)
        .map_err(|_| "진단 정보를 정리하지 못했습니다.")?
        .len()
        > MAX_PAYLOAD
    {
        return Err("진단 정보를 제한된 크기로 정리하지 못했습니다.".into());
    }
    Ok(payload)
}
fn prepare(directory: &Path, source: &str, now: u64) -> Result<FeedbackPrepared, String> {
    let payload = collect_payload(directory, source, now)?;
    Ok(FeedbackPrepared {
        id: Uuid::new_v4().to_string(),
        event_count: payload["eventCount"].as_u64().unwrap_or_default() as usize,
        bytes: serde_json::to_vec(&payload)
            .map_err(|_| "진단 정보를 정리하지 못했습니다.")?
            .len(),
        created_at: now,
        payload,
    })
}
fn submit(
    store: &impl IdentityStore,
    transport: &impl Transport,
    draft: &Draft,
) -> Result<FeedbackReceipt, String> {
    // Unlike a community author, an unconfirmed feedback identity owns no work
    // that the user must recover: signup completes durably before the report
    // RPC is sent. A timed-out signup or an app exit can leave an orphan auth
    // account, but must not permanently disable the error-reporting feature.
    // Only this explicit feedback send may retry issuance; never reset Ready
    // identities, unreadable vaults, or the separate community author vault.
    if matches!(store.load()?, Identity::Issuing) {
        store.save(&Identity::Unissued)?;
    }
    let session = ensure_session(store, transport, now_seconds(), true)?;
    let result = transport
        .post(
            "rest/v1/rpc/feedback_v1_submit",
            json!({"p_id":draft.prepared.id,"p_source":draft.source,
        "p_comment":draft.comment.as_deref().unwrap_or(""),"p_payload":draft.prepared.payload}),
            Some(&session.access_token),
        )
        .map_err(|error| error.message.replace("커뮤니티", "진단 제보"))?;
    let receipt = result
        .get("receiptId")
        .and_then(Value::as_str)
        .filter(|v| Uuid::parse_str(v).is_ok())
        .ok_or("제보 접수번호를 확인하지 못했습니다. 같은 내용으로 다시 시도해 주세요.")?;
    Ok(FeedbackReceipt {
        receipt_id: receipt.into(),
    })
}
fn identity_path(local_data: &Path) -> PathBuf {
    local_data.join("feedback-identity").join(PROJECT_FILE)
}

#[tauri::command]
pub async fn feedback_prepare(
    window: tauri::Webview,
    source: String,
) -> Result<FeedbackPrepared, String> {
    require_main(&window)?;
    validate_source(&source)?;
    // No account, WebView profile, database, arbitrary path, or network on preview.
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("진단 로그 위치를 확인하지 못했습니다.")?
        .join("Atsumi/Logs/diagnostics");
    tauri::async_runtime::spawn_blocking(move || {
        crate::diagnostics::flush();
        let now = now_millis();
        let prepared = prepare(&directory, &source, now)?;
        drafts()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(prepared.clone(), source, now)?;
        Ok(prepared)
    })
    .await
    .map_err(|_| "진단 미리보기를 준비하지 못했습니다.")?
}
#[tauri::command]
pub async fn feedback_send(
    window: tauri::Webview,
    draft_id: String,
    comment: String,
) -> Result<FeedbackReceipt, String> {
    require_main(&window)?;
    let id = Uuid::parse_str(&draft_id).map_err(|_| "진단 미리보기가 올바르지 않습니다.")?;
    let comment = validate_comment(&comment)?;
    let claim = drafts()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .claim(id, &comment, now_millis())?;
    let draft = match claim {
        Claim::Complete(receipt) => return Ok(receipt),
        Claim::Send(draft) => draft,
    };
    let lease = SendLease(Some(id));
    let path = identity_path(
        &window
            .app_handle()
            .path()
            .app_local_data_dir()
            .map_err(|_| "진단 제보 키 저장 위치를 확인하지 못했습니다.")?,
    );
    let guard = IDENTITY_REQUESTS.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        let mut lease = lease;
        let _guard = guard;
        let result = (|| {
            let store = vault::FileVault::open(path)?;
            submit(&store, &SupabaseTransport::new()?, &draft)
        })();
        lease.finish(result.as_ref().ok().cloned());
        result
    })
    .await
    .map_err(|_| "진단 제보 전송을 완료하지 못했습니다. 다시 시도해 주세요.")?
}

#[cfg(test)]
mod tests;
