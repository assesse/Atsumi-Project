//! Synthetic VP9/AAC MP4 compatibility only. Never loads user media/accounts.
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{atomic::{AtomicBool, AtomicI32, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};
use tauri::{WebviewBuilder, WebviewUrl};

static RESULT: AtomicI32 = AtomicI32::new(2);
const MAX_MEDIA: u64 = 32 * 1024 * 1024;
const HTML: &str = "<!doctype html><meta charset=utf-8><title>Isolated synthetic VP9 compatibility</title><video autoplay muted controls playsinline src='/synthetic-copy.mp4' width='640' height='360'></video>";
const FACTS: &str = "(()=>{const v=document.querySelector('video');return v?{time:Number.isFinite(v.currentTime)?v.currentTime:null,duration:Number.isFinite(v.duration)?v.duration:null,ready:v.readyState,paused:v.paused,ended:v.ended,error:v.error?.code??null,frames:v.getVideoPlaybackQuality?.().totalVideoFrames??null,width:v.videoWidth,height:v.videoHeight}:null})()";

fn range(header: &str, length: usize) -> Result<Option<(usize, usize)>, ()> {
    let mut values = header.lines().filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("Range"));
    let Some((_, value)) = values.next() else { return Ok(None); };
    if values.next().is_some() || length == 0 { return Err(()); }
    let (start, end) = value.trim().strip_prefix("bytes=").ok_or(())?.split_once('-').ok_or(())?;
    if start.is_empty() || !start.bytes().all(|b| b.is_ascii_digit())
        || !end.bytes().all(|b| b.is_ascii_digit()) { return Err(()); }
    let start: usize = start.parse().map_err(|_| ())?;
    let end = if end.is_empty() { length - 1 } else { end.parse::<usize>().map_err(|_| ())?.min(length - 1) };
    if start > end || start >= length { return Err(()); }
    Ok(Some((start, end)))
}

fn serve(listener: TcpListener, media: Arc<Vec<u8>>, done: Arc<AtomicBool>) {
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut requests = 0;
    while !done.load(Ordering::Acquire) && Instant::now() < deadline && requests < 128 {
        let (mut socket, _) = match listener.accept() {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20)); continue;
            }
            Err(_) => return,
        };
        requests += 1;
        let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = socket.set_write_timeout(Some(Duration::from_secs(2)));
        let mut request = Vec::new(); let mut buffer = [0; 1024];
        while request.len() < 8192 && !request.windows(4).any(|v| v == b"\r\n\r\n") {
            match socket.read(&mut buffer) { Ok(0) | Err(_) => break, Ok(n) => request.extend_from_slice(&buffer[..n]) }
        }
        if request.len() > 8192 || !request.windows(4).any(|v| v == b"\r\n\r\n") { continue; }
        let Ok(header) = std::str::from_utf8(&request) else { continue; };
        let first = header.lines().next().unwrap_or("");
        let (status, mime, body, extra): (&str, &str, &[u8], String) = match first {
            "GET / HTTP/1.1" => ("200 OK", "text/html; charset=utf-8", HTML.as_bytes(), String::new()),
            "GET /synthetic-copy.mp4 HTTP/1.1" => match range(header, media.len()) {
                Ok(Some((start, end))) => ("206 Partial Content", "video/mp4", &media[start..=end], format!("Content-Range: bytes {start}-{end}/{}\r\n", media.len())),
                Ok(None) => ("200 OK", "video/mp4", &media, String::new()),
                Err(()) => ("416 Range Not Satisfiable", "text/plain", &[], format!("Content-Range: bytes */{}\r\n", media.len())),
            },
            _ => ("404 Not Found", "text/plain", &[], String::new()),
        };
        let _ = write!(socket, "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{extra}Cache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; media-src 'self'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n", body.len());
        let _ = socket.write_all(body);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(debug_assertions) { return Err("Debug-only synthetic QA".into()); }
    let id = std::env::args().nth(1).ok_or("Synthetic fixture UUID is required")?;
    if uuid::Uuid::parse_str(&id)?.to_string() != id { return Err("Canonical fixture UUID required".into()); }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().ok_or("Workspace missing")?
        .join(".runtime/youtube-vp9-feasibility").join(id);
    let path = root.join("synthetic-copy.mp4");
    for entry in [&root, &path] {
        let metadata = std::fs::symlink_metadata(entry)?;
        #[cfg(windows)] {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 { return Err("Reparse fixture rejected".into()); }
        }
    }
    let meta = std::fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file() || !(1..=MAX_MEDIA).contains(&meta.len()) { return Err("Invalid synthetic fixture size/type".into()); }
    let media = Arc::new(std::fs::read(path)?);
    let profile = tempfile::Builder::new().prefix("atsumi-vp9-local-").tempdir()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let done = Arc::new(AtomicBool::new(false));
    let server_done = done.clone();
    let server = thread::spawn(move || serve(listener, media, server_done));
    let finish = done.clone(); let fail = done.clone();
    let profile_path = profile.path().join("webview");
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = "local.atsumi.syntheticvp9qa".into();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().setup(move |app| {
        let window = tauri::window::WindowBuilder::new(app, "synthetic-vp9")
            .title("Atsumi synthetic VP9 playback QA").inner_size(640., 400.)
            .position(-16000., -16000.).focused(false).skip_taskbar(true).build()?;
        let expected = origin.clone();
        let view = window.add_child(WebviewBuilder::new("synthetic-video", WebviewUrl::External(origin.parse()?))
            .data_directory(profile_path).focused(false)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(move |url| url.as_str() == format!("{expected}/"))
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
            tauri::LogicalPosition::new(0., 0.), tauri::LogicalSize::new(640., 400.))?;
        let handle = app.handle().clone();
        let deadline = handle.clone(); let timeout_done = done.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(35));
            if !timeout_done.load(Ordering::Acquire) { eprintln!("SYNTHETIC_VP9_DEADLINE"); deadline.exit(3); }
        });
        thread::spawn(move || {
            let start = Instant::now(); let mut first = None; let mut first_frames = None;
            let mut latest = Value::Null; let mut success = false;
            while start.elapsed() < Duration::from_secs(25) {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                if view.eval_with_callback(FACTS, move |v| { let _ = tx.send(v); }).is_ok() {
                    if let Ok(raw) = rx.recv_timeout(Duration::from_secs(2)) {
                        if let Ok(value) = serde_json::from_str::<Value>(&raw) { latest = value; }
                    }
                }
                if latest["error"].is_number() { break; }
                if latest["paused"] == false && latest["ready"].as_u64().unwrap_or(0) >= 2 {
                    if let (Some(time), Some(frames), Some(duration)) = (latest["time"].as_f64(), latest["frames"].as_u64(), latest["duration"].as_f64()) {
                        let initial = *first.get_or_insert(time);
                        let initial_frames = *first_frames.get_or_insert(frames);
                        success = time > initial + 2. && frames >= initial_frames + 30 && (8.0..=12.0).contains(&duration);
                        if success { break; }
                    }
                }
                thread::sleep(Duration::from_millis(250));
            }
            println!("{}", json!({"kind":"synthetic_vp9_mp4_playback_qa","success":success,"synthetic":true,"isolated":true,"productionCaptureTested":false,"facts":latest}));
            RESULT.store(if success { 0 } else { 2 }, Ordering::Release);
            done.store(true, Ordering::Release); handle.exit(if success { 0 } else { 2 });
        });
        Ok(())
    }).build(context);
    let app = match app { Ok(value) => value, Err(error) => { fail.store(true, Ordering::Release); let _ = server.join(); return Err(error.into()); } };
    app.run_return(|_, _| {});
    finish.store(true, Ordering::Release); let _ = server.join(); drop(profile);
    if RESULT.load(Ordering::Acquire) != 0 { return Err("Synthetic VP9 WebView playback not verified".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::range;
    #[test]
    fn fixed_fixture_ranges_reject_invalid_and_duplicate_headers() {
        assert_eq!(range("GET / HTTP/1.1\r\n", 100), Ok(None));
        assert_eq!(range("Range: bytes=0-\r\n", 100), Ok(Some((0, 99))));
        assert_eq!(range("Range: bytes=3-5\r\n", 100), Ok(Some((3, 5))));
        for invalid in ["bytes=-4", "bytes=99-3", "bytes=100-", "bytes=0-1,3-4", "bytes=+1-3", "bytes=1-+3"] {
            assert!(range(&format!("Range: {invalid}\r\n"), 100).is_err());
        }
        assert!(range("Range: bytes=0-1\r\nrange: bytes=2-3\r\n", 100).is_err());
    }
}
