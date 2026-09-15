//! Offline WebView2 visibility/lazy-player regression. Disposable profile only;
//! no account, network, user recordings or AppState. The OS window stays hidden.
//! cargo run --offline --example auto_receiver_probe
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[path = "../src/streaming/browser_auto_surface.rs"]
mod auto_surface;

#[cfg(windows)]
fn main() {
    use serde_json::{json, Value};
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
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
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = finished.clone();
    std::thread::spawn(move || {
        for _ in 0..60 {
            if watchdog.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        eprintln!("AUTO_RECEIVER_PROBE_TIMEOUT");
        std::process::exit(3);
    });
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    let exit_code = Arc::new(AtomicI32::new(2));
    let worker_exit = exit_code.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    tauri::Builder::default().setup(move |app| {
        let parent = tauri::window::WindowBuilder::new(app, "main")
            .title("Isolated auto receiver probe").inner_size(1280.0, 800.0)
            .visible(false).focused(false).skip_taskbar(true).build()?;
        let app = app.handle().clone();
        std::thread::spawn(move || {
            for mode in ["hidden", "parked-visible", "hidden-raf-fallback"] {
                let view = parent.add_child(
                    WebviewBuilder::new(mode, WebviewUrl::External("about:blank".parse().unwrap()))
                        .data_directory(profile.clone())
                        .additional_browser_args("--host-resolver-rules=MAP * ~NOTFOUND")
                        .on_navigation(|url| url.as_str() == "about:blank")
                        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
                    LogicalPosition::new(-2560.0, 0.0), LogicalSize::new(1280.0, 720.0),
                ).unwrap();
                if mode == "parked-visible" { auto_surface::prepare(&view).unwrap(); } else { view.hide().unwrap(); }
                if mode == "hidden-raf-fallback" {
                    script(&view, "window.requestAnimationFrame = cb => setTimeout(() => cb(performance.now()), 50); window.cancelAnimationFrame = clearTimeout; true");
                }
                script(&view, r#"(() => {
                    window.probe = {frames:0,resizes:0,timers:0,video:false};
                    document.body.innerHTML='<div id="player" style="width:100%;height:500px"></div>';
                    const frame = () => {
                      if (!probe.video) { const v=document.createElement('video'); document.getElementById('player').appendChild(v); probe.video=true; }
                      probe.frames++; requestAnimationFrame(frame);
                    };
                    requestAnimationFrame(frame);
                    new ResizeObserver(() => { probe.resizes++; }).observe(document.getElementById('player'));
                    window.probeTimer=setInterval(() => { probe.timers++; document.getElementById('player').style.height=(500+probe.timers%2)+'px'; },100);
                    return true;
                })()"#);
                std::thread::sleep(Duration::from_millis(1500));
                parent.set_size(LogicalSize::new(1000.0, 600.0)).unwrap();
                std::thread::sleep(Duration::from_millis(1500));
                let sample = script(&view, "({...probe,visibility:document.visibilityState,width:innerWidth,height:innerHeight})");
                println!("{}", json!({"mode":mode,"sample":sample}));
                if mode == "parked-visible" && (sample["frames"].as_u64().unwrap_or(0) < 2
                    || sample["resizes"].as_u64().unwrap_or(0) < 2 || sample["video"] != true
                    || sample["width"] != 1280 || sample["height"] != 720 || parent.is_visible().unwrap()) {
                    eprintln!("AUTO_RECEIVER_INITIALIZATION_FAILED"); app.exit(2); return;
                }
                view.close().unwrap();
            }
            worker_exit.store(0, Ordering::Release);
            app.exit(0);
        });
        Ok(())
    }).run(context).unwrap();
    finished.store(true, Ordering::Release);
    drop(directory);
    std::process::exit(exit_code.load(Ordering::Acquire));
}
