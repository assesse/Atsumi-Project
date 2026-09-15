//! Bounded Windows WebView2/MediaRecorder smoke probe, without real broadcasts.
//!
//! Run explicitly: cargo run --example chzzk_browser_capture_probe
//! No frontend server, accounts, application DB, existing recordings or network
//! media are used. Tauri's Wry runtime serves one in-memory custom-protocol page.
//! A NEW temporary root holds the isolated WebView profile, store and recording.
//! That root is intentionally retained; stdout prints each segment's absolute
//! path for a separate ffprobe check. Nothing is deleted from existing paths.
//!
//! The production bridge is included unchanged except for ONE allowed-origin
//! literal replacement. Thus capability detection, real video.captureStream(),
//! MediaRecorder.start(1000), 15-second recorder replacement and ACK queueing all
//! run as production code. This is a codec/transport smoke check, not a CHZZK,
//! network-latency or background-timer guarantee. Synthetic input alone cannot
//! prove that captureStream works with every site's MSE/DRM playback pipeline.

#[cfg(not(windows))]
fn main() {
    eprintln!("chzzk_browser_capture_probe requires Windows/WebView2");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    match windows_probe::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("PROBE_SETUP_FAILED: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
mod windows_probe {
    use std::{
        collections::BTreeMap,
        fs::{self, File},
        io::Read,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicI32, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    use atsumi_lib::streaming::browser_store::{
        BrowserCaptureStore, BrowserRecording, BrowserRecordingStatus,
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::{json, Value};
    use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
    use webview2_com::{CoTaskMemPWSTR, ProcessFailedEventHandler, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;

    const SCHEME: &str = "atsumi-capture-probe";
    const ORIGIN: &str = "https://atsumi-capture-probe.localhost";
    const CHANNEL: &str = "00000000000000000000000000000001";
    const STOP_AFTER: Duration = Duration::from_secs(19);
    const DEADLINE: Duration = Duration::from_secs(45);
    const MAX_MESSAGE: usize = 300_000;
    static EXIT_CODE: AtomicI32 = AtomicI32::new(2);
    const DIAGNOSTIC: &str = r#"(() => {
      if (!window.chrome?.webview?.postMessage) return;
      window.chrome.webview.postMessage('ATSUMI_BROWSER_CAPTURE:'+JSON.stringify({atsumiCaptureProbe:1,kind:'page_diagnostic',
        origin:location.origin,path:location.pathname,readyState:document.readyState,
        bridgeInstalled:!!window.__atsumiBrowserCaptureInstalled,
        videoCount:document.querySelectorAll('video').length,secureContext:isSecureContext}));
      window.dispatchEvent(new Event('atsumi-probe-ready-request'));
    })();"#;

    // Everything is synthesized locally. The AudioContext destination is NOT
    // connected to the speaker, and the video is muted. Muting the playback
    // element should not remove its captured audio track; this probe verifies it.
    const HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Atsumi synthetic capture probe</title>
<style>body{margin:0;background:#172333;color:#fff;font:16px sans-serif}video{width:640px;height:360px}canvas{display:none}</style>
</head><body><video autoplay muted playsinline></video><canvas width="640" height="360"></canvas>
<script>
(() => {
  let latestFixtureState = null;
  const report = (kind, fields = {}) => {
    const message = {atsumiCaptureProbe:1,kind,...fields};
    if (kind === 'fixture_ready' || kind === 'fixture_error' || kind === 'fixture_stage') latestFixtureState = message;
    window.chrome.webview.postMessage('ATSUMI_BROWSER_CAPTURE:'+JSON.stringify(message));
  };
  window.addEventListener('atsumi-probe-ready-request',() => {
    if (latestFixtureState) window.chrome.webview.postMessage('ATSUMI_BROWSER_CAPTURE:'+JSON.stringify(latestFixtureState));
  });
  window.addEventListener('securitypolicyviolation',event => report('fixture_error',
    {code:'csp_violation',directive:event.effectiveDirective}));
  const run = async () => {
    report('fixture_stage',{stage:'script_running'});
    const canvas = document.querySelector('canvas');
    const context = canvas.getContext('2d');
    let frame = 0;
    const draw = () => {
      frame++;
      context.fillStyle = `hsl(${frame % 360} 55% 22%)`;
      context.fillRect(0,0,640,360);
      context.fillStyle = '#4df2a4';
      context.fillRect((frame*5)%600,70,40,180);
      context.fillStyle = '#fff';
      context.font = '32px monospace';
      context.fillText('LOCAL SYNTHETIC '+frame,30,45);
      context.fillText((performance.now()/1000).toFixed(3)+' s',30,325);
    };
    draw();
    const canvasStream = canvas.captureStream(30);
    const audio = new AudioContext({sampleRate:48000});
    const oscillator = audio.createOscillator();
    const gain = audio.createGain();
    const destination = audio.createMediaStreamDestination();
    oscillator.frequency.value = 440;
    gain.gain.value = 0.05;
    oscillator.connect(gain).connect(destination);
    oscillator.start();
    report('fixture_stage',{stage:'audio_resume_start',audioState:audio.state});
    await audio.resume();
    report('fixture_stage',{stage:'audio_running',audioState:audio.state});
    const source = new MediaStream([...canvasStream.getVideoTracks(),...destination.stream.getAudioTracks()]);
    const video = document.querySelector('video');
    video.srcObject = source;
    report('fixture_stage',{stage:'video_play_start'});
    await video.play();
    report('fixture_stage',{stage:'video_playing'});
    // Observe, but do not substitute, the REAL API called by the production
    // bridge. The returned tracks are passed through unchanged to MediaRecorder.
    if (typeof video.captureStream === 'function') {
      const capture = video.captureStream.bind(video);
      video.captureStream = (...args) => {
        const captured = capture(...args);
        report('captured_stream',{videoTracks:captured.getVideoTracks().length,
          audioTracks:captured.getAudioTracks().length,
          allTracksLive:captured.getTracks().every(track=>track.readyState==='live')});
        return captured;
      };
    }
    setInterval(draw,1000/30);
    report('fixture_ready',{secureContext:isSecureContext,source:'canvas+oscillator->video.srcObject',
      videoTracks:source.getVideoTracks().length,audioTracks:source.getAudioTracks().length,
      captureStream:typeof video.captureStream==='function',audioState:audio.state,
      mediaRecorder:typeof MediaRecorder!=='undefined',width:video.videoWidth,height:video.videoHeight});
  };
  run().catch(error => report('fixture_error',{code:'fixture_setup_failed',name:String(error?.name??'Error').slice(0,64)}));
})();
</script></body></html>"#;

    struct Probe {
        store: Arc<BrowserCaptureStore>,
        data_dir: PathBuf,
        output_root: PathBuf,
        request_id: String,
        fixture_ready: bool,
        captured_stream_verified: bool,
        start_sent: bool,
        stop_sent: bool,
        began: Option<Instant>,
        recording: Option<BrowserRecording>,
        finished: Option<BrowserRecording>,
        chunk_count: u64,
        next_chunk: BTreeMap<u64, u64>,
        arrival_groups: Vec<f64>,
        success: bool,
    }

    impl Probe {
        fn message(
            &mut self,
            window: &WebviewWindow,
            value: &Value,
        ) -> Result<Option<Value>, String> {
            if value.get("atsumiCaptureProbe").and_then(Value::as_u64) == Some(1) {
                match value.get("kind").and_then(Value::as_str) {
                    Some("page_diagnostic" | "fixture_stage") => {
                        println!("{value}");
                    }
                    Some("fixture_ready") => {
                        println!("{value}");
                        if value["secureContext"] != true
                            || value["captureStream"] != true
                            || value["mediaRecorder"] != true
                            || value["videoTracks"] != 1
                            || value["audioTracks"] != 1
                            || value["audioState"] != "running"
                        {
                            return Err("fixture_capability_failed".into());
                        }
                        self.fixture_ready = true;
                    }
                    Some("fixture_error") => return Err("fixture_setup_failed".into()),
                    Some("captured_stream") => {
                        println!("{value}");
                        if value["videoTracks"] != 1
                            || value["audioTracks"] != 1
                            || value["allTracksLive"] != true
                        {
                            return Err("actual_video_capture_tracks_failed".into());
                        }
                        self.captured_stream_verified = true;
                    }
                    Some("renderer_failed") => return Err("renderer_failed".into()),
                    _ => return Err("unexpected_fixture_message".into()),
                }
                return Ok(None);
            }
            if value.get("atsumiBrowserCapture").and_then(Value::as_u64) != Some(1) {
                return Err("unexpected_bridge_marker".into());
            }
            let id = field(value, "id")?;
            uuid::Uuid::parse_str(id).map_err(|_| "invalid_message_id")?;
            let kind = field(value, "kind")?;
            if kind == "status" {
                if field(value, "channelId")? != CHANNEL {
                    return Err("wrong_status_channel".into());
                }
                if !self.fixture_ready {
                    // Document load can beat native handler attachment. Ask the
                    // fixture to replay its readiness/error, rather than losing it.
                    window
                        .eval("window.dispatchEvent(new Event('atsumi-probe-ready-request'));")
                        .map_err(|_| "fixture_state_request_failed")?;
                }
                let detail = value["detail"].as_str().unwrap_or("");
                if self.finished.is_some() && value["recording"] == false {
                    if detail != "saved" {
                        return Err(format!("bridge_final_status_{detail}"));
                    }
                    self.verify()?;
                    self.success = true;
                } else if self.start_sent
                    && matches!(
                        detail,
                        "no_audio"
                            | "native_rejected"
                            | "recorder_error"
                            | "queue_overflow"
                            | "empty_segment"
                            | "video_changed"
                            | "rate_change"
                            | "seek"
                    )
                {
                    return Err(format!("bridge_status_{detail}"));
                }
                if self.fixture_ready && value["ready"] == true && !self.start_sent {
                    self.start_sent = true;
                    command(
                        window,
                        json!({"kind":"start","channelId":CHANNEL,
                        "requestId":self.request_id,"rightsAcknowledged":true}),
                    )?;
                }
                return Ok(None);
            }
            let result = match kind {
                "begin" => {
                    if !self.start_sent
                        || self.recording.is_some()
                        || field(value, "requestId")? != self.request_id
                        || field(value, "channelId")? != CHANNEL
                    {
                        return Err("begin_not_armed".into());
                    }
                    let record = self
                        .store
                        .begin(
                            &self.output_root,
                            CHANNEL,
                            "Atsumi local synthetic capture probe",
                            field(value, "mimeType")?,
                        )
                        .map_err(|error| error.code)?;
                    println!(
                        "{}",
                        json!({"kind":"recording_started","id":record.id,
                        "mimeType":record.mime_type,"outputDir":record.output_dir})
                    );
                    self.began = Some(Instant::now());
                    let reply = json!({"id":record.id});
                    self.recording = Some(record);
                    reply
                }
                "chunk" => {
                    let record = self.match_recording(value)?;
                    let segment = integer(value, "segmentIndex")?;
                    let chunk = integer(value, "chunkIndex")?;
                    if segment > 1 {
                        return Err("too_many_segments".into());
                    }
                    if chunk != *self.next_chunk.get(&segment).unwrap_or(&0) {
                        return Err("non_monotonic_chunk_index".into());
                    }
                    let encoded = field(value, "data")?;
                    if encoded.len() > 180_000 {
                        return Err("chunk_payload_too_large".into());
                    }
                    let bytes = STANDARD.decode(encoded).map_err(|_| "invalid_base64")?;
                    if bytes.is_empty() || bytes.len() > 128 * 1024 {
                        return Err("chunk_size_out_of_bounds".into());
                    }
                    let ack = self
                        .store
                        .append(&record.id, segment, chunk, &bytes)
                        .map_err(|error| error.code)?;
                    self.next_chunk.insert(segment, chunk + 1);
                    self.chunk_count += 1;
                    let seconds = self
                        .began
                        .ok_or("chunk_before_begin")?
                        .elapsed()
                        .as_secs_f64();
                    if self
                        .arrival_groups
                        .last()
                        .is_none_or(|last| seconds - last > 0.25)
                    {
                        if self.arrival_groups.len() >= 128 {
                            return Err("too_many_blob_arrival_groups".into());
                        }
                        self.arrival_groups.push(seconds);
                    }
                    serde_json::to_value(ack).map_err(|_| "ack_serialization_failed")?
                }
                "segment" => {
                    let record = self.match_recording(value)?;
                    let segment = integer(value, "segmentIndex")?;
                    let seconds = value["durationSeconds"]
                        .as_f64()
                        .ok_or("invalid_segment_duration")?;
                    let result = self
                        .store
                        .finish_segment(&record.id, segment, seconds)
                        .map_err(|error| error.code)?;
                    println!(
                        "{}",
                        json!({"kind":"segment_committed","index":segment,
                        "durationSeconds":seconds,"segmentCount":result.segment_count})
                    );
                    json!({"saved":true,"segmentIndex":segment})
                }
                "finish" => {
                    let record = self.match_recording(value)?;
                    let interrupted = value["interrupted"].as_bool().ok_or("invalid_finish")?;
                    let record = self
                        .store
                        .finish(&record.id, interrupted, value["reason"].as_str())
                        .map_err(|error| error.code)?;
                    let reply = json!({"stopped":true,
                        "interrupted":record.status == BrowserRecordingStatus::Interrupted,"status":record.status});
                    self.finished = Some(record);
                    reply
                }
                _ => return Err("unexpected_bridge_kind".into()),
            };
            Ok(Some(result))
        }

        fn match_recording(&self, value: &Value) -> Result<BrowserRecording, String> {
            let record = self.recording.as_ref().ok_or("recording_not_started")?;
            if field(value, "recordingId")? != record.id {
                return Err("wrong_recording_id".into());
            }
            Ok(record.clone())
        }

        fn tick(&mut self, window: &WebviewWindow) -> Result<(), String> {
            if !self.stop_sent
                && self
                    .began
                    .is_some_and(|began| began.elapsed() >= STOP_AFTER)
            {
                self.stop_sent = true;
                command(window, json!({"kind":"stop","channelId":CHANNEL}))?;
            }
            Ok(())
        }

        fn verify(&self) -> Result<(), String> {
            let record = self.finished.as_ref().ok_or("missing_finished_recording")?;
            if !self.captured_stream_verified
                || record.status != BrowserRecordingStatus::Stopped
                || record.partial.is_some()
                || record.segment_count != 2
                || record.segments.len() != 2
                || record.bytes_written == 0
                || !(18.0..=21.5).contains(&record.duration_seconds)
            {
                return Err("final_store_invariants_failed".into());
            }
            if !(14.0..=17.0).contains(&record.segments[0].duration_seconds)
                || record.segments[0].index != 0
                || record.segments[1].index != 1
            {
                return Err("independent_15_second_rotation_failed".into());
            }
            // Each Blob may be split into several 128 KiB messages. Group close
            // arrivals before checking cadence, rather than counting chunks as seconds.
            let mut intervals = self
                .arrival_groups
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            intervals.sort_by(f64::total_cmp);
            let median = intervals.get(intervals.len() / 2).copied().unwrap_or(0.0);
            if self.arrival_groups.len() < 10 || !(0.5..=2.5).contains(&median) {
                return Err("periodic_chunk_cadence_failed".into());
            }
            let allowed_root = self
                .output_root
                .canonicalize()
                .map_err(|_| "probe_output_missing")?;
            for segment in &record.segments {
                let path = Path::new(&record.output_dir)
                    .join(&segment.file)
                    .canonicalize()
                    .map_err(|_| "segment_missing")?;
                if !path.starts_with(&allowed_root) {
                    return Err("segment_outside_probe_root".into());
                }
                let mut file = File::open(&path).map_err(|_| "segment_open_failed")?;
                let size = file
                    .metadata()
                    .map_err(|_| "segment_metadata_failed")?
                    .len();
                let mut header = [0u8; 12];
                file.read_exact(&mut header)
                    .map_err(|_| "segment_header_missing")?;
                let independent_header = if record.mime_type.starts_with("video/webm") {
                    header[..4] == [0x1a, 0x45, 0xdf, 0xa3]
                } else {
                    &header[4..8] == b"ftyp"
                };
                if size != segment.bytes || !independent_header {
                    return Err("independent_container_header_failed".into());
                }
                println!(
                    "{}",
                    json!({"kind":"segment_file","path":path,"bytes":size,
                    "durationSeconds":segment.duration_seconds,"containerHeaderVerified":true})
                );
            }
            self.store.shutdown().map_err(|error| error.code)?;
            let recovered = BrowserCaptureStore::new(&self.data_dir).map_err(|error| error.code)?;
            let recovered = recovered.snapshot().map_err(|error| error.code)?;
            let recovered = recovered
                .iter()
                .find(|item| item.id == record.id)
                .ok_or("reopen_missing_recording")?;
            if recovered.status != BrowserRecordingStatus::Stopped
                || recovered.segment_count != 2
                || recovered.partial.is_some()
            {
                return Err("reopen_store_invariants_failed".into());
            }
            println!(
                "{}",
                json!({"kind":"probe_pass","recordingId":record.id,
                "durationSeconds":record.duration_seconds,"segmentCount":record.segment_count,
                "chunkCount":self.chunk_count,"blobArrivalGroupsSeconds":self.arrival_groups,
                "medianGroupIntervalSeconds":median,"reopenedStoreVerified":true,
                "realVideoCaptureAudioAndVideoVerified":self.captured_stream_verified,
                "ffprobeStillRequired":true})
            );
            Ok(())
        }
    }

    fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
        value
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing_{name}"))
    }
    fn integer(value: &Value, name: &str) -> Result<u64, String> {
        value
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("missing_{name}"))
    }
    fn command(window: &WebviewWindow, value: Value) -> Result<(), String> {
        window.eval(format!("window.dispatchEvent(new CustomEvent('atsumi-browser-command',{{detail:{value}}}));"))
            .map_err(|_| "command_dispatch_failed".into())
    }
    fn reply(window: &WebviewWindow, id: &str, result: &Result<Option<Value>, String>) {
        let response = match result {
            Ok(Some(data)) => json!({"id":id,"ok":true,"data":data}),
            Err(code) => json!({"id":id,"ok":false,"error":{"code":code}}),
            Ok(None) => return,
        };
        let _ = window.eval(format!(
            "window.dispatchEvent(new CustomEvent('atsumi-browser-reply',{{detail:{response}}}));"
        ));
    }
    fn fail(app: &AppHandle, store: &BrowserCaptureStore, code: &str) {
        let _ = store.shutdown();
        eprintln!("{}", json!({"kind":"probe_failed","code":code}));
        exit(app, 3);
    }
    fn exit(app: &AppHandle, code: i32) {
        EXIT_CODE.store(code, Ordering::Release);
        app.exit(code);
    }

    fn attach(window: &WebviewWindow, mut probe: Probe) -> Result<(), Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::sync_channel::<Value>(8);
        let worker_window = window.clone();
        let app = window.app_handle().clone();
        thread::Builder::new()
            .name("synthetic-capture-writer".into())
            .spawn(move || {
                let started = Instant::now();
                let mut next_diagnostic = Duration::from_secs(1);
                loop {
                    if started.elapsed() >= DEADLINE {
                        fail(&app, &probe.store, "deadline");
                        break;
                    }
                    if let Err(code) = probe.tick(&worker_window) {
                        fail(&app, &probe.store, &code);
                        break;
                    }
                    if !probe.fixture_ready && started.elapsed() >= next_diagnostic {
                        let result = worker_window.eval(DIAGNOSTIC);
                        println!(
                            "{}",
                            json!({"kind":"diagnostic_eval","accepted":result.is_ok()})
                        );
                        next_diagnostic += Duration::from_secs(5);
                    }
                    let value = match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(value) => value,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => {
                            fail(&app, &probe.store, "bridge_disconnected");
                            break;
                        }
                    };
                    let result = probe.message(&worker_window, &value);
                    if let Some(id) = value["id"].as_str() {
                        reply(&worker_window, id, &result);
                    }
                    if let Err(code) = result {
                        fail(&app, &probe.store, &code);
                        break;
                    }
                    if probe.success {
                        exit(&app, 0);
                        break;
                    }
                }
            })?;
        let event_app = window.app_handle().clone();
        let attach_app = event_app.clone();
        let failed_sender = sender.clone();
        println!("{}", json!({"kind":"native_attach_requested"}));
        window.with_webview(move |platform| unsafe {
            println!("{}", json!({"kind":"native_attach_entered"}));
            let outcome = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                let mut token = 0;
                let message_count = AtomicUsize::new(0);
                core.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let mut source = PWSTR::null();
                        args.Source(&mut source)?;
                        let source = CoTaskMemPWSTR::from(source).to_string();
                        let allowed = source == format!("{ORIGIN}/live/{CHANNEL}");
                        if message_count.fetch_add(1, Ordering::Relaxed) < 6 {
                            let parsed = tauri::Url::parse(&source).ok();
                            println!("{}", json!({"kind":"native_message_source","accepted":allowed,
                                "origin":parsed.as_ref().map(|url|url.origin().ascii_serialization()),
                                "path":parsed.as_ref().map(|url|url.path())}));
                        }
                        if !allowed { return Ok(()); }
                        let mut body = PWSTR::null();
                        args.WebMessageAsJson(&mut body)?;
                        let body = CoTaskMemPWSTR::from(body).to_string();
                        if body.len() > MAX_MESSAGE + 4096 { return Ok(()); }
                        let Ok(payload) = serde_json::from_str::<String>(&body) else { return Ok(()); };
                        let Some(body) = payload.strip_prefix("ATSUMI_BROWSER_CAPTURE:") else { return Ok(()); };
                        if body.len() > MAX_MESSAGE { return Ok(()); }
                        if let Ok(value) = serde_json::from_str(body) {
                            if sender.try_send(value).is_err() {
                                eprintln!("PROBE_NATIVE_QUEUE_FULL");
                                exit(&event_app, 3);
                            }
                        }
                    }
                    Ok(())
                })), &mut token)?;
                core.add_ProcessFailed(&ProcessFailedEventHandler::create(Box::new(move |_, _| {
                    let _ = failed_sender.try_send(json!({"atsumiCaptureProbe":1,"kind":"renderer_failed"}));
                    Ok(())
                })), &mut token)?;
                Ok(())
            })();
            if let Err(error) = outcome {
                eprintln!("{}", json!({"kind":"native_attach_failed","hresult":error.code().0}));
                exit(&attach_app, 3);
            } else { println!("{}", json!({"kind":"native_attach_complete"})); }
        })?;
        Ok(())
    }

    pub fn run() -> Result<i32, Box<dyn std::error::Error>> {
        if std::env::args().len() != 1 {
            return Err("This synthetic probe accepts no paths, URLs or account arguments".into());
        }
        let base = tempfile::Builder::new()
            .prefix("atsumi-browser-capture-probe-")
            .tempdir()?
            .keep();
        let data_dir = base.join("isolated-app-data");
        let output_root = base.join("synthetic-recordings");
        let profile = base.join("webview-profile");
        for path in [&data_dir, &output_root, &profile] {
            fs::create_dir(path)?;
        }
        let store = Arc::new(BrowserCaptureStore::new(&data_dir)?);
        println!(
            "{}",
            json!({"kind":"probe_paths","root":base,"profile":profile,
            "appData":data_dir,"recordingRoot":output_root,"retainedForFfprobe":true})
        );
        // This historical codec probe explicitly opts into retired output capture.
        // The production source never enables it or falls back automatically.
        let source = include_str!("../src/streaming/browser_capture.js").replace(
            "const ALLOW_REENCODED_CAPTURE = false;",
            "const ALLOW_REENCODED_CAPTURE = true;",
        );
        if source.matches("https://chzzk.naver.com").count() != 1 {
            return Err(
                "Production bridge origin literal changed; audit the probe substitution".into(),
            );
        }
        let bridge = source.replacen("https://chzzk.naver.com", ORIGIN, 1);
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        // The synthetic response supplies its own strict CSP. Do not inherit a
        // production app policy that might prohibit the fixture's inline script.
        context.config_mut().app.security.csp = None;
        context.config_mut().app.security.dev_csp = None;
        tauri::Builder::default()
            .register_uri_scheme_protocol(SCHEME, move |_context, request| {
                let allowed = request.uri().path() == format!("/live/{CHANNEL}");
                println!("{}", json!({"kind":"protocol_request","path":request.uri().path(),"accepted":allowed}));
                tauri::http::Response::builder().status(if allowed { 200 } else { 404 })
                    .header("content-type", "text/html; charset=utf-8")
                    .header("cache-control", "no-store")
                    .header("content-security-policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; media-src blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
                    .body(if allowed { HTML.as_bytes().to_vec() } else { Vec::new() }).unwrap()
            })
            .setup(move |app| {
                let window = WebviewWindowBuilder::new(app, "synthetic-browser-capture-probe",
                    WebviewUrl::CustomProtocol(format!("{SCHEME}://localhost/live/{CHANNEL}").parse()?))
                    .title("Atsumi synthetic browser capture verification")
                    .inner_size(960.0, 600.0).visible(false).use_https_scheme(true)
                    .data_directory(profile)
                    .additional_browser_args("--autoplay-policy=no-user-gesture-required --disable-background-networking --disable-component-update --disable-sync --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
                    .initialization_script(bridge)
                    .on_navigation(|url| {
                        let allowed = url.as_str() == format!("{ORIGIN}/live/{CHANNEL}") ||
                            url.as_str() == format!("{SCHEME}://localhost/live/{CHANNEL}");
                        println!("{}", json!({"kind":"navigation","origin":url.origin().ascii_serialization(),
                            "scheme":url.scheme(),"path":url.path(),"accepted":allowed}));
                        allowed
                    })
                    .on_page_load(|window, payload| {
                        let started = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
                        println!("{}", json!({"kind":"page_load","started":started,
                            "origin":payload.url().origin().ascii_serialization(),"path":payload.url().path()}));
                        if !started { let _ = window.eval(DIAGNOSTIC); }
                    })
                    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                    .build()?;
                attach(&window, Probe { store, data_dir, output_root,
                    request_id: uuid::Uuid::new_v4().to_string(), fixture_ready: false,
                    captured_stream_verified: false,
                    start_sent: false, stop_sent: false, began: None, recording: None,
                    finished: None, chunk_count: 0, next_chunk: BTreeMap::new(),
                    arrival_groups: Vec::new(), success: false })?;
                // Independent hard watchdog also handles a wedged disk/renderer.
                let watchdog = app.handle().clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(55));
                    eprintln!("PROBE_HARD_DEADLINE");
                    exit(&watchdog, 2);
                });
                Ok(())
            })
            .run(context)?;
        Ok(EXIT_CODE.load(Ordering::Acquire))
    }
}
