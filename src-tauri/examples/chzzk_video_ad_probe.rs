//! Offline WebView2 request interception fixture. All HTTP(S) requests receive
//! local responses; DNS is also disabled. No user profile, AppState or DB.
//! cargo run --offline --example chzzk_video_ad_probe

#[cfg(windows)]
mod model {
    pub use atsumi_lib::streaming::model::StreamError;
}
#[cfg(windows)]
#[path = "../src/streaming/browser_video_ads.rs"]
mod browser_video_ads;

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows/WebView2 required");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("CHZZK_VIDEO_AD_PROBE_FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use serde_json::{json, Value};
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };
    use tauri::{Manager, Webview, WebviewUrl, WebviewWindowBuilder};
    use webview2_com::{
        CoTaskMemPWSTR,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_2, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
        },
        WebResourceRequestedEventHandler, WebResourceResponseReceivedEventHandler,
    };
    use windows::{
        core::{Interface, HSTRING, PWSTR},
        Win32::UI::Shell::SHCreateMemStream,
    };

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const LIVE: &str = "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CHAT: &str = "https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/chat";
    const ACCOUNT: &str = "https://nid.naver.com/nidlogin.login";
    const ARGS: &str = "--disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --disable-features=DnsOverHttps,MediaRouter --host-resolver-rules=MAP * ~NOTFOUND";
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);

    // The first requests are initiated by the initial document, without a
    // follow-up injection. This catches late installation on initial playback.
    const HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>Offline video ad fixture</title><body><script>
    (() => {
      const mode = location.hostname === 'nid.naver.com' ? 'account' : location.pathname.endsWith('/chat') ? 'chat' : 'live';
      const current='https://api.chzzk.naver.com/service/v1/lives/123456/ads/current';
      const mark=(url,id)=>url+(url.includes('?')?'&':'?')+'fixture='+mode+'_'+id;
      const output={mode,done:false,results:[]};window.__videoAdProbe=output;
      const finish=(id,kind,status,outcome)=>output.results.push({id:mode+'_'+id,kind,status,outcome});
      const request=async(id,url,method='GET')=>{
        const controller=new AbortController(), timer=setTimeout(()=>controller.abort(),4000);
        try { const response=await fetch(mark(url,id),{method,credentials:'omit',cache:'no-store',signal:controller.signal});
          await response.text();finish(id,'fetch',response.status,'response');
        } catch (_) {finish(id,'fetch',0,'rejected');} finally {clearTimeout(timer);}
      };
      const xhr=(id,url)=>new Promise(resolve=>{
        const x=new XMLHttpRequest();x.open('GET',mark(url,id));x.timeout=4000;
        x.onload=()=>{finish(id,'xhr',x.status,'response');resolve();};
        x.onerror=()=>{finish(id,'xhr',0,'rejected');resolve();};
        x.ontimeout=()=>{finish(id,'xhr',0,'timeout');resolve();};x.send();
      });
      const media=(id,url)=>new Promise(resolve=>{
        const audio=document.createElement('audio');audio.muted=true;audio.preload='auto';
        const timer=setTimeout(()=>done('timeout'),4000);let completed=false;
        const done=outcome=>{if(completed)return;completed=true;clearTimeout(timer);finish(id,'media',null,outcome);resolve();};
        audio.onloadedmetadata=()=>done('loaded');audio.onerror=()=>done('error');
        audio.src=mark(url,id);document.body.appendChild(audio);audio.load();
      });
      const tasks=mode==='live'?[
        request('ad_current_fetch',current),
        xhr('ad_polling_xhr','https://api.chzzk.naver.com/ad-polling/v1/lives/123456/ad'),
        request('ad_schedule_fetch','https://nam.veta.naver.com/gfp/v1/vas/vas?vsi=LIVE_CHZZK_NDP_SCH'),
        request('allow_auth','https://nid.naver.com/nidlogin.login'),
        request('allow_chat','https://api.chzzk.naver.com/service/v1/chats/access-token'),
        request('allow_manifest','https://livecloud.pstatic.net/synthetic/master.m3u8'),
        request('allow_shared_gfp','https://nam.veta.naver.com/gfp/v1?u=display_banner'),
        request('allow_wrong_schedule','https://nam.veta.naver.com/gfp/v1/vas/vas?vsi=OTHER_SCHEDULE'),
        request('allow_post',current,'POST'),
        request('allow_unknown','https://example.invalid/local-fixture'),
        media('allow_media_cdn','https://livecloud.pstatic.net/synthetic/audio.wav'),
        media('allow_media_ad_path',current)
      ]:[request('allow_ad_route',current)];
      Promise.all(tasks).then(()=>{output.done=true;});
    })();
    </script>"#;

    #[derive(Clone, serde::Serialize)]
    struct Seen {
        id: String,
        method: String,
        context: Option<i32>,
        status: Option<i32>,
    }
    #[derive(Default)]
    struct Ledger {
        requests: Vec<Seen>,
        responses: Vec<Seen>,
        local_errors: usize,
    }

    fn fixture_id(uri: &str) -> Option<String> {
        let url = tauri::Url::parse(uri).ok()?;
        let value = url
            .query_pairs()
            .find(|(name, _)| name == "fixture")?
            .1
            .into_owned();
        (value.len() <= 64 && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
            .then_some(value)
    }

    fn wav() -> Vec<u8> {
        let samples = 800_u32;
        let mut bytes = Vec::with_capacity(44 + samples as usize * 2);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + samples * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&8000_u32.to_le_bytes());
        bytes.extend_from_slice(&16000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples * 2).to_le_bytes());
        bytes.resize(44 + samples as usize * 2, 0);
        bytes
    }

    fn install_local_responder(view: &Webview, ledger: Arc<Mutex<Ledger>>) -> Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            let outcome = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                let environment = platform.environment();
                let request_ledger = ledger.clone();
                let mut token = 0;
                core.add_WebResourceRequested(&WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {return Ok(());};
                    let outcome = (|| -> windows::core::Result<()> {
                        let request = args.Request()?;
                        let mut uri = PWSTR::null();request.Uri(&mut uri)?;
                        let uri = CoTaskMemPWSTR::from(uri).to_string();
                        let mut method = PWSTR::null();request.Method(&mut method)?;
                        let method = CoTaskMemPWSTR::from(method).to_string();
                        let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;args.ResourceContext(&mut context)?;
                        let id = fixture_id(&uri);
                        if let Some(id) = &id {
                            request_ledger.lock().unwrap().requests.push(Seen{id:id.clone(),method:method.clone(),context:Some(context.0),status:None});
                        }
                        let is_document = context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT && [LIVE,CHAT,ACCOUNT].contains(&uri.as_str());
                        let is_media = id.as_ref().is_some_and(|id|id.contains("allow_media_"));
                        let bytes = if is_document {HTML.as_bytes().to_vec()} else if is_media {wav()} else {b"local synthetic fixture".to_vec()};
                        let mime = if is_document {"text/html; charset=utf-8"} else if is_media {"audio/wav"} else {"text/plain; charset=utf-8"};
                        let status = if is_document || id.is_some() {200} else {404};
                        let headers = format!("Content-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: *",bytes.len());
                        let stream = SHCreateMemStream(Some(&bytes)).ok_or_else(windows::core::Error::from_win32)?;
                        let response = environment.CreateWebResourceResponse(&stream,status,&HSTRING::from("Local fixture"),&HSTRING::from(headers))?;
                        // This handler always supplies a local response. The
                        // subsequently installed production handler may replace
                        // it only for matched video-ad requests.
                        args.SetResponse(&response)?;Ok(())
                    })();
                    if outcome.is_err() {request_ledger.lock().unwrap().local_errors += 1;}
                    outcome
                })),&mut token)?;
                core.AddWebResourceRequestedFilter(&HSTRING::from("*"),COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
                core.cast::<ICoreWebView2_2>()?.add_WebResourceResponseReceived(&WebResourceResponseReceivedEventHandler::create(Box::new(move |_,args| {
                    let Some(args) = args else {return Ok(());};
                    let request = args.Request()?;
                    let mut uri = PWSTR::null();request.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    if let Some(id) = fixture_id(&uri) {
                        let mut method = PWSTR::null();request.Method(&mut method)?;
                        let method = CoTaskMemPWSTR::from(method).to_string();
                        let mut status = 0;args.Response()?.StatusCode(&mut status)?;
                        ledger.lock().unwrap().responses.push(Seen{id,method,context:None,status:Some(status)});
                    }
                    Ok(())
                })),&mut token)?;
                Ok(())
            })();
            let _ = tx.try_send(outcome.map_err(|error|error.to_string()));
        })?;
        rx.recv_timeout(Duration::from_secs(3))??;
        Ok(())
    }

    fn script(view: &Webview, javascript: &str) -> Result<Value> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.eval_with_callback(javascript, move |value| {
            let _ = tx.try_send(value);
        })?;
        Ok(serde_json::from_str(
            &rx.recv_timeout(Duration::from_secs(2))?,
        )?)
    }

    fn run_case(
        view: &Webview,
        ledger: &Arc<Mutex<Ledger>>,
        mode: &str,
        url: &str,
    ) -> Result<Value> {
        view.navigate(url.parse()?)?;
        let start = Instant::now();
        let result = loop {
            let result = script(view, "window.__videoAdProbe || null")?;
            if result["mode"] == mode && result["done"] == true {
                break result;
            }
            if start.elapsed() > Duration::from_secs(8) {
                return Err(format!("{mode}: synthetic document timeout").into());
            }
            thread::sleep(Duration::from_millis(80));
        };
        let items = result["results"].as_array().ok_or("fixture result array")?;
        let expected = if mode == "live" { 12 } else { 1 };
        if items.len() != expected {
            return Err(format!("{mode}: unexpected fixture count").into());
        }
        // ResponseReceived may be queued just behind JS completion.
        loop {
            let all_received = {
                let seen = ledger.lock().unwrap();
                items.iter().all(|item| {
                    seen.responses
                        .iter()
                        .any(|r| r.id == item["id"].as_str().unwrap_or(""))
                })
            };
            if all_received {
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                let seen = ledger.lock().unwrap();
                eprintln!(
                    "{}",
                    json!({"kind":"native_response_timeout","javascript":result,"requests":seen.requests,"responses":seen.responses,"localErrors":seen.local_errors})
                );
                return Err(format!("{mode}: native response observation timeout").into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        let seen = ledger.lock().unwrap();
        if seen.local_errors != 0 {
            return Err("local responder failed".into());
        }
        for item in items {
            let id = item["id"].as_str().ok_or("fixture ID")?;
            let blocked = id.starts_with("live_ad_");
            let expected_status = if blocked { 204 } else { 200 };
            let responses = seen
                .responses
                .iter()
                .filter(|r| r.id == id)
                .collect::<Vec<_>>();
            if responses.is_empty() || responses.iter().any(|r| r.status != Some(expected_status)) {
                return Err(format!("{id}: native status was not {expected_status}").into());
            }
            if item["kind"] == "media" {
                if item["outcome"] != "loaded"
                    || !seen.requests.iter().any(|r| {
                        r.id == id && r.context == Some(COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA.0)
                    })
                {
                    return Err(format!("{id}: native media fixture did not load").into());
                }
            } else if blocked {
                // Exact-origin CORS must let the document consume the empty
                // response; a network error cannot satisfy either assertion.
                if item["status"] != 204 || item["outcome"] != "response" {
                    return Err(format!("{id}: unexpected blocked JS outcome").into());
                }
            } else if item["status"] != 200 || item["outcome"] != "response" {
                return Err(format!("{id}: unfiltered fixture did not return local 200").into());
            }
        }
        Ok(
            json!({"mode":mode,"results":items,"nativeStatusesVerified":expected,"nativeMediaContextsVerified":if mode=="live"{2}else{0}}),
        )
    }

    pub fn run() -> Result<()> {
        if std::env::var("ATSUMI_CHZZK_VIDEO_AD_FILTER")
            .ok()
            .as_deref()
            == Some("0")
        {
            return Err("probe requires ATSUMI_CHZZK_VIDEO_AD_FILTER to be enabled".into());
        }
        let temp = tempfile::tempdir()?;
        let profile = temp.path().join("isolated-profile");
        let finished = Arc::new(AtomicBool::new(false));
        let completed = finished.clone();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        tauri::Builder::default().setup(move |app| {
            WebviewWindowBuilder::new(app,"probe",WebviewUrl::External("about:blank".parse()?))
                .title("Offline CHZZK video ad probe").visible(false).focused(false).skip_taskbar(true)
                .data_directory(profile.clone()).additional_browser_args(ARGS)
                .on_navigation(|url|["about:blank",LIVE,CHAT,ACCOUNT].contains(&url.as_str()))
                .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).build()?;
            let view = app.get_webview("probe").ok_or("probe WebView")?;
            let app = app.handle().clone();
            let watchdog_app = app.clone();
            let watchdog_finished = completed.clone();
            thread::spawn(move || {
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(40) {
                    if watchdog_finished.load(Ordering::Acquire) {return;}
                    thread::sleep(Duration::from_millis(100));
                }
                eprintln!("CHZZK_VIDEO_AD_PROBE_DEADLINE");
                EXIT_CODE.store(3,Ordering::Release);
                watchdog_app.exit(3);
            });
            thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let ledger = Arc::new(Mutex::new(Ledger::default()));
                    install_local_responder(&view,ledger.clone())?;
                    super::browser_video_ads::install(&view)?;
                    let mut cases=Vec::new();
                    for (mode,url) in [("live",LIVE),("chat",CHAT),("account",ACCOUNT)] {
                        cases.push(run_case(&view,&ledger,mode,url)?);
                    }
                    println!("{}",json!({"kind":"chzzk_video_ad_probe","success":true,"network":"all requests answered locally, DNS disabled","firstNavigationFiltered":true,"cases":cases,"actualChzzkAdCoverage":"not measured"}));
                    Ok(())
                }));
                let code=match result {Ok(Ok(()))=>0,Ok(Err(error))=>{eprintln!("CHZZK_VIDEO_AD_PROBE_CHECK: {error}");2},Err(_)=>2};
                EXIT_CODE.store(code,Ordering::Release);
                app.exit(code);
            });
            Ok(())
        }).build(context)?.run_return(|_, _| {});
        finished.store(true, Ordering::Release);
        drop(temp);
        if EXIT_CODE.load(Ordering::Acquire) == 0 {
            Ok(())
        } else {
            Err("offline native request checks failed".into())
        }
    }
}
