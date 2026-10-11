//! Actual production multiview player, disposable profile, no recording/account.
//! cargo run --example chzzk_player_layout_probe -- PUBLIC_CHANNEL_ID
use atsumi_lib::streaming::browser::{multiview::MultiViewEntry, BrowserViewport, OfficialBrowser};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::Manager;

fn inspect(view: &tauri::Webview) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(r#"(() => {
      const root=document.querySelector('[data-atsumi-mado-player]');
      const play=root?.querySelector('.pzp-pc__playback-switch');
      // Exercise keyboard focus/overlay visibility without clicking, muting,
      // seeking, authenticating or touching any page outside this test profile.
      play?.focus();
      const box=el=>{if(!el)return null;const r=el.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom,right:r.right}};
      const info=root?.querySelector('.player_header .header_info');
      const video=root?.querySelector('video');
      const controls=[...(root?.querySelectorAll('.pzp-pc__bottom-buttons-left > button, .pzp-pc__volume-control > button, .pzp-pc__bottom-buttons-right > button, #atsumi-player-controls > button')??[])].filter(el=>el.getBoundingClientRect().width>0);
      return {width:innerWidth,height:innerHeight,ready:video?.readyState??0,time:video?.currentTime??0,player:box(root),
        wide:!!root?.querySelector('button.pzp-pc__viewmode-button[aria-label="좁은 화면"]'),
        profile:!!info?.querySelector('img'),headerCharacters:info?.textContent?.length??0,headerOpacity:info?getComputedStyle(info).opacity:null,
        controls:controls.map(el=>({label:el.getAttribute('aria-label'),box:box(el)}))};
    })()"#, move |result| { let _ = tx.send(result); }).map_err(|e| e.to_string())?;
    serde_json::from_str(
        &rx.recv_timeout(Duration::from_secs(3))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn run_probe(
    app: &tauri::AppHandle,
    host: &OfficialBrowser,
    channel: String,
) -> Result<(), String> {
    let snapshot = serde_json::to_value(
        host.configure_multiview(
            app,
            vec![MultiViewEntry {
                channel_id: channel,
                video: true,
                chat: false,
            }],
        )
        .map_err(|e| e.message)?,
    )
    .unwrap();
    let epoch = snapshot["epoch"].as_u64().unwrap();
    let id = snapshot["panes"][0]["paneId"].as_str().unwrap();
    let view = app.get_webview(id).ok_or("Missing native player")?;
    let start = Instant::now();
    for (sequence, width) in [603.0, 960.0, 360.0, 603.0].into_iter().enumerate() {
        host.set_multiview_viewport(
            app,
            id,
            BrowserViewport {
                epoch,
                request_sequence: Some(sequence as u64 + 1),
                width,
                height: 551.0,
                visible: true,
                ..Default::default()
            },
        )
        .map_err(|e| e.message)?;
        let mut last = Value::Null;
        while start.elapsed() < Duration::from_secs(40) {
            std::thread::sleep(Duration::from_millis(400));
            let value = inspect(&view)?;
            if value["width"].as_f64() == Some(width)
                && value["ready"].as_u64().unwrap_or(0) >= 2
                && value["wide"] == true
                && value["profile"] == true
                && value["headerOpacity"] == "1"
            {
                let controls = value["controls"].as_array().unwrap();
                if controls.len() < 5 {
                    last = value;
                    continue;
                }
                let centers: Vec<f64> = controls
                    .iter()
                    .map(|c| {
                        c["box"]["y"].as_f64().unwrap() + c["box"]["height"].as_f64().unwrap() / 2.0
                    })
                    .collect();
                if width > 400.0 && centers.iter().any(|y| (y - centers[0]).abs() > 1.0) {
                    return Err(format!("Control baselines differ: {value}"));
                }
                if controls.iter().any(|c| {
                    c["box"]["x"].as_f64().unwrap() < 0.0
                        || c["box"]["right"].as_f64().unwrap() > width + 1.0
                        || c["box"]["bottom"].as_f64().unwrap() > 552.0
                }) {
                    return Err(format!("Control clipped: {value}"));
                }
                println!("{}", json!({"kind":"player_layout_pass","state":value}));
                last = Value::Null;
                break;
            }
            last = value;
        }
        if !last.is_null() || start.elapsed() >= Duration::from_secs(40) {
            return Err(format!("Public player unavailable or incomplete: {last}"));
        }
    }
    Ok(())
}

fn main() {
    let channel = std::env::args().nth(1).expect("Public channel ID required");
    assert!(channel.len() == 32 && channel.bytes().all(|b| b.is_ascii_hexdigit()));
    let profile = tempfile::Builder::new()
        .prefix("atsumi-player-layout-probe-")
        .tempdir()
        .unwrap();
    let host = Arc::new(OfficialBrowser::new(profile.path().to_path_buf()).unwrap());
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = done.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(55));
        if !watchdog.load(Ordering::Acquire) {
            eprintln!("PLAYER_LAYOUT_PROBE_TIMEOUT");
            std::process::exit(2);
        }
    });
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    tauri::Builder::default()
        .setup(move |app| {
            tauri::window::WindowBuilder::new(app, "main")
                .title("Isolated player layout test")
                .inner_size(1280.0, 800.0)
                .visible(false)
                .build()?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let result = run_probe(&app, &host, channel);
                if let Err(error) = &result {
                    eprintln!("PLAYER_LAYOUT_PROBE_FAILED: {error}");
                }
                host.shutdown_and_wait(&app);
                done.store(true, Ordering::Release);
                app.exit(if result.is_ok() { 0 } else { 1 });
            });
            Ok(())
        })
        .run(context)
        .expect("run isolated layout test");
}
