//! One opt-in public CHZZK receiver -> existing owner's unlisted Live archive.
//! Fresh viewer profile; credentials stay in the existing isolated QA vault.
use super::*;
use crate::{
    cloud_recording::{catalog::Catalog, upload, vault, CloudLiveOperation, SourceResolver},
    interface::{AppQuitRequest, AppState},
    streaming::browser::{BrowserViewport, OfficialBrowser, WINDOW_LABEL},
};
use serde_json::{json, Value};
use std::{
    fs, io::{Read, Write}, os::windows::fs::MetadataExt, path::{Path, PathBuf},
    sync::{atomic::AtomicUsize, Condvar, Mutex}, thread, time::Instant,
};
type Checked<T> = Result<T, String>;

fn held<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
fn require(value: bool, code: &'static str) -> Checked<()> {
    if value { Ok(()) } else { Err(code.into()) }
}
fn code(error: CloudError) -> String {
    if error.code.len() <= 80 && error.code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
        error.code
    } else { "QA_CLOUD_ERROR".into() }
}
fn regular(path: &Path) -> Checked<fs::Metadata> {
    for ancestor in path.ancestors() {
        let m = fs::symlink_metadata(ancestor).map_err(|_| "QA_PATH")?;
        require(!m.file_type().is_symlink() && m.file_attributes() & 0x400 == 0, "QA_LINK")?;
    }
    fs::metadata(path).map_err(|_| "QA_PATH".into())
}
fn baseline(catalog: &Catalog) -> Checked<Value> {
    let mut value = serde_json::to_value(catalog).map_err(|_| "QA_BASELINE")?;
    let object = value.as_object_mut().ok_or("QA_BASELINE")?;
    // Only this existing Live grant may be refreshed, and one receipt added.
    object.remove("live_grant");
    object.remove("live_receipts");
    Ok(value)
}

struct RefreshOnly {
    inner: upload::GoogleTransport,
    calls: AtomicUsize,
}
impl upload::Transport for RefreshOnly {
    fn send(&self, request: upload::Request) -> Result<upload::Response, CloudError> {
        if request.method != reqwest::Method::POST || request.url != "https://oauth2.googleapis.com/token"
            || request.bearer.is_some() || request.body.len() > 32768
            || request.headers != vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())]
        { return Err(CloudError::protocol()); }
        let mut url = reqwest::Url::parse("https://localhost/").map_err(|_| CloudError::protocol())?;
        url.set_query(Some(std::str::from_utf8(&request.body).map_err(|_| CloudError::protocol())?));
        let pairs: Vec<_> = url.query_pairs().collect();
        let keys: std::collections::BTreeSet<_> = pairs.iter().map(|(k, _)| k.as_ref()).collect();
        if pairs.len() != 4 || keys != ["client_id", "client_secret", "grant_type", "refresh_token"].into_iter().collect()
            || !pairs.iter().any(|(k, v)| k == "grant_type" && v == "refresh_token")
            || self.calls.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err()
        { return Err(CloudError::protocol()); }
        upload::Transport::send(&self.inner, request)
    }
}

fn host(vault: vault::Vault, catalog: Catalog, transport: Arc<RefreshOnly>) -> Arc<CloudService> {
    let resolver: SourceResolver = Arc::new(|_, _| Err("QA_RECORDING_SOURCE_FORBIDDEN".into()));
    Arc::new(CloudService {
        inner: Mutex::new(Inner {
            catalog, account_error: None, authorizing: false, auth_workers: 0, auth_generation: 0,
            auth_cancel: None, active: None, worker_running: false, closing: false,
            persistence_error: None, preparation_progress_at: None, automatic_running: false,
            automatic_observations: Default::default(), automatic_error: None, live: Default::default(),
        }),
        wake: Condvar::new(), threads: Mutex::new(Vec::new()), vault, resolver, planner: None,
        open_browser: Arc::new(|_| Err("QA_AUTHORIZATION_FORBIDDEN".into())), transport,
    })
}

// Fixed native receiver DOM facts only; never return page text, URLs or chat.
const OBSERVE: &str = r#"(()=>{
const vs=Array.from(document.querySelectorAll('video')).filter(v=>v.isConnected);
vs.sort((a,b)=>{const x=a.getBoundingClientRect(),y=b.getBoundingClientRect();return y.width*y.height-x.width*x.height;});
const v=vs[0];if(!v)return {present:false};
if(window.__atsumiCloudLiveQaVideo!==v){const quiet=()=>{if(v.volume!==0)v.volume=0;if(!v.muted)v.muted=true};
v.addEventListener('volumechange',quiet);window.__atsumiCloudLiveQaVideo=v;window.__atsumiCloudLiveQaQuiet=quiet;}
window.__atsumiCloudLiveQaQuiet();const q=v.getVideoPlaybackQuality?.();
return {present:true,time:v.currentTime,ready:v.readyState,paused:v.paused,ended:v.ended,rate:v.playbackRate,
frames:q?.totalVideoFrames??null,dropped:q?.droppedVideoFrames??null,muted:v.muted,volume:v.volume,
bufferEnd:v.buffered.length?v.buffered.end(v.buffered.length-1):null};})()"#;

fn observe(app: &AppHandle, channel: &str, end: Instant) -> Checked<Value> {
    let view = app.get_webview(WINDOW_LABEL).ok_or("QA_RECEIVER")?;
    let url = view.url().map_err(|_| "QA_RECEIVER_URL")?;
    require(url.scheme() == "https" && url.host_str() == Some("chzzk.naver.com")
        && url.path() == format!("/live/{channel}"), "QA_SOURCE_NAVIGATION")?;
    let wait = end.saturating_duration_since(Instant::now()).min(Duration::from_secs(3));
    require(!wait.is_zero(), "QA_DEADLINE")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(OBSERVE, move |value| { let _ = tx.send(value); }).map_err(|_| "QA_OBSERVE")?;
    let raw = rx.recv_timeout(wait).map_err(|_| "QA_OBSERVE_TIMEOUT")?;
    require(raw.len() <= 2048, "QA_OBSERVE_BOUND")?;
    let value: Value = serde_json::from_str(&raw).map_err(|_| "QA_OBSERVE_SHAPE")?;
    if value["present"] == true { require(value["muted"] == true && value["volume"] == 0, "QA_MUTE")?; }
    Ok(value)
}
fn progressed(first: &Value, last: &Value) -> bool {
    first["paused"] == false && last["paused"] == false
        && [first,last].iter().all(|v| v["rate"].as_f64().is_some_and(|r| r.is_finite() && r > 0.))
        && first["time"].as_f64().zip(last["time"].as_f64()).is_some_and(|(a,b)| a.is_finite() && b.is_finite() && b > a + 1.)
        && first["frames"].as_u64().zip(last["frames"].as_u64()).is_some_and(|(a,b)| b > a)
}
fn observation_window(app: &AppHandle, channel: &str, end: Instant) -> Checked<Value> {
    let first = observe(app, channel, end)?;
    require(end.saturating_duration_since(Instant::now()) >= Duration::from_secs(3), "QA_DEADLINE")?;
    thread::sleep(Duration::from_secs(3));
    let last = observe(app, channel, end)?;
    Ok(json!({"progressed":progressed(&first,&last),"first":first,"last":last}))
}

struct BrowserFacts { ready: bool, reason: Option<String>, recording_count: usize }
fn browser_facts(browser: &OfficialBrowser) -> Checked<BrowserFacts> {
    // Use the existing serialized public snapshot; do not widen production fields.
    let snapshot = serde_json::to_value(browser.snapshot().map_err(|_| "QA_SOURCE_SNAPSHOT")?)
        .map_err(|_| "QA_SOURCE_SNAPSHOT_SHAPE")?;
    let ready = snapshot.get("ready").and_then(Value::as_bool).ok_or("QA_SOURCE_SNAPSHOT_SHAPE")?;
    let recording_count = snapshot.get("recordings").and_then(Value::as_array)
        .ok_or("QA_SOURCE_SNAPSHOT_SHAPE")?.len();
    let reason = match snapshot.get("captureDiagnostics") {
        Some(Value::Null) => None,
        Some(Value::Object(diagnostics)) => match diagnostics.get("reason") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if value.len() <= 80 => Some(value.clone()),
            _ => return Err("QA_SOURCE_SNAPSHOT_SHAPE".into()),
        },
        _ => return Err("QA_SOURCE_SNAPSHOT_SHAPE".into()),
    };
    Ok(BrowserFacts { ready, reason, recording_count })
}

fn video_files(data: &Path) -> Checked<usize> {
    let mut todo = vec![data.to_path_buf()];
    let mut entries = 0usize;
    let mut media = 0usize;
    while let Some(dir) = todo.pop() {
        for entry in fs::read_dir(&dir).map_err(|_| "QA_FILES")? {
            let entry = entry.map_err(|_| "QA_FILES")?;
            // WebView's HTTP cache is not an Atsumi media recording.
            if dir == data && entry.file_name().to_string_lossy().contains("browser-profile") { continue; }
            entries += 1;
            require(entries <= 8192, "QA_FILES_BOUND")?;
            let path = entry.path();
            let m = regular(&path)?;
            if m.is_dir() { todo.push(path); continue; }
            let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "mp4"|"m4s"|"ts"|"webm"|"mkv"|"flv") { media += 1; }
        }
    }
    Ok(media)
}

fn operation(service: &CloudService, id: &str) -> Checked<CloudLiveOperation> {
    service.live_snapshot().map_err(code)?.operations.into_iter()
        .find(|v| v.selection_id == id).ok_or("QA_OPERATION_MISSING".into())
}
fn operation_facts(view: &CloudLiveOperation) -> Value {
    json!({"state":view.state,"stage":view.preparation_stage,"revision":view.revision,
        "receiptPersisted":view.receipt_persisted,"streamPresent":view.stream_id.is_some(),
        "broadcastPresent":view.broadcast_id.is_some(),"outcomeUncertain":view.outcome_uncertain,
        "segmentAcks":view.segment_acks,"playlistAcks":view.playlist_acks,"endListAcked":view.end_list_acked,
        "stopRequested":view.stop_requested,"managerJoined":view.manager_joined,
        "workerJoined":view.worker_joined,"sidecarJoined":view.sidecar_joined,
        "interrupted":view.interrupted,"assetsIncomplete":view.assets_incomplete,
        "timingIncomplete":view.timing_incomplete,"errorCode":view.error.clone().map(code),
        "archive":view.archive_verification.as_ref().map(|a| json!({"state":a.state,"reason":a.reason,
            "rounds":a.rounds,"durationSeconds":a.duration_seconds,"embeddable":a.embeddable,
            "playbackVerified":a.playback_verified,"remoteTimeOrigin":a.remote_time_origin,
            "synchronizedReplayReady":a.synchronized_replay_ready}))})
}
fn stop_selected(service: &CloudService, id: &str) -> Checked<()> {
    // Retry only a stale revision rejected before mutation, never a start/create.
    for _ in 0..3 {
        let current = operation(service, id)?;
        if current.stop_requested || live_store::terminal(&current.state) { return Ok(()); }
        match service.stop_live(id, current.revision) {
            Ok(_) => return Ok(()),
            Err(e) if e.code == "CLOUD_LIVE_CHANGED" => {},
            Err(e) => return Err(code(e)),
        }
    }
    Err("QA_STOP_REVISION_CHANGED".into())
}

fn exercise(app: &AppHandle, service: &Arc<CloudService>, channel: &str, data: &Path,
    selected: &Mutex<Option<String>>, report: &mut Value) -> Checked<()> {
    let activity_end = Instant::now() + Duration::from_secs(300);
    let startup = app.state::<Arc<crate::startup::Startup>>();
    crate::initialize_backend(app, &startup).map_err(|_| "QA_BACKEND")?;
    let state = app.state::<AppState>();
    let _admission = state.begin_cloud_work().map_err(|_| "QA_ADMISSION")?;
    let source_end = Instant::now() + Duration::from_secs(65);
    let browser: OfficialBrowser = loop {
        if let Ok(browser) = state.deferred_browser().get() { break browser; }
        require(Instant::now() < source_end, "QA_BROWSER_TIMEOUT")?;
        thread::sleep(Duration::from_millis(100));
    };
    browser.open(app, channel).map_err(|_| "QA_SOURCE_OPEN")?;
    browser.set_viewport(app, BrowserViewport { x:0., y:0., width:960., height:720., visible:true,
        request_sequence:Some(1), ..Default::default() }).map_err(|_| "QA_SOURCE_VIEWPORT")?;
    loop {
        require(Instant::now() < source_end, "QA_SOURCE_TIMEOUT")?;
        let s = browser_facts(&browser)?;
        let reason = s.reason.as_deref();
        if reason.is_some_and(|v| matches!(v,"encrypted"|"codec_unsupported"|"container_unsupported"|"tracks_unsupported"|"native_init_unsupported"|"source_blocked")) {
            return Err("QA_SOURCE_UNSUPPORTED".into());
        }
        if s.ready && reason == Some("ready") {
            let sample = observe(app, channel, source_end)?;
            if sample["ready"].as_u64().unwrap_or(0) >= 2 && sample["paused"] == false { break; }
        }
        thread::sleep(Duration::from_millis(250));
    }
    report["beforeRelay"] = observation_window(app, channel, activity_end)?;
    require(report["beforeRelay"]["progressed"] == true, "QA_SOURCE_STALLED")?;
    require(browser_facts(&browser)?.recording_count == 0
        && video_files(data)? == 0, "QA_RECORDING_BASELINE")?;
    let source = NativeRelaySource::Chzzk(browser.select_relay_source_by_channel(channel).map_err(|_| "QA_SELECT_SOURCE")?);
    require(source.current(app), "QA_SOURCE_CHANGED")?;
    let snapshot = service.select_live(app, source).map_err(code)?;
    require(snapshot.operations.len() == 1, "QA_SELECTION_COUNT")?;
    let view = &snapshot.operations[0];
    let id = view.selection_id.clone();
    *held(selected) = Some(id.clone());
    report["selectionId"] = json!(id);
    report["startCalls"] = json!(1);
    service.start_live(app.clone(), &id, view.revision, CloudLivePlan {
        title: "Atsumi selected CHZZK relay verification".into(), scheduled_start_ms: now(), made_for_kids: false,
    }).map_err(code)?;
    let transfer_end = (Instant::now() + Duration::from_secs(150)).min(activity_end);
    let mut running_since: Option<Instant> = None;
    let mut samples = Vec::new();
    loop {
        require(Instant::now() < transfer_end, "QA_TRANSFER_DEADLINE")?;
        let view = operation(service, &id)?;
        report["operation"] = operation_facts(&view);
        require(view.error.is_none() && (!view.outcome_uncertain || view.state == "preparing"), "QA_RELAY_FAILED")?;
        require(!live_store::terminal(&view.state), "QA_RELAY_STOPPED_EARLY")?;
        if view.segment_acks > 0 {
            let began = *running_since.get_or_insert_with(Instant::now);
            require(samples.len() < 100, "QA_SAMPLE_BOUND")?;
            samples.push(observe(app, channel, transfer_end)?);
            report["duringRelaySamples"] = json!(samples);
            if began.elapsed() >= Duration::from_secs(45) && view.segment_acks >= 6 { break; }
            require(began.elapsed() < Duration::from_secs(90), "QA_TRANSFER_ACK_DEADLINE")?;
        }
        thread::sleep(Duration::from_secs(1));
    }
    require(samples.first().zip(samples.last()).is_some_and(|(a,b)| progressed(a,b)), "QA_SOURCE_STALLED_DURING")?;
    stop_selected(service, &id)?;
    report["normalStopRequested"] = json!(true);
    loop {
        let view = operation(service, &id)?;
        report["operation"] = operation_facts(&view);
        if view.manager_joined && view.worker_joined && view.sidecar_joined { break; }
        require(Instant::now() < activity_end, "QA_JOIN_DEADLINE")?;
        thread::sleep(Duration::from_millis(100));
    }
    let view = operation(service, &id)?;
    require(view.state == "joined" && view.error.is_none() && !view.interrupted && view.end_list_acked,
        "QA_RELAY_JOIN_INVARIANT")?;
    report["afterRelay"] = observation_window(app, channel, activity_end)?;
    require(report["afterRelay"]["progressed"] == true, "QA_SOURCE_STALLED_AFTER")?;
    let history_end = (Instant::now() + Duration::from_secs(15)).min(activity_end);
    loop {
        match service.open_live_history(&id) {
            Ok(history) => {
                report["history"] = serde_json::to_value(history.facts()).map_err(|_| "QA_HISTORY")?;
                let page = history.page(None, 12).map_err(code)?;
                match page {
                    crate::cloud_recording::live_sidecar::LiveHistoryPage::Chzzk { items, .. } => report["historyFirstPageCount"] = json!(items.len()),
                    _ => return Err("QA_HISTORY_PROVIDER".into()),
                }
                break;
            }
            Err(_) if Instant::now() < history_end => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(code(e)),
        }
    }
    require(report["history"]["remoteAnchor"] == "unknown", "QA_UNEXPECTED_REMOTE_ANCHOR")?;
    let archive_end = Instant::now() + Duration::from_secs(180);
    loop {
        let view = operation(service, &id)?;
        report["operation"] = operation_facts(&view);
        let terminal = view.archive_verification.as_ref().is_some_and(|a|
            !matches!(a.state, crate::cloud_recording::live_archive::ArchiveState::Pending
                | crate::cloud_recording::live_archive::ArchiveState::Checking));
        if terminal || Instant::now() >= archive_end { break; }
        thread::sleep(Duration::from_millis(500));
    }
    report["recordedVideoFiles"] = json!(video_files(data)?);
    require(report["recordedVideoFiles"] == 0 && browser_facts(&browser)?.recording_count == 0, "QA_VIDEO_RECORDED")?;
    report["relayAndLocalHistoryVerified"] = json!(true);
    Ok(())
}

fn drain(service: &Arc<CloudService>) -> Checked<()> {
    service.request_live_shutdown().map_err(code)?;
    let handles = std::mem::take(&mut *service.threads.lock().map_err(|_| "QA_THREADS")?);
    for handle in handles { handle.join().map_err(|_| "QA_JOIN")?; }
    Ok(())
}
fn write_report(path: &Path, value: &Value) -> Checked<()> {
    let mut file = fs::OpenOptions::new().create(true).truncate(true).write(true).open(path).map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|_| "QA_REPORT")?)
        .and_then(|_| file.sync_all()).map_err(|_| "QA_REPORT".into())
}

#[test]
#[ignore = "One explicitly selected public CHZZK source to unlisted YouTube Live; existing isolated owner only"]
fn actual_chzzk_selected_unlisted_live_and_history() -> Checked<()> {
    require(std::env::var("ATSUMI_SELECTED_LIVE_QA").as_deref() == Ok("one-chzzk-unlisted-live"), "QA_OPT_IN")?;
    let channel = std::env::var("ATSUMI_SELECTED_LIVE_QA_CHANNEL").map_err(|_| "QA_CHANNEL")?;
    require(channel.len() == 32 && channel.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "QA_CHANNEL")?;
    let qa_data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?)
        .join("local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510");
    let state_file = qa_data.join("cloud-recording/state.bin");
    let meta = regular(&state_file)?;
    require(meta.is_file() && meta.len() <= 8*1024*1024 && regular(&qa_data.join("cloud-recording/state.lock"))?.is_file(), "QA_STATE_BOUND")?;
    let vault = vault::Vault::open(&qa_data).map_err(|_| "QA_VAULT_BUSY")?;
    let catalog = vault.load().map_err(code)?;
    let owner = catalog.grant.as_ref().filter(|v| v.valid()).ok_or("QA_UPLOAD_OWNER")?.channel_id.clone();
    let live = catalog.live_grant.as_ref().filter(|v| v.valid() && v.grant.channel_id == owner).ok_or("QA_LIVE_GRANT_REQUIRED")?;
    let expected_scopes = live.scopes.clone();
    let expected_generation = catalog.live_generation;
    require(catalog.live_receipts.is_empty(), "QA_PRIOR_LIVE_RECEIPT_NO_RECREATE")?;
    require(catalog.jobs.len() == 5 && catalog.jobs.iter().all(|v| v.view.state == "ready" && v.channel_id == owner)
        && serde_json::to_value(&catalog.automatic).map_err(|_| "QA_BASELINE")?["rules"].as_array().is_some_and(|v| v.len() == 1), "QA_BASELINE")?;
    let expected = baseline(&catalog)?;
    let transport = Arc::new(RefreshOnly { inner:upload::GoogleTransport::new().map_err(code)?, calls:AtomicUsize::new(0) });
    let service = host(vault, catalog, transport.clone());
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    let root = workspace.join(".runtime/selected-live-native-qa").join(&uuid);
    fs::create_dir_all(root.parent().ok_or("QA_ROOT")?).map_err(|_| "QA_ROOT")?;
    fs::create_dir(&root).map_err(|_| "QA_ROOT")?;
    let identity = format!("local.atsumi.selectedliveqa.{uuid}");
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(&identity);
    require(!data.exists(), "QA_FRESH_PROFILE_REQUIRED")?;
    let result = Arc::new(Mutex::new(json!({"trialCompleted":false,"provider":"chzzk","sourceChannelId":channel,
        "freshViewerProfile":true,"existingQaVaultInPlace":true,"credentialCopies":0,
        "ordinaryProfileAccess":false,"sourceOpenAttempts":1,"startCalls":0,"desktopInput":false,
        "liveResourceDeleteCalls":0,"recordingUploadCalls":0,"unlistedRequested":true,"madeForKids":false,
        "remotePlaybackVerified":false,"synchronizedReplayVerified":false,"remoteAnchor":"unknown",
        "limits":{"sourceReadySeconds":65,"transferSeconds":90,"archiveObservationSeconds":180},
        "qualifications":["Creation uses production straight-line stream/broadcast/bind, one start; no independent HTTP count claimed", "Browser cache excluded from no recorded video claim", "Source progress samples do not establish universal live latency", "Remote playback and synchronized replay require separate proof"]})));
    let selected = Arc::new(Mutex::new(None::<String>));
    let worker_slot = Arc::new(Mutex::new(None::<thread::JoinHandle<()>>));
    let setup_slot = worker_slot.clone();
    let output = result.clone();
    let worker_service = service.clone();
    let worker_selected = selected.clone();
    let worker_root = root.clone();
    let worker_data = data.clone();
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
            if app.path().app_data_dir()? != data { return Err("QA_PROFILE_PATH".into()); }
            tauri::window::WindowBuilder::new(app,"main").title("Atsumi selected Live verification")
                .inner_size(960.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle = app.handle().clone();
            *held(&setup_slot) = Some(thread::spawn(move || {
                let mut report = held(&output).clone();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                    exercise(&handle,&worker_service,&channel,&worker_data,&worker_selected,&mut report)))
                    .unwrap_or_else(|_| Err("QA_WORKER_PANIC".into()));
                report["exerciseErrorCode"] = json!(outcome.as_ref().err());
                let _ = write_report(&worker_root.join("pending-result.json"),&report);
                if let Some(id) = held(&worker_selected).as_ref() {
                    report["cleanupStopAccepted"] = json!(stop_selected(&worker_service,id).is_ok());
                }
                let drained = drain(&worker_service);
                report["cloudThreadsDrained"] = json!(drained.is_ok());
                if let Some(id) = held(&worker_selected).as_ref() {
                    if let Ok(view) = operation(&worker_service,id) { report["finalOperation"] = operation_facts(&view); }
                }
                report["trialCompleted"] = json!(outcome.is_ok() && drained.is_ok());
                *held(&output) = report;
                let _ = write_report(&worker_root.join("pending-result.json"),&held(&output));
                if let Some(state) = handle.try_state::<AppState>() {
                    match state.active_work_snapshot().map_err(|_| ()).and_then(|s| state.request_graceful_quit(handle.clone(), AppQuitRequest {
                        expected_work_set_fingerprint:s.work_set_fingerprint,confirm_active_work:true,force_when_status_unknown:false,
                    }).map_err(|_| ())) {
                        Ok(answer) if answer.accepted => { held(&output)["normalQuitAccepted"] = json!(true); },
                        _ => { held(&output)["trialCompleted"] = json!(false); held(&output)["quitError"] = json!("QA_NORMAL_QUIT_REJECTED"); },
                    }
                } else {
                    held(&output)["backendNeverInitialized"] = json!(true);
                    handle.exit(2);
                }
            }));
            Ok(())
        }).build(context).map_err(|_| "QA_BUILD")?;
    app.run_return(|_,_|{});
    let observer_joined = held(&worker_slot).take().is_some_and(|h| h.join().is_ok());
    let saved = service.vault.load().map_err(code)?;
    let preserved = baseline(&saved)? == expected;
    let owner_preserved = saved.live_generation == expected_generation && saved.live_grant.as_ref()
        .is_some_and(|g| g.valid() && g.grant.channel_id == owner && g.scopes == expected_scopes);
    let selected_id = held(&selected).clone();
    let receipt_count_valid = saved.live_receipts.len() == usize::from(selected_id.is_some())
        && selected_id.as_ref().is_none_or(|id| saved.live_receipts[0].view.selection_id == *id);
    let mut report = held(&result).clone();
    report["qaObserverJoined"] = json!(observer_joined);
    report["normalAppExitReturned"] = json!(true);
    report["priorCatalogPreserved"] = json!(preserved && owner_preserved);
    report["newReceiptCountValid"] = json!(receipt_count_valid);
    report["newReceiptCount"] = json!(saved.live_receipts.len());
    report["refreshRequests"] = json!(transport.calls.load(Ordering::SeqCst));
    if !observer_joined || !preserved || !owner_preserved || !receipt_count_valid || report["normalQuitAccepted"] != true {
        report["trialCompleted"] = json!(false);
    }
    let final_path = root.join("result.json");
    write_report(&final_path,&report)?;
    println!("QA_SELECTED_LIVE_RESULT={}",final_path.display());
    require(report["trialCompleted"] == true, "QA_TRIAL_UNVERIFIED")
}
