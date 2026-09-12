//! Isolated, offline about:blank child. Uses the production viewport path only.
//! No accounts, installed extensions, live pages, recording, or Atsumi AppState.
//! cargo run --offline --example chzzk_viewport_probe
use atsumi_lib::streaming::browser::{BrowserClip, BrowserViewport, OfficialBrowser, WINDOW_LABEL};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use windows::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{GetWindowRgnBox, ScreenToClient, NULLREGION, SIMPLEREGION},
        UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled, SetFocus},
        UI::WindowsAndMessaging::{
            GetParent, GetWindowLongPtrW, GetWindowRect, GWL_STYLE, WS_VISIBLE,
        },
    },
};

static CHILD: AtomicUsize = AtomicUsize::new(0);
static SHOWS: AtomicU64 = AtomicU64::new(0);
static HIDES: AtomicU64 = AtomicU64::new(0);
type WinEventHook = *mut std::ffi::c_void;
type WinEventProc = unsafe extern "system" fn(WinEventHook, u32, HWND, i32, i32, u32, u32);
#[link(name = "user32")]
unsafe extern "system" {
    fn SetWinEventHook(
        min: u32,
        max: u32,
        module: *mut std::ffi::c_void,
        callback: WinEventProc,
        process: u32,
        thread: u32,
        flags: u32,
    ) -> WinEventHook;
    fn UnhookWinEvent(hook: WinEventHook) -> BOOL;
}
unsafe extern "system" fn visibility_event(
    _: WinEventHook,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if hwnd.0 as usize == CHILD.load(Ordering::Relaxed) && object == 0 && child == 0 {
        if event == 0x8002 {
            SHOWS.fetch_add(1, Ordering::Relaxed);
        }
        if event == 0x8003 {
            HIDES.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[derive(Debug)]
struct Native {
    bounds: [i32; 4],
    clip: Option<[i32; 4]>,
    visible: bool,
    enabled: bool,
    owns_focus: bool,
}
fn inspect(view: &tauri::Webview) -> Native {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let controller = platform.controller();
        let mut hwnd = HWND::default();
        controller.ParentWindow(&mut hwnd).unwrap();
        let parent = GetParent(hwnd).unwrap();
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).unwrap();
        let mut origin = POINT {
            x: r.left,
            y: r.top,
        };
        assert!(ScreenToClient(parent, &mut origin).as_bool());
        let bounds = [origin.x, origin.y, r.right - r.left, r.bottom - r.top];
        let mut region = RECT::default();
        let region_kind = GetWindowRgnBox(hwnd, &mut region);
        // Match the production inspector: no HWND region and an explicitly
        // empty region are distinct. Hidden views may legitimately have none.
        let clip = if region_kind == SIMPLEREGION {
            Some([region.left, region.top, region.right, region.bottom])
        } else if region_kind == NULLREGION {
            Some([0; 4])
        } else if region_kind.0 == 0 {
            None
        } else {
            panic!("unexpected viewport probe region kind: {region_kind:?}");
        };
        let mut visible = BOOL::default();
        controller.IsVisible(&mut visible).unwrap();
        tx.send(Native {
            bounds,
            clip,
            visible: visible.as_bool()
                && GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_VISIBLE.0 as isize != 0,
            enabled: IsWindowEnabled(hwnd).as_bool(),
            owns_focus: GetFocus() == hwnd
                || windows::Win32::UI::WindowsAndMessaging::IsChild(hwnd, GetFocus()).as_bool(),
        })
        .unwrap();
    })
    .unwrap();
    rx.recv_timeout(Duration::from_secs(2)).unwrap()
}
fn check(view: &tauri::Webview, viewport: &BrowserViewport) {
    let actual = inspect(view);
    assert_eq!(actual.visible, viewport.visible, "{actual:?}");
    if !viewport.visible {
        return;
    }
    assert_eq!(actual.enabled, !viewport.occluded, "{actual:?}");
    if viewport.occluded {
        assert!(!actual.owns_focus, "{actual:?}");
    }
    let scale = view.window().scale_factor().unwrap();
    assert_eq!(
        actual.bounds,
        [viewport.x, viewport.y, viewport.width, viewport.height]
            .map(|v| (v * scale).round() as i32)
    );
    let clip = viewport.clip.clone().unwrap_or(BrowserClip {
        x: 0.0,
        y: 0.0,
        width: viewport.width,
        height: viewport.height,
    });
    assert_eq!(
        actual.clip,
        Some(if viewport.occluded {
            [0; 4]
        } else {
            [
                (clip.x * scale).ceil() as i32,
                (clip.y * scale).ceil() as i32,
                ((clip.x + clip.width) * scale).floor() as i32,
                ((clip.y + clip.height) * scale).floor() as i32,
            ]
        })
    );
}
fn script(view: &tauri::Webview, javascript: &str) -> serde_json::Value {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(javascript, move |value| {
        let _ = tx.send(value);
    })
    .unwrap();
    serde_json::from_str(&rx.recv_timeout(Duration::from_secs(3)).unwrap()).unwrap()
}
fn media_state(view: &tauri::Webview) -> serde_json::Value {
    script(view, "(() => { const v=document.querySelector('video'); return {time:v?.currentTime,paused:v?.paused,visibility:document.visibilityState,width:v?.videoWidth,error:window.probeError||null}; })()")
}
fn main() {
    let finished = Arc::new(AtomicBool::new(false));
    let deadline = finished.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(35));
        if !deadline.load(Ordering::Acquire) {
            eprintln!("viewport_probe_deadline");
            std::process::exit(3);
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let host = OfficialBrowser::new(temp.path().to_owned()).unwrap();
    let profile = temp.path().join("isolated-webview");
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    tauri::Builder::default().setup(move |app| {
        // WinEvent visibility observations require an actually visible parent;
        // keep this blank temporary window off-screen, unfocused, and off the
        // taskbar. It closes itself after the bounded local media checks.
        let main = tauri::window::WindowBuilder::new(app, "main").title("Isolated viewport regression probe").inner_size(900.0,650.0).position(-16000.0,-16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
        let _trusted = main.add_child(tauri::WebviewBuilder::new("main",tauri::WebviewUrl::External("about:blank".parse().unwrap())).data_directory(profile.clone()),tauri::LogicalPosition::new(0.0,0.0),tauri::LogicalSize::new(900.0,650.0))?;
        let view = main.add_child(tauri::WebviewBuilder::new(WINDOW_LABEL,tauri::WebviewUrl::External("about:blank".parse().unwrap())).data_directory(profile),tauri::LogicalPosition::new(0.0,0.0),tauri::LogicalSize::new(1.0,1.0))?;
        let (tx,rx)=std::sync::mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            let mut hwnd=HWND::default(); platform.controller().ParentWindow(&mut hwnd).unwrap();
            CHILD.store(hwnd.0 as usize,Ordering::Relaxed);
            // Only this test process and this exact child HWND are observed.
            let hook=SetWinEventHook(0x8002,0x8003,std::ptr::null_mut(),visibility_event,std::process::id(),0,0);
            assert!(!hook.is_null()); tx.send(hook as usize).unwrap();
        })?;
        let hook=rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let app=app.handle().clone();
        std::thread::spawn(move || {
            let test = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            eprintln!("viewport_probe phase=initial_show");
            let mut viewport=BrowserViewport{x:20.0,y:20.0,width:800.0,height:600.0,visible:true,..BrowserViewport::default()};
            host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            // In-memory synthetic pixels only: no downloads, codecs, file
            // recording, application data, account or live-service navigation.
            eprintln!("viewport_probe phase=synthetic_media_setup");
            script(&view, r#"(() => {
                const canvas=document.createElement('canvas'); canvas.width=320; canvas.height=180;
                const ctx=canvas.getContext('2d'); let n=0;
                const draw=()=>{ctx.fillStyle=`hsl(${n++%360} 90% 40%)`;ctx.fillRect(0,0,320,180);};
                draw(); window.probeTimer=setInterval(draw,33);
                const video=document.createElement('video'); video.autoplay=true; video.muted=true; video.playsInline=true;
                video.srcObject=canvas.captureStream(30); document.body.replaceChildren(video);
                video.play().catch(e=>window.probeError=String(e)); return true;
            })()"#);
            std::thread::sleep(Duration::from_millis(1200));
            let media_before=media_state(&view);
            assert_eq!(media_before["paused"],false,"{media_before}");
            assert_eq!(media_before["width"],320,"{media_before}");
            eprintln!("viewport_probe phase=remote_focus_positive_control");
            view.with_webview(|platform| unsafe {
                let mut hwnd=HWND::default(); platform.controller().ParentWindow(&mut hwnd).unwrap();
                let _=SetFocus(Some(hwnd));
            }).unwrap();
            assert!(inspect(&view).owns_focus,"focus positive control failed");
            let media_bounds=inspect(&view).bounds;
            eprintln!("viewport_probe phase=occlude");
            viewport.occluded=true;
            host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            assert_eq!(inspect(&view).bounds,media_bounds);
            std::thread::sleep(Duration::from_millis(2200));
            eprintln!("viewport_probe phase=occluded_media_check");
            let media_occluded=media_state(&view);
            assert_eq!(media_occluded["paused"],false,"{media_occluded}");
            let advanced=media_occluded["time"].as_f64().unwrap()-media_before["time"].as_f64().unwrap();
            assert!(advanced>0.8,"masked media stopped: {media_before} -> {media_occluded}");
            println!("{}",serde_json::json!({"kind":"viewport_probe_media","success":true,"modalMediaBefore":media_before,"modalMediaAfter":media_occluded,"modalAdvancedSeconds":advanced,"modalBoundsPreserved":true,"modalInputBlocked":true,"modalFocusReturned":true}));
            eprintln!("viewport_probe phase=modal_restore");
            viewport.occluded=false;
            host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            std::thread::sleep(Duration::from_millis(250));
            SHOWS.store(0,Ordering::Relaxed); HIDES.store(0,Ordering::Relaxed);
            eprintln!("viewport_probe phase=scrolling");
            for step in 0..120 {
                let offset=((step%60) as f64-30.0).abs()*4.0;
                viewport.y=20.0-offset;
                viewport.clip=Some(BrowserClip{x:0.0,y:offset,width:800.0,height:600.0-offset});
                host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
                std::thread::sleep(Duration::from_millis(8));
            }
            std::thread::sleep(Duration::from_millis(150));
            let scroll_shows=SHOWS.load(Ordering::Relaxed); let scroll_hides=HIDES.load(Ordering::Relaxed);
            assert_eq!((scroll_shows,scroll_hides),(0,0),"visible scrolling toggled the HWND");
            eprintln!("viewport_probe phase=intentional_hard_hide");
            viewport.visible=false; host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            std::thread::sleep(Duration::from_millis(100));
            eprintln!("viewport_probe phase=intentional_show");
            viewport.visible=true; host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            std::thread::sleep(Duration::from_millis(100));
            assert!(SHOWS.load(Ordering::Relaxed)>0 && HIDES.load(Ordering::Relaxed)>0,"visibility observer positive control failed");
            eprintln!("viewport_probe phase=detach");
            host.detach_viewport(&app); assert!(!inspect(&view).visible);
            assert_eq!(host.set_viewport(&app,viewport.clone()).unwrap_err().code,"VIEWPORT_STALE");
            assert!(!inspect(&view).visible);
            viewport.epoch=serde_json::to_value(host.snapshot().unwrap()).unwrap()["viewportEpoch"].as_u64().unwrap();
            eprintln!("viewport_probe phase=new_epoch_restore");
            host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            eprintln!("viewport_probe phase=invalid_request");
            let mut invalid=viewport.clone(); invalid.x=f64::NAN;
            assert_eq!(host.set_viewport(&app,invalid).unwrap_err().code,"VIEWPORT_INVALID"); assert!(!inspect(&view).visible);
            eprintln!("viewport_probe phase=final_restore");
            host.set_viewport(&app,viewport.clone()).unwrap(); check(&view,&viewport);
            println!("{}",serde_json::json!({"kind":"viewport_probe","success":true,"scrollUpdates":120,"scrollShows":scroll_shows,"scrollHides":scroll_hides,"intentionalHideShow":true,"staleEpochBlocked":true,"invalidHidden":true,"restored":true,"modalMediaBefore":media_before,"modalMediaAfter":media_occluded,"modalAdvancedSeconds":advanced,"modalBoundsPreserved":true,"modalInputBlocked":true,"modalFocusReturned":true,"scaleFactor":view.window().scale_factor().unwrap()}));
            }));
            view.with_webview(move |_| unsafe { assert!(UnhookWinEvent(hook as WinEventHook).as_bool()); }).unwrap();
            host.shutdown_and_wait(&app); app.exit(if test.is_ok() { 0 } else { 2 });
        });
        Ok(())
    }).run(context).expect("viewport probe failed");
    finished.store(true, Ordering::Release);
    drop(temp);
}
