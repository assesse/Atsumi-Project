//! Opt-in actual native production React/YouTube document + exact SOOP42 receipt.
//! No production hook, upload, original-video read, account action or desktop input.
use super::*;
use crate::cloud_recording::{CloudJobSnapshot, replay::{CloudReplayAuthorization, CloudReplayOpen}};
use crate::youtube::original::{OriginalSnapshot, VodClock, VodControlRequest, YouTubeOriginalHost};
use serde_json::Value;
use std::{io::Write, sync::atomic::AtomicBool, thread, time::{Duration, Instant}};
use tauri::{Manager, State, Webview};

const ENTRY: &str = "/.runtime/requirements-audit/20261117-cloud-goal-resume/actual42-native-replay.html";
struct QaState {
    media: PlatformMediaService,
    authorization: CloudReplayAuthorization,
    job: CloudJobSnapshot,
    leases: Mutex<Vec<String>>,
    video_tokens: Mutex<Vec<String>>,
    deadline: Instant,
    closed: AtomicBool,
}
fn guard(window: &Webview, state: &QaState) -> Result<tauri::AppHandle, String> {
    let app = window.app_handle();
    let url = window.url().map_err(|_| "QA_WINDOW")?;
    if state.closed.load(Ordering::Acquire) || Instant::now() >= state.deadline
        || url.path() != ENTRY
        || !crate::frontend_origin::trusted_main(window.label(), &url, true, app.config().build.dev_url.as_ref()) {
        return Err("QA_SCOPE".into());
    }
    Ok(app.clone())
}
fn job_allowed(state: &QaState, id: &str) -> Result<(), String> {
    if id == state.job.id { Ok(()) } else { Err("QA_JOB".into()) }
}
fn lease_allowed(state: &QaState, token: &str) -> Result<(), String> {
    if state.leases.lock().unwrap().iter().any(|value| value == token) { Ok(()) } else { Err("QA_LEASE".into()) }
}
#[tauri::command]
fn qa_actual42_bootstrap(window: Webview, state: State<'_, Arc<QaState>>) -> Result<Value, String> {
    guard(&window, &state)?;
    Ok(json!({"job":state.job,"expectedRows":14,"durationSeconds":42.102521}))
}
#[tauri::command]
fn qa_actual42_describe(window: Webview, state: State<'_, Arc<QaState>>, job_id: String) -> Result<crate::cloud_recording::source::SourceDescription, String> {
    guard(&window, &state)?; job_allowed(&state, &job_id)?;
    Ok(state.authorization.description.clone())
}
#[tauri::command]
fn qa_actual42_open(window: Webview, state: State<'_, Arc<QaState>>, job_id: String) -> Result<CloudReplayOpen, String> {
    guard(&window, &state)?; job_allowed(&state, &job_id)?;
    let mut leases = state.leases.lock().unwrap();
    if leases.len() >= 2 { return Err("QA_LEASE_LIMIT".into()); }
    let token = state.media.open_cloud_chat(&state.authorization).map_err(|_| "QA_SIDECAR_OPEN")?;
    leases.push(token.clone());
    Ok(CloudReplayOpen { provider:"soop".into(), token, cohort_id:state.authorization.cohort_id.clone(),
        description:state.authorization.description.clone(), session:None })
}
#[tauri::command]
fn qa_actual42_page(window: Webview, state: State<'_, Arc<QaState>>, token: String, position_ms: u64, cursor: Option<String>, limit: usize) -> Result<replay_chat::ReplayChatPage, String> {
    guard(&window, &state)?; lease_allowed(&state, &token)?;
    if position_ms > 43_000 || limit > 120 || limit == 0 || cursor.as_ref().is_some_and(|v| v.len() > 1024) { return Err("QA_PAGE_BOUND".into()); }
    state.media.cloud_chat_page(&token, position_ms, cursor.as_deref(), limit).map_err(|_| "QA_PAGE".into())
}
#[tauri::command]
fn qa_actual42_asset(window: Webview, state: State<'_, Arc<QaState>>, token: String, asset_id: String) -> Result<live_chat::ChatAssetRead, String> {
    guard(&window, &state)?; lease_allowed(&state, &token)?;
    state.media.cloud_chat_asset(&token, &asset_id).map_err(|_| "QA_ASSET".into())
}
#[tauri::command]
fn qa_actual42_close(window: Webview, state: State<'_, Arc<QaState>>, provider: String, token: String) -> Result<(), String> {
    guard(&window, &state)?;
    if provider != "soop" { return Err("QA_PROVIDER".into()); }
    state.media.close_cloud_chat(&token);
    state.leases.lock().unwrap().retain(|value| value != &token);
    Ok(())
}
// Only the normal command's AppState work-admission wrapper differs in this test.
// The real receiver, native document bridge, clock, geometry and controls are used.
#[tauri::command]
async fn youtube_original_open(window: Webview, state: State<'_, Arc<QaState>>, channel_id: String, request_id: String, playback_kind: Option<String>) -> Result<OriginalSnapshot, String> {
    let app = guard(&window, &state)?;
    if playback_kind.as_deref() != Some("vod") || state.job.video_id.as_deref() != Some(channel_id.as_str()) { return Err("QA_VIDEO".into()); }
    let owned = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = app.state::<YouTubeOriginalHost>().open_vod(&app, channel_id, request_id)?;
        owned.video_tokens.lock().unwrap().push(value.token.clone());
        Ok(value)
    }).await.map_err(|_| "QA_OPEN_WORKER".to_owned())?
}

fn evaluate(view: &Webview, script: &str) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script, move |raw| { let _ = tx.try_send((raw.len() <= 8192).then_some(raw)); }).map_err(|_| "QA_EVAL")?;
    let raw = rx.recv_timeout(Duration::from_secs(3)).map_err(|_| "QA_EVAL_TIMEOUT")?.ok_or("QA_EVAL_BOUND")?;
    serde_json::from_str(&raw).map_err(|_| "QA_EVAL_JSON".into())
}
const DOM: &str = r#"(()=>{const q=window.__atsumiActual42Qa,h=document.querySelector('aside[data-provider="soop"]'),s=h?.shadowRoot,a=s?.querySelector('.author'),m=s?.querySelector('.msg'),area=s?.querySelector('#chatArea'),head=s?.querySelector('.area_header'),foot=s?.querySelector('.actionbox');return {ready:q?.ready===true,dialog:!!document.querySelector('.cloud-recording-replay'),shadow:!!s,rows:s?.querySelectorAll('.chat_memoyo>.chatting-list-item').length??0,fans:s?.querySelectorAll('.grade-badge-fan').length??0,font:a?getComputedStyle(a).fontSize:null,line:m?getComputedStyle(m).lineHeight:null,header:head?.getBoundingClientRect().height??0,footer:foot?.getBoundingClientRect().height??0,padding:area?getComputedStyle(area).padding:null,overflow:!!area&&area.scrollWidth>area.clientWidth+1,fontReady:document.fonts.check('16px "Atsumi SOOP Pretendard"'),alerts:document.querySelectorAll('[role="alert"]').length,hasEnabledSend:!!s?.querySelector('.btn_send:not(:disabled)')}})()"#;
const VIDEO: &str = r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video');if(v)v.muted=true;return {ready:v?.readyState??0,time:v?.currentTime??null,duration:Number.isFinite(v?.duration)?v.duration:null,paused:v?.paused??null,frames:v?.getVideoPlaybackQuality?.().totalVideoFrames??0,ad:!!p?.classList.contains('ad-showing'),capture:!!window.__atsumiYouTubeEncodedCapture,chat:!!document.querySelector('iframe#chatframe')}})()"#;
fn wait_until(deadline: Instant, mut read: impl FnMut() -> Result<bool, String>) -> Result<(), String> {
    loop {
        if Instant::now() >= deadline { return Err("QA_OBSERVATION_TIMEOUT".into()); }
        if read()? { return Ok(()); }
        thread::sleep(Duration::from_millis(150));
    }
}
fn clock(app: &tauri::AppHandle, token: &str) -> Result<VodClock, String> {
    app.state::<YouTubeOriginalHost>().vod_clock(token)?.filter(|v| v.fields.valid && v.fields.ad_state == "content" && v.age_ms <= 1500).ok_or("QA_CONTENT_CLOCK".into())
}
fn control(app: &tauri::AppHandle, token: &str, action: &str, seconds: Option<f64>) -> Result<(), String> {
    let observed = clock(app, token)?;
    app.state::<YouTubeOriginalHost>().vod_control(app, token, VodControlRequest {
        request_id:uuid::Uuid::new_v4().to_string(), document_epoch:observed.document_epoch,
        source_id:observed.fields.source_id, source_generation:observed.fields.source_generation,
        action:action.into(), seconds, rate:None,
    }).map_err(|_| "QA_NATIVE_CONTROL".into())
}
fn sample_frames(app: &tauri::AppHandle, state: &QaState, token: &str, output: &Path) -> Result<Value, String> {
    let before = clock(app, token)?;
    let selected_video = state.job.video_id.as_deref().ok_or("QA_REMOTE_IDENTITY")?;
    let result = app.state::<YouTubeOriginalHost>().vod_sample_frames(app, token, selected_video, &state.closed);
    let (facts, failure) = match result {
        Ok(window) => {
            let after = clock(app, token).ok();
            let same_source = window.video_id == selected_video
                && window.video_id == before.fields.video_id
                && window.document_epoch == before.document_epoch
                && window.source_id.to_string() == before.fields.source_id
                && window.source_generation == before.fields.source_generation
                && window.seek_generation == before.fields.seek_generation
                && after.as_ref().is_some_and(|value| value.document_epoch == window.document_epoch
                    && value.fields.video_id == window.video_id
                    && value.fields.source_id == window.source_id.to_string()
                    && value.fields.source_generation == window.source_generation
                    && value.fields.seek_generation == window.seek_generation);
            let distinct = window.frames.iter().enumerate().filter(|(index, frame)|
                !window.frames[..*index].iter().any(|previous| previous.feature == frame.feature)).count();
            let span = window.frames.first().zip(window.frames.last())
                .map(|(first, last)| last.media_pts90k.saturating_sub(first.media_pts90k)).unwrap_or(0);
            let continuous = after.as_ref().is_some_and(|value| !value.fields.paused
                && !value.fields.seeking && !value.fields.buffering && !value.fields.ended
                && value.fields.playback_rate == before.fields.playback_rate
                && value.fields.current_time.zip(before.fields.current_time).is_some_and(|(end, start)| end >= start));
            let failure = (!same_source || !continuous).then(|| "QA_REMOTE_FRAME_CONTINUITY".to_owned());
            (json!({"attempted":true,"sampled":true,"frameCount":window.frames.len(),
                "spanTicks90k":span,"spanSeconds":span as f64 / 90_000.0,
                "distinctFeatureCount":distinct,"multipleDistinctFeatures":distinct > 1,
                "sameSourceBeforeAndAfter":same_source,"playbackContinued":continuous,
                "uncertaintyTicksMaximum":window.frames.iter().map(|frame| frame.uncertainty_ticks).max(),
                "pixelOrMediaFilesWritten":false,"sourceToRemoteAlignmentVerified":false}), failure)
        },
        Err(error) => {
            // Native errors are fixed codes. Never copy arbitrary page/URL text into the report.
            let safe = if error.starts_with("remote_frame_") && error.len() <= 64
                && error.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_') {
                error.as_str()
            } else { "remote_frame_native_rejected" };
            (json!({"attempted":true,"sampled":false,"errorCode":safe,
                "pixelOrMediaFilesWritten":false,"sourceToRemoteAlignmentVerified":false}),
                Some(format!("QA_REMOTE_FRAME_SAMPLE:{safe}")))
        }
    };
    // Keep this bounded, non-media evidence even if later pause/seek checks fail.
    fs::write(output.join("remote-frame-sample.json"), serde_json::to_vec_pretty(&facts).map_err(|_| "QA_SAMPLE_REPORT")?)
        .map_err(|_| "QA_SAMPLE_REPORT")?;
    if let Some(error) = failure { return Err(error); }
    Ok(facts)
}
fn capture(view: &Webview, path: &Path) -> Result<(), String> {
    use webview2_com::{CapturePreviewCompletedHandler, Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG};
    use windows::Win32::{System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET}, UI::Shell::SHCreateMemStream};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let Some(stream) = SHCreateMemStream(None) else { let _ = tx.try_send(Err("QA_CAPTURE_STREAM")); return; };
        let data = stream.clone(); let completed = tx.clone();
        let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
            let result = (|| -> Result<Vec<u8>, &'static str> {
                status.map_err(|_| "QA_CAPTURE")?;
                let mut stat = STATSTG::default(); data.Stat(&mut stat, STATFLAG_NONAME).map_err(|_| "QA_CAPTURE_STAT")?;
                if stat.cbSize == 0 || stat.cbSize > 16 * 1024 * 1024 { return Err("QA_CAPTURE_BOUND"); }
                data.Seek(0, STREAM_SEEK_SET, None).map_err(|_| "QA_CAPTURE_SEEK")?;
                let mut bytes = vec![0; stat.cbSize as usize]; let mut read = 0;
                data.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut read)).ok().map_err(|_| "QA_CAPTURE_READ")?;
                if read as usize != bytes.len() || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("QA_CAPTURE_PNG"); }
                Ok(bytes)
            })(); let _ = completed.try_send(result); Ok(())
        }));
        if platform.controller().CoreWebView2().and_then(|core| core.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &callback)).is_err() { let _ = tx.try_send(Err("QA_CAPTURE_DISPATCH")); }
    }).map_err(|_| "QA_CAPTURE_DISPATCH")?;
    let bytes = rx.recv_timeout(Duration::from_secs(6)).map_err(|_| "QA_CAPTURE_TIMEOUT")?.map_err(str::to_owned)?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|_| "QA_CAPTURE_CREATE")?;
    file.write_all(&bytes).map_err(|_| "QA_CAPTURE_SAVE".into())
}

fn exercise(app: &tauri::AppHandle, state: &QaState, output: &Path) -> Result<Value, String> {
    let main = app.get_webview("main").ok_or("QA_MAIN")?;
    let ready = Instant::now() + Duration::from_secs(70);
    wait_until(ready, || Ok(evaluate(&main, DOM)?["ready"] == true && !state.video_tokens.lock().unwrap().is_empty()))?;
    let token = state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_VIDEO_TOKEN")?;
    let mut first_time = None; let mut progressed = false;
    wait_until(ready, || {
        if let Ok(c) = clock(app, &token) {
            if c.fields.video_id != state.job.video_id.as_deref().unwrap_or("") || c.fields.duration.is_none_or(|v| (v - 42.102521).abs() > 1.0) { return Err("QA_REMOTE_IDENTITY".into()); }
            if let Some(t) = c.fields.current_time { let first = *first_time.get_or_insert(t); progressed |= t > first + 0.7; }
        }
        Ok(progressed)
    })?;
    let remote = app.webviews().into_values().find(|v| v.label().starts_with("youtube-original-vod-")).ok_or("QA_REMOTE_VIEW")?;
    let video = evaluate(&remote, VIDEO)?;
    if video["frames"].as_u64().unwrap_or(0) == 0 || video["ad"] != false || video["capture"] != false || video["chat"] != false { return Err("QA_REMOTE_DECODE".into()); }
    let frame_sample = sample_frames(app, state, &token, output)?;
    control(app, &token, "pause", None)?;
    let actions = (Instant::now() + Duration::from_secs(40)).min(state.deadline);
    wait_until(actions, || Ok(clock(app, &token).is_ok_and(|v| v.fields.paused)))?;
    let paused_clock = clock(app, &token)?;
    let before = paused_clock.fields.current_time.ok_or("QA_PAUSE_TIME")?;
    thread::sleep(Duration::from_millis(800));
    wait_until(actions, || Ok(clock(app, &token).is_ok_and(|value| value.document_epoch==paused_clock.document_epoch
        && value.fields.source_id==paused_clock.fields.source_id && value.fields.source_generation==paused_clock.fields.source_generation
        && value.fields.sequence>paused_clock.fields.sequence && value.fields.paused)))?;
    let after = clock(app, &token)?.fields.current_time.ok_or("QA_PAUSE_TIME")?;
    if (after - before).abs() > 0.15 { return Err("QA_PAUSE_DRIFT".into()); }
    control(app, &token, "seek", Some(0.0))?;
    wait_until(actions, || Ok(clock(app, &token).is_ok_and(|v| v.fields.current_time.is_some_and(|t| t < 0.5)) && evaluate(&main, DOM)?["rows"] == 0))?;
    let backward = evaluate(&main, DOM)?;
    control(app, &token, "seek", Some(40.0))?;
    wait_until(actions, || Ok(clock(app, &token).is_ok_and(|v| v.fields.current_time.is_some_and(|t| (t - 40.0).abs() < 0.5)) && evaluate(&main, DOM)?["rows"] == 14))?;
    let presentation = evaluate(&main, DOM)?;
    if presentation["shadow"] != true || presentation["fans"] != 14 || presentation["font"] != "16px" || presentation["line"] != "25px"
        || presentation["header"] != 48 || presentation["footer"] != 108 || presentation["overflow"] != false
        || presentation["fontReady"] != true || presentation["hasEnabledSend"] != false { return Err("QA_PRESENTATION".into()); }
    capture(&main, &output.join("actual42-native-chat.png"))?;
    capture(&remote, &output.join("actual42-native-video.png"))?;
    let old_leases = state.leases.lock().unwrap().clone();
    evaluate(&main, "(()=>{window.__atsumiActual42Qa.close();return true})()")?;
    wait_until(actions, || Ok(state.leases.lock().unwrap().is_empty() && !evaluate(&main, DOM)?["dialog"].as_bool().unwrap_or(true)))?;
    if old_leases.iter().any(|token| state.media.cloud_chat_page(token, 0, None, 1).is_ok()) { return Err("QA_REVOKE".into()); }
    evaluate(&main, "(()=>{window.__atsumiActual42Qa.reopen();return true})()")?;
    wait_until(actions, || Ok(state.video_tokens.lock().unwrap().last().is_some_and(|value| value != &token) && !state.leases.lock().unwrap().is_empty()))?;
    let new_token = state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_REOPEN")?;
    wait_until(actions, || Ok(clock(app, &new_token).is_ok()))?;
    control(app, &new_token, "pause", None)?;
    control(app, &new_token, "seek", Some(40.0))?;
    wait_until(actions, || Ok(clock(app,&new_token).is_ok_and(|value|value.fields.paused&&value.fields.current_time.is_some_and(|t|(t-40.).abs()<0.5))
        && evaluate(&main, DOM)?["rows"] == 14))?;
    evaluate(&main, "(()=>{window.__atsumiActual42Qa.close();return true})()")?;
    wait_until(actions, || Ok(state.leases.lock().unwrap().is_empty()))?;
    Ok(json!({"passed":true,"actualRemoteVideoProgressed":progressed,"decodedFramesObserved":video["frames"],
        "actualNativeClock":true,"actualNativeProductionReact":true,"nativeFrameSample":frame_sample,"pauseDriftSeconds":(after-before).abs(),
        "backward":backward,"forwardPresentation":presentation,"closeRevokesSidecar":true,"reopen":true,
        "donationAvVerified":false,"ogqAvVerified":false,"localVideoBytesRead":0,"desktopInput":false,"audioAudibilityVerified":false}))
}

#[test]
#[ignore = "Actual native isolated unlisted SOOP42 replay; explicit scoped environment required"]
fn actual_soop42_native_remote_video_chat_and_capture() {
    assert_eq!(std::env::var("ATSUMI_SOOP_NATIVE_REPLAY_QA").as_deref(), Ok("actual-unlisted-readonly"), "QA_OPT_IN");
    let host = ActualReceiptHost::open_current();
    let authorization = host.authorization(); let job = host.job_view();
    let catalog = host.profile().join("platform-media/jobs").join(format!("{}.json", actual::RECORDING));
    let raw = host.read(&catalog, 16 * 1024); assert_eq!(actual::digest(&raw), CATALOG_SHA, "QA_CATALOG");
    let source_job: MediaJob = serde_json::from_slice(&raw).unwrap();
    let directory = storage::job_dir(&source_job.download_root, "soop", actual::RECORDING).unwrap();
    let before = snapshot(&host, &directory);
    let media = no_workers(host.scratch(), source_job);
    media.validate_cloud_chat_source(&authorization).unwrap_or_else(|_| panic!("QA_SOURCE"));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let output = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(format!("actual42-native-{run_id}"));
    fs::create_dir(&output).unwrap();
    let identity = format!("local.atsumi.cloudreplayqa.{run_id}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let url: tauri::Url = std::env::var("ATSUMI_SOOP_NATIVE_REPLAY_QA_URL").unwrap_or_else(|_| format!("http://127.0.0.1:5181{ENTRY}")).parse().unwrap();
    assert!(url.scheme()=="http" && url.host_str()==Some("127.0.0.1") && url.port()==Some(5181) && url.path()==ENTRY
        && url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(), "QA_ENTRY");
    let state = Arc::new(QaState {media:media.clone(),authorization,job,leases:Mutex::new(Vec::new()),video_tokens:Mutex::new(Vec::new()),deadline:Instant::now()+Duration::from_secs(120),closed:AtomicBool::new(false)});
    let outcome = Arc::new(Mutex::new(None::<Result<Value,String>>));
    let outcome_worker = outcome.clone(); let worker_state = state.clone(); let output_worker = output.clone(); let profile_check = profile.clone();
    let old_args = std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--mute-audio");
    let mut context = tauri::generate_context!();
    context.config_mut().identifier=identity;
    context.config_mut().build.dev_url=Some(url.clone());
    context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app = tauri::Builder::default().any_thread().manage(state.clone()).manage(YouTubeOriginalHost::default())
        .invoke_handler(tauri::generate_handler![qa_actual42_bootstrap,qa_actual42_describe,qa_actual42_open,qa_actual42_page,qa_actual42_asset,qa_actual42_close,youtube_original_open,
            crate::youtube::original_commands::youtube_original_snapshot,crate::youtube::original_commands::youtube_original_vod_clock,crate::youtube::original_commands::youtube_original_vod_control,
            crate::youtube::original_commands::youtube_original_viewport,crate::youtube::original_commands::youtube_original_close,crate::youtube::original_commands::youtube_original_controls,crate::youtube::original_commands::youtube_original_fullscreen])
        .setup(move |app| {
            if app.path().app_data_dir()? != profile_check { return Err("QA_PROFILE_PATH".into()); }
            tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::External(url)).data_directory(profile_check.join("main-webview")).title("Atsumi native replay verification").inner_size(1280.,760.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle=app.handle().clone();
            thread::spawn(move || {
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| exercise(&handle,&worker_state,&output_worker)))
                    .unwrap_or_else(|_| Err("QA_WORKER_PANIC".into()));
                if value.is_err() {
                    let main = handle.get_webview("main");
                    let remote = handle.webviews().into_values().find(|v| v.label().starts_with("youtube-original-vod-"));
                    let facts = json!({"main":main.as_ref().and_then(|view|evaluate(view,DOM).ok()),"video":remote.as_ref().and_then(|view|evaluate(view,VIDEO).ok())});
                    let _=fs::write(output_worker.join("failure-facts.json"),serde_json::to_vec_pretty(&facts).unwrap());
                    if let Some(view)=main {let _=capture(&view,&output_worker.join("failure-main.png"));}
                    if let Some(view)=remote {let _=capture(&view,&output_worker.join("failure-video.png"));}
                }
                worker_state.closed.store(true,Ordering::Release);
                for token in worker_state.leases.lock().unwrap().drain(..) { worker_state.media.close_cloud_chat(&token); }
                let receiver=handle.state::<YouTubeOriginalHost>();
                for token in worker_state.video_tokens.lock().unwrap().drain(..) { let _=receiver.close(&handle,&token); }
                receiver.shutdown(&handle);
                *outcome_worker.lock().unwrap()=Some(value);
                handle.exit(0);
            }); Ok(())
        }).build(context).unwrap_or_else(|_| panic!("QA_NATIVE_BUILD"));
    app.run_return(|_,_|{});
    if let Some(value)=old_args {std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",value)} else {std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")}
    assert_eq!(snapshot(&host,&directory),before,"QA_SOURCE_SIDECARS_CHANGED");
    assert_eq!(actual::digest(&host.read(&catalog,16*1024)),CATALOG_SHA,"QA_CATALOG_CHANGED");
    assert!(media.inner.playback.lock().unwrap().is_empty() && media.inner.cloud_chat.lock().unwrap().is_empty() && media.inner.upload_leases.lock().unwrap().is_empty(),"QA_LEASE_REMAINS");
    { let saved=media.inner.state.lock().unwrap(); assert!(saved.queue.is_empty()&&saved.cancel.is_empty()&&!saved.shutting_down,"QA_WORK_STARTED"); }
    host.assert_unchanged(); drop(state); drop(media); let read_bytes=host.finish();
    // Only the fresh UUID profile may be cleaned; a busy WebView profile is retained.
    let cleaned = cleanup_profile(&profile,&format!("local.atsumi.cloudreplayqa.{run_id}"));
    let result=outcome.lock().unwrap().take().unwrap_or_else(||Err("QA_NO_RESULT".into()));
    let report=match &result {Ok(value)=>json!({"result":value,"sourceHashesUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes}),Err(code)=>json!({"passed":false,"errorCode":code,"sourceHashesUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes})};
    fs::write(output.join("result.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("QA_NATIVE_RESULT={}",output.join("result.json").display());
    assert!(result.is_ok(),"Actual native playback unverified; inspect sanitized result");
}

fn cleanup_profile(profile:&Path, identity:&str)->bool {
    use std::os::windows::fs::MetadataExt;
    if !profile.exists() {return true;}
    let Some(parent)=profile.parent() else{return false;};
    let Ok(base)=fs::canonicalize(parent) else{return false;};
    let Some(appdata)=std::env::var_os("APPDATA") else{return false;};
    if fs::canonicalize(PathBuf::from(appdata)).ok().as_ref()!=Some(&base) {return false;}
    let Ok(resolved)=fs::canonicalize(profile) else{return false;};
    if resolved.parent()!=Some(base.as_path()) || resolved.file_name().and_then(|v|v.to_str())!=Some(identity)
        || !identity.starts_with("local.atsumi.cloudreplayqa.") {return false;}
    for path in profile.ancestors() {
        let Ok(meta)=fs::symlink_metadata(path) else{return false;};
        if meta.file_type().is_symlink() || meta.file_attributes()&0x400!=0 {return false;}
    }
    fs::remove_dir_all(&resolved).is_ok()
}

// Additional opt-in full playback check; the original short test stays unchanged.
#[path = "cloud_replay_native_full_test.rs"]
mod full_playthrough;
