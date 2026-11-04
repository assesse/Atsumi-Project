//! Full production app composition with a fresh data identity and a deliberately
//! unsupported DB schema. The only updater endpoint is an in-process loopback
//! fixture serving INVALID bytes: no installer can run or touch the installed app.
//! Build with --release --features tauri/custom-protocol.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("Use a release build so the production update mode is exercised.".into());
    }
    let id = format!("local.atsumi.updateqa.{}", uuid::Uuid::new_v4().simple());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.runtime/startup-update-qa")
        .join(&id);
    std::fs::create_dir_all(&root)?;
    std::env::set_var("LOCALAPPDATA", root.join("local"));
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA missing")?).join(&id);
    if data.exists() {
        return Err("Refusing to reuse data.".into());
    }
    std::fs::create_dir_all(&data)?;
    let db_path = data.join("atsumi-next.sqlite3");
    {
        let db = rusqlite::Connection::open(&db_path)?;
        db.execute_batch("PRAGMA journal_mode=WAL;
          CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);
          INSERT INTO schema_migrations VALUES(999999,'qa_future_schema','2026-11-04T00:00:00Z');
          CREATE TABLE qa_preservation(value TEXT NOT NULL);
          INSERT INTO qa_preservation VALUES('preserve-me');
          PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    let before = std::fs::read(&db_path)?;
    std::fs::write(root.join("database-before.sqlite3"), &before)?;
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = id.clone();
    context.config_mut().build.dev_url = None;
    context.package_info_mut().version = "0.0.1".parse()?;
    for window in &mut context.config_mut().app.windows {
        window.title = "Atsumi · 업데이트 장애 검증 (격리)".into();
        window.data_directory = Some(root.join("webview"));
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;
    let endpoint = format!("http://{address}/latest.json");
    let updater = context
        .config_mut()
        .plugins
        .0
        .get_mut("updater")
        .ok_or("updater config missing")?;
    updater["endpoints"] = serde_json::json!([endpoint]);
    updater["dangerousInsecureTransportProtocol"] = true;
    let download = format!("http://{address}/invalid-update.exe");
    let metadata = serde_json::json!({
        "version":"99.0.0", "notes":"격리 검증 · 서명이 잘못된 테스트 파일입니다. 실제 설치는 실행되지 않습니다.",
        "platforms":{"windows-x86_64":{"url":download,"signature":"aW52YWxpZA=="}}
    }).to_string();
    let server_root = root.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let mut request = [0; 4096];
            let Ok(length) = stream.read(&mut request) else {
                continue;
            };
            let route = String::from_utf8_lossy(&request[..length])
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(server_root.join("requests.jsonl"))
            {
                let _ = writeln!(log, "{}", serde_json::json!({"request":route}));
            }
            let (status, body) = if route.starts_with("GET /latest.json ") {
                ("200 OK", metadata.as_bytes().to_vec())
            } else if route.starts_with("GET /invalid-update.exe ") {
                ("200 OK", vec![0x51; 65536])
            } else {
                ("404 Not Found", Vec::new())
            };
            let _ = write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
            let _ = stream.write_all(&body);
        }
    });
    std::fs::write(
        root.join("identity.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "id":id,"data":data,"root":root,"fixtureEndpoint":endpoint,
            "installedAppTouched":false,"fixtureOnly":true
        }))?,
    )?;
    println!("QA_ROOT={}", root.display());
    atsumi_lib::run_with_context(context)?;
    let db = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.pragma_update(None, "query_only", true)?;
    let value: String = db.query_row("SELECT value FROM qa_preservation", [], |r| r.get(0))?;
    let version: i64 = db.query_row("SELECT max(version) FROM schema_migrations", [], |r| {
        r.get(0)
    })?;
    std::fs::write(
        root.join("preservation.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "sentinelPreserved":value=="preserve-me","schemaUnchanged":version==999999,
            "databaseBytesUnchanged":std::fs::read(&db_path)?==before
        }))?,
    )?;
    Ok(())
}
