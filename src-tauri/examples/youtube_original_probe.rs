//! Public official YouTube watch/chat probe. Root coordinates execution.
//! Temporary profile; default UA; no login, cookies, media saves or Atsumi data.
//! ATSUMI_YOUTUBE_PROBE_ID may supply a canonical11-character public video ID.
use serde_json::{json, Value};
use std::{
    sync::{atomic::{AtomicBool, AtomicI32, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};
use tauri::{Webview, WebviewBuilder, WebviewUrl};

static RESULT: AtomicI32 = AtomicI32::new(2);
const SNAPSHOT: &str = r#"(() => {
  const p=window.__atsumiYouTubeOriginalProbe;
  if(p)return p.snapshot();
  const h=location.hostname;
  return {version:1,document:h==='accounts.google.com'||h==='accounts.youtube.com'?'auth':h==='consent.google.com'||h==='consent.youtube.com'?'consent':'not_observed',readyState:document.readyState};
})()"#;
fn read(view: &Webview) -> Result<Value, ()> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(SNAPSHOT, move |value| { let _ = tx.send(value); }).map_err(|_| ())?;
    serde_json::from_str(&rx.recv_timeout(Duration::from_secs(3)).map_err(|_| ())?).map_err(|_| ())
}
fn approved(url: &tauri::Url) -> bool {
    url.scheme() == "https" && url.port().is_none() && url.username().is_empty() && url.password().is_none() &&
        matches!(url.host_str(), Some("www.youtube.com" | "youtube.com" | "accounts.google.com" | "accounts.youtube.com" | "consent.google.com" | "consent.youtube.com"))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id = std::env::var("ATSUMI_YOUTUBE_PROBE_ID").unwrap_or_else(|_| "cswFQYam8Ic".into());
    if id.len() != 11 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) {
        return Err("ATSUMI_YOUTUBE_PROBE_ID must be a canonical11-character public video ID".into());
    }
    let profile = tempfile::Builder::new().prefix("atsumi-youtube-original-").tempdir()?;
    let profile_path = profile.path().join("webview-profile");
    let done = Arc::new(AtomicBool::new(false));
    let final_done = done.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().setup(move |app| {
        let window = tauri::window::WindowBuilder::new(app, "youtube-original-probe")
            .title("Isolated YouTube original UI probe").inner_size(1400.0, 900.0).position(-16000.0, -16000.0)
            .skip_taskbar(true).focused(false).visible(true).build()?;
        let watch = window.add_child(WebviewBuilder::new("youtube-original-watch", WebviewUrl::External(format!("https://www.youtube.com/watch?v={id}&autoplay=1&mute=1").parse()?))
            .focused(false).data_directory(profile_path.clone()).initialization_script(include_str!("youtube_original_probe.js"))
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(approved).on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
            tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(1040.0, 900.0))?;
        let chat = window.add_child(WebviewBuilder::new("youtube-original-chat", WebviewUrl::External(format!("https://www.youtube.com/live_chat?v={id}&is_popout=1").parse()?))
            .focused(false).data_directory(profile_path).initialization_script(include_str!("youtube_original_probe.js"))
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(approved).on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
            tauri::LogicalPosition::new(1040.0, 0.0), tauri::LogicalSize::new(360.0, 900.0))?;
        let handle = app.handle().clone();
        let deadline_handle = handle.clone(); let deadline_done = done.clone();
        thread::spawn(move || { thread::sleep(Duration::from_secs(70)); if !deadline_done.load(Ordering::Acquire) { eprintln!("YOUTUBE_ORIGINAL_PROBE_DEADLINE"); deadline_handle.exit(3); } });
        thread::spawn(move || {
            let started = Instant::now(); let mut first_time = None; let mut first_frames = None; let mut progressed = false; let mut decoded_progress = false;
            let mut last_watch = Value::Null; let mut last_chat = Value::Null;
            let mut samples = Vec::new();
            while started.elapsed() < Duration::from_secs(55) {
                if let Ok(value) = read(&watch) {
                    if value["advertisement"] == false && value["video"]["paused"] == false && value["video"]["readyState"].as_u64().unwrap_or(0) >= 2 {
                        if let Some(time) = value["video"]["currentTime"].as_f64() {
                            let first = *first_time.get_or_insert(time);
                            progressed |= time > first + 2.0;
                        }
                        if let Some(frames) = value["video"]["decodedFrames"].as_u64() {
                            let first = *first_frames.get_or_insert(frames);
                            decoded_progress |= frames > first;
                        }
                    }
                    last_watch = value;
                }
                if let Ok(value) = read(&chat) { last_chat = value; }
                if samples.len() < 12 { samples.push(json!({"seconds":started.elapsed().as_secs(),"watch":last_watch,"chat":last_chat})); }
                thread::sleep(Duration::from_secs(5));
            }
            let success = progressed && (first_frames.is_none() || decoded_progress) && last_watch["controls"]["displayed"] == true && last_watch["advertisement"] == false;
            println!("{}", json!({"kind":"youtube_original_probe","sample":id,"isolatedProfile":true,"defaultUserAgent":true,"loginTested":false,"recordingTested":false,"playbackProgressed":progressed,"decodedFramesProgressed":decoded_progress,"success":success,"samples":samples}));
            let code = if success { 0 } else { 2 }; RESULT.store(code, Ordering::Release); done.store(true, Ordering::Release); handle.exit(code);
        });
        Ok(())
    }).build(context)?;
    app.run_return(|_, _| {});
    final_done.store(true, Ordering::Release);
    drop(profile);
    if RESULT.load(Ordering::Acquire) != 0 { return Err("Original YouTube playback was not verified; inspect bounded probe summary".into()); }
    Ok(())
}
