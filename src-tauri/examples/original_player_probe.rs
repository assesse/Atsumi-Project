//! Actual original CHZZK SDK, opaque iframe and production Range transport.
//! Disposable synthetic media/profile only; no AppState, user DB or accounts.
//! cargo run --offline --example original_player_probe
#[cfg(windows)]
#[allow(dead_code)]
#[path = "chzzk_replay_probe.rs"]
mod replay_probe_support;

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows/WebView2 required");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("ORIGINAL_PLAYER_PROBE_FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use super::replay_probe_support::probe::prepare;
    use atsumi_lib::streaming::original_player;
    use base64::Engine;
    use serde_json::{json, Value};
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };
    use tauri::{
        http::{header, Request, Response, StatusCode},
        Webview, WebviewBuilder, WebviewUrl,
    };

    const DEADLINE: Duration = Duration::from_secs(65);
    const MAX_BODY: usize = 1024 * 1024;
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    // The privileged test host has no unsafe-eval, matching the real app.
    const HOST_CSP: &str = "default-src 'none'; script-src http://tauri.localhost; \
        style-src 'unsafe-inline'; frame-src http://atsumi-player.localhost; \
        connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action 'none'";
    const HOST_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Original player isolated probe</title>
        <style>html,body,iframe{margin:0;width:100%;height:100%;border:0;overflow:hidden;background:#000}</style></head>
        <body><iframe id="player" sandbox="allow-scripts" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen></iframe>
        <script src="http://tauri.localhost/probe.js"></script></body></html>"#;
    const HOST_SCRIPT: &str = r#"(() => {
      const nonce=__NONCE__,url=__MEDIA_URL__,frame=document.getElementById('player');
      const channel='atsumi-replay-player-v1';
      window.originalProbe={ready:false,state:null,diagnostics:null,error:null,hostIpc:null};
      window.__TAURI_INTERNALS__.invoke('atsumi_original_probe_ping').then(v=>window.originalProbe.hostIpc=v).catch(e=>window.originalProbe.error=String(e));
      window.originalSend=(type,data)=>frame.contentWindow.postMessage({channel,nonce,type,data},'*');
      window.originalInspect=(action='inspect')=>frame.contentWindow.postMessage({channel:'atsumi-original-player-probe',nonce,action},'*');
      window.addEventListener('message',event=>{
        if(event.source!==frame.contentWindow||event.origin!=='null'||event.data?.nonce!==nonce)return;
        const m=event.data;
        if(m.channel==='atsumi-original-player-probe'){window.originalProbe.diagnostics=m.data;return;}
        if(m.channel!==channel)return;
        if(m.type==='ready'){window.originalProbe.ready=true;window.originalSend('init',{url,duration:8,privacy:false,recording:{title:'합성 저장 방송',channelName:'합성 저장 채널',recordedAt:1789272000000,profileImage:__PROFILE_IMAGE__}});}
        if(m.type==='state')window.originalProbe.state=m.data;
        if(m.type==='error')window.originalProbe.error=m.data||'original runtime error';
      });
      frame.src='http://atsumi-player.localhost/frame.html#'+nonce;
    })();"#;
    // Test-only diagnostics are appended by this example's protocol adapter.
    // They execute inside the actual opaque document, not by bypassing its
    // origin isolation from the parent. No such route exists in production.
    const FRAME_PROBE: &str = r#"(() => {
      const nonce=location.hash.slice(1),channel='atsumi-original-player-probe';
      const violations=[],errors=[];
      document.addEventListener('securitypolicyviolation',e=>{violations.push({directive:e.violatedDirective,blocked:e.blockedURI});if(violations.length>20)violations.shift();});
      window.addEventListener('error',e=>{if(errors.length<12)errors.push(String(e.message||e.target?.src||e.target?.href||'resource load failed'));},true);
      window.addEventListener('unhandledrejection',e=>{if(errors.length<12)errors.push(String(e.reason));});
      const inaccessible=fn=>{try{fn();return false;}catch{return true;}};
      const inspect=()=>{
        const video=document.querySelector('video');
        return {time:video?.currentTime??null,paused:video?.paused??null,seeking:video?.seeking??null,
          rate:video?.playbackRate??null,ready:video?.readyState??null,width:video?.videoWidth??null,height:video?.videoHeight??null,
          src:video?.getAttribute('src')??null,crossorigin:video?.getAttribute('crossorigin')??null,
          controls:document.querySelectorAll('[class*="pzp-pc__"] button,button[class*="pzp-"]').length,
          savedProfileLoaded:(()=>{const img=document.querySelector('.header_info img');return !!img?.getAttribute('src')?.startsWith('data:image/png;base64,')&&img.naturalWidth>0;})(),
          savedHeader:document.querySelector('.header_info')?.textContent??'',
          parentBlocked:inaccessible(()=>parent.document.body),storageBlocked:inaccessible(()=>localStorage.length),
          cookieBlocked:inaccessible(()=>document.cookie),storageIsMemory:(()=>{try{return !(localStorage instanceof Storage)&&!(sessionStorage instanceof Storage);}catch{return false;}})(),
          tauriAbsent:typeof window.__TAURI_INTERNALS__==='undefined',origin:window.origin,
          violationCount:violations.length,violations,errors,error:video?.error?.message??null};
      };
      window.addEventListener('message',e=>{
        const m=e.data;if(e.source!==parent||m?.channel!==channel||m.nonce!==nonce)return;
        const video=document.querySelector('video');
        if(m.action==='play'&&video){video.muted=true;video.play().catch(()=>{});}
        if(m.action==='rate2'&&video)video.playbackRate=2;
        if(m.action==='pause'&&video)video.pause();
        if(m.action==='egress')fetch('https://example.invalid/blocked').catch(()=>{});
        parent.postMessage({channel,nonce,data:inspect()},'*');
      });
    })();"#;

    fn script(view: &Webview, javascript: &str) -> Result<Value> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.eval_with_callback(javascript, move |value| {
            let _ = tx.send(value);
        })?;
        Ok(serde_json::from_str(
            &rx.recv_timeout(Duration::from_secs(3))?,
        )?)
    }
    fn state(view: &Webview) -> Result<Value> {
        script(view, "window.originalInspect?.();window.originalProbe||{}")
    }
    fn wait_state(
        view: &Webview,
        start: Instant,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let mut last = Value::Null;
        while start.elapsed() < DEADLINE {
            let value = state(view)?;
            if !value["error"].is_null() {
                return Err(format!("original player error: {value}").into());
            }
            if predicate(&value) {
                return Ok(value);
            }
            last = value;
            thread::sleep(Duration::from_millis(100));
        }
        Err(format!("original player deadline; last state: {last}").into())
    }

    fn public_response(bytes: Vec<u8>, mime: &str) -> Response<Vec<u8>> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .header(header::CONTENT_LENGTH, bytes.len().to_string())
            .header(header::CACHE_CONTROL, "no-store")
            .header("Content-Security-Policy", HOST_CSP)
            .header("X-Content-Type-Options", "nosniff")
            .body(bytes)
            .unwrap()
    }
    fn request_log(label: &str, request: &Request<Vec<u8>>, response: &Response<Vec<u8>>) -> Value {
        json!({"label":label,"range":request.headers().get(header::RANGE).and_then(|v|v.to_str().ok()),
            "origin":request.headers().get(header::ORIGIN).and_then(|v|v.to_str().ok()),
            "status":response.status().as_u16(),"bodyBytes":response.body().len()})
    }

    pub fn run() -> Result<()> {
        let start = Instant::now();
        let (fixture, token, media_bytes) = prepare(start)?;
        let media_url = format!("http://atsumi-replay.localhost/{token}");
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let controller = HOST_SCRIPT
            .replace("__NONCE__", &serde_json::to_string(&nonce)?)
            .replace("__MEDIA_URL__", &serde_json::to_string(&media_url)?)
            .replace(
                "__PROFILE_IMAGE__",
                &serde_json::to_string(&format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                        "../../public/original-player/assets/default_profile_dark.png"
                    ))
                ))?,
            );
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let protocol_requests = requests.clone();
        let protocol_service = fixture.service.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let watchdog_done = finished.clone();
        let app_slot = Arc::new(Mutex::new(None::<tauri::AppHandle>));
        let watchdog_app = app_slot.clone();
        let watchdog_service = fixture.service.clone();
        let watchdog_merge = fixture.merge.clone();
        thread::spawn(move || {
            while start.elapsed() < DEADLINE + Duration::from_secs(5) {
                if watchdog_done.load(Ordering::Acquire) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            EXIT_CODE.store(3, Ordering::Release);
            eprintln!("ORIGINAL_PLAYER_PROBE_DEADLINE");
            watchdog_service.shutdown_and_wait();
            watchdog_merge.shutdown_and_wait();
            if let Some(app) = watchdog_app
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                app.exit(3);
            }
        });
        let profile = fixture.root.path().join("original-player-profile");
        fs::create_dir(&profile)?;
        let closing_service = fixture.service.clone();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        let result = tauri::Builder::default()
            .invoke_handler(|invoke| {
                if invoke.message.command() == "atsumi_original_probe_ping" {
                    invoke.resolver.resolve("pong");
                    true
                } else {
                    false
                }
            })
            .register_uri_scheme_protocol("tauri", move |context, request| {
                if context.webview_label() != "main" {
                    return Response::builder().status(403).body(Vec::new()).unwrap();
                }
                match request.uri().path() {
                    "/probe.html" => public_response(HOST_HTML.as_bytes().to_vec(), "text/html; charset=utf-8"),
                    "/probe.js" => public_response(controller.as_bytes().to_vec(), "text/javascript; charset=utf-8"),
                    _ => Response::builder().status(404).body(Vec::new()).unwrap(),
                }
            })
            .register_uri_scheme_protocol("atsumi-player", |context, request| {
                if context.webview_label() == "main" && request.uri().path() == "/probe.js" {
                    let template = Request::builder().uri("http://atsumi-player.localhost/runtime.js").body(Vec::new()).unwrap();
                    let mut response = original_player::response(&template, "main");
                    *response.body_mut() = FRAME_PROBE.as_bytes().to_vec();
                    response.headers_mut().insert(header::CONTENT_LENGTH, FRAME_PROBE.len().to_string().parse().unwrap());
                    return response;
                }
                let mut response = original_player::response(&request, context.webview_label());
                println!("{}",json!({"kind":"original_player_asset_request","uri":request.uri().to_string(),"origin":request.headers().get(header::ORIGIN).and_then(|v|v.to_str().ok()),"status":response.status().as_u16(),"cors":response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).and_then(|v|v.to_str().ok())}));
                if request.uri().path() == "/frame.html" && response.status() == StatusCode::OK {
                    let html = String::from_utf8(response.body().clone()).unwrap();
                    let html = html.replace("</body>", "<script src=\"./probe.js\"></script></body>");
                    response.headers_mut().insert(header::CONTENT_LENGTH, html.len().to_string().parse().unwrap());
                    *response.body_mut() = html.into_bytes();
                }
                response
            })
            .register_asynchronous_uri_scheme_protocol("atsumi-replay", move |context, request, responder| {
                let label = context.webview_label().to_owned();
                let service = protocol_service.clone();
                let requests = protocol_requests.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let response = service.media_response(&request, &label);
                    let record = request_log(&label, &request, &response);
                    println!("{}",json!({"kind":"original_player_media_request","request":record}));
                    let mut saved = requests.lock().unwrap_or_else(|p|p.into_inner());
                    if saved.len() < 128 { saved.push(record); }
                    drop(saved);
                    responder.respond(response);
                });
            })
            .setup(move |app| {
                *app_slot.lock().unwrap() = Some(app.handle().clone());
                let window = tauri::window::WindowBuilder::new(app, "main")
                    .title("Isolated original CHZZK player probe").inner_size(960.0, 540.0)
                    .position(-16000.0, -16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
                let builder = WebviewBuilder::new("main", WebviewUrl::External("http://tauri.localhost/probe.html".parse().unwrap()))
                    .data_directory(profile)
                    .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --host-resolver-rules=MAP * ~NOTFOUND")
                    .on_navigation(|url| matches!(url.host_str(),Some("tauri.localhost"|"atsumi-player.localhost")))
                    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
                let view = window.add_child(builder,tauri::LogicalPosition::new(0.0,0.0),tauri::LogicalSize::new(960.0,540.0))?;
                let app = app.handle().clone();
                thread::spawn(move || {
                    let test = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
                        let first=wait_state(&view,start,|v|v["hostIpc"]=="pong"&&v["ready"]==true&&v["diagnostics"]["controls"].as_u64().is_some_and(|n|n>=5))?;
                        let security=&first["diagnostics"];
                        if security["parentBlocked"]!=true||security["cookieBlocked"]!=true||(security["storageBlocked"]!=true&&security["storageIsMemory"]!=true)||security["tauriAbsent"]!=true||security["origin"]!="null" {
                            return Err(format!("opaque iframe security failed: {security}").into());
                        }
                        if !security["crossorigin"].is_null()||security["controls"].as_u64().unwrap_or(0)<5 {
                            return Err(format!("original DOM or no-CORS source failed: {security}").into());
                        }
                        script(&view,"window.originalSend('mute');window.originalSend('toggle');true")?;
                        let playing=wait_state(&view,start,|v|v["diagnostics"]["time"].as_f64().is_some_and(|n|n>0.4)&&v["diagnostics"]["paused"]==false&&v["diagnostics"]["width"]==1280)?;
                        if playing["diagnostics"]["savedProfileLoaded"]!=true||!playing["diagnostics"]["savedHeader"].as_str().unwrap_or("").contains("합성 저장 채널"){return Err("saved channel profile not rendered by original UI".into());}
                        script(&view,"window.originalSend('seek',5);true")?;
                        let forward=wait_state(&view,start,|v|v["state"]["time"].as_f64().is_some_and(|n|n>=5.0)&&v["state"]["seeking"]==false&&v["diagnostics"]["seeking"]==false)?;
                        script(&view,"window.originalSend('seek',1);window.originalInspect('rate2');true")?;
                        let backward=wait_state(&view,start,|v|v["state"]["time"].as_f64().is_some_and(|n|(1.0..2.0).contains(&n))&&v["state"]["seeking"]==false&&v["diagnostics"]["seeking"]==false&&v["diagnostics"]["rate"]==2)?;
                        thread::sleep(Duration::from_millis(700));
                        let fast=wait_state(&view,start,|v|v["diagnostics"]["rate"]==2&&v["diagnostics"]["time"].as_f64().is_some_and(|n|n>2.0))?;
                        script(&view,"window.originalSend('privacy',true);true")?;
                        let paused=wait_state(&view,start,|v|v["state"]["paused"]==true&&v["diagnostics"]["paused"]==true)?;
                        thread::sleep(Duration::from_millis(350));
                        let held=state(&view)?;
                        if (held["diagnostics"]["time"].as_f64().ok_or("heldtime")?-paused["diagnostics"]["time"].as_f64().ok_or("pausetime")?).abs()>0.15{return Err("privacy-paused video advanced".into());}
                        script(&view,"window.originalInspect('egress');true")?;
                        let egress=wait_state(&view,start,|v|v["diagnostics"]["violations"].as_array().is_some_and(|rows|rows.iter().any(|r|r["directive"]=="connect-src"&&r["blocked"]=="https://example.invalid/blocked")))?;
                        script(&view,"window.originalSend('dispose');true")?;
                        let disposed=wait_state(&view,start,|v|v["diagnostics"]["ready"]==0&&v["diagnostics"]["paused"]==true)?;
                        closing_service.close(&token)?;
                        let denied=Request::builder().uri(&media_url).header(header::RANGE,"bytes=0-1").body(Vec::new())?;
                        if closing_service.media_response(&denied,"main").status()!=StatusCode::NOT_FOUND{return Err("closed token still readable".into());}
                        let logs=requests.lock().unwrap_or_else(|p|p.into_inner());
                        if !logs.iter().any(|r|r["status"]==206&&!r["range"].is_null()&&r["origin"].is_null()){return Err("opaque original video did not produce no-Origin Range206".into());}
                        if logs.iter().any(|r|r["bodyBytes"].as_u64().is_some_and(|n|n>MAX_BODY as u64)){return Err("media response exceeded1MiB".into());}
                        Ok(json!({"kind":"original_player_probe","success":true,"synthetic":true,"mediaBytes":media_bytes,"first":first,"playing":playing,"forward":forward,"backward":backward,"rate2":fast,"privacy":held,"egress":egress,"disposed":disposed,"tokenRevoked":true,"originPolicyUnchanged":true,"requests":logs.len()}))
                    }));
                    let code=match test {Ok(Ok(value))=>{println!("{value}");0},Ok(Err(error))=>{eprintln!("ORIGINAL_PLAYER_PROBE_ERROR: {error}");2},Err(_)=>2};
                    EXIT_CODE.store(code,Ordering::Release);app.exit(code);
                });
                Ok(())
            }).build(context).map(|app| app.run_return(|_, _| {}));
        finished.store(true, Ordering::Release);
        // Return from the event loop so fixture Drop runs and failures have a
        // real nonzero process exit, unlike a process-exiting event loop.
        thread::sleep(Duration::from_millis(200));
        drop(fixture);
        result?;
        let code = EXIT_CODE.load(Ordering::Acquire);
        if code == 0 {
            Ok(())
        } else {
            Err(format!("original player probe exit {code}").into())
        }
    }
}
