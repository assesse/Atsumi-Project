//! Isolated real-keyboard harness: production DetailWorkspace, temporary WebView2
//! profiles, synthetic images. Never starts Atsumi workers or opens its database.
use std::{
    io::{Read, Write},
    time::Duration,
};
use tauri::Manager;
#[path = "../src/native_focus.rs"]
mod native_focus;
fn log(line: &str) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.runtime/detail-keyboard-probe/events.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}
fn main() {
    log("START");
    let native_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.runtime/detail-keyboard-probe/native.log"),
        )
        .unwrap();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(native_log)
        .init();
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.runtime/detail-keyboard-probe");
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/", server.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut bytes = Vec::new();
            let mut chunk = [0; 8192];
            let mut header_end = None;
            let mut content_length = 0;
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if header_end.is_none() {
                    if let Some(at) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        header_end = Some(at + 4);
                        content_length = String::from_utf8_lossy(&bytes[..at])
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                    }
                }
                if header_end.is_some_and(|end| bytes.len() >= end + content_length) {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&bytes);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let (mime, body) = match path {
                "/log" => { log(&format!("DOM {}", String::from_utf8_lossy(&bytes[header_end.unwrap_or(bytes.len())..]))); ("text/plain", Vec::new()) },
                "/fixture.js" => ("text/javascript", std::fs::read(directory.join("fixture.js")).unwrap()),
                "/fixture.css" => ("text/css", std::fs::read(directory.join("fixture.css")).unwrap()),
                "/away" => ("text/html", b"<html><body><h1>Alt+Tab target</h1><p>Keyboard test only. Return to the detail window with Alt+Tab.</p></body></html>".to_vec()),
                _ => ("text/html", std::fs::read(directory.join("index.html")).unwrap()),
            };
            let _ = write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
            let _ = stream.write_all(&body);
        }
    });
    let profiles = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    context.config_mut().build.dev_url = Some(url.parse().unwrap());
    tauri::Builder::default()
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                log(&format!("NATIVE {} focused={focused}", window.label()));
            }
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                window.app_handle().exit(0);
            }
        })
        .setup(move |app| {
            tauri::WebviewWindowBuilder::new(
                app,
                "away",
                tauri::WebviewUrl::External(format!("{url}away").parse().unwrap()),
            )
            .title("Atsumi keyboard test - AltTab target")
            .inner_size(640.0, 480.0)
            .data_directory(profiles.path().join("away"))
            .build()?;
            let main = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse().unwrap()),
            )
            .title("Atsumi keyboard test - Detail")
            .inner_size(1280.0, 820.0)
            .data_directory(profiles.path().join("main"))
            .build()?;
            native_focus::install(&main)?;
            app.manage(profiles);
            Ok(())
        })
        .run(context)
        .expect("keyboard harness");
}
