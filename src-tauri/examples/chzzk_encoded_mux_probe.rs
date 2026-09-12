//! Explicit, network-free WebCodecs -> hls.js fMP4 -> production Rust remux probe.
//! Run with a LOCAL bundled generator .js path. Creates only a fresh temporary
//! profile/output root, retains outputs for the independent segment decode probe.
//! No Atsumi AppState, account, user DB, real broadcast or existing profile opens.
mod streaming {
    pub use atsumi_lib::streaming::model;
}
#[path = "../src/streaming/browser_fmp4.rs"]
mod fmp4;

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows/WebView2 required");
    std::process::exit(2);
}
#[cfg(windows)]
fn main() {
    if let Err(error) = probe::run() {
        eprintln!("ENCODED_PROBE_FAILED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod probe {
    use super::fmp4::{EncodedMuxer, EncodedSegment, EncodedTrackInput};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::{json, Value};
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            mpsc, Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    const SCHEME: &str = "atsumi-encoded-probe";
    const ORIGIN: &str = "https://atsumi-encoded-probe.localhost/probe";
    const PREFIX: &str = "ATSUMI_ENCODED_PROBE:";
    static CODE: AtomicI32 = AtomicI32::new(2);
    fn exit(app: &tauri::AppHandle, code: i32) {
        CODE.store(code, Ordering::Release);
        app.exit(code);
    }
    struct Sink {
        tracks: Vec<EncodedTrackInput>,
        mux: Option<EncodedMuxer>,
        output: PathBuf,
        files: usize,
        duration: f64,
    }
    impl Sink {
        fn save(&mut self, segments: Vec<EncodedSegment>) -> Result<(), String> {
            for segment in segments {
                if self.files >= 8 || segment.bytes.len() > 32 * 1024 * 1024 {
                    return Err("output_bound".into());
                }
                let path = self.output.join(format!("segment-{:03}.mp4", self.files));
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                file.write_all(&segment.bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
                self.files += 1;
                self.duration += segment.duration_seconds;
                println!(
                    "{}",
                    json!({"kind":"encoded_segment","path":path,"bytes":segment.bytes.len(),
                    "durationSeconds":segment.duration_seconds,"sourceStart":segment.source_start_seconds,"sourceEnd":segment.source_end_seconds})
                );
            }
            Ok(())
        }
        fn accept(&mut self, value: &Value) -> Result<bool, String> {
            match value["kind"].as_str().unwrap_or("") {
                "error" => Err(format!(
                    "encoder: {}",
                    value["error"]
                        .as_str()
                        .unwrap_or("unknown")
                        .chars()
                        .take(200)
                        .collect::<String>()
                )),
                "init" => {
                    if self.mux.is_some() || self.tracks.len() >= 2 {
                        return Err("too_many_init".into());
                    }
                    let index = value["trackIndex"]
                        .as_u64()
                        .filter(|n| *n < 2)
                        .ok_or("invalid_track")? as u32;
                    let mime = value["mimeType"]
                        .as_str()
                        .filter(|s| s.len() < 120)
                        .ok_or("invalid_mime")?;
                    let bytes = STANDARD
                        .decode(
                            value["data"]
                                .as_str()
                                .filter(|s| s.len() < 90_000)
                                .ok_or("invalid_init")?,
                        )
                        .map_err(|_| "invalid_base64")?;
                    self.tracks.push(EncodedTrackInput {
                        track_index: index,
                        mime_type: mime.into(),
                        init: bytes,
                    });
                    if self.tracks.len() == 2 {
                        self.mux = Some(
                            EncodedMuxer::new(std::mem::take(&mut self.tracks))
                                .map_err(|e| format!("{}: {}", e.code, e.message))?,
                        );
                    }
                    Ok(false)
                }
                "append" => {
                    let index = value["trackIndex"]
                        .as_u64()
                        .filter(|n| *n < 2)
                        .ok_or("invalid_track")? as u32;
                    let bytes = STANDARD
                        .decode(
                            value["data"]
                                .as_str()
                                .filter(|s| s.len() <= 175_000)
                                .ok_or("invalid_append")?,
                        )
                        .map_err(|_| "invalid_base64")?;
                    let segments = self
                        .mux
                        .as_mut()
                        .ok_or("append_before_init")?
                        .push(index, &bytes)
                        .map_err(|e| format!("{}: {}", e.code, e.message))?;
                    self.save(segments)?;
                    Ok(false)
                }
                "done" => {
                    let segments = self
                        .mux
                        .as_mut()
                        .ok_or("missing_mux")?
                        .finish()
                        .map_err(|e| format!("{}: {}", e.code, e.message))?;
                    self.save(segments)?;
                    if self.files < 2 || !(15.0..17.0).contains(&self.duration) {
                        return Err("duration_or_rotation_failed".into());
                    }
                    println!(
                        "{}",
                        json!({"kind":"encoded_mux_probe_pass","segments":self.files,"durationSeconds":self.duration,
                        "source":"local WebCodecs synthetic encoder","independentDecodeStillRequired":true})
                    );
                    Ok(true)
                }
                _ => Err("unknown_message".into()),
            }
        }
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let supplied = std::env::args_os()
            .nth(1)
            .ok_or("Pass the local bundled probe generator .js path")?;
        let script_path = PathBuf::from(supplied).canonicalize()?;
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("workspace")?
            .canonicalize()?;
        if !script_path.starts_with(workspace.join(".runtime"))
            || script_path.extension().and_then(|v| v.to_str()) != Some("js")
        {
            return Err("generator must be .runtime/*.js".into());
        }
        if fs::metadata(&script_path)?.len() > 2 * 1024 * 1024 {
            return Err("generator_too_large".into());
        }
        let script = fs::read_to_string(&script_path)?;
        let root = tempfile::Builder::new()
            .prefix("atsumi-encoded-mux-probe-")
            .tempdir()?
            .keep();
        let output = root.join("output");
        fs::create_dir(&output)?;
        println!(
            "{}",
            json!({"kind":"encoded_probe_root","path":root,"networkMedia":false,"userDatabase":false})
        );
        let html = format!(
            "<!doctype html><meta charset=utf-8><title>Local codec test</title><script>{}</script>",
            script.replace("</script", "<\\/script")
        );
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        context.config_mut().app.security.csp = None;
        context.config_mut().app.security.dev_csp = None;
        tauri::Builder::default().register_uri_scheme_protocol(SCHEME, move |_, request| {
            if request.uri().path() != "/probe" || request.uri().query().is_some() {
                return tauri::http::Response::builder().status(403).body(Vec::new()).unwrap();
            }
            tauri::http::Response::builder().header("content-type", "text/html; charset=utf-8")
                .header("content-security-policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'none'; media-src blob:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
                .body(html.as_bytes().to_vec()).unwrap()
        }).setup(move |app| {
            let window = WebviewWindowBuilder::new(app,"encoded-synthetic-probe",WebviewUrl::CustomProtocol(format!("{SCHEME}://localhost/probe").parse()?))
                .visible(false).inner_size(640.0,360.0).use_https_scheme(true).data_directory(root.join("profile"))
                .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
                .on_navigation(|url| url.as_str() == ORIGIN || url.as_str() == format!("{SCHEME}://localhost/probe"))
                .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).build()?;
            let attached = Arc::new(AtomicBool::new(false)); let started = Arc::new(AtomicBool::new(false)); let done = Arc::new(AtomicBool::new(false));
            let (sender,receiver) = mpsc::sync_channel::<Value>(4);
            let target = window.clone(); let worker_app = app.handle().clone(); let worker_done = done.clone(); let worker_started = started.clone();
            thread::spawn(move || {
                let mut sink = Sink {tracks: Vec::new(),mux: None,output,files:0,duration:0.0};
                while let Ok(value) = receiver.recv() {
                    worker_started.store(true,Ordering::Release);
                    let result = sink.accept(&value); let success = result.is_ok();
                    let reply = json!({"id":value["id"],"ok":success});
                    let _ = target.eval(&format!("window.dispatchEvent(new CustomEvent('atsumi-encoded-probe-reply',{{detail:{reply}}}));"));
                    match result {
                        Ok(true) => { worker_done.store(true,Ordering::Release); exit(&worker_app,0); return; },
                        Err(cause) => { eprintln!("ENCODED_SYNTHETIC_FAILURE: {cause}"); worker_done.store(true,Ordering::Release); exit(&worker_app,3); return; },
                        _ => {},
                    }
                }
            });
            let native_attached = attached.clone(); let attach_app = app.handle().clone();
            window.with_webview(move |platform| unsafe {
                let outcome = (|| -> windows::core::Result<()> {
                    let core = platform.controller().CoreWebView2()?; let mut token=0;
                    core.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_,args| {
                        if let Some(args)=args {
                            let mut source=PWSTR::null(); args.Source(&mut source)?;
                            if CoTaskMemPWSTR::from(source).to_string()!=ORIGIN { return Ok(()); }
                            let mut body=PWSTR::null(); if args.TryGetWebMessageAsString(&mut body).is_err() {return Ok(());}
                            let body=CoTaskMemPWSTR::from(body).to_string();
                            if body.len()>300_000 {return Ok(());} let Some(body)=body.strip_prefix(PREFIX) else{return Ok(());};
                            let Ok(value)=serde_json::from_str::<Value>(body) else{return Ok(());};
                            if !value["id"].as_str().is_some_and(|v| uuid::Uuid::parse_str(v).is_ok()) {return Ok(());}
                            let _=sender.try_send(value);
                        } Ok(())
                    })),&mut token)?; native_attached.store(true,Ordering::Release); Ok(())
                })();
                if outcome.is_err(){exit(&attach_app,3);}
            })?;
            let watchdog=app.handle().clone();
            thread::spawn(move || {let begin=Instant::now(); while begin.elapsed()<Duration::from_secs(50) {
                if done.load(Ordering::Acquire){return;}
                if attached.load(Ordering::Acquire)&&!started.load(Ordering::Acquire) {
                    let _=window.eval("if(window.__atsumiEncodedProbeRun)void window.__atsumiEncodedProbeRun();");
                }
                thread::sleep(Duration::from_millis(250));
            } eprintln!("ENCODED_PROBE_DEADLINE"); exit(&watchdog,2);});
            Ok(())
        }).run(context)?;
        let code = CODE.load(Ordering::Acquire);
        if code != 0 {
            return Err(format!("probe exit {code}").into());
        }
        Ok(())
    }
}
