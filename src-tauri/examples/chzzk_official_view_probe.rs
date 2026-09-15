//! Public-channel viewing smoke test. Recording requires --record-30-seconds.
//! Uses a temporary WebView profile and capture catalog, not Atsumi's DB/profile.
//! cargo run --example chzzk_official_view_probe -- CHANNEL_URL [--standard-quality] [--connect-extension] [--remembered-reconnect] [--screenshot] [--chrome-identity] [--ack-network-notice]
//! The optional flag clicks only the exact observed normal-quality choice once.
use atsumi_lib::streaming::browser::{BrowserViewport, OfficialBrowser};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::Manager;

// TEST ONLY. The official player-vendor-BYg0wCyN.js chooses the Edge
// extension ID whenever navigator.userAgent contains "edg". Preserve the real
// Chromium version and change only that product token for an isolated A/B run.
#[cfg(windows)]
fn chrome_identity(view: &tauri::Webview) -> Result<String, &'static str> {
    use webview2_com::{CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2};
    use windows::core::{Interface, HSTRING, PWSTR};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let result = (|| -> Result<String, &'static str> {
            let settings = platform
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|settings| settings.cast::<ICoreWebView2Settings2>())
                .map_err(|_| "user_agent_settings_unavailable")?;
            let mut original = PWSTR::null();
            settings
                .UserAgent(&mut original)
                .map_err(|_| "user_agent_read_failed")?;
            let original = CoTaskMemPWSTR::from(original).to_string();
            if original.len() > 1024 || !original.contains("Chrome/") {
                return Err("user_agent_unexpected");
            }
            let selected = original
                .split_ascii_whitespace()
                .filter(|token| !token.to_ascii_lowercase().starts_with("edg/"))
                .collect::<Vec<_>>()
                .join(" ");
            settings
                .SetUserAgent(&HSTRING::from(&selected))
                .map_err(|_| "user_agent_set_failed")?;
            Ok(selected)
        })();
        let _ = tx.try_send(result);
    })
    .map_err(|_| "user_agent_dispatch_failed")?;
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "user_agent_timeout")?
}

// Explicit diagnostic only: one bounded PNG of the isolated public page. The
// resulting named temporary file is retained so it can be inspected after exit.
#[cfg(windows)]
fn screenshot(view: &tauri::Webview) -> Result<std::path::PathBuf, &'static str> {
    use std::io::Write;
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::{
        System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET},
        UI::Shell::SHCreateMemStream,
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let Some(stream) = SHCreateMemStream(None) else {
            let _ = tx.try_send(Err("capture_stream_failed"));
            return;
        };
        let captured = stream.clone();
        let completed = tx.clone();
        let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
            let result = (|| -> Result<Vec<u8>, &'static str> {
                status.map_err(|_| "capture_failed")?;
                let mut stat = STATSTG::default();
                captured
                    .Stat(&mut stat, STATFLAG_NONAME)
                    .map_err(|_| "capture_stat_failed")?;
                if stat.cbSize == 0 || stat.cbSize > 16 * 1024 * 1024 {
                    return Err("capture_size_limit");
                }
                captured
                    .Seek(0, STREAM_SEEK_SET, None)
                    .map_err(|_| "capture_seek_failed")?;
                let mut bytes = vec![0u8; stat.cbSize as usize];
                let mut read = 0;
                captured
                    .Read(
                        bytes.as_mut_ptr().cast(),
                        bytes.len() as u32,
                        Some(&mut read),
                    )
                    .ok()
                    .map_err(|_| "capture_read_failed")?;
                if read as usize != bytes.len() || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return Err("capture_incomplete");
                }
                Ok(bytes)
            })();
            let _ = completed.try_send(result);
            Ok(())
        }));
        let result = platform.controller().CoreWebView2().and_then(|core| {
            core.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &callback,
            )
        });
        if result.is_err() {
            let _ = tx.try_send(Err("capture_dispatch_failed"));
        }
    })
    .map_err(|_| "capture_dispatch_failed")?;
    let bytes = rx
        .recv_timeout(Duration::from_secs(6))
        .map_err(|_| "capture_timeout")??;
    let mut file = tempfile::Builder::new()
        .prefix("atsumi-public-view-probe-")
        .suffix(".png")
        .tempfile()
        .map_err(|_| "capture_file_failed")?;
    file.write_all(&bytes).map_err(|_| "capture_file_failed")?;
    file.flush().map_err(|_| "capture_file_failed")?;
    file.keep()
        .map(|(_, path)| path)
        .map_err(|_| "capture_file_failed")
}

#[cfg(windows)]
fn observe_probe_messages(
    view: &tauri::Webview,
    standard: Arc<AtomicBool>,
    network: Arc<AtomicBool>,
    recording_diagnostics: Option<std::path::PathBuf>,
    confirmed_rates: Arc<AtomicU8>,
) -> tauri::Result<()> {
    view.with_webview(move |platform| unsafe {
        use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
        use windows::core::PWSTR;
        let core = platform.controller().CoreWebView2().unwrap();
        core.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut body = PWSTR::null();
                    if args.TryGetWebMessageAsString(&mut body).is_ok() {
                        let body = CoTaskMemPWSTR::from(body).to_string();
                        if let Some(json) = body
                            .strip_prefix("ATSUMI_BROWSER_CAPTURE:")
                            .filter(|body| body.len() <= 65536)
                        {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
                                if value["kind"] == "view_probe" {
                                    let bit = match value["probeRate"].as_f64() { Some(1.03)=>1, Some(2.0)=>2, Some(0.75)=>4, _=>0 };
                                    confirmed_rates.fetch_or(bit, Ordering::Relaxed);
                                }
                                if value["kind"] == "encoded_begin" {
                                    // Explicit short recording probe only: retain bounded
                                    // codec headers for parser regression, never media URLs.
                                    if let (Some(root), Some(tracks)) = (&recording_diagnostics, value["tracks"].as_array()) {
                                        use base64::Engine;
                                        for (index, track) in tracks.iter().take(2).enumerate() {
                                            if let Some(init) = track["init"].as_str().filter(|s|s.len() <= 88000) {
                                                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(init) {
                                                    let path = root.join(format!("observed-init-{index}.mp4"));
                                                    let _ = std::fs::write(&path, bytes);
                                                    println!("{}", serde_json::json!({"kind":"observed_init", "path":path, "mime":track["mimeType"]}));
                                                }
                                            }
                                        }
                                    }
                                }
                                if value["kind"] == "standard_quality_probe"
                                    && value["attempted"] == true
                                {
                                    standard.store(true, Ordering::Release);
                                }
                                if value["kind"] == "network_notice_probe"
                                    && value["attempted"] == true
                                {
                                    network.store(true, Ordering::Release);
                                }
                                if value["kind"] == "view_probe"
                                    || value["kind"] == "original_capture_error"
                                    || value["kind"] == "encoded_finish"
                                    || value["kind"] == "acl_probe"
                                    || value["kind"] == "standard_quality_probe"
                                    || value["kind"] == "network_notice_probe"
                                {
                                    println!("{value}");
                                }
                            }
                        }
                    }
                }
                Ok(())
            })),
            &mut 0,
        )
        .unwrap();
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("CHZZK channel URL is required");
    let (
        mut standard_quality,
        mut connect_extension,
        mut capture_screenshot,
        mut use_chrome_identity,
    ) = (false, false, false, false);
    let mut ack_network_notice = false;
    let mut remembered_reconnect = false;
    let mut record_seconds = None;
    for flag in args {
        match flag.as_str() {
            "--standard-quality" if !standard_quality => standard_quality = true,
            "--connect-extension" if !connect_extension => connect_extension = true,
            "--screenshot" if !capture_screenshot => capture_screenshot = true,
            "--chrome-identity" if !use_chrome_identity => use_chrome_identity = true,
            "--ack-network-notice" if !ack_network_notice => ack_network_notice = true,
            "--remembered-reconnect" if !remembered_reconnect => remembered_reconnect = true,
            "--record-30-seconds" if record_seconds.is_none() => record_seconds = Some(30u64),
            _ => panic!("Only documented CHANNEL_URL probe flags are accepted"),
        }
    }
    if remembered_reconnect {
        connect_extension = true;
    }
    // This isolated executable must also terminate when the UI dispatcher is
    // stuck: AppHandle::exit would only enqueue another blocked UI message.
    // Never used by Atsumi itself. A deadline can leave this probe's temporary
    // profile for inspection because process::exit intentionally skips drops.
    let finished = Arc::new(AtomicBool::new(false));
    let deadline_finished = finished.clone();
    std::thread::spawn(move || {
        let seconds = if record_seconds.is_some() {
            150
        } else if remembered_reconnect {
            90
        } else {
            60
        };
        std::thread::sleep(Duration::from_secs(seconds));
        if !deadline_finished.load(Ordering::Acquire) {
            eprintln!(
                "{}",
                serde_json::json!({"kind":"official_view_probe_failed","code":"overall_deadline","seconds":seconds})
            );
            std::process::exit(3);
        }
    });
    let profile = tempfile::Builder::new()
        .prefix("atsumi-original-receive-probe-")
        .tempdir()
        .unwrap()
        .keep();
    let isolated_data_dir = profile.clone();
    let output_root = profile.join("capture");
    std::fs::create_dir(&output_root).unwrap();
    println!(
        "{}",
        serde_json::json!({"kind":"isolated_probe_directory", "path":profile, "recordRequested":record_seconds.is_some()})
    );
    let media_bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let media_tools = record_seconds.map(|_| atsumi_lib::streaming::browser_merge::MediaTools {
        ffmpeg: media_bin.join("ffmpeg.exe"),
        ffprobe: media_bin.join("ffprobe.exe"),
    });
    let host =
        Arc::new(OfficialBrowser::new_with_media_tools(profile.clone(), media_tools).unwrap());
    let test_result = Arc::new(AtomicBool::new(record_seconds.is_none()));
    let completed_result = test_result.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let running = host.clone();
    tauri::Builder::default().setup(move |app| {
        // No Atsumi AppState or user profile. Match the production native parent
        // and child arrangement, but keep this diagnostic OS window hidden.
        tauri::window::WindowBuilder::new(app, "main")
            .title("Isolated CHZZK view probe").inner_size(1280.0, 800.0).visible(false).build()?;
        running.set_viewport(app.handle(), BrowserViewport {
            epoch: 0, request_sequence: None, x: 0.0, y: 0.0, width: 1280.0, height: 800.0, visible: true, clip: None,
            ..Default::default()
        }).map_err(|error| std::io::Error::other(error.message))?;
        running.open(app.handle(), &input).map_err(|error| std::io::Error::other(error.message))?;
        let view=app.get_webview(atsumi_lib::streaming::browser::WINDOW_LABEL).unwrap();
        #[cfg(windows)]
        if use_chrome_identity {
            let selected = chrome_identity(&view).map_err(std::io::Error::other)?;
            println!("{}", serde_json::json!({"kind":"test_chrome_identity", "userAgent":selected}));
            // With extension loading, its completion callback performs the one
            // reload after both changes; otherwise reload immediately here.
            if !connect_extension { view.reload()?; }
        }
        if connect_extension {
            // Exact production opt-in path, including setting persistence and
            // official-page reload. Do not call the low-level loader directly.
            running.connect_extension(app.handle()).map_err(|e| std::io::Error::other(e.message))?;
            println!("{}", serde_json::json!({"kind":"extension_connect_requested","path":"production_helper"}));
        }
        let standard_attempted = Arc::new(AtomicBool::new(false));
        let network_notice_attempted = Arc::new(AtomicBool::new(false));
        let confirmed_rates = Arc::new(AtomicU8::new(0));
        observe_probe_messages(&view, standard_attempted.clone(), network_notice_attempted.clone(), record_seconds.map(|_| isolated_data_dir.clone()), confirmed_rates.clone())?;
        let app = app.handle().clone();
        let mut host = running.clone();
        std::thread::spawn(move || {
            let mut view = view;
            let mut attempted_record = false;
            let mut recording_began = None;
            let mut stop_requested = false;
            let mut checked_rates = 0;
            let mut recording_verified = false;
            for sample in 0..if record_seconds.is_some() { 45 } else if remembered_reconnect { 20 } else { 10 } {
                if remembered_reconnect && sample == 10 {
                    let before = serde_json::to_value(host.snapshot().unwrap()).unwrap();
                    assert_eq!(before["extensionReconnectEnabled"], true, "explicit connection was not remembered");
                    host.shutdown_and_wait(&app);
                    view.close().expect("close isolated child");
                    for _ in 0..100 {
                        if app.get_webview(atsumi_lib::streaming::browser::WINDOW_LABEL).is_none() { break; }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    assert!(app.get_webview(atsumi_lib::streaming::browser::WINDOW_LABEL).is_none(), "old child still exists");
                    // Recreate both host state and the child from the same
                    // isolated app directory. No second manual connect call.
                    host = Arc::new(OfficialBrowser::new(isolated_data_dir.clone()).unwrap());
                    host.set_viewport(&app, BrowserViewport { width:1280.0, height:800.0, visible:true, ..BrowserViewport::default() }).unwrap();
                    host.open(&app, &input).unwrap();
                    view = app.get_webview(atsumi_lib::streaming::browser::WINDOW_LABEL).unwrap();
                    standard_attempted.store(false,Ordering::Release);
                    network_notice_attempted.store(false,Ordering::Release);
                    observe_probe_messages(&view, standard_attempted.clone(), network_notice_attempted.clone(), record_seconds.map(|_| isolated_data_dir.clone()), confirmed_rates.clone()).unwrap();
                    println!("{}",serde_json::json!({"kind":"remembered_reconnect","manualConnect":false,"freshHost":true,"freshChild":true}));
                }
                std::thread::sleep(Duration::from_secs(3));
                let allow_standard = standard_quality && !standard_attempted.load(Ordering::Acquire);
                let allow_network_notice = ack_network_notice && !network_notice_attempted.load(Ordering::Acquire);
                let script = r#"(() => {
                    const send=v=>window.chrome.webview.postMessage('ATSUMI_BROWSER_CAPTURE:'+JSON.stringify(v));
                    if(__ACK_NETWORK_NOTICE__ && !window.__atsumiNetworkNoticeAttempted){
                        const text=e=>(e.textContent??'').replace(/\s+/g,' ').trim();
                        const visible=e=>e.getClientRects().length>0&&getComputedStyle(e).visibility!=='hidden'&&getComputedStyle(e).display!=='none';
                        const dialogs=[...document.querySelectorAll('[role="dialog"],[role="alertdialog"]')].filter(d=>visible(d)&&text(d).startsWith('브라우저 권한 설정 안내')&&text(d).includes('[로컬 네트워크 액세스]'));
                        const buttons=dialogs.length===1?[...dialogs[0].querySelectorAll('button,[role="button"]')].filter(b=>visible(b)&&text(b)==='확인'&&!b.disabled&&b.getAttribute('aria-disabled')!=='true'):[];
                        if(buttons.length===1){
                            // Acknowledge this public informational dialog only.
                            // This never grants a browser/native network permission.
                            window.__atsumiNetworkNoticeAttempted=true;
                            send({kind:'network_notice_probe',attempted:true});
                            try{buttons[0].click();send({kind:'network_notice_probe',attempted:true,clicked:true});}
                            catch{send({kind:'network_notice_probe',attempted:true,clicked:false});}
                        }else{send({kind:'network_notice_probe',attempted:false,dialogs:dialogs.length,buttons:buttons.length});}
                    }
                    if(__STANDARD_QUALITY__ && !window.__atsumiStandardQualityAttempted){
                        const label='설치없이 일반 화질 시청';
                        const matches=[...document.querySelectorAll('button,a,[role="button"],[role="link"]')]
                            .filter(element=>(element.textContent??'').replace(/\s+/g,' ').trim()===label &&
                                element.getClientRects().length>0 && element.getAttribute('aria-disabled')!=='true' && !element.disabled);
                        if(matches.length===1){
                            window.__atsumiStandardQualityAttempted=true;
                            // Report the consumed attempt before click/navigation. The
                            // native flag prevents another document from repeating it.
                            send({kind:'standard_quality_probe',attempted:true,label,matches:1});
                            try{matches[0].click();send({kind:'standard_quality_probe',attempted:true,clicked:true,label});}
                            catch{send({kind:'standard_quality_probe',attempted:true,clicked:false,code:'click_failed'});}
                        }else{send({kind:'standard_quality_probe',attempted:false,matches:matches.length,label});}
                    }
                    const rect=e=>{const r=e.getBoundingClientRect();return {x:Math.round(r.x),y:Math.round(r.y),width:Math.round(r.width),height:Math.round(r.height)}};
                    const ancestors=e=>{const a=[];for(let p=e;p&&a.length<8;p=p.parentElement)a.push({tag:p.tagName,classes:[...p.classList].slice(0,8).map(c=>c.slice(0,100)),rect:rect(p)});return a};
                    const excluded='aside,textarea,input,[contenteditable="true"],[class*="chat" i],[class*="account" i],[class*="profile" i]';
                    const visible=e=>!e.closest(excluded)&&e.getClientRects().length>0&&getComputedStyle(e).visibility!=='hidden'&&getComputedStyle(e).display!=='none';
                    const safeText=(e,max)=>{const c=e.cloneNode(true);c.querySelectorAll(excluded+',script,style').forEach(n=>n.remove());return(c.textContent??'').replace(/\s+/g,' ').trim().slice(0,max)};
                    // Only visible public controls/notices, never chat text,
                    // input values, titles, profile/account elements or cookies.
                    send({kind:'view_probe',ready:document.readyState,visibility:document.visibilityState,
                        userAgent:navigator.userAgent.slice(0,512),
                        brands:(navigator.userAgentData?.brands??[]).slice(0,8).map(b=>({brand:String(b.brand).slice(0,80),version:String(b.version).slice(0,32)})),
                        buttonLabels:[...document.querySelectorAll('button,[role="button"],a')].filter(visible).slice(0,40).map(e=>({tag:e.tagName,text:safeText(e,100),ariaLabel:(e.getAttribute('aria-label')??'').slice(0,100)})),
                        dialogs:[...document.querySelectorAll('[role="dialog"],[role="alert"],[aria-modal="true"]')].filter(visible).slice(0,6).map(e=>({role:e.getAttribute('role'),text:safeText(e,1200),rect:rect(e)})),
                        publicNotices:document.querySelector('video')?[]:[...document.querySelectorAll('[class*="error" i],[class*="notice" i],[class*="install" i],[class*="modal" i],[class*="popup" i]')].filter(e=>!['HTML','BODY','MAIN'].includes(e.tagName)&&visible(e)).slice(0,6).map(e=>({classes:[...e.classList].slice(0,8),text:safeText(e,500),rect:rect(e)})),
                        emptyPlayerWrappers:document.querySelector('video')?[]:[...document.querySelectorAll('[class*="player" i],[id*="player" i]')].filter(visible).slice(0,16).map(e=>({tag:e.tagName,classes:[...e.classList].slice(0,8).map(c=>c.slice(0,100)),rect:rect(e)})),
                        presentation:window.__atsumiPresentation?.getState?.()??null,
                        encoded:window.__atsumiEncodedCapture?.getDiagnostics?.()??null,
                        cleanGeometry:[...document.querySelectorAll('[data-atsumi-clean-player],[data-atsumi-clean-video],[data-atsumi-clean-media-path],#atsumi-presentation-toggle,#atsumi-browser-capture-status')].slice(0,24).map(e=>({tag:e.tagName,control:['atsumi-presentation-toggle','atsumi-browser-capture-status'].includes(e.id)?e.id:null,markers:[...e.attributes].map(a=>a.name).filter(n=>n.startsWith('data-atsumi-clean-')),rect:rect(e),objectFit:getComputedStyle(e).objectFit,transform:getComputedStyle(e).transform})),
                        video:[...document.querySelectorAll('video')].slice(0,4).map(v=>({width:v.videoWidth,height:v.videoHeight,ready:v.readyState,paused:v.paused,time:Math.round(v.currentTime),error:v.error?.code??null,capture:typeof v.captureStream,frames:v.getVideoPlaybackQuality?.().totalVideoFrames??null,ancestors:ancestors(v)})),
                        chatInputs:[...document.querySelectorAll('textarea,[contenteditable="true"],[role="textbox"]')].slice(0,8).map(e=>({placeholder:(e.getAttribute('placeholder')??'').slice(0,128),ariaLabel:(e.getAttribute('aria-label')??'').slice(0,128),ancestors:ancestors(e)})),
                        frameOrigins:[...document.querySelectorAll('iframe')].slice(0,8).map(f=>{try{return new URL(f.src).origin}catch{return ''}})});
                    if(!window.__atsumiErrorProbe){window.__atsumiErrorProbe=true;window.addEventListener('atsumi-browser-reply',e=>{if(e.detail?.ok===false)send({kind:'original_capture_error',error:e.detail.error});});}
                    if(!window.__atsumiAclProbed && window.__TAURI_INTERNALS__?.invoke){
                        window.__atsumiAclProbed=true;
                        window.__TAURI_INTERNALS__.invoke('plugin:__TAURI_CHANNEL__|fetch',{}, {headers:{}})
                            .then(()=>send({kind:'acl_probe',blocked:false}))
                            .catch(e=>send({kind:'acl_probe',blocked:String(e).includes('Tauri IPC is disabled for remote content'),reason:String(e).slice(0,160)}));
                    }
                })();"#.replace("__STANDARD_QUALITY__",if allow_standard{"true"}else{"false"})
                    .replace("__ACK_NETWORK_NOTICE__",if allow_network_notice{"true"}else{"false"});
                let _=view.eval(&script);
                let snapshot = serde_json::to_value(host.snapshot().unwrap()).unwrap();
                if attempted_record && snapshot["recordingId"].is_null() && snapshot["error"].is_string() {
                    println!("{}", serde_json::json!({"kind":"original_record_probe_failed", "error":snapshot["error"]}));
                    break;
                }
                if record_seconds.is_some() && snapshot["ready"] == true && snapshot["videoPaused"] == false && !attempted_record {
                    attempted_record = true;
                    let result = host.arm(&app, output_root.clone(), true, true);
                    println!("{}", serde_json::json!({"kind":"original_record_probe_start", "error":result.err().map(|e|e.message)}));
                }
                if let Some(record_seconds) = record_seconds.filter(|_| !snapshot["recordingId"].is_null()) {
                    let began = recording_began.get_or_insert_with(std::time::Instant::now);
                    checked_rates = confirmed_rates.load(Ordering::Relaxed).count_ones() as usize;
                    if !stop_requested && checked_rates < 3 && began.elapsed().as_secs() >= 3 * (checked_rates as u64 + 1) {
                        let rate = [1.03, 2.0, 0.75][checked_rates];
                        let script = format!("(() => {{ const b=window.__atsumiEncodedCapture; const v=[...document.querySelectorAll('video')].find(v=>b?.canChangePlaybackRate(v)); if(v) {{v.playbackRate={rate};window.chrome.webview.postMessage('ATSUMI_BROWSER_CAPTURE:'+JSON.stringify({{kind:'view_probe',probeRate:{rate},encoded:b.getDiagnostics()}}));}} }})();");
                        let _ = view.eval(&script);
                    }
                    if began.elapsed().as_secs() >= record_seconds && !stop_requested {
                        stop_requested = true;
                        let _ = view.eval("for(const v of document.querySelectorAll('video')) v.playbackRate=1;");
                        let result = host.stop(&view);
                        println!("{}",serde_json::json!({"kind":"original_record_probe_stop","error":result.err().map(|e|e.message)}));
                    }
                }
                if record_seconds.is_some() && attempted_record && snapshot["recordingId"].is_null() && snapshot["recordings"].as_array().is_some_and(|v|!v.is_empty()) {
                    let recordings = snapshot["recordings"].as_array().unwrap();
                    if recordings.iter().any(|r|r["merge"]["status"]=="queued" || r["merge"]["status"]=="merging") { continue; }
                    recording_verified = stop_requested && confirmed_rates.load(Ordering::Relaxed) == 7 && recordings.iter().all(|r|r["mimeType"]=="video/mp4" && r["status"]=="stopped" && r["segmentCount"].as_u64().unwrap_or(0)>0 && r["merge"]["status"]=="complete");
                    println!("{}",serde_json::json!({"kind":"original_record_probe_result","success":recording_verified,"rateChanges":checked_rates,"recordings":recordings}));
                    break;
                }
                println!("{}", serde_json::json!({
                    "sample":sample+1,
                    "phase":if sample < 10 {"explicit"} else {"remembered"},
                    "extensionStatus":snapshot["extensionStatus"],
                    "extensionReconnectEnabled":snapshot["extensionReconnectEnabled"],
                    "windowOpen":snapshot["windowOpen"],"ready":snapshot["ready"],
                    "status":snapshot["status"],"error":snapshot["error"],
                    "recordingId":snapshot["recordingId"],"recordings":snapshot["recordings"].as_array().map(Vec::len)
                }));
                #[cfg(windows)]
                if capture_screenshot && sample % 10 == 8 {
                    match screenshot(&view) {
                        Ok(path) => println!("{}", serde_json::json!({"kind":"public_page_screenshot", "path":path})),
                        Err(code) => println!("{}", serde_json::json!({"kind":"public_page_screenshot", "errorCode":code})),
                    }
                }
            }
            host.shutdown_and_wait(&app);
            completed_result.store(recording_verified || record_seconds.is_none(), Ordering::Relaxed);
            app.exit(if record_seconds.is_some() && !recording_verified { 4 } else { 0 });
        });
        Ok(())
    }).run(context).expect("official view probe failed");
    finished.store(true, Ordering::Release);
    drop(host);
    // Retain only this isolated diagnostic directory for independent decode checks.
    drop(profile);
    if !test_result.load(Ordering::Relaxed) {
        std::process::exit(4);
    }
}
