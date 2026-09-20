//! Offline borrowing/parking regression: one disposable WebView, one synthetic
//! canvas/video/recorder, no AppState, real account, network, or user recording.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use atsumi_lib::streaming::browser::{
        multiview::auto_watch::apply_auto_receiver_viewport, BrowserViewport,
    };
    use serde_json::{json, Value};
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tauri::{LogicalPosition, LogicalSize, WebviewBuilder, WebviewUrl};
    fn script(view: &tauri::Webview, source: &str) -> Value {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.eval_with_callback(source, move |value| {
            let _ = tx.send(value);
        })
        .unwrap();
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap()
    }
    fn gesture(view: &tauri::Webview, expression: &str) -> Value {
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
        use windows::core::HSTRING;
        let params = json!({"expression": expression, "userGesture": true, "awaitPromise": true, "returnByValue": true}).to_string();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |status, body| {
                    let _ = tx.send(if status.is_ok() { body } else { "null".into() });
                    Ok(())
                },
            ));
            platform
                .controller()
                .CoreWebView2()
                .unwrap()
                .CallDevToolsProtocolMethod(
                    &HSTRING::from("Runtime.evaluate"),
                    &HSTRING::from(params),
                    &handler,
                )
                .unwrap();
        })
        .unwrap();
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(6)).unwrap()).unwrap()
    }
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = finished.clone();
    std::thread::spawn(move || {
        for _ in 0..45 {
            if watchdog.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        eprintln!("AUTO_WATCH_PROBE_TIMEOUT");
        std::process::exit(3);
    });
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("isolated-profile");
    let exit_code = Arc::new(AtomicI32::new(2));
    let worker_exit = exit_code.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    tauri::Builder::default().setup(move |app| {
        let parent = tauri::window::WindowBuilder::new(app, "main").title("Isolated automatic watch probe")
            .inner_size(1280.0, 800.0).visible(false).focused(false).skip_taskbar(true).build()?;
        let app = app.handle().clone();
        std::thread::spawn(move || {
            let navigation_count = Arc::new(AtomicU64::new(0)); let count = navigation_count.clone();
            let view = parent.add_child(WebviewBuilder::new("offline-auto-watch", WebviewUrl::External("about:blank".parse().unwrap()))
                .data_directory(profile).on_navigation(move |url| { count.fetch_add(1, Ordering::AcqRel); url.as_str() == "about:blank" })
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
                LogicalPosition::new(-2560.0, 0.0), LogicalSize::new(1280.0, 720.0)).unwrap();
            let revision = Arc::new(AtomicU64::new(1));
            apply_auto_receiver_viewport(&view, &BrowserViewport::default(), revision.clone(), 1).unwrap();
            script(&view, r#"(() => {
                document.body.innerHTML='<canvas width="320" height="180"></canvas><video autoplay muted playsinline></video>';
                const canvas=document.querySelector('canvas'), ctx=canvas.getContext('2d'), video=document.querySelector('video');
                window.watchProbe={frames:0,chunks:0,bytes:0}; window.originalVideo=video;
                const draw=()=>{watchProbe.frames++;ctx.fillStyle=`hsl(${watchProbe.frames%360},70%,50%)`;ctx.fillRect(0,0,320,180);requestAnimationFrame(draw);};draw();
                const stream=canvas.captureStream(30); video.srcObject=stream;video.muted=true;video.play().catch(()=>{});
                window.originalStream=stream;window.originalRecorder=new MediaRecorder(stream,{mimeType:'video/webm;codecs=vp8'});
                originalRecorder.ondataavailable=e=>{if(e.data.size){watchProbe.chunks++;watchProbe.bytes+=e.data.size;}};originalRecorder.start(200);
                return true;
            })()"#);
            let mut before_chunks = 0; let mut before_bytes = 0;
            let initial_navigation = navigation_count.load(Ordering::Acquire);
            // Only this disposable synthetic video receives test user activation.
            std::thread::sleep(Duration::from_millis(500));
            let pip = gesture(&view, "(async()=>{originalVideo.volume=0.4;originalVideo.muted=false;await originalVideo.requestPictureInPicture();return document.pictureInPictureElement===originalVideo})()");
            println!("{}", json!({"pip":pip}));
            if pip["result"]["value"] != true { app.exit(2); return; }
            for (index, phase) in ["background", "watch", "resized", "background-again", "watch-again"].iter().enumerate() {
                let expected = revision.fetch_add(1, Ordering::AcqRel) + 1;
                let visible = matches!(*phase, "watch" | "resized" | "watch-again");
                let viewport = BrowserViewport { visible, x: 16.0, y: 64.0, width: if index == 2 { 640.0 } else { 960.0 }, height: if index == 2 { 360.0 } else { 540.0 }, ..BrowserViewport::default() };
                apply_auto_receiver_viewport(&view, &viewport, revision.clone(), expected).unwrap();
                std::thread::sleep(Duration::from_millis(1200));
                let sample = script(&view, "({...watchProbe,recording:originalRecorder.state,sameVideo:originalVideo===document.querySelector('video'),sameStream:originalVideo.srcObject===originalStream,width:innerWidth,height:innerHeight,pip:document.pictureInPictureElement===originalVideo,muted:originalVideo.muted,volume:originalVideo.volume})");
                println!("{}", json!({"phase":phase,"sample":sample}));
                let chunks = sample["chunks"].as_u64().unwrap_or(0); let bytes = sample["bytes"].as_u64().unwrap_or(0);
                if chunks <= before_chunks || bytes <= before_bytes || sample["recording"] != "recording" || sample["sameVideo"] != true || sample["sameStream"] != true
                    || (!visible && (sample["width"] != 1280 || sample["height"] != 720)) || parent.is_visible().unwrap()
                    || navigation_count.load(Ordering::Acquire) != initial_navigation {
                    eprintln!("AUTO_WATCH_CONTINUITY_FAILED"); app.exit(2); return;
                }
                if sample["pip"] != true || sample["muted"] != false || sample["volume"] != 0.4 {
                    eprintln!("AUTO_WATCH_PIP_OR_AUDIO_CHANGED"); app.exit(2); return;
                }
                before_chunks = chunks; before_bytes = bytes;
                let old = BrowserViewport { visible: true, width: 320.0, height: 180.0, ..BrowserViewport::default() };
                assert!(apply_auto_receiver_viewport(&view, &old, revision.clone(), expected - 1).is_err());
            }
            gesture(&view, "(async()=>{await document.exitPictureInPicture();return true})()");
            script(&view, "originalRecorder.stop();originalStream.getTracks().forEach(track=>track.stop());true");
            view.close().unwrap(); worker_exit.store(0, Ordering::Release); app.exit(0);
        });
        Ok(())
    }).run(context).unwrap();
    finished.store(true, Ordering::Release);
    drop(directory);
    std::process::exit(exit_code.load(Ordering::Acquire));
}
