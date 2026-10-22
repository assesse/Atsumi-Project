//! Bounded public YouTube iframe smoke test with a disposable WebView2 profile.
//! No Atsumi state, user profile, cookies, login, recording or account mutation.
//! cargo run --offline --example youtube_embed_probe
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
const SCRIPT: &str = r#"(() => {
  const frame=document.getElementById('player');
  window.youtubeProbe={origin:location.origin,ready:false,state:null,time:0,error:null,messages:0};
  window.addEventListener('message',e=>{
    if(e.origin!=='https://www.youtube.com'||e.source!==frame.contentWindow)return;
    let m;try{m=typeof e.data==='string'?JSON.parse(e.data):e.data}catch{return;}
    if(!m||typeof m!=='object')return;
    const p=window.youtubeProbe;p.messages++;
    if(m.event==='onReady'||m.event==='initialDelivery')p.ready=true;
    if(m.event==='onError')p.error=m.info;
    if(m.event==='onStateChange')p.state=m.info;
    if(m.info&&typeof m.info==='object'){
      if(typeof m.info.currentTime==='number')p.time=m.info.currentTime;
      if(typeof m.info.playerState==='number')p.state=m.info.playerState;
    }
  });
  const timer=setInterval(()=>{
    frame.contentWindow.postMessage(JSON.stringify({event:'listening',id:'atsumi-youtube'}),'https://www.youtube.com');
    for(const name of ['onReady','onError','onStateChange'])frame.contentWindow.postMessage(JSON.stringify({event:'command',func:'addEventListener',args:[name]}),'https://www.youtube.com');
    frame.contentWindow.postMessage(JSON.stringify({event:'command',func:'mute',args:[]}),'https://www.youtube.com');
  },600);
  setTimeout(()=>clearInterval(timer),55000);
})();"#;
const HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><style>html,body,iframe{margin:0;width:100%;height:100%;border:0;background:#000}</style></head><body>
<iframe id="player" title="YouTube public sample" sandbox="allow-scripts allow-same-origin allow-presentation" allow="autoplay; encrypted-media; fullscreen; picture-in-picture" referrerpolicy="strict-origin-when-cross-origin" src="https://www.youtube.com/embed/M7lc1UVf-VE?enablejsapi=1&amp;playsinline=1&amp;autoplay=1&amp;mute=1&amp;origin=http%3A%2F%2Ftauri.localhost"></iframe><script src="http://tauri.localhost/probe.js"></script></body></html>"#;
fn script(view: &Webview, value: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(value, move |value| {
        let _ = tx.send(value);
    })?;
    Ok(serde_json::from_str(
        &rx.recv_timeout(Duration::from_secs(3))?,
    )?)
}
fn response(bytes: &[u8], content_type: &str) -> Response<Vec<u8>> {
    Response::builder().status(200).header(header::CONTENT_TYPE, content_type).header("Content-Security-Policy", "default-src 'none'; script-src http://tauri.localhost; style-src 'unsafe-inline'; frame-src https://www.youtube.com; object-src 'none'; base-uri 'none'").body(bytes.to_vec()).unwrap()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile = tempfile::Builder::new()
        .prefix("atsumi-youtube-embed-")
        .tempdir()?;
    let profile_path = profile.path().join("webview-profile");
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().register_uri_scheme_protocol("tauri", |_, request| {
        match request.uri().path() { "/probe.html" => response(HTML.as_bytes(), "text/html; charset=utf-8"), "/probe.js" => response(SCRIPT.as_bytes(), "text/javascript; charset=utf-8"), _ => Response::builder().status(404).body(Vec::new()).unwrap() }
    }).setup(move |app| {
        let window = tauri::window::WindowBuilder::new(app, "main").title("Isolated YouTube embed probe").inner_size(960.0, 540.0).position(-16000.0,-16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
        let view = window.add_child(WebviewBuilder::new("main", WebviewUrl::External("http://tauri.localhost/probe.html".parse().unwrap())).data_directory(profile_path).additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-networking --disable-component-update --disable-sync --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows").on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny),tauri::LogicalPosition::new(0.0,0.0),tauri::LogicalSize::new(960.0,540.0))?;
        let app = app.handle().clone(); let deadline_app = app.clone(); let deadline_done = done.clone();
        thread::spawn(move || { thread::sleep(Duration::from_secs(65)); if !deadline_done.load(Ordering::Acquire) { eprintln!("YOUTUBE_EMBED_PROBE_DEADLINE"); deadline_app.exit(3); } });
        thread::spawn(move || {
            let start=Instant::now();let mut last=Value::Null;let mut code=2;
            while start.elapsed()<Duration::from_secs(55) {
                match script(&view,"window.youtubeProbe||{}") {
                    Ok(value) => {
                        last = value;
                        if !last["error"].is_null() { break; }
                        if last["state"] == 1 && last["time"].as_f64().is_some_and(|v| v > 2.0) { code = 0; break; }
                    },
                    Err(error)=>{eprintln!("YOUTUBE_EMBED_PROBE_EVAL: {error}");}
                }
                thread::sleep(Duration::from_millis(500));
            }
            println!("{}",json!({"kind":"youtube_embed_probe","success":code==0,"isolatedProfile":true,"sample":"M7lc1UVf-VE","state":last}));
            RESULT.store(code,Ordering::Release);done.store(true,Ordering::Release);app.exit(code);
        }); Ok(())
    }).build(context)?;
    app.run_return(|_, _| {});
    finished.store(true, Ordering::Release);
    drop(profile);
    let result = RESULT.load(Ordering::Acquire);
    if result != 0 {
        return Err(format!("YouTube playback verification failed with code {result}").into());
    }
    Ok(())
}
