//! Opt-in real CHZZK receiver -> localhost trial. No Google API or existing profile.
//! The parent browser module wires this only for Windows tests. Root owns execution.
use super::*;
use crate::interface::AppQuitRequest;
use crate::streaming::{relay_status::Inspection, replay_assets};
use sha2::{Digest, Sha256};
use std::{fs, io::Read};
use uuid::Uuid;

// Reuse the already bounded RAM-only localhost receiver without editing it.
#[path = "../soop/original_relay_hls_server.rs"]
mod server;
#[path = "../../examples/support/relay_pipe_process.rs"]
mod process;

type Checked<T> = Result<T, String>;
fn held<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
fn check(ok: bool, code: &'static str) -> Checked<()> {
    if ok { Ok(()) } else { Err(code.into()) }
}
fn remaining(end: Instant) -> Checked<Duration> {
    end.checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| "qa_activity_deadline".into())
}

// Only a fixed video selector and numerical/boolean public DOM observations.
// No source URL, page text, account value, chat message or private JS state is returned.
const OBSERVE: &str = r#"(()=>{
 const code=(v,allowed)=>allowed.includes(v)?v:null;
 let d=null;try{d=window.__atsumiEncodedCapture?.getDiagnostics?.()??null;}catch{}
 const facts={documentReady:code(document.readyState,['loading','interactive','complete']),
 documentHidden:document.hidden,bridgePost:typeof window.chrome?.webview?.postMessage==='function',
 captureInstalled:window.__atsumiBrowserCaptureInstalled===true,encodedObject:!!window.__atsumiEncodedCapture,
 encodedGetter:typeof window.__atsumiEncodedCapture?.getDiagnostics==='function',
 mseInstalled:d?.installed===true,mseReason:code(d?.reason,['ready','buffering','waiting_video','waiting_tracks','waiting_init','mse_unavailable','observer_changed','encrypted','source_object_unsupported','source_not_observed','timeline_unsupported','timeline_changed','tracks_unsupported','codec_unsupported','source_blocked','buffer_blocked','native_init_unsupported','init_unsupported','init_changed','container_unsupported','append_unsupported','observer_failed','source_buffer_error','codec_changed','track_changed']),
 mseSources:Number.isInteger(d?.sourceCount)?Math.min(8,d.sourceCount):null,
 mseAppends:Number.isSafeInteger(d?.appendCount)?d.appendCount:null,
 mseAppendBytes:Number.isSafeInteger(d?.appendBytes)?d.appendBytes:null,
 alertPresent:!!document.querySelector('[role="alert"]'),dialogPresent:!!document.querySelector('[role="dialog"],[role="alertdialog"]'),
 playerErrorPresent:!!document.querySelector('.pzp-pc-ui-error,.pzp-ui-error,[class*="player_error"],[class*="playerError"]')};
 const vs=Array.from(document.querySelectorAll('video')).filter(v=>v.isConnected);
 vs.sort((a,b)=>{const x=a.getBoundingClientRect(),y=b.getBoundingClientRect();return y.width*y.height-x.width*x.height;});
 const v=vs[0]; if(!v)return {...facts,present:false,videoElements:0};
 if(window.__atsumiChzzkRelayQaVideo!==v){
   const old=window.__atsumiChzzkRelayQaVideo,fn=window.__atsumiChzzkRelayQaQuiet;
   if(old&&fn)old.removeEventListener('volumechange',fn);
   const quiet=()=>{if(v.volume!==0)v.volume=0;if(!v.muted)v.muted=true;};
   window.__atsumiChzzkRelayQaVideo=v;window.__atsumiChzzkRelayQaQuiet=quiet;
   v.addEventListener('volumechange',quiet);quiet();
 }
 window.__atsumiChzzkRelayQaQuiet();
 const q=v.getVideoPlaybackQuality?.(),r=v.getBoundingClientRect();
 return {...facts,present:true,videoElements:vs.length,time:v.currentTime,ready:v.readyState,
 networkState:v.networkState,mediaError:code(v.error?.code,[1,2,3,4]),
 paused:v.paused,ended:v.ended,rate:v.playbackRate,frames:q?.totalVideoFrames??null,
 dropped:q?.droppedVideoFrames??null,bufferEnd:v.buffered.length?v.buffered.end(v.buffered.length-1):null,
 muted:v.muted,volume:v.volume,width:r.width,height:r.height};
})()"#;

fn url_class(app: &AppHandle, channel: &str) -> &'static str {
    let Some(view)=app.get_webview(WINDOW_LABEL) else { return "receiver_absent"; };
    let Ok(url)=view.url() else { return "url_unavailable"; };
    if url.as_str()=="about:blank" { "startup_blank" }
    else if url.scheme()=="https" && url.host_str()==Some("chzzk.naver.com")
        && url.path()==format!("/live/{channel}") { "selected_live" }
    else if url.host_str()==Some("chzzk.naver.com") { "other_chzzk" }
    else if url.host_str()==Some("nid.naver.com") { "login_origin" }
    else { "other_origin" }
}
fn native_code(code: &str) -> &str {
    if !code.is_empty() && code.len()<=80
        && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b==b'_') { code }
    else { "OTHER" }
}
fn observe(app: &AppHandle, channel: &str, end: Instant) -> Checked<Value> {
    let timeout = remaining(end)?.min(Duration::from_secs(3));
    let view = app.get_webview(WINDOW_LABEL).ok_or("qa_receiver_absent")?;
    let location=url_class(app,channel);
    check(matches!(location,"startup_blank"|"selected_live"), "qa_source_navigation_changed")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(OBSERVE, move |value| { let _ = tx.send(value); })
        .map_err(|_| "qa_observe_dispatch")?;
    let bytes = rx.recv_timeout(timeout).map_err(|_| "qa_observe_timeout")?;
    check(bytes.len() <= 4096, "qa_observe_bound")?;
    let mut value: Value = serde_json::from_str(&bytes).map_err(|_| "qa_observe_shape")?;
    check(value.is_object(),"qa_observe_shape")?;
    value["urlClass"]=json!(location);
    if value["present"] == true {
        check(value["muted"] == true && value["volume"] == 0, "qa_mute_observation")?;
    }
    Ok(value)
}
fn progressed(before: &Value, after: &Value) -> bool {
    // The official live player may briefly catch up at 1.03x. Observe its
    // actual clock/frame progress without setting or assuming its playback rate.
    [before, after].iter().all(|v| v["rate"].as_f64().is_some_and(|r| r.is_finite() && r > 0.))
        && before["paused"] == false && after["paused"] == false
        && before["muted"] == true && after["muted"] == true
        && before["volume"] == 0 && after["volume"] == 0
        && after["time"].as_f64().zip(before["time"].as_f64())
            .is_some_and(|(a,b)| a.is_finite() && b.is_finite() && a > b + 1.)
        && after["frames"].as_u64().zip(before["frames"].as_u64())
            .is_some_and(|(a,b)| a > b)
}
fn observation_window(app: &AppHandle, channel: &str, end: Instant) -> Checked<Value> {
    let mut samples = Vec::with_capacity(5);
    for index in 0..5 {
        if index != 0 {
            check(remaining(end)? >= Duration::from_secs(1), "qa_activity_deadline")?;
            thread::sleep(Duration::from_secs(1));
        }
        samples.push(observe(app, channel, end)?);
    }
    let progressed = progressed(&samples[0], samples.last().unwrap());
    let dropped_delta = samples.last().unwrap()["dropped"].as_u64()
        .zip(samples[0]["dropped"].as_u64()).and_then(|(a,b)| a.checked_sub(b));
    Ok(json!({"samples":samples,"progressed":progressed,"droppedFramesDelta":dropped_delta}))
}

fn regular(path: &Path) -> Checked<fs::Metadata> {
    use std::os::windows::fs::MetadataExt;
    let meta = fs::symlink_metadata(path).map_err(|_| "qa_metadata")?;
    check(!meta.file_type().is_symlink() && meta.file_attributes() & 0x400 == 0,
        "qa_reparse_path")?;
    Ok(meta)
}
fn read_bounded(path: &Path, bound: u64) -> Checked<Vec<u8>> {
    let meta = regular(path)?;
    check(meta.is_file() && meta.len() <= bound, "qa_file_bound")?;
    let mut data = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path).map_err(|_| "qa_read")?.take(bound + 1)
        .read_to_end(&mut data).map_err(|_| "qa_read")?;
    check(data.len() as u64 == meta.len(), "qa_file_changed")?;
    Ok(data)
}
fn json_file(path: &Path) -> Checked<Value> {
    serde_json::from_slice(&read_bounded(path, 262144)?).map_err(|_| "qa_json_shape".into())
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

// Exclude the official browser's HTTP cache: the claim is no Atsumi recording
// output, not that a WebView's independently managed network cache is diskless.
fn app_files(data: &Path) -> Checked<Value> {
    let mut todo = vec![data.to_path_buf()];
    let (mut files, mut media, mut jobs) = (0usize, 0usize, 0usize);
    while let Some(dir) = todo.pop() {
        for entry in fs::read_dir(&dir).map_err(|_| "qa_scan")? {
            let entry = entry.map_err(|_| "qa_scan")?;
            let path = entry.path();
            if dir == data && entry.file_name().to_string_lossy().contains("browser-profile") {
                continue;
            }
            let meta = regular(&path)?;
            files += 1;
            check(files <= 4096, "qa_scan_bound")?;
            if meta.is_dir() { todo.push(path); continue; }
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "mp4"|"m4s"|"ts"|"webm"|"mkv"|"flv") { media += 1; }
            if path.parent() == Some(data.join("platform-media").join("jobs").as_path())
                && ext == "json" { jobs += 1; }
        }
    }
    Ok(json!({"scannedEntries":files,"recordedVideoFiles":media,"platformMediaJobs":jobs,
        "officialBrowserCacheExcluded":true}))
}

fn timing_proof(root: &Path, owner: &Value, inspection: &Inspection) -> Checked<Value> {
    let bytes = read_bounded(&root.join("relay-timing.jsonl"),64*1024*1024)?;
    check(bytes.last()==Some(&b'\n'),"qa_timing_tail")?;
    let mut observations = 0u64;
    let mut usable = 0u64;
    let mut segment_hashes = Vec::new();
    let mut stream: Option<String> = None;
    let mut origin: Option<(u64,u64)> = None;
    let mut ending: Option<Value> = None;
    let mut records = 0u64;
    for line in bytes.split(|b| *b==b'\n').filter(|s| !s.is_empty()) {
        check(line.len()<=4096,"qa_timing_line_bound")?;
        let v: Value = serde_json::from_slice(line).map_err(|_| "qa_timing_shape")?;
        let before = v["nativeBeforeNs"].as_u64().ok_or("qa_timing_bracket")?;
        let after = v["nativeAfterNs"].as_u64().ok_or("qa_timing_bracket")?;
        check(before<=after,"qa_timing_bracket")?;
        match v["kind"].as_str() {
            Some("source_observation") => {
                observations += 1;
                if !v["sample"].is_null() {
                    let sample = &v["sample"];
                    check(sample["sourceId"]==owner["sourceId"] && sample["clock"]=="mse_presentation_v1",
                        "qa_timing_source_binding")?;
                    let current = sample["currentTime"].as_f64().ok_or("qa_timing_clock")?;
                    let offset = sample["timestampOffset"].as_f64().ok_or("qa_timing_clock")?;
                    let source = sample["sourceTimeSeconds"].as_f64().ok_or("qa_timing_clock")?;
                    check(current.is_finite() && offset.is_finite() && source.is_finite()
                        && (current-offset-source).abs()<=1e-6,"qa_timing_clock")?;
                    usable += 1;
                }
            }
            Some("segment_ack") => {
                check(ending.is_none() && v["ordinal"].as_u64()==Some(segment_hashes.len() as u64),"qa_timing_ordinal")?;
                let id = v["streamId"].as_str().ok_or("qa_timing_stream")?;
                Uuid::parse_str(id).map_err(|_| "qa_timing_stream")?;
                if let Some(old)=&stream { check(old==id,"qa_timing_stream_changed")?; }
                else { stream=Some(id.into()); }
                let source = &v["source"];
                let ticks=source["originTicks"].as_u64().ok_or("qa_timing_origin")?;
                let scale=source["originTimescale"].as_u64().filter(|n| *n>0).ok_or("qa_timing_origin")?;
                if let Some(old)=origin { check(old==(ticks,scale),"qa_timing_origin_changed")?; }
                else { origin=Some((ticks,scale)); }
                for track in ["video","audio"] {
                    check(source[track]["timescale"].as_u64().unwrap_or(0)>0
                        && source[track]["samples"].as_u64().unwrap_or(0)>0,"qa_timing_track")?;
                }
                let sha=v["sha256"].as_str().ok_or("qa_timing_sha")?;
                check(sha.len()==64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),"qa_timing_sha")?;
                segment_hashes.push(sha.to_owned());
            }
            Some("end_ack") => {
                check(ending.is_none() && stream.as_deref()==v["streamId"].as_str()
                    && v["segments"].as_u64()==Some(segment_hashes.len() as u64),"qa_timing_end")?;
                let (ticks,scale)=origin.ok_or("qa_timing_end_origin")?;
                check(v["origin"]["ticks"]==ticks && v["origin"]["timescale"]==scale,
                    "qa_timing_end_origin")?;
                for key in ["videoSamples","audioSamples","bytes"] {
                    check(v["preroll"][key].as_u64().is_some(),"qa_timing_preroll")?;
                }
                ending=Some(v.clone());
            }
            _ => return Err("qa_timing_unknown_kind".into()),
        }
        records += 1;
    }
    let sidecar=inspection.sidecar.as_ref().ok_or("qa_timing_summary")?;
    let delivery=inspection.delivery.as_ref().ok_or("qa_timing_delivery")?;
    check(records==sidecar.timing_records && !sidecar.timing_incomplete && usable>0
        && segment_hashes.len() as u64==delivery.segment_acks && ending.is_some()
        && delivery.end_list_acked,"qa_timing_coverage")?;
    Ok(json!({"records":records,"sourceObservations":observations,"usableSourceObservations":usable,
        "segmentAcks":segment_hashes.len(),"segmentSha256":segment_hashes,
        "sha256":digest(&bytes),"bytes":bytes.len(),"end":ending,
        "nativeBracketVerified":true,"sourceIdentityVerified":true,"remoteArchiveMapping":"unknown"}))
}

fn sidecar_proof(data: &Path, source: &ChzzkRelaySource, inspection: &Inspection) -> Checked<Value> {
    let root = inspection.sidecar_directory.as_ref().ok_or("qa_sidecar_absent")?;
    let expected = data.join("streaming").join("relay-sidecars").join(&inspection.run_id);
    check(fs::canonicalize(&expected).map_err(|_| "qa_sidecar_path")? == *root,
        "qa_sidecar_path")?;
    for ancestor in root.ancestors() { regular(ancestor)?; }
    let owner = json_file(&root.join("relay-owner.json"))?;
    let proof = json_file(&root.join("relay-summary.json"))?;
    check(owner["provider"] == "chzzk" && owner["channelId"] == source.channel()
        && owner["runId"] == inspection.run_id && owner["documentEpoch"] == source.epoch()
        && owner["accountRevision"] == source.account_revision() && owner["sessionId"] == source.session_id()
        && proof["binding"] == owner && proof["remoteArchiveMapping"] == "unknown",
        "qa_sidecar_binding")?;
    let sidecar = inspection.sidecar.as_ref().ok_or("qa_sidecar_summary")?;
    check(sidecar.joined && sidecar.text_state == "ended" && !sidecar.failure
        && !sidecar.interrupted && !sidecar.assets_incomplete,
        "qa_sidecar_incomplete")?;
    let mut streams = serde_json::Map::new();
    for (name, expected_rows) in [("chat.jsonl",sidecar.event_rows),("chat.events.jsonl",sidecar.raw_rows)] {
        let bytes = read_bounded(&root.join(name), 64*1024*1024)?;
        check(bytes.is_empty() || bytes.last() == Some(&b'\n'), "qa_chat_tail")?;
        let mut count = 0u64;
        for line in bytes.split(|b| *b == b'\n').filter(|s| !s.is_empty()) {
            check(line.len() <= 64*1024, "qa_chat_line_bound")?;
            let v: Value = serde_json::from_slice(line).map_err(|_| "qa_chat_shape")?;
            if name == "chat.events.jsonl" {
                let obj = v.as_object().ok_or("qa_public_event_shape")?;
                check(obj.keys().all(|k| matches!(k.as_str(), "sender"|"text"|"serverTime"|"receivedAt"|"replayClock"|"senderKey"|"rich")), "qa_public_event_projection")?;
            } else {
                let _: crate::streaming::model::ChatMessage = serde_json::from_value(v)
                    .map_err(|_| "qa_normalized_chat_shape")?;
            }
            count += 1;
        }
        check(count == expected_rows, "qa_chat_count")?;
        streams.insert(name.into(),json!({"rows":count,"bytes":bytes.len(),"sha256":digest(&bytes)}));
    }
    // This is a small real chat trial; a quiet source does not establish chat preservation.
    check(sidecar.raw_rows > 0 && sidecar.raw_rows == sidecar.event_rows, "qa_chat_unobserved")?;
    let page = crate::streaming::chat_store::read_chat_page(root, None)
        .map_err(|_| "qa_sidecar_history_reopen")?;
    check(!page.items.is_empty(), "qa_sidecar_history_empty")?;
    let mut asset_files = Vec::new();
    let asset_dir = root.join("replay-assets");
    if asset_dir.exists() {
        regular(&asset_dir)?;
        for entry in fs::read_dir(&asset_dir).map_err(|_| "qa_assets_scan")? {
            let path = entry.map_err(|_| "qa_assets_scan")?.path();
            check(asset_files.len() < 4096, "qa_assets_bound")?;
            let id = path.file_stem().and_then(|s| s.to_str()).ok_or("qa_asset_name")?;
            let _ = read_bounded(&path, 1_500_000)?;
            let image = replay_assets::read_recording(root,id).map_err(|_| "qa_asset_read_decode")?;
            asset_files.push(json!({"id":id,"bytes":image.bytes.len(),"sha256":digest(&image.bytes),"mime":image.mime}));
        }
    }
    asset_files.sort_by(|a,b| a["id"].as_str().cmp(&b["id"].as_str()));
    check(proof["assetExpected"].as_u64() == Some(asset_files.len() as u64), "qa_asset_count")?;
    check(proof["summary"]["rawRows"] == sidecar.raw_rows
        && proof["summary"]["eventRows"] == sidecar.event_rows
        && proof["summary"]["textState"] == "ended" && proof["summary"]["joined"] == false,
        "qa_durable_writer_summary")?;
    let timing=timing_proof(root,&owner,inspection)?;
    Ok(json!({"ownerBindingVerified":true,"streams":streams,"timing":timing,"historyReopened":true,
        "historyPageRows":page.items.len(),"runtimeSidecarJoined":sidecar.joined,
        "durableSummaryJoined":false,"assetsExpected":proof["assetExpected"],
        "assetsLocallyValidated":asset_files.len(),"assets":asset_files,
        "imageEventObserved":proof["assetExpected"].as_u64().unwrap_or(0)>0,
        "renderedImagesVerified":false,"remoteArchiveMapping":"unknown"}))
}

fn receiver_proof(receiver: &server::Server, inspection: &Inspection) -> Checked<Value> {
    let snapshot = receiver.snapshot();
    check(snapshot["error"].is_null(), "qa_local_receiver_error")?;
    let objects = snapshot["objects"].as_array().ok_or("qa_local_objects")?;
    let segments = objects.iter().filter(|v| v["playlist"] == false).count();
    let playlists = objects.iter().filter(|v| v["playlist"] == true).count();
    let ends = objects.iter().filter(|v| v["endList"] == true).count();
    let delivery = inspection.delivery.as_ref().ok_or("qa_delivery_absent")?;
    check(segments >= 2 && delivery.segment_acks == segments as u64
        && delivery.playlist_acks == playlists as u64 && delivery.requests == objects.len() as u64
        && delivery.end_list_acked && ends == 1 && delivery.fault.is_none(), "qa_local_ack_invariant")?;
    let mut report = snapshot;
    report["nativeAcknowledgementsMatched"] = json!(true);
    report["fullCodecDecodeVerified"] = json!(false);
    Ok(report)
}

fn decode_segments(receiver: &server::Server, root: &Path, end: Instant) -> Checked<Value> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let exe = workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin/ffmpeg.exe");
    for part in exe.ancestors() { regular(part)?; }
    check(digest(&read_bounded(&exe, 32 * 1024 * 1024)?)
        == "c2a51bdad17d321c9c11768a4bc6137ac6c666d62ba82fe4b4050fbd1db9d2a0", "qa_decoder_revision")?;
    let exe = fs::canonicalize(exe).map_err(|_| "qa_decoder_path")?;
    let mut results = Vec::new();
    for bytes in receiver.segments() {
        check(remaining(end)? >= Duration::from_secs(54), "qa_decoder_deadline")?;
        let sha = digest(&bytes);
        let output = process::run(&exe, root, &[
            "-hide_banner", "-nostdin", "-loglevel", "error", "-xerror", "-err_detect", "explode",
            "-protocol_whitelist", "pipe", "-f", "mpegts", "-i", "pipe:0",
            "-map", "0:v:0", "-map", "0:a:0", "-c:v", "rawvideo", "-c:a", "pcm_s16le",
            "-copyts", "-fps_mode:v", "passthrough", "-enc_time_base:v", "demux",
            "-threads", "2", "-f", "framehash", "-hash", "sha256", "pipe:1"
        ], Some(bytes), 2 * 1024 * 1024, false)?;
        let text = std::str::from_utf8(&output.bytes).map_err(|_| "qa_decode_output")?;
        let video = text.lines().filter(|s| s.trim_start().starts_with("0,")).count();
        let audio = text.lines().filter(|s| s.trim_start().starts_with("1,")).count();
        check(video > 0 && audio > 0, "qa_both_streams_decoded")?;
        let mut facts = output.facts;
        check(facts["success"] == true && facts["stderrRetainedBytes"] == 0, "qa_live_ts_decode")?;
        facts["videoDecodedFrames"] = json!(video);
        facts["audioDecodedFrames"] = json!(audio);
        facts["segmentSha256"] = json!(sha);
        results.push(facts);
    }
    check(results.len() == 2, "qa_two_received_segments")?;
    Ok(json!({"sampledSegments":results,"allSegmentsDecoded":false,"mediaFilesWritten":0}))
}

fn exercise(app: &AppHandle, channel: &str, receiver: &server::Server, root: &Path,
    selected: &Mutex<Option<ChzzkSelectedRelayHandle>>, report: &mut Value) -> Checked<()> {
    let started = Instant::now();
    let end = started + Duration::from_secs(240);
    report["startupStages"]=json!({"beforeBackendMs":started.elapsed().as_millis()});
    let startup = app.state::<Arc<crate::startup::Startup>>();
    crate::initialize_backend(app,&startup).map_err(|_| "qa_backend_init")?;
    report["startupStages"]["afterBackendMs"]=json!(started.elapsed().as_millis());
    let state = app.state::<AppState>();
    let readiness = Instant::now()+Duration::from_secs(65);
    let browser = loop {
        if let Ok(browser) = state.deferred_browser().get() { break browser; }
        check(Instant::now()<readiness, "qa_browser_init_timeout")?;
        thread::sleep(Duration::from_millis(100));
    };
    report["startupStages"]["browserAvailableMs"]=json!(started.elapsed().as_millis());
    let data = browser.inner.data_dir.clone();
    report["startupStages"]["beforeOpenMs"]=json!(started.elapsed().as_millis());
    browser.open(app,channel).map_err(|e| {
        report["startupStages"]["openErrorCode"]=json!(native_code(&e.code));
        "qa_source_open"
    })?;
    report["startupStages"]["afterOpenMs"]=json!(started.elapsed().as_millis());
    browser.set_viewport(app,BrowserViewport { x:0.,y:0.,width:960.,height:720.,
        visible:true,request_sequence:Some(1),..Default::default() })
        .map_err(|e| {
            report["startupStages"]["viewportErrorCode"]=json!(native_code(&e.code));
            "qa_source_viewport"
        })?;
    report["startupStages"]["afterViewportMs"]=json!(started.elapsed().as_millis());
    report["sourceReadiness"]=json!({"attempts":0,"video":null,"previousVideo":null});
    let mut attempts=0u64;
    loop {
        check(Instant::now()<readiness,"qa_source_not_ready")?;
        let s = browser.snapshot().map_err(|_| "qa_source_snapshot")?;
        let reason = s.capture_diagnostics.as_ref().and_then(|d| d["reason"].as_str());
        attempts+=1;
        let status=match s.status.as_str() {
            "closed"|"loading"|"ready"|"unavailable"|"error"|"starting"|"recording"|"saving"|"waiting_source"=>s.status.as_str(),
            _=>"other"
        };
        report["sourceReadiness"]["native"]=json!({"nativeReady":s.ready,"captureReason":reason,
            "status":status,"errorPresent":s.error.is_some(),"windowOpen":s.window_open,
            "videoWidth":s.video_width,"videoHeight":s.video_height,"videoPaused":s.video_paused,
            "viewportEpoch":s.viewport_epoch,"recordings":s.recordings.len(),"recordingActive":s.recording_id.is_some()});
        report["sourceReadiness"]["attempts"]=json!(attempts);
        report["sourceReadiness"]["elapsedMs"]=json!(started.elapsed().as_millis());
        report["sourceReadiness"]["urlClass"]=json!(url_class(app,channel));
        if reason.is_some_and(|r| matches!(r,"encrypted"|"codec_unsupported"|"container_unsupported"|"tracks_unsupported"|"native_init_unsupported"|"source_blocked")) {
            return Err("qa_source_unsupported".into());
        }
        let f = observe(app,channel,readiness.min(end)).map_err(|cause| {
            report["sourceReadiness"]["lastObserveError"]=json!(cause);
            if Instant::now()>=readiness { "qa_source_not_ready".into() } else { cause }
        })?;
        report["sourceReadiness"]["previousVideo"]=report["sourceReadiness"]["video"].clone();
        report["sourceReadiness"]["video"] = f.clone();
        if s.ready && reason == Some("ready") && f["ready"].as_u64().unwrap_or(0)>=2
            && f["paused"]==false && f["rate"].as_f64().is_some_and(|r| r.is_finite() && r > 0.) { break; }
        check(Instant::now()<readiness,"qa_source_not_ready")?;
        thread::sleep(Duration::from_millis(250));
    }
    report["baseline"] = observation_window(app,channel,end)?;
    check(report["baseline"]["progressed"]==true,"qa_baseline_stalled")?;
    check(browser.snapshot().map_err(|_| "qa_snapshot")?.recordings.is_empty(), "qa_recording_baseline")?;
    report["baselineFiles"] = app_files(&data)?;
    check(report["baselineFiles"]["recordedVideoFiles"]==0 && report["baselineFiles"]["platformMediaJobs"]==0,"qa_media_baseline")?;
    let before_work = state.active_work_snapshot().map_err(|_| "qa_work_before")?;
    let source = browser.select_relay_source_by_channel(channel).map_err(|_| "qa_native_source_select")?;
    check(source.current(),"qa_source_stale")?;
    let run = Uuid::new_v4();
    let handle = source.start(app,run,receiver.target()).map_err(|_| "qa_selected_start")?;
    *held(selected) = Some(handle);
    let after_work = state.active_work_snapshot().map_err(|_| "qa_work_active")?;
    report["workFingerprintChanged"] = json!(before_work.work_set_fingerprint != after_work.work_set_fingerprint);
    report["receiverCount"] = json!(after_work.live_receivers.as_ref().map(|r| r.active_count));
    report["sourceSelection"] = json!({"nativeBound":true,"currentAtStart":true,"runId":run,
        "documentEpoch":source.epoch(),"accountRevision":source.account_revision()});
    check(report["workFingerprintChanged"]==true && report["receiverCount"]==1,"qa_receiver_workset")?;
    let transfer = Instant::now();
    let transfer_end = (transfer + Duration::from_secs(60)).min(end);
    let mut samples = Vec::new();
    loop {
        remaining(transfer_end)?;
        let s = held(selected).as_ref().unwrap().inspection();
        report["relayDuring"] = serde_json::to_value(&s).map_err(|_| "qa_inspection")?;
        if s.fault.is_some() || s.manager_joined { return Err("qa_relay_failed_during_transfer".into()); }
        let sample = observe(app,channel,transfer_end)?;
        check(samples.len()<61,"qa_observation_count")?;
        samples.push(sample);
        report["duringSamples"] = json!(samples);
        if transfer.elapsed()>=Duration::from_secs(20)
            && s.delivery.as_ref().is_some_and(|d| d.segment_acks>=2) { break; }
        thread::sleep(remaining(transfer_end)?.min(Duration::from_secs(1)));
    }
    report["transferSeconds"] = json!(transfer.elapsed().as_secs_f64());
    check(progressed(&samples[0],samples.last().unwrap()),"qa_source_stalled_during_relay")?;
    check(held(selected).as_ref().unwrap().source_current(),"qa_source_changed_during_relay")?;
    held(selected).as_ref().unwrap().request_stop(false);
    report["normalStopRequested"] = json!(true);
    // Continue real reaping only within the activity budget. Beyond it normal
    // AppState drain owns the join; no forced disposal is used to manufacture pass.
    let final_inspection = loop {
        let s = held(selected).as_ref().unwrap().inspection();
        report["relay"] = serde_json::to_value(&s).map_err(|_| "qa_inspection")?;
        if s.manager_joined { break s; }
        remaining(end)?;
        thread::sleep(Duration::from_millis(100));
    };
    check(final_inspection.worker_joined && final_inspection.input_ended
        && final_inspection.codec_admitted && final_inspection.fault.is_none()
        && !final_inspection.interrupted && final_inspection.video_samples>0
        && final_inspection.audio_samples>0, "qa_relay_final_invariant")?;
    report["afterStop"] = observation_window(app,channel,end)?;
    check(report["afterStop"]["progressed"]==true,"qa_source_stalled_after_stop")?;
    report["receiver"] = receiver_proof(receiver,&final_inspection)?;
    remaining(end)?;
    report["sidecar"] = sidecar_proof(&data,&source,&final_inspection)?;
    // Only this live native handle supplies ownership. Saved JSON is not used
    // to recreate a capture capability or to assert a remote archive exists.
    let native_source = crate::cloud_recording::live_relay::NativeRelaySource::Chzzk(source.clone());
    let history_snapshot = crate::cloud_recording::live_relay::LiveRelaySnapshot {
        selection_id: Uuid::new_v4().to_string(), provider: "chzzk".into(),
        channel_id: source.channel().into(), broadcast_key: source.broadcast().map(str::to_owned),
        state: crate::cloud_recording::live_relay::SelectionPhase::Joined,
        run_id: Some(run.to_string()), destination: None, stop_requested: true,
        interrupted: false, fault: None, first_relay_fault: None, manager_joined: true,
        archive_ready: false, relay: Some(final_inspection.clone()),
    };
    report["nativeHistoryReopen"] = serde_json::to_value(
        crate::cloud_recording::verify_actual_joined_history(
            &root.join("joined-history-vault"), native_source.facts(), &history_snapshot,
        ).map_err(|_| "qa_native_history_seal_reopen")?
    ).map_err(|_| "qa_native_history_report")?;
    check(report["nativeHistoryReopen"]["reopened"] == true
        && report["nativeHistoryReopen"]["archiveReady"] == false
        && report["nativeHistoryReopen"]["remoteAnchor"] == "unknown"
        && report["nativeHistoryReopen"]["localMediaJobs"] == 0
        && report["nativeHistoryReopen"]["events"].as_u64()
            == final_inspection.sidecar.as_ref().map(|s| s.event_rows)
        && report["nativeHistoryReopen"]["assetsRead"] == report["sidecar"]["assetsExpected"],
        "qa_native_history_qualification")?;
    let received: Vec<_> = report["receiver"]["objects"].as_array().ok_or("qa_local_objects")?
        .iter().filter(|v| v["playlist"]==false).map(|v| v["sha256"].clone()).collect();
    check(report["sidecar"]["timing"]["segmentSha256"]==json!(received),"qa_timing_payload_ack_mismatch")?;
    report["actualTsDecode"] = decode_segments(receiver,root,end)?;
    report["finalFiles"] = app_files(&data)?;
    check(report["finalFiles"]["recordedVideoFiles"]==0 && report["finalFiles"]["platformMediaJobs"]==0
        && browser.snapshot().map_err(|_| "qa_snapshot")?.recordings.is_empty(),"qa_video_or_recording_created")?;
    report["activitySeconds"] = json!(started.elapsed().as_secs_f64());
    report["relayAlreadyJoinedBeforeAppQuit"] = json!(true);
    remaining(end)?;
    Ok(())
}

#[test]
#[ignore = "Explicit public CHZZK source -> localhost, isolated actual AppState; root supplies verified channel"]
fn actual_chzzk_selected_receiver_local_hls_sidecar_and_graceful_quit() {
    assert_eq!(std::env::var("ATSUMI_CHZZK_RELAY_QA").as_deref(),Ok("public-source-loopback-only"),"QA_OPT_IN");
    let channel = std::env::var("ATSUMI_CHZZK_RELAY_QA_CHANNEL").expect("QA_CHANNEL_REQUIRED");
    assert!(channel.len()==32 && channel.bytes().all(|b| b.is_ascii_digit()||(b'a'..=b'f').contains(&b)),"QA_CHANNEL");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let id = Uuid::new_v4().simple().to_string();
    let root = workspace.join(".runtime/chzzk-relay-native-qa").join(&id);
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::create_dir(&root).unwrap();
    let identity = format!("local.atsumi.chzzkrelayqa.{id}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(),"QA_PROFILE_EXISTS");
    let result = Arc::new(Mutex::new(json!({"passed":false,"isolated":true,
        "googleApiCalls":0,"existingProfileAccess":false,"originalMediaAccess":false,
        "desktopInput":false,"sourceOpenAttempts":1,"sourceChannelId":channel,
        "scope":"one public receiver -> bounded localhost HLS + light sidecar; not YouTube or full archive sync",
        "limits":{"activitySeconds":240,"readinessSeconds":65,"transferSeconds":60},
        "limitations":["Short sampled playback progress is not universal unchanged latency", "Audio is observed muted at samples, not continuous audible-output proof", "Only two RAM segments are decoded; remaining segments are not decoded", "No historical official event appearance or remote mapping proof", "Browser HTTP cache is outside app media-output claim"]})));
    let selected = Arc::new(Mutex::new(None::<ChzzkSelectedRelayHandle>));
    let qa_thread = Arc::new(Mutex::new(None::<thread::JoinHandle<()>>));
    let thread_slot = qa_thread.clone();
    let receiver = Arc::new(server::Server::start().expect("QA_LOCAL_RECEIVER"));
    let output = result.clone();
    let worker_selected = selected.clone();
    let worker_receiver = receiver.clone();
    let worker_root = root.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = identity;
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().any_thread()
        .manage(crate::soop::original::SoopOriginalHost::default())
        .manage(crate::youtube::original::YouTubeOriginalHost::default())
        .manage(Arc::new(crate::startup::Startup::default()))
        .manage(Arc::new(crate::ui_download_events::DownloadEvents::default()))
        .setup(move |app| {
            if app.path().app_data_dir()? != profile { return Err("QA_PROFILE_PATH".into()); }
            tauri::window::WindowBuilder::new(app,"main")
                .title("Atsumi CHZZK relay verification").inner_size(960.,720.)
                .position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle = app.handle().clone();
            let worker = thread::spawn(move || {
                let mut report = held(&output).clone();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                    exercise(&handle,&channel,&worker_receiver,&worker_root,&worker_selected,&mut report)))
                    .unwrap_or_else(|_| Err("qa_worker_panic".into()));
                report["passed"] = json!(outcome.is_ok());
                report["error"] = json!(outcome.err());
                if let Some(selected) = held(&worker_selected).as_ref() {
                    // Normal cleanup even on source/codec/assertion failure; never start a retry.
                    selected.request_stop(false);
                    report["beforeQuitRelay"] = serde_json::to_value(selected.inspection()).unwrap();
                }
                *held(&output) = report;
                let _ = fs::write(worker_root.join("pending-result.json"),serde_json::to_vec_pretty(&*held(&output)).unwrap());
                if let Some(state) = handle.try_state::<AppState>() {
                    match state.active_work_snapshot().map_err(|_| ()).and_then(|s| state.request_graceful_quit(handle.clone(),AppQuitRequest {
                        expected_work_set_fingerprint:s.work_set_fingerprint,confirm_active_work:true,force_when_status_unknown:false,
                    }).map_err(|_| ())) {
                        Ok(answer) if answer.accepted => { held(&output)["normalQuitAccepted"]=json!(true); return; }
                        _ => { held(&output)["passed"]=json!(false); held(&output)["quitError"]=json!("normal_quit_rejected_or_unknown"); }
                    }
                    let _ = fs::write(worker_root.join("pending-result.json"),serde_json::to_vec_pretty(&*held(&output)).unwrap());
                    // No force exit: retained work remains owned and visible to the operator.
                } else {
                    held(&output)["noBackendInitialized"] = json!(true);
                    handle.exit(2);
                }
            });
            *held(&thread_slot) = Some(worker);
            Ok(())
        }).build(context).expect("QA_BUILD");
    app.run_return(|_,_|{});
    let observer_joined = held(&qa_thread).take().is_some_and(|t| t.join().is_ok());
    let mut report = held(&result).clone();
    report["qaObserverJoined"] = json!(observer_joined);
    if !observer_joined { report["passed"] = json!(false); }
    report["normalAppExitReturned"] = json!(true);
    if let Some(selected) = held(&selected).as_ref() {
        let final_state = selected.inspection();
        let joined = final_state.manager_joined && final_state.worker_joined
            && final_state.sidecar.as_ref().is_some_and(|s| s.joined);
        report["afterQuitRelay"] = serde_json::to_value(final_state).unwrap();
        if !joined { report["passed"]=json!(false); report["joinError"]=json!("qa_actual_join_missing"); }
    } else { report["passed"]=json!(false); }
    report["receiverAfterQuit"] = receiver.snapshot();
    if report["normalQuitAccepted"]!=true { report["passed"]=json!(false); }
    // The last server reference outlives real AppState exit and the delivery manager.
    drop(receiver);
    report["localReceiverJoined"] = json!(true);
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("QA_CHZZK_RELAY_RESULT={}",root.join("result.json").display());
    assert_eq!(report["passed"],true,"Actual CHZZK relay remains unverified; inspect bounded result");
}
