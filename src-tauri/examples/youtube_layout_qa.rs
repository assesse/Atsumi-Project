//! Diagnostics-only original public YouTube layout. No account/recording/UI automation.
use atsumi_lib::{platform_media::{PlatformMediaService, PlatformMediaTools}, youtube::original::YouTubeOriginalHost};
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
use tauri::{Manager, Webview};

fn inspect(view: &Webview) -> Value {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if view.eval_with_callback(include_str!("youtube_layout_probe.js"), move |value| { let _ = tx.send(value); }).is_err() { return json!({"probeError":"eval_unavailable"}); }
    let Ok(raw) = rx.recv_timeout(Duration::from_secs(2)) else { return json!({"probeError":"eval_timeout"}); };
    if raw.len() > 48 * 1024 { return json!({"probeError":"result_limit"}); }
    serde_json::from_str(&raw).unwrap_or_else(|_| json!({"probeError":"result_invalid"}))
}
fn aligned(facts: &Value, node: &str, expected: &Value) -> bool {
    ["x", "y", "width", "height"].iter().all(|key| facts[node]["rect"][key].as_f64().zip(expected[key].as_f64()).is_some_and(|(actual, requested)| (actual-requested).abs() <= 1.0))
}
fn observe(app: &tauri::AppHandle, elapsed: u64, requested: &Value) -> Value {
    let views = app.webviews();
    let player = views.values().find(|v| v.label().starts_with("youtube-original-") && !v.label().starts_with("youtube-original-chat-"));
    let chat = views.values().find(|v| v.label().starts_with("youtube-original-chat-"));
    let player_facts = player.map(inspect).unwrap_or(Value::Null);
    let chat_facts = chat.map(inspect).unwrap_or(Value::Null);
    json!({"elapsedSeconds":elapsed,"requested":requested,"playerAligned":aligned(&player_facts,"player",&requested["player"]),"chatAligned":aligned(&chat_facts,"chat",&requested["chat"]),"playerFacts":player_facts,"chatFacts":chat_facts})
}
fn run(app: &tauri::AppHandle, root: &Path) -> Result<Value, String> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let ff = workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let media = PlatformMediaService::new(root.join("data"), PlatformMediaTools {
        yt_dlp: workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"), deno: workspace.join(".runtime/platform-tools/bin/deno.exe"), ffmpeg: ff.join("ffmpeg.exe"), ffprobe: ff.join("ffprobe.exe"),
    }, None)?;
    let host = app.state::<YouTubeOriginalHost>();
    host.set_tray_muted(app, true);
    let opened = host.open(app, media.clone(), "cswFQYam8Ic".into(), uuid::Uuid::new_v4().to_string(), root.join("media"))?;
    let result = (|| -> Result<Value, String> {
        let viewport = json!({"sequence":1,"player":{"x":34,"y":176,"width":951,"height":539},"chat":{"x":993,"y":176,"width":239,"height":539},"visible":true,"muted":true,"occlusions":[]});
        host.viewport(app, &opened.token, serde_json::from_value(viewport.clone()).map_err(|_| "invalid diagnostic viewport")?)?;
        let start = Instant::now(); let mut observations = Vec::new();
        for at in [5, 15, 45] {
            let remaining = Duration::from_secs(at).saturating_sub(start.elapsed());
            if !remaining.is_zero() { thread::sleep(remaining); }
            observations.push(observe(app, start.elapsed().as_secs(), &viewport));
        }
        let resized = json!({"sequence":2,"player":{"x":76,"y":90,"width":750,"height":422},"chat":{"x":834,"y":90,"width":330,"height":422},"visible":true,"muted":true,"occlusions":[]});
        host.viewport(app, &opened.token, serde_json::from_value(resized.clone()).map_err(|_| "invalid resized diagnostic viewport")?)?;
        thread::sleep(Duration::from_secs(5));
        observations.push(observe(app, start.elapsed().as_secs(), &resized));
        let collected = observations.iter().all(|v| v["playerFacts"]["supported"]==true && v["chatFacts"]["supported"]==true);
        let final_aligned = observations.last().is_some_and(|v| v["playerAligned"]==true && v["chatAligned"]==true);
        let no_jobs = media.jobs(Some("youtube")).is_empty();
        Ok(json!({"kind":"youtube_layout_qa","diagnosticsCollected":collected,"finalLayoutAligned":final_aligned,"isolated":true,"muted":true,"accountTested":false,"recordingStarted":false,"noMediaJobs":no_jobs,"observations":observations}))
    })();
    let _ = host.close(app, &opened.token); host.shutdown(app); media.shutdown();
    result
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let root = workspace.join(".runtime/youtube-layout-qa").join(&id);
    std::fs::create_dir_all(root.join("profile"))?; std::fs::create_dir_all(root.join("media"))?;
    std::env::set_var("APPDATA", root.join("profile"));
    let done = Arc::new(AtomicBool::new(false)); let finished = done.clone();
    let mut context = tauri::generate_context!(); context.config_mut().identifier = format!("local.atsumi.youtubelayoutqa.{id}"); context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app = tauri::Builder::default().manage(YouTubeOriginalHost::default()).setup(move |app| {
        tauri::window::WindowBuilder::new(app,"main").title("Isolated YouTube layout diagnostics").inner_size(1266.,782.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle = app.handle().clone(); let deadline = handle.clone(); let deadline_done = done.clone();
        thread::spawn(move || { thread::sleep(Duration::from_secs(75)); if !deadline_done.load(Ordering::Acquire) { eprintln!("YOUTUBE_LAYOUT_QA_DEADLINE"); deadline.state::<YouTubeOriginalHost>().shutdown(&deadline); deadline.exit(3); } });
        thread::spawn(move || {
            let result = run(&handle,&root);
            let collected = result.as_ref().is_ok_and(|value|value["diagnosticsCollected"]==true);
            match result { Ok(value)=>println!("{value}"),Err(_)=>println!("{}",json!({"kind":"youtube_layout_qa","diagnosticsCollected":false,"probeError":"host_setup_or_viewport_failed"})) }
            done.store(true,Ordering::Release); handle.exit(if collected {0}else{2});
        }); Ok(())
    }).build(context)?;
    app.run_return(|_,_|{}); finished.store(true,Ordering::Release); Ok(())
}
