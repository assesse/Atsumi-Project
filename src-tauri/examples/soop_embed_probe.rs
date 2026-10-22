//! Bounded SOOP iframe playback test, using a disposable WebView2 profile.
//! No Atsumi database, account session, login, stream extraction or recording.
//! cargo run --offline --example soop_embed_probe -- devil0108
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{
    http::{header, Response},
    Webview, WebviewBuilder, WebviewUrl,
};

static RESULT: AtomicI32 = AtomicI32::new(2);
const SCRIPT: &str = include_str!("soop_embed_probe.js");

fn script(view: &Webview, value: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(value, move |value| {
        let _ = tx.send(value);
    })?;
    Ok(serde_json::from_str(
        &rx.recv_timeout(Duration::from_secs(3))?,
    )?)
}
fn response(bytes: &[u8]) -> Response<Vec<u8>> {
    Response::builder().status(200).header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; frame-src https://play.sooplive.com; object-src 'none'; base-uri 'none'")
        .body(bytes.to_vec()).unwrap()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let channel = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "devil0108".into());
    assert!(
        (1..=40).contains(&channel.len())
            && channel
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    );
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>html,body,iframe{{margin:0;width:100%;height:100%;border:0;background:#000}}</style></head><body><iframe id="player" title="SOOP public live" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen referrerpolicy="strict-origin-when-cross-origin" src="https://play.sooplive.com/{channel}/embed"></iframe></body></html>"#
    );
    let profile = tempfile::Builder::new()
        .prefix("atsumi-soop-embed-")
        .tempdir()?;
    let profile_path = profile.path().join("webview-profile");
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().register_uri_scheme_protocol("tauri", move |_, request| {
        if request.uri().path()=="/soop-probe.html" {response(html.as_bytes())} else {Response::builder().status(404).body(Vec::new()).unwrap()}
    }).setup(move |app| {
        let window=tauri::window::WindowBuilder::new(app,"main").title("Isolated SOOP embed probe").inner_size(960.0,540.0).position(-16000.0,-16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
        let view=window.add_child(WebviewBuilder::new("main",WebviewUrl::External("http://tauri.localhost/soop-probe.html".parse().unwrap()))
            .data_directory(profile_path).focused(false).initialization_script_for_all_frames(SCRIPT)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-networking --disable-component-update --disable-sync --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny),tauri::LogicalPosition::new(0.0,0.0),tauri::LogicalSize::new(960.0,540.0))?;
        let app=app.handle().clone();let deadline_app=app.clone();let deadline_done=done.clone();
        thread::spawn(move||{thread::sleep(Duration::from_secs(100));if !deadline_done.load(Ordering::Acquire){eprintln!("SOOP_EMBED_PROBE_DEADLINE");deadline_app.exit(3);}});
        thread::spawn(move||{
            let start=Instant::now();let mut last=Value::Null;let mut code=2;let mut baseline:Option<(f64,u64)>=None;let mut timeline=Vec::new();
            while start.elapsed()<Duration::from_secs(90){
                match script(&view,"window.soopProbe||{}") {
                    Ok(value)=>{
                        last=value;
                        if let Some(videos)=last["frame"]["videos"].as_array(){
                            if let Some(video)=videos.iter().find(|v|v["id"]=="livePlayer"&&v["ad"]==false&&v["width"].as_u64().unwrap_or(0)>0&&v["height"].as_u64().unwrap_or(0)>0&&v["paused"]==false&&v["decoded"].as_u64().unwrap_or(0)>30){
                                let time=video["time"].as_f64().unwrap_or(0.0);
                                let decoded=video["decoded"].as_u64().unwrap_or(0);
                                if timeline.len()<12{timeline.push(json!({"at":start.elapsed().as_secs_f32(),"time":time,"decoded":decoded}));}
                                if let Some((previous_time,previous_decoded))=baseline{
                                    if time-previous_time>2.0&&decoded>previous_decoded+20{code=0;break;}
                                    if time<previous_time||decoded<previous_decoded{baseline=Some((time,decoded));}
                                }else{baseline=Some((time,decoded));}
                            }
                        }
                    },
                    Err(_)=>eprintln!("SOOP_EMBED_PROBE_EVAL_FAILED"),
                }
                thread::sleep(Duration::from_millis(700));
            }
            println!("{}",json!({"kind":"soop_embed_probe","success":code==0,"proof":"original_non_ad_playback_only","captureValidated":false,"externalAgentAbsenceVerified":false,"isolatedProfile":true,"muted":true,"unfocused":true,"channel":channel,"elapsedSeconds":start.elapsed().as_secs(),"timeline":timeline,"state":last}));
            RESULT.store(code,Ordering::Release);done.store(true,Ordering::Release);app.exit(code);
        });Ok(())
    }).build(context)?;
    app.run_return(|_, _| {});
    finished.store(true, Ordering::Release);
    drop(profile);
    let code = RESULT.load(Ordering::Acquire);
    if code != 0 {
        return Err(format!("SOOP playback verification failed with code {code}").into());
    }
    Ok(())
}
