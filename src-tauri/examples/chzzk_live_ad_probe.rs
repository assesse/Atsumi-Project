//! Live multi-channel verification, anonymous or with an explicitly supplied
//! existing profile. No account/cookie/body logging, no app DB or recording.
#[cfg(windows)]
mod model {
    pub use atsumi_lib::streaming::model::StreamError;
}
#[cfg(windows)]
#[path = "../src/streaming/browser_compat.rs"]
#[allow(dead_code)] // This diagnostic reuses only part of the production module.
mod browser_compat;
#[cfg(windows)]
#[path = "../src/streaming/browser_video_ads.rs"]
mod browser_video_ads;
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    use webview2_com::{
        CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2_2,
        WebResourceResponseReceivedEventHandler,
    };
    use windows::core::{Interface, PWSTR};
    let channels: Vec<String> = std::env::var("ATSUMI_PROBE_CHANNELS")
        .or_else(|_| std::env::var("ATSUMI_PROBE_CHANNEL"))
        .expect("public live channels required")
        .split(',')
        .map(|v| v.trim().to_string())
        .collect();
    assert!(!channels.is_empty() && channels.len() <= 12);
    assert!(channels
        .iter()
        .all(|channel| channel.len() == 32 && channel.bytes().all(|b| b.is_ascii_hexdigit())));
    let samples: usize = std::env::var("ATSUMI_PROBE_SAMPLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
        .clamp(4, 120);
    let temp = tempfile::tempdir().unwrap();
    let existing = std::env::var_os("ATSUMI_PROBE_EXISTING_PROFILE").map(std::path::PathBuf::from);
    if let Some(path) = &existing {
        assert!(
            path.is_absolute() && path.is_dir(),
            "existing profile required"
        );
    }
    let profile = existing
        .clone()
        .unwrap_or_else(|| temp.path().join("anonymous-live-probe"));
    let app_ui = std::env::var("ATSUMI_PROBE_APP_UI").as_deref() == Ok("1");
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let exit_code = tauri::Builder::default().setup(move |app| {
        WebviewWindowBuilder::new(app, "live-ad-probe", WebviewUrl::External("about:blank".parse()?))
            .title(if existing.is_some(){"Atsumi · live ad verification (existing session)"}else{"Atsumi · live ad verification (anonymous)"}).inner_size(1120.0, 730.0)
            .data_directory(profile.clone()).browser_extensions_enabled(false)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding")
            .initialization_script("window.__atsumiAdProbe={adPlayEvents:0};document.addEventListener('playing',e=>{if(e.target?.tagName==='VIDEO'&&e.target.closest('#preAdPlayerWrapper,.pzp-pc--ad,.pzp-pc--ad-break'))window.__atsumiAdProbe.adPlayEvents++;},true);")
            .initialization_script(browser_video_ads::initialization_script())
            .initialization_script(if app_ui { include_str!("../src/streaming/browser_multiview.js") } else { "" })
            .initialization_script(if app_ui { include_str!("../src/streaming/browser_player_ui.js") } else { "" })
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny).build()?;
        let view = app.get_webview("live-ad-probe").unwrap(); let handle = app.handle().clone();
        std::thread::spawn(move || {
            if existing.is_some() {
                view.navigate("https://comm-api.game.naver.com/nng_main/v1/user/getUserStatus".parse().unwrap()).unwrap();
                let mut status=String::new();
                for _ in 0..16 {
                    std::thread::sleep(Duration::from_millis(500));
                    let (tx,rx)=std::sync::mpsc::sync_channel(1);
                    let _=view.eval_with_callback(r#"(()=>{try{const v=JSON.parse(document.body.textContent);return v.code===200&&typeof v.content?.loggedIn==='boolean'?(v.content.loggedIn?'signed_in':'signed_out'):'unknown'}catch{return 'unknown'}})()"#,move|v|{let _=tx.try_send(v);});
                    status=rx.recv_timeout(Duration::from_secs(2)).ok().and_then(|v|serde_json::from_str::<String>(&v).ok()).unwrap_or_default();
                    if matches!(status.as_str(),"signed_in"|"signed_out"){break;}
                }
                println!("existing-session-status={status}");
                if status!="signed_in" {handle.exit(2);return;}
            }
            let ledger = Arc::new(Mutex::new(std::collections::BTreeMap::<String,(u32,i32)>::new()));
            let seen = ledger.clone();
            let _ = view.with_webview(move |platform| unsafe {
                let _ = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?.cast::<ICoreWebView2_2>()?; let mut token=0;
                core.add_WebResourceResponseReceived(&WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let req=args.Request()?; let mut uri=PWSTR::null(); req.Uri(&mut uri)?;
                        if let Ok(url)=tauri::Url::parse(&CoTaskMemPWSTR::from(uri).to_string()) {
                            let host=url.host_str().unwrap_or_default(); let path=url.path();
                            // Fixed public endpoint classes, never identifiers, queries, headers or response bodies.
                            let class=if host=="nam.veta.naver.com" {
                                let mut method=PWSTR::null(); req.Method(&mut method)?;
                                let method=CoTaskMemPWSTR::from(method).to_string();
                                let path_class = if matches!(path,"/vas"|"/call"|"/gfp/v1"|"/gfp/v1/vas/vas"|"/nac/1") { path } else { "other" };
                                Some(format!("ad-provider:{}:{}",if matches!(method.as_str(),"GET"|"POST"|"OPTIONS") {method.as_str()} else {"other"},path_class))
                            }
                                else if host=="api.chzzk.naver.com" {Some(if path.contains("seoraksan") {"ad-flags"} else if path.contains("/ad") {"ad-state"} else if path.contains("live-detail") {"live-detail"} else {"other-live-api"}.to_string())}
                                else if path.ends_with(".m3u8") {Some("media-manifest".into())} else {None};
                            if let Some(class)=class { let mut status=0; args.Response()?.StatusCode(&mut status)?; let mut rows=seen.lock().unwrap(); let entry=rows.entry(class).or_default(); entry.0+=1; entry.1=status; }
                        }
                    } Ok(())
                })), &mut token)
                })();
            });
            browser_compat::configure(&view).unwrap(); browser_video_ads::install(&view).unwrap();
            let mut failed_channels = 0;
            for (channel_index, channel) in channels.iter().enumerate() {
            ledger.lock().unwrap().clear();
            println!("channel-start={} channel={channel}", channel_index+1);
            view.navigate(format!("https://chzzk.naver.com/live/{channel}").parse().unwrap()).unwrap();
            let mut saw_ad=false; let mut live_times=Vec::new();
            for i in 0..samples {
                std::thread::sleep(Duration::from_secs(5));
                let (tx,rx)=std::sync::mpsc::sync_channel(1);
                let _=view.eval_with_callback(include_str!("chzzk_live_ad_probe.js"),move |v|{let _=tx.try_send(v);});
                let state=rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
                if let Ok(value)=serde_json::from_str::<serde_json::Value>(&state) {
                    saw_ad |= value["adPlaying"].as_bool()==Some(true) || value["adText"].as_bool()==Some(true) || value["adWarning"].as_bool()==Some(true) || value["adPlayEvents"].as_u64().unwrap_or(0)>0;
                    if value["livePlayer"].as_bool()==Some(true) { if let Some(videos)=value["videos"].as_array() {for video in videos {if video["width"].as_u64().unwrap_or(0)>0 && video["inAd"].as_bool()!=Some(true) && video["paused"].as_bool()==Some(false) {if let Some(time)=video["time"].as_f64(){live_times.push(time);}}}} }
                }
                println!("sample={} state={} network={}",i,state,serde_json::to_string(&*ledger.lock().unwrap()).unwrap());
            }
            let playing=live_times.windows(2).any(|times| times[1]>times[0]+1.0);
            let passed=!saw_ad&&playing;
            if !passed {failed_channels+=1;}
            println!("channel-result={} passed={passed} observed_ad={saw_ad} live_progress={playing}",channel_index+1);
            }
            println!("probe-result channels={} failed={failed_channels}",channels.len());
            handle.exit(if failed_channels==0 {0}else{3});
        }); Ok(())
    }).build(context).unwrap().run_return(|_,_|{});
    drop(temp);
    std::process::exit(exit_code);
}
