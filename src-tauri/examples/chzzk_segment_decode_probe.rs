//! Read-only independent decoding check for explicitly named synthetic segments.
//! cargo run --example chzzk_segment_decode_probe -- SEGMENT_0.webm SEGMENT_1.webm
//!
//! No application DB, network media, existing profile or recording writer is
//! opened. Each allowlisted file is opened read-only and held against replacement.
//! A fresh video element independently decodes the beginning of EACH file. This
//! checks dimensions, decoded frames and non-silent captured audio (the synthetic
//! source contains a 440 Hz tone), not every packet or an entire recording.

#[cfg(not(windows))]
fn main() {
    eprintln!("chzzk_segment_decode_probe requires Windows/WebView2");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    match windows_probe::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("DECODE_PROBE_SETUP_FAILED: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
mod windows_probe {
    use serde_json::{json, Value};
    use std::{
        collections::HashSet,
        fs::{self, File, OpenOptions},
        io::{Read, Seek, SeekFrom},
        os::windows::fs::{MetadataExt, OpenOptionsExt},
        path::{Component, PathBuf, Prefix},
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;

    const SCHEME: &str = "atsumi-segment-decode";
    const ORIGIN: &str = "https://atsumi-segment-decode.localhost";
    const MAX_FILE: u64 = 64 * 1024 * 1024;
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);

    struct AllowedFile {
        path: PathBuf,
        alias: String,
        mime: &'static str,
        bytes: u64,
        file: Mutex<File>,
    }

    const HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<title>Atsumi independent segment decoding probe</title>
<style>body{margin:0;background:#172333}video{width:640px;height:360px}</style>
</head><body><script>
(() => {
  const files = __FILES__;
  const send = message => window.chrome.webview.postMessage('ATSUMI_BROWSER_CAPTURE:'+JSON.stringify({atsumiSegmentDecodeProbe:1,...message}));
  let running = false;
  window.__atsumiDecodeRun = async () => {
    if (running) return;
    running = true;
    send({kind:'decode_started',count:files.length});
    for (const file of files) send({kind:'probe_result',index:file.index,...await inspect(file)});
  };
  async function inspect(file) {
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.preload = 'auto';
    document.body.appendChild(video);
    let captured, audio, timeout, cancelled = false;
    const operation = async () => {
      const metadata = new Promise((resolve,reject) => {
        const clean = () => {video.removeEventListener('loadedmetadata',loaded);video.removeEventListener('error',failed);};
        const loaded = () => {clean();resolve();};
        const failed = () => {clean();reject(new Error('media_error_'+(video.error?.code??0)));};
        video.addEventListener('loadedmetadata',loaded);
        video.addEventListener('error',failed);
      });
      video.src = file.url;
      await metadata;
      if (cancelled) throw new Error('file_timeout');
      await video.play();
      if (cancelled) throw new Error('file_timeout');
      if (typeof video.captureStream !== 'function') throw new Error('capture_stream_unavailable');
      captured = video.captureStream();
      const audioTracks = captured.getAudioTracks();
      const videoTracks = captured.getVideoTracks();
      if (!audioTracks.length || !videoTracks.length) throw new Error('captured_track_missing');
      audio = new AudioContext();
      await audio.resume();
      if (cancelled) throw new Error('file_timeout');
      const source = audio.createMediaStreamSource(new MediaStream(audioTracks));
      const analyzer = audio.createAnalyser();
      analyzer.fftSize = 4096;
      const silent = audio.createGain();
      silent.gain.value = 0;
      // A zero-gain destination keeps the processing graph active without sound.
      source.connect(analyzer).connect(silent).connect(audio.destination);
      const samples = new Float32Array(analyzer.fftSize);
      const spectrum = new Float32Array(analyzer.frequencyBinCount);
      let peakRms = 0, dominantFrequencyHz = 0;
      for (let round=0;round<12;round++) {
        await new Promise(resolve=>setTimeout(resolve,80));
        if (cancelled) throw new Error('file_timeout');
        analyzer.getFloatTimeDomainData(samples);
        const rms = Math.sqrt(samples.reduce((sum,sample)=>sum+sample*sample,0)/samples.length);
        if (rms > peakRms) {
          peakRms = rms;
          analyzer.getFloatFrequencyData(spectrum);
          let bin = 1;
          for(let index=2;index<spectrum.length;index++) if(spectrum[index]>spectrum[bin]) bin=index;
          dominantFrequencyHz = bin*audio.sampleRate/analyzer.fftSize;
        }
      }
      const frames = video.getVideoPlaybackQuality?.().totalVideoFrames ?? 0;
      const result = {videoWidth:video.videoWidth,videoHeight:video.videoHeight,
        durationSeconds:Number.isFinite(video.duration)?video.duration:null,
        currentTime:video.currentTime,totalVideoFrames:frames,audioTracks:audioTracks.length,
        videoTracks:videoTracks.length,audioPeakRms:peakRms,dominantFrequencyHz,
        sampleRate:audio.sampleRate,freshVideoElement:true};
      return {...result,ok:video.videoWidth>0&&video.videoHeight>0&&frames>0&&peakRms>0.0001};
    };
    try {
      return await Promise.race([operation(),new Promise((_,reject)=>{
        timeout=setTimeout(()=>reject(new Error('file_timeout')),8000);
      })]);
    } catch(error) {
      const code = String(error?.message??'decode_failed');
      return {ok:false,code:/^[a-z0-9_]+$/.test(code)?code:'decode_failed',freshVideoElement:true};
    } finally {
      cancelled = true;
      clearTimeout(timeout);
      video.pause();
      captured?.getTracks().forEach(track=>track.stop());
      if (audio) await audio.close().catch(()=>{});
      video.removeAttribute('src');
      video.load();
      video.remove();
    }
  }
})();
</script></body></html>"#;

    fn open_allowlist() -> Result<Vec<AllowedFile>, Box<dyn std::error::Error>> {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        if args.is_empty() || args.len() > 4 {
            return Err("Pass 1 to 4 explicit synthetic .webm/.mp4 segment paths".into());
        }
        let mut paths = HashSet::new();
        let mut files = Vec::new();
        for argument in args {
            let input = PathBuf::from(argument);
            if input
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            {
                return Err("Parent traversal is not accepted".into());
            }
            let absolute = if input.is_absolute() {
                input
            } else {
                std::env::current_dir()?.join(input)
            };
            if !matches!(absolute.components().next(), Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            {
                return Err(
                    "Only local disk files are accepted; UNC/device paths are not allowed".into(),
                );
            }
            for ancestor in absolute.ancestors() {
                let metadata = fs::symlink_metadata(ancestor)?;
                if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 {
                    return Err("Symlinks, junctions and reparse points are not accepted".into());
                }
            }
            let path = absolute.canonicalize()?;
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mime = match extension.as_str() {
                "webm" => "video/webm",
                "mp4" => "video/mp4",
                _ => return Err("Only .webm/.mp4 files are accepted".into()),
            };
            if !paths.insert(path.to_string_lossy().to_ascii_lowercase()) {
                return Err("Duplicate input file".into());
            }
            // OPEN_REPARSE_POINT + READ-only sharing keeps the selected source
            // identity stable; no rename, write, copy or repair is performed.
            let file = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(&path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.file_attributes() & 0x400 != 0
                || metadata.len() == 0
                || metadata.len() > MAX_FILE
            {
                return Err("Invalid source or source exceeds 64 MiB".into());
            }
            files.push(AllowedFile {
                path,
                alias: format!("/media/{}.{}", files.len(), extension),
                mime,
                bytes: metadata.len(),
                file: Mutex::new(file),
            });
        }
        Ok(files)
    }

    fn range(header: Option<&str>, total: u64) -> Option<(u64, u64, bool)> {
        let Some(header) = header else {
            return Some((0, total - 1, false));
        };
        let value = header.strip_prefix("bytes=")?;
        if value.contains(',') {
            return None;
        }
        let (start, end) = value.split_once('-')?;
        let (start, end) = if start.is_empty() {
            let suffix: u64 = end.parse().ok()?;
            if suffix == 0 {
                return None;
            }
            (total.saturating_sub(suffix), total - 1)
        } else {
            let start = start.parse::<u64>().ok()?;
            let end = if end.is_empty() {
                total - 1
            } else {
                end.parse::<u64>().ok()?.min(total - 1)
            };
            (start, end)
        };
        (start < total && end >= start).then_some((start, end, true))
    }

    fn exit(app: &tauri::AppHandle, code: i32) {
        EXIT_CODE.store(code, Ordering::Release);
        app.exit(code);
    }

    pub fn run() -> Result<i32, Box<dyn std::error::Error>> {
        let files = Arc::new(open_allowlist()?);
        let profile = tempfile::Builder::new()
            .prefix("atsumi-segment-decode-profile-")
            .tempdir()?
            .keep();
        println!(
            "{}",
            json!({"kind":"decode_probe_profile","path":profile,"fileCount":files.len(),"sourceReadOnly":true})
        );
        let manifest = files
            .iter()
            .enumerate()
            .map(|(index, file)| json!({"index":index,"url":file.alias}))
            .collect::<Vec<_>>();
        let html = HTML.replace("__FILES__", &serde_json::to_string(&manifest)?);
        let protocol_files = files.clone();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        context.config_mut().app.security.csp = None;
        context.config_mut().app.security.dev_csp = None;
        tauri::Builder::default()
            .register_uri_scheme_protocol(SCHEME, move |_context, request| {
                let empty = |status| tauri::http::Response::builder().status(status).body(Vec::new()).unwrap();
                if request.method() != "GET" && request.method() != "HEAD" { return empty(405); }
                if request.uri().query().is_some() { return empty(404); }
                if request.uri().path() == "/probe" {
                    return tauri::http::Response::builder().header("content-type", "text/html; charset=utf-8")
                        .header("content-security-policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; media-src 'self' blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
                        .header("cache-control", "no-store").body(html.as_bytes().to_vec()).unwrap();
                }
                let Some(file) = protocol_files.iter().find(|file| request.uri().path() == file.alias) else { return empty(404); };
                let header = request.headers().get("range").and_then(|value| value.to_str().ok());
                let Some((start, end, partial)) = range(header, file.bytes) else {
                    return tauri::http::Response::builder().status(416)
                        .header("content-range", format!("bytes */{}", file.bytes)).body(Vec::new()).unwrap();
                };
                let length = (end - start + 1) as usize;
                let mut bytes = Vec::new();
                if request.method() != "HEAD" {
                    bytes.resize(length, 0);
                    let Ok(mut source) = file.file.lock() else { return empty(500); };
                    if source.seek(SeekFrom::Start(start)).is_err() || source.read_exact(&mut bytes).is_err() { return empty(500); }
                }
                let mut response = tauri::http::Response::builder().status(if partial { 206 } else { 200 })
                    .header("content-type", file.mime).header("content-length", length)
                    .header("accept-ranges", "bytes").header("cache-control", "no-store");
                if partial { response = response.header("content-range", format!("bytes {start}-{end}/{}", file.bytes)); }
                response.body(bytes).unwrap()
            })
            .setup(move |app| {
                let window = WebviewWindowBuilder::new(app, "synthetic-segment-decode-probe",
                    WebviewUrl::CustomProtocol(format!("{SCHEME}://localhost/probe").parse()?))
                    .title("Atsumi independent segment decode verification")
                    .inner_size(800.0, 500.0).visible(false).use_https_scheme(true).data_directory(profile)
                    .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
                    .on_navigation(|url| url.as_str() == format!("{ORIGIN}/probe") || url.as_str() == format!("{SCHEME}://localhost/probe"))
                    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny).build()?;
                let attached = Arc::new(AtomicBool::new(false));
                let started = Arc::new(AtomicBool::new(false));
                let done = Arc::new(AtomicBool::new(false));
                let native_attached = attached.clone();
                let native_started = started.clone();
                let native_done = done.clone();
                let event_app = app.handle().clone();
                let attach_app = event_app.clone();
                window.with_webview(move |platform| unsafe {
                    let outcome = (|| -> windows::core::Result<()> {
                        let core = platform.controller().CoreWebView2()?;
                        let results = Mutex::new(vec![None; files.len()]);
                        let mut token = 0;
                        core.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                            if let Some(args) = args {
                                let mut source = PWSTR::null(); args.Source(&mut source)?;
                                if CoTaskMemPWSTR::from(source).to_string() != format!("{ORIGIN}/probe") { return Ok(()); }
                                let mut body = PWSTR::null(); args.WebMessageAsJson(&mut body)?;
                                let body = CoTaskMemPWSTR::from(body).to_string();
                                if body.len() > 12288 { return Ok(()); }
                                let Ok(payload) = serde_json::from_str::<String>(&body) else { return Ok(()); };
                                let Some(body) = payload.strip_prefix("ATSUMI_BROWSER_CAPTURE:") else { return Ok(()); };
                                if body.len() > 8192 { return Ok(()); }
                                let Ok(value) = serde_json::from_str::<Value>(body) else { return Ok(()); };
                                if value["atsumiSegmentDecodeProbe"] != 1 { return Ok(()); }
                                if value["kind"] == "decode_started" {
                                    native_started.store(true, Ordering::Release);
                                    println!("{}", json!({"kind":"decode_started","fileCount":files.len()}));
                                } else if value["kind"] == "probe_result" {
                                    let Some(index) = value["index"].as_u64().and_then(|index| usize::try_from(index).ok()).filter(|index| *index < files.len()) else { return Ok(()); };
                                    let valid = value["ok"] == true && value["freshVideoElement"] == true &&
                                        value["videoWidth"].as_u64().is_some_and(|width| width > 0) &&
                                        value["videoHeight"].as_u64().is_some_and(|height| height > 0) &&
                                        value["totalVideoFrames"].as_u64().is_some_and(|frames| frames > 0) &&
                                        value["videoTracks"].as_u64().is_some_and(|tracks| tracks > 0) &&
                                        value["audioTracks"].as_u64().is_some_and(|tracks| tracks > 0) &&
                                        value["audioPeakRms"].as_f64().is_some_and(|rms| rms.is_finite() && rms > 0.0001);
                                    let mut results = results.lock().unwrap_or_else(|poison| poison.into_inner());
                                    if results[index].is_none() {
                                        println!("{}", json!({"kind":"probe_result","path":files[index].path,"ok":valid,"result":value}));
                                        results[index] = Some(valid);
                                    }
                                    if results.iter().all(Option::is_some) {
                                        let passed = results.iter().all(|value| *value == Some(true));
                                        println!("{}", json!({"kind":"decode_probe_complete","ok":passed,"fileCount":files.len()}));
                                        native_done.store(true, Ordering::Release);
                                        exit(&event_app, if passed { 0 } else { 3 });
                                    }
                                }
                            }
                            Ok(())
                        })), &mut token)?;
                        native_attached.store(true, Ordering::Release);
                        Ok(())
                    })();
                    if let Err(error) = outcome {
                        eprintln!("DECODE_NATIVE_ATTACH_FAILED: {}", error.code().0);
                        exit(&attach_app, 3);
                    }
                })?;
                let watchdog = app.handle().clone();
                thread::spawn(move || {
                    let beginning = Instant::now();
                    while beginning.elapsed() < Duration::from_secs(45) {
                        if done.load(Ordering::Acquire) { return; }
                        if attached.load(Ordering::Acquire) && !started.load(Ordering::Acquire) {
                            let _ = window.eval("if(window.__atsumiDecodeRun)void window.__atsumiDecodeRun();");
                        }
                        thread::sleep(Duration::from_millis(250));
                    }
                    eprintln!("DECODE_PROBE_DEADLINE");
                    exit(&watchdog, 2);
                });
                Ok(())
            }).run(context)?;
        Ok(EXIT_CODE.load(Ordering::Acquire))
    }
}
