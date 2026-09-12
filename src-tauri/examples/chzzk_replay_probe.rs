//! Offline WebView2 replay transport regression, using a disposable profile and
//! synthetic media only. No AppState, accounts, existing recordings or user DB.
//! cargo run --offline --example chzzk_replay_probe
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows/WebView2 required");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("REPLAY_PROBE_FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use atsumi_lib::streaming::{
        browser_merge::{BrowserMergeWorker, MediaTools},
        browser_store::{BrowserCaptureStore, BrowserMergeStatus},
        replay::ReplayService,
    };
    use serde_json::{json, Value};
    use std::{
        fs::{self, File},
        io::Read,
        os::windows::process::CommandExt,
        path::{Path, PathBuf},
        process::{Command, Stdio},
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

    const DEADLINE: Duration = Duration::from_secs(40);
    const MAX_BODY: usize = 1024 * 1024;
    const REMOTE_LABEL: &str = "chzzk-official";
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    struct Fixture {
        service: ReplayService,
        merge: BrowserMergeWorker,
        store: Arc<Mutex<BrowserCaptureStore>>,
        // Drop temporary files only after services have released their handles.
        root: tempfile::TempDir,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.service.shutdown_and_wait();
            self.merge.shutdown_and_wait();
            let _ = self
                .store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .shutdown();
        }
    }

    fn check_deadline(start: Instant) -> Result<()> {
        if start.elapsed() >= DEADLINE {
            Err("40-second probe deadline".into())
        } else {
            Ok(())
        }
    }

    fn synthetic_mp4(root: &Path, tools: &MediaTools, start: Instant) -> Result<PathBuf> {
        let output = root.join("synthetic-input.mp4");
        let mut child = Command::new(&tools.ffmpeg)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-n",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "8",
                "-c:v",
                "libopenh264",
                "-b:v",
                "6000k",
                "-g",
                "30",
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
            ])
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()?;
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(format!("synthetic FFmpeg failed: {status}").into());
                }
                break;
            }
            if let Err(error) = check_deadline(start) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            thread::sleep(Duration::from_millis(40));
        }
        let length = fs::metadata(&output)?.len();
        if length <= MAX_BODY as u64 || length > 32 * MAX_BODY as u64 {
            return Err(format!("synthetic MP4 must be >1 MiB and <=32 MiB, got {length}").into());
        }
        println!(
            "{}",
            json!({"kind":"replay_probe_fixture","bytes":length,"durationSeconds":8,"synthetic":true})
        );
        Ok(output)
    }

    fn prepare(start: Instant) -> Result<(Fixture, String, u64)> {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("workspace")?
            .to_owned();
        // The existing pinned, application-managed LGPL build only. Never PATH.
        let bin = workspace
            .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        let tools = MediaTools {
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        };
        if !tools.ffmpeg.is_file() || !tools.ffprobe.is_file() {
            return Err("pinned FFmpeg/ffprobe missing".into());
        }
        let root = tempfile::Builder::new()
            .prefix("atsumi-replay-probe-")
            .tempdir()?;
        let media = synthetic_mp4(root.path(), &tools, start)?;
        let data = root.path().join("data");
        let downloads = root.path().join("downloads");
        fs::create_dir(&data)?;
        fs::create_dir(&downloads)?;
        let store = Arc::new(Mutex::new(BrowserCaptureStore::new(&data)?));
        let recording = {
            let store = store.lock().unwrap();
            let recording = store.begin(
                &downloads,
                &"a".repeat(32),
                "Synthetic offline replay probe",
                "video/mp4",
            )?;
            let mut input = File::open(media)?;
            let mut bytes = vec![0u8; 128 * 1024];
            let mut chunk = 0;
            loop {
                check_deadline(start)?;
                let read = input.read(&mut bytes)?;
                if read == 0 {
                    break;
                }
                store.append(&recording.id, 0, chunk, &bytes[..read])?;
                chunk += 1;
            }
            store.finish_segment(&recording.id, 0, 8.0)?;
            store.finish_with_chat(&recording.id, false, None, false, "disabled", 0)?;
            recording
        };
        let merge = BrowserMergeWorker::start(store.clone(), Some(tools))?;
        let fixture = Fixture {
            service: ReplayService::new(&data, store.clone()),
            merge,
            store,
            root,
        };
        loop {
            check_deadline(start)?;
            let recordings = fixture.store.lock().unwrap().snapshot()?;
            let saved = recordings
                .iter()
                .find(|item| item.id == recording.id)
                .ok_or("missing fixture recording")?;
            match saved.merge.as_ref().map(|merge| merge.status) {
                Some(BrowserMergeStatus::Complete) => break,
                Some(BrowserMergeStatus::Failed | BrowserMergeStatus::Blocked) => {
                    return Err(format!("synthetic merge failed: {:?}", saved.merge).into())
                }
                _ => thread::sleep(Duration::from_millis(40)),
            }
        }
        let path = fixture.store.lock().unwrap().merged_file(&recording.id)?;
        let length = fs::metadata(path)?.len();
        if length <= MAX_BODY as u64 {
            return Err("merged fixture is too small to test required Range".into());
        }
        let session = fixture.service.open(&recording.id)?;
        Ok((fixture, session.token, length))
    }

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
        script(view, "(() => {const v=document.querySelector('video');return {time:v?.currentTime,paused:v?.paused,seeking:v?.seeking,rate:v?.playbackRate,ready:v?.readyState,duration:Number.isFinite(v?.duration)?v.duration:null,width:v?.videoWidth,height:v?.videoHeight,frames:v?.getVideoPlaybackQuality?.().totalVideoFrames,error:window.probeError||null};})()")
    }
    fn wait_state(
        view: &Webview,
        start: Instant,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        loop {
            check_deadline(start)?;
            let value = state(view)?;
            if !value["error"].is_null() {
                return Err(format!("WebView2 media error: {value}").into());
            }
            if predicate(&value) {
                return Ok(value);
            }
            thread::sleep(Duration::from_millis(80));
        }
    }
    fn install_video(view: &Webview, url: &str) -> Result<()> {
        let url = serde_json::to_string(url)?;
        script(
            view,
            &format!(
                r#"(() => {{
            const csp=document.createElement('meta');csp.httpEquiv='Content-Security-Policy';
            csp.content="default-src 'none'; media-src http://atsumi-replay.localhost; connect-src 'none'; object-src 'none'; frame-src 'none'; base-uri 'none'";document.head.append(csp);
            const v=document.createElement('video');v.muted=true;v.autoplay=true;v.playsInline=true;v.preload='auto';
            // Intentionally no crossOrigin attribute: this exercises native
            // media requests from about:blank without changing Origin policy.
            v.addEventListener('error',()=>window.probeError='media:'+v.error?.code+':'+v.error?.message);
            document.body.replaceChildren(v);v.src={url};v.play().catch(e=>window.probeError=String(e));return true;
        }})()"#
            ),
        )?;
        Ok(())
    }

    fn log_response(
        label: &str,
        request: &Request<Vec<u8>>,
        response: &Response<Vec<u8>>,
    ) -> Value {
        json!({"kind":"replay_probe_request","label":label,"method":request.method().as_str(),
            "range":request.headers().get(header::RANGE).and_then(|v|v.to_str().ok()),
            "origin":request.headers().get(header::ORIGIN).and_then(|v|v.to_str().ok()),
            "status":response.status().as_u16(),"bodyBytes":response.body().len(),
            "contentRange":response.headers().get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok())})
    }

    pub fn run() -> Result<()> {
        let start = Instant::now();
        let (fixture, token, media_bytes) = prepare(start)?;
        let url = format!("http://atsumi-replay.localhost/{token}");
        for (label, range, origin, expected) in [
            ("main", None, None, StatusCode::BAD_REQUEST),
            ("main", Some("bytes=0-"), None, StatusCode::PARTIAL_CONTENT),
            (REMOTE_LABEL, Some("bytes=0-"), None, StatusCode::FORBIDDEN),
            (
                "main",
                Some("bytes=0-"),
                Some("null"),
                StatusCode::FORBIDDEN,
            ),
        ] {
            let mut request = Request::builder().uri(&url);
            if let Some(range) = range {
                request = request.header(header::RANGE, range);
            }
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let request = request.body(Vec::new())?;
            let response = fixture.service.media_response(&request, label);
            println!(
                "{}",
                json!({"kind":"replay_probe_direct_policy","request":log_response(label,&request,&response)})
            );
            if response.status() != expected || response.body().len() > MAX_BODY {
                return Err("production transport policy regression".into());
            }
        }
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let protocol_service = fixture.service.clone();
        let protocol_requests = requests.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let app_slot = Arc::new(Mutex::new(None::<tauri::AppHandle>));
        let watchdog_done = finished.clone();
        let watchdog_app = app_slot.clone();
        let watchdog_service = fixture.service.clone();
        let watchdog_merge = fixture.merge.clone();
        thread::spawn(move || {
            while start.elapsed() < DEADLINE {
                if watchdog_done.load(Ordering::Acquire) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            eprintln!("REPLAY_PROBE_DEADLINE");
            EXIT_CODE.store(3, Ordering::Release);
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
        let profile = fixture.root.path().join("profile");
        fs::create_dir(&profile)?;
        fs::create_dir(profile.join("main"))?;
        fs::create_dir(profile.join("remote"))?;
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        let result = tauri::Builder::default()
            .register_asynchronous_uri_scheme_protocol("atsumi-replay", move |context, request, responder| {
                let label = context.webview_label().to_owned();
                let service = protocol_service.clone(); let requests = protocol_requests.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    // Exact production policy and actual native Webview label.
                    let response = service.media_response(&request, &label);
                    let record = log_response(&label, &request, &response); println!("{record}");
                    let mut saved = requests.lock().unwrap_or_else(|p|p.into_inner());
                    if saved.len() < 128 { saved.push(record); }
                    drop(saved); responder.respond(response);
                });
            })
            .setup(move |app| {
                *app_slot.lock().unwrap() = Some(app.handle().clone());
                let window = tauri::window::WindowBuilder::new(app, "main")
                    .title("Isolated replay transport probe").inner_size(640.0, 360.0)
                    .position(-16000.0, -16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
                let builder = |label: &str, directory: PathBuf| {
                    WebviewBuilder::new(label, WebviewUrl::External("about:blank".parse().unwrap()))
                        .data_directory(directory)
                        .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --host-resolver-rules=MAP * ~NOTFOUND")
                        .on_navigation(|url| url.as_str() == "about:blank")
                        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                };
                let main = window.add_child(builder("main", profile.join("main")), tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(640.0, 360.0))?;
                let remote = window.add_child(builder(REMOTE_LABEL, profile.join("remote")), tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(1.0, 1.0))?;
                let app = app.handle().clone();
                thread::spawn(move || {
                    let test = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
                        for view in [&remote, &main] {
                            while script(view, "Boolean(document.body && document.head)")? != true {
                                check_deadline(start)?;
                                thread::sleep(Duration::from_millis(50));
                            }
                        }
                        // A production-denied remote child is a positive control.
                        install_video(&remote, &url)?; install_video(&main, &url)?;
                        let first = wait_state(&main, start, |v| v["time"].as_f64().is_some_and(|t| t > 0.4) && v["width"] == 1280)?;
                        script(&main, "document.querySelector('video').pause();true")?;
                        let paused = state(&main)?; thread::sleep(Duration::from_millis(300));
                        let held = state(&main)?;
                        if (held["time"].as_f64().ok_or("pause time")? - paused["time"].as_f64().ok_or("pause time")?).abs() > 0.1 { return Err("paused media advanced".into()); }
                        script(&main, "document.querySelector('video').currentTime=5;true")?;
                        let forward = wait_state(&main, start, |v| v["seeking"] == false && v["time"].as_f64().is_some_and(|t|(t-5.0).abs()<0.2))?;
                        script(&main, "document.querySelector('video').currentTime=1;true")?;
                        let backward = wait_state(&main, start, |v| v["seeking"] == false && v["time"].as_f64().is_some_and(|t|(t-1.0).abs()<0.2))?;
                        script(&main, "(() => {const v=document.querySelector('video');v.playbackRate=2;v.play().catch(e=>window.probeError=String(e));return true;})()")?;
                        let rate_start = Instant::now(); thread::sleep(Duration::from_millis(1000));
                        let fast = state(&main)?; let elapsed = rate_start.elapsed().as_secs_f64();
                        let advanced = fast["time"].as_f64().ok_or("rate time")? - backward["time"].as_f64().ok_or("rate time")?;
                        if !fast["error"].is_null() || fast["paused"] != false || fast["rate"] != 2 || advanced < elapsed * 1.3 || advanced > elapsed * 2.8 { return Err(format!("2x playback failed: {fast}, elapsed={elapsed}, advance={advanced}").into()); }
                        let requests = requests.lock().unwrap_or_else(|p|p.into_inner());
                        if !requests.iter().any(|r|r["label"]=="main" && r["status"]==206 && !r["range"].is_null()) { return Err("WebView2 did not issue a successful main Range request".into()); }
                        if !requests.iter().any(|r|r["label"]==REMOTE_LABEL && r["status"]==403) { return Err("actual remote Webview request was not denied".into()); }
                        if requests.iter().any(|r|r["bodyBytes"].as_u64().is_some_and(|n|n>MAX_BODY as u64)) { return Err("protocol response exceeded 1 MiB".into()); }
                        Ok(json!({"kind":"replay_probe","success":true,"mediaBytes":media_bytes,"directLargeNoRangeStatus":400,"originPolicyUnchanged":true,"remoteLabelDenied":true,"first":first,"paused":held,"forwardSeek":forward,"backwardSeek":backward,"rate2":fast,"rateWallSeconds":elapsed,"rateAdvancedSeconds":advanced,"protocolRequests":requests.len()}))
                    }));
                    let code = match test {
                        Ok(Ok(summary)) => { println!("{summary}"); 0 },
                        Ok(Err(error)) => { eprintln!("REPLAY_PROBE_MEDIA_FAILURE: {error}"); 2 },
                        Err(_) => { eprintln!("REPLAY_PROBE_PANIC"); 2 },
                    };
                    EXIT_CODE.store(code, Ordering::Release); app.exit(code);
                });
                Ok(())
            }).run(context);
        finished.store(true, Ordering::Release);
        // The fixture's Drop always shuts down replay/index and merge workers,
        // then drops only this fresh temporary media/profile root.
        drop(fixture);
        result?;
        let code = EXIT_CODE.load(Ordering::Acquire);
        if code == 0 {
            Ok(())
        } else {
            Err(format!("isolated replay probe exit {code}").into())
        }
    }
}
