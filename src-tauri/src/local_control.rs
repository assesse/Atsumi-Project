//! Loopback-only rescue channel. No shell, arbitrary JS, file deletion, or process kill endpoint.
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Manager, State};

pub struct LocalControl {
    directory: PathBuf,
    checkpoint: Mutex<Option<Value>>,
}

fn valid_checkpoint(value: &Value) -> bool {
    value.get("version").and_then(Value::as_u64) == Some(1)
        && value.get("savedAt").and_then(Value::as_u64).is_some()
        && value
            .get("tabs")
            .and_then(Value::as_array)
            .is_some_and(|a| a.len() <= 64)
        && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 128 * 1024)
}

#[tauri::command]
pub fn work_checkpoint_get(state: State<'_, Arc<LocalControl>>) -> Result<Option<Value>, String> {
    // A slow disk save must never make the invoking UI thread wait for this lock.
    state
        .checkpoint
        .try_lock()
        .map(|value| value.clone())
        .map_err(|_| "Checkpoint save in progress".into())
}

#[tauri::command]
pub async fn work_checkpoint_save(
    state: State<'_, Arc<LocalControl>>,
    checkpoint: Value,
) -> Result<(), String> {
    if !valid_checkpoint(&checkpoint) {
        return Err("Invalid or oversized navigation checkpoint".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut current = state
            .checkpoint
            .lock()
            .map_err(|_| "Checkpoint lock unavailable")?;
        if current
            .as_ref()
            .and_then(|v| v.get("savedAt"))
            .and_then(Value::as_u64)
            > checkpoint.get("savedAt").and_then(Value::as_u64)
        {
            return Ok(());
        }
        let temporary = state.directory.join("navigation-checkpoint.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec(&checkpoint).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(
            temporary,
            state.directory.join("navigation-checkpoint.json"),
        )
        .map_err(|e| e.to_string())?;
        *current = Some(checkpoint);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("diagnostics");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let checkpoint_path = directory.join("navigation-checkpoint.json");
    let checkpoint = std::fs::metadata(&checkpoint_path)
        .ok()
        .filter(|m| m.len() <= 128 * 1024)
        .and_then(|_| std::fs::read(checkpoint_path).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(valid_checkpoint);
    app.manage(Arc::new(LocalControl {
        directory: directory.clone(),
        checkpoint: Mutex::new(checkpoint),
    }));
    let listener =
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let descriptor = json!({"pid":std::process::id(),"url":format!("http://{address}"),"token":token,"version":1});
    std::fs::write(
        directory.join("control-endpoint.json"),
        serde_json::to_vec(&descriptor).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("atsumi-local-control".into())
        .spawn(move || {
            let workers = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                if workers
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                        (n < 4).then_some(n + 1)
                    })
                    .is_err()
                {
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                    respond(&mut stream, 503, json!({"error":"Control channel busy"}));
                    continue;
                }
                let app = app.clone();
                let token = token.clone();
                let address = address.clone();
                let workers = workers.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    match read_request(&mut stream, &token, &address) {
                        Ok((method, path, body)) => {
                            let (status, result) = route(&app, &method, &path, &body);
                            respond(&mut stream, status, result);
                        }
                        Err(()) => respond(
                            &mut stream,
                            403,
                            json!({"error":"Unauthorized or invalid local request"}),
                        ),
                    }
                    workers.fetch_sub(1, Ordering::AcqRel);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn read_request(
    stream: &mut TcpStream,
    token: &str,
    address: &str,
) -> Result<(String, String, Vec<u8>), ()> {
    let started = std::time::Instant::now();
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 8192
            || started.elapsed() > Duration::from_secs(2)
            || stream.read(&mut byte).map_err(|_| ())? != 1
        {
            return Err(());
        }
        bytes.push(byte[0]);
    }
    let header = std::str::from_utf8(&bytes).map_err(|_| ())?;
    let mut lines = header.split("\r\n");
    let request = lines
        .next()
        .ok_or(())?
        .split_whitespace()
        .collect::<Vec<_>>();
    if request.len() != 3 || request[2] != "HTTP/1.1" || !matches!(request[0], "GET" | "POST") {
        return Err(());
    }
    let mut fields = std::collections::HashMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line.split_once(':').ok_or(())?;
        if fields
            .insert(key.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return Err(());
        }
    }
    if fields.contains_key("origin")
        || fields.contains_key("transfer-encoding")
        || fields.get("host") != Some(&address)
        || fields.get("authorization").copied() != Some(format!("Bearer {token}").as_str())
    {
        return Err(());
    }
    let length = fields
        .get("content-length")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| ())?
        .unwrap_or(0);
    if length > 32768 {
        return Err(());
    }
    let mut body = vec![0; length];
    let mut received = 0;
    while received < length {
        let remaining = Duration::from_secs(2)
            .checked_sub(started.elapsed())
            .filter(|t| !t.is_zero())
            .ok_or(())?;
        stream.set_read_timeout(Some(remaining)).map_err(|_| ())?;
        let read = stream.read(&mut body[received..]).map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        received += read;
    }
    Ok((request[0].into(), request[1].into(), body))
}

fn route(app: &tauri::AppHandle, method: &str, path: &str, body: &[u8]) -> (u16, Value) {
    let recorder = app.state::<Arc<crate::ui_diagnostics::UiDiagnostics>>();
    if method == "GET" && path == "/status" {
        return (
            200,
            json!({"pid":std::process::id(),"health":recorder.snapshot(),"checkpointSavedAt":app.state::<Arc<LocalControl>>().checkpoint.try_lock().ok().and_then(|v|v.as_ref().and_then(|v|v.get("savedAt")).cloned())}),
        );
    }
    if method == "GET" && path == "/checkpoint" {
        return match work_checkpoint_get(app.state()) {
            Ok(value) => (200, value.unwrap_or(Value::Null)),
            Err(error) => (503, json!({"error":error})),
        };
    }
    if method == "GET" && path.split('?').next() == Some("/queue") {
        let query = match queue_query(path) {
            Ok(query) => query,
            Err(()) => return (400, json!({"error":"Invalid queue query"})),
        };
        let path = app
            .path()
            .app_data_dir()
            .unwrap_or_default()
            .join("atsumi-next.sqlite3");
        return match crate::work_console::snapshot(&path, &query) {
            Ok(s) => (200, json!(s)),
            Err(e) => (503, json!({"error":e})),
        };
    }
    let payload: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if method != "POST"
        || payload.get("confirm") != Some(&Value::Bool(true))
        || payload.get("epoch").and_then(Value::as_str) != Some(recorder.epoch().as_str())
    {
        return (
            403,
            json!({"error":"Explicit confirmation and current renderer epoch required"}),
        );
    }
    if path == "/recover-ui" {
        if app
            .try_state::<crate::interface::AppState>()
            .is_some_and(|s| s.is_quitting())
        {
            return (409, json!({"error":"Application is quitting"}));
        }
        crate::renderer_recovery::request_reload(app, "local_control", recorder.epoch(), false);
        recorder.record("local_control_recovery", json!({"requested":true}));
        return (
            202,
            json!({"requested":true,"note":"Rate-limited UI-only reload; observe a new epoch to confirm completion"}),
        );
    }
    if path == "/cancel-downloads" {
        let Some(ids) = payload.get("entryIds").and_then(Value::as_array) else {
            return (400, json!({"error":"entryIds required"}));
        };
        if ids.is_empty()
            || ids.len() > 200
            || ids
                .iter()
                .any(|id| id.as_str().is_none_or(|s| s.len() > 128 || s.is_empty()))
        {
            return (400, json!({"error":"1..200 exact entry IDs required"}));
        }
        let Some(state) = app.try_state::<crate::interface::AppState>() else {
            return (503, json!({"error":"Backend not ready"}));
        };
        let ids = ids
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        // The same transactional, revision-checked command used by the app; no direct SQL mutation.
        return match tauri::async_runtime::block_on(crate::interface::download_cancel(
            app.clone(),
            state,
            ids,
        )) {
            Ok(result) => {
                recorder.record("local_control_cancel", json!({"requested":true}));
                (200, json!(result))
            }
            Err(error) => (409, json!({"error":error})),
        };
    }
    (404, json!({"error":"Unknown operation"}))
}

fn queue_query(path: &str) -> Result<crate::work_console::QueueQuery, ()> {
    let mut result = crate::work_console::QueueQuery::default();
    let mut seen = std::collections::HashSet::new();
    if let Some((_, query)) = path.split_once('?') {
        for part in query.split('&') {
            let (key, value) = part.split_once('=').ok_or(())?;
            if !seen.insert(key) {
                return Err(());
            }
            match key {
                "page" => {
                    let page = value.parse::<u32>().map_err(|_| ())?;
                    if !(1..=100_000).contains(&page) {
                        return Err(());
                    }
                    result.page = page;
                }
                "includeSettled" => {
                    result.include_settled = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(()),
                    }
                }
                _ => return Err(()),
            }
        }
    }
    Ok(result)
}

fn respond(stream: &mut TcpStream, status: u16, value: Value) {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let header=format!("HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",body.len());
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_queue_paging_is_typed_and_bounded() {
        let query = queue_query("/queue?page=2&includeSettled=true").unwrap();
        assert_eq!(query.page, 2);
        assert!(query.include_settled);
        for path in [
            "/queue?page=0",
            "/queue?page=100001",
            "/queue?page=1&page=2",
            "/queue?includeSettled=yes",
            "/queue?sql=delete",
        ] {
            assert!(queue_query(path).is_err());
        }
    }
    fn request(raw: String) -> Result<(String, String, Vec<u8>), ()> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            read_request(&mut stream, "secret", "127.0.0.1:12345")
        });
        let mut sender = TcpStream::connect(address).unwrap();
        let _ = sender.write_all(raw.as_bytes());
        drop(sender);
        worker.join().unwrap()
    }
    #[test]
    fn request_requires_current_token_exact_host_and_no_browser_origin() {
        let header =
            "GET /status HTTP/1.1\r\nHost: 127.0.0.1:12345\r\nAuthorization: Bearer secret\r\n";
        assert!(request(format!("{header}\r\n")).is_ok());
        for changed in [
            header.replace("Bearer secret", "Bearer wrong"),
            header.replace("127.0.0.1:12345", "evil.test"),
            format!("{header}Origin: http://localhost\r\n"),
            format!("{header}Authorization: Bearer secret\r\n"),
            format!("{header}Transfer-Encoding: chunked\r\n"),
        ] {
            assert!(request(format!("{changed}\r\n")).is_err());
        }
    }
    #[test]
    fn request_is_bounded_and_rejects_truncated_bodies() {
        let header="POST /cancel-downloads HTTP/1.1\r\nHost: 127.0.0.1:12345\r\nAuthorization: Bearer secret\r\n";
        assert_eq!(
            request(format!("{header}Content-Length: 2\r\n\r\n{{}}"))
                .unwrap()
                .2,
            b"{}"
        );
        assert!(request(format!("{header}Content-Length: 32769\r\n\r\n")).is_err());
        assert!(request(format!("{header}Content-Length: 5\r\n\r\n{{}}")).is_err());
        assert!(request(format!("{header}X-Padding: {}\r\n\r\n", "x".repeat(8192))).is_err());
    }
    #[test]
    fn checkpoint_rejects_wrong_shape_and_large_payload() {
        assert!(!valid_checkpoint(
            &json!({"version":2,"savedAt":1,"tabs":[]})
        ));
        assert!(valid_checkpoint(
            &json!({"version":1,"savedAt":1,"tabs":[]})
        ));
        assert!(!valid_checkpoint(
            &json!({"version":1,"savedAt":1,"tabs":[],"x":"a".repeat(128*1024)})
        ));
    }
}
