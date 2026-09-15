//! Offline comparison of Tauri child autoresize and direct parent WM_SIZE.
//! Uses disposable profiles, synthetic DOM/media and an unfocused off-screen
//! window. No AppState, existing account, recording or user database is opened.
//! cargo run --offline --example main_resize_probe
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows/WebView2 required");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("MAIN_RESIZE_PROBE_FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use atsumi_lib::streaming::browser::{BrowserViewport, OfficialBrowser, WINDOW_LABEL};
    use serde_json::{json, Value};
    use std::{
        cell::Cell,
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    use tauri::{Manager, PhysicalSize, Webview, WebviewBuilder, WebviewUrl, WebviewWindowBuilder};
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
            UI::WindowsAndMessaging::{
                GetClientRect, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, WM_SIZE,
            },
        },
    };

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    type SubclassProc =
        unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;
    const SUBCLASS_ID: usize = 0x41545355;
    const STEPS: usize = 90;
    const DEADLINE: Duration = Duration::from_secs(45);
    const BROWSER_ARGS: &str = "--autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --host-resolver-rules=MAP * ~NOTFOUND";
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);

    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(hwnd: HWND, callback: SubclassProc, id: usize, data: usize) -> BOOL;
        fn RemoveWindowSubclass(hwnd: HWND, callback: SubclassProc, id: usize) -> BOOL;
        fn DefSubclassProc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    }
    struct DirectResize {
        controller: ICoreWebView2Controller,
        child: HWND,
        applying: Cell<bool>,
        failed: Cell<bool>,
    }

    unsafe extern "system" fn direct_resize(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        data: usize,
    ) -> LRESULT {
        if message == WM_SIZE && wparam.0 != 1 {
            let resize = &*(data as *const DirectResize);
            if !resize.applying.replace(true) {
                let mut rect = RECT::default();
                let result = GetClientRect(hwnd, &mut rect).and_then(|_| {
                    resize.controller.SetBounds(rect)?;
                    SetWindowPos(
                        resize.child,
                        None,
                        0,
                        0,
                        rect.right,
                        rect.bottom,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                });
                resize.failed.set(resize.failed.get() || result.is_err());
                resize.applying.set(false);
            }
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }

    fn script(view: &Webview, javascript: &str) -> Result<Value> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.eval_with_callback(javascript, move |result| {
            let _ = tx.try_send(result);
        })?;
        Ok(serde_json::from_str(
            &rx.recv_timeout(Duration::from_secs(2))?,
        )?)
    }

    fn native_size(view: &Webview) -> Result<[i32; 2]> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.with_webview(move |platform| unsafe {
            let result = (|| -> windows::core::Result<[i32; 2]> {
                let mut hwnd = HWND::default();
                platform.controller().ParentWindow(&mut hwnd)?;
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect)?;
                Ok([rect.right - rect.left, rect.bottom - rect.top])
            })();
            let _ = tx.try_send(result.map_err(|_| "native bounds inspection"));
        })?;
        Ok(rx.recv_timeout(Duration::from_secs(2))??)
    }

    fn install_direct(main: &Webview, parent: usize) -> Result<usize> {
        main.set_auto_resize(false)?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        main.with_webview(move |platform| unsafe {
            let result = (|| -> Result<usize> {
                let controller = platform.controller();
                let mut child = HWND::default();
                controller.ParentWindow(&mut child)?;
                let state = Box::into_raw(Box::new(DirectResize {
                    controller,
                    child,
                    applying: Cell::new(false),
                    failed: Cell::new(false),
                }));
                if !SetWindowSubclass(
                    HWND(parent as *mut _),
                    direct_resize,
                    SUBCLASS_ID,
                    state as usize,
                )
                .as_bool()
                {
                    drop(Box::from_raw(state));
                    return Err("direct resize subclass installation".into());
                }
                Ok(state as usize)
            })();
            let _ = tx.try_send(result.map_err(|e| e.to_string()));
        })?;
        Ok(rx.recv_timeout(Duration::from_secs(2))??)
    }

    fn uninstall_direct(main: &Webview, parent: usize, state: usize) -> Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        main.with_webview(move |_| unsafe {
            let removed =
                RemoveWindowSubclass(HWND(parent as *mut _), direct_resize, SUBCLASS_ID).as_bool();
            let success = if removed {
                !Box::from_raw(state as *mut DirectResize).failed.get()
            } else {
                false
            };
            let _ = tx.try_send(success);
        })?;
        if !rx.recv_timeout(Duration::from_secs(2))? {
            return Err("direct resize subclass failed".into());
        }
        main.set_auto_resize(true)?;
        Ok(())
    }

    fn install_dom(main: &Webview) -> Result<()> {
        let result = script(
            main,
            r#"(() => {
            document.head.innerHTML='<style>html,body{margin:0;height:100%;overflow:hidden;background:#101820;color:#ddd;font:12px system-ui}header{height:46px;padding:8px;box-sizing:border-box}.cards{height:calc(100% - 46px);overflow:auto;display:grid;grid-template-columns:repeat(auto-fill,minmax(170px,1fr));gap:12px;padding:12px;box-sizing:border-box}.card{height:210px;border:1px solid #435;background:linear-gradient(130deg,#135,#374);border-radius:8px}.art{height:160px;background:linear-gradient(25deg,#254,#735,#346)}p{margin:8px}</style>';
            document.body.innerHTML='<header>Isolated synthetic resize fixture</header><div class="cards">'+Array.from({length:60},(_,i)=>'<article class="card"><div class="art"></div><p>Fixture '+i+'</p></article>').join('')+'</div>';
            window.probeResizeCount=0;window.addEventListener('resize',()=>window.probeResizeCount++);
            return Boolean(document.querySelectorAll('.card').length===60);
        })()"#,
        )?;
        if result != true {
            return Err("synthetic DOM setup".into());
        }
        Ok(())
    }

    fn install_media(view: &Webview) -> Result<()> {
        script(
            view,
            r#"(() => {
            document.head.innerHTML='<style>html,body{margin:0;overflow:hidden;background:#080b10}video{display:block;width:100vw;height:100vh;object-fit:contain}</style>';
            const canvas=document.createElement('canvas');canvas.width=640;canvas.height=360;
            const context=canvas.getContext('2d');let frame=0;
            const draw=()=>{context.fillStyle=`hsl(${frame++%360} 80% 40%)`;context.fillRect(0,0,640,360)};
            draw();window.probeDraw=setInterval(draw,33);
            const video=document.createElement('video');video.muted=true;video.autoplay=true;video.playsInline=true;video.srcObject=canvas.captureStream(30);document.body.replaceChildren(video);
            video.play().catch(()=>window.probeMediaError=true);return true;
        })()"#,
        )?;
        Ok(())
    }

    fn summary(values: &mut [f64]) -> Value {
        values.sort_by(f64::total_cmp);
        json!({"medianMs":values[values.len()/2],"p95Ms":values[(values.len()-1)*95/100],"maxMs":values[values.len()-1]})
    }

    fn resize_case(
        main: &Webview,
        host: &OfficialBrowser,
        app: &tauri::AppHandle,
        sibling: &str,
        mode: &str,
    ) -> Result<Value> {
        let window = main.window();
        let parent = window.hwnd()?.0 as usize;
        let direct = if mode == "direct_wm_size" {
            Some(install_direct(main, parent)?)
        } else {
            None
        };
        let measured = (|| -> Result<Value> {
            let before = script(
                main,
                "({width:innerWidth,height:innerHeight,resizes:window.probeResizeCount})",
            )?;
            let mut times = Vec::with_capacity(STEPS);
            let mut viewport_times = Vec::new();
            let start = Instant::now();
            let mut final_size = [0, 0];
            for step in 0..STEPS {
                let triangle = if step < STEPS / 2 { step } else { STEPS - step };
                let width = 980 + triangle as u32 * 6;
                let height = 650 + triangle as u32 * 3;
                let resize_start = Instant::now();
                window.set_size(PhysicalSize::new(width, height))?;
                let actual = native_size(main)?;
                let expected = window.inner_size()?;
                if actual != [expected.width as i32, expected.height as i32] {
                    return Err(format!("main bounds mismatch: {actual:?} vs {expected:?}").into());
                }
                times.push(resize_start.elapsed().as_secs_f64() * 1000.0);
                if sibling == "visible_media" {
                    let scale = window.scale_factor()?;
                    let at = Instant::now();
                    host.set_viewport(
                        app,
                        BrowserViewport {
                            x: 180.0,
                            y: 70.0,
                            width: (width as f64 / scale - 210.0).max(160.0),
                            height: (height as f64 / scale - 110.0).max(90.0),
                            visible: true,
                            ..Default::default()
                        },
                    )?;
                    viewport_times.push(at.elapsed().as_secs_f64() * 1000.0);
                }
                final_size = actual;
                thread::sleep(Duration::from_millis(8));
            }
            let final_start = Instant::now();
            let scale = window.scale_factor()?;
            let after = loop {
                let value = script(
                    main,
                    "({width:innerWidth,height:innerHeight,resizes:window.probeResizeCount})",
                )?;
                let width = value["width"].as_f64().ok_or("DOM width")?;
                let height = value["height"].as_f64().ok_or("DOM height")?;
                if (width * scale - final_size[0] as f64).abs() <= scale
                    && (height * scale - final_size[1] as f64).abs() <= scale
                {
                    break value;
                }
                if final_start.elapsed() > Duration::from_secs(2) {
                    return Err("DOM resize did not reach final native size".into());
                }
                thread::sleep(Duration::from_millis(10));
            };
            Ok(
                json!({"kind":"main_resize_case","success":true,"mode":mode,"sibling":sibling,"steps":STEPS,"nativeAck":summary(&mut times),"viewportAck":if viewport_times.is_empty(){Value::Null}else{summary(&mut viewport_times)},"elapsedMs":start.elapsed().as_secs_f64()*1000.0,"finalDomAckMs":final_start.elapsed().as_secs_f64()*1000.0,"domBefore":before,"domAfter":after,"scaleFactor":scale}),
            )
        })();
        if let Some(state) = direct {
            uninstall_direct(main, parent, state)?;
        }
        measured
    }

    pub fn run() -> Result<()> {
        let finished = Arc::new(AtomicBool::new(false));
        let watchdog = finished.clone();
        thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < DEADLINE {
                if watchdog.load(Ordering::Acquire) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            eprintln!("MAIN_RESIZE_PROBE_DEADLINE");
            std::process::exit(3);
        });
        let root = tempfile::tempdir()?;
        let host_root = root.path().join("synthetic-host");
        std::fs::create_dir(&host_root)?;
        let host = OfficialBrowser::new(host_root)?;
        let profile = root.path().join("profile");
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        tauri::Builder::default().setup(move |app| {
            let _window = WebviewWindowBuilder::new(app,"main",WebviewUrl::External("about:blank".parse()?))
                .title("Isolated main resize probe").inner_size(980.0,650.0).position(-16000.0,-16000.0)
                .skip_taskbar(true).focused(false).visible(true).data_directory(profile.clone())
                .additional_browser_args(BROWSER_ARGS).on_navigation(|url|url.as_str()=="about:blank")
                .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).build()?;
            let main = app.get_webview("main").ok_or("main WebView")?;
            let app = app.handle().clone();
            thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    install_dom(&main)?;
                    for mode in ["tauri_child", "direct_wm_size"] { println!("{}",resize_case(&main,&host,&app,"absent",mode)?); }
                    let child = main.window().add_child(
                        WebviewBuilder::new(WINDOW_LABEL,WebviewUrl::External("about:blank".parse()?))
                            .data_directory(profile).additional_browser_args(BROWSER_ARGS)
                            .on_navigation(|url|url.as_str()=="about:blank")
                            .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny),
                        tauri::LogicalPosition::new(180.0,70.0),tauri::LogicalSize::new(640.0,360.0))?;
                    child.hide()?;
                    for mode in ["tauri_child", "direct_wm_size"] { println!("{}",resize_case(&main,&host,&app,"hidden",mode)?); }
                    install_media(&child)?;
                    host.set_viewport(&app,BrowserViewport{x:180.0,y:70.0,width:640.0,height:360.0,visible:true,..Default::default()})?;
                    for mode in ["tauri_child", "direct_wm_size"] { println!("{}",resize_case(&main,&host,&app,"visible_media",mode)?); }
                    let media=script(&child,"({time:document.querySelector('video').currentTime,paused:document.querySelector('video').paused,error:Boolean(window.probeMediaError)})")?;
                    if media["paused"]!=false || media["error"]!=false || !media["time"].as_f64().is_some_and(|time|time>0.1) { return Err(format!("synthetic media stopped: {media}").into()); }
                    println!("{}",json!({"kind":"main_resize_probe","success":true,"cases":6,"syntheticMedia":media,"measurement":"programmatic native resize acknowledgment; not user drag frame rate"}));
                    Ok(())
                }));
                let code = match result { Ok(Ok(()))=>0, Ok(Err(error))=>{eprintln!("MAIN_RESIZE_PROBE_CHECK: {error}");2},Err(_)=>2 };
                host.shutdown_and_wait(&app);
                EXIT_CODE.store(code,Ordering::Release);
                app.exit(code);
            });
            Ok(())
        }).run(context)?;
        finished.store(true, Ordering::Release);
        drop(root);
        if EXIT_CODE.load(Ordering::Acquire) == 0 {
            Ok(())
        } else {
            Err("isolated resize checks failed".into())
        }
    }
}
