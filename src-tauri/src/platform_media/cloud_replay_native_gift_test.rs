//! Opt-in actual native production React/YouTube document + exact SOOP gift receipt.
//! No production hook, upload, original-video read, account action or desktop input.
use super::*;
use crate::cloud_recording::{CloudJobSnapshot, replay::{CloudReplayAuthorization, CloudReplayOpen}};
use crate::youtube::original::{OriginalSnapshot, VodClock, VodControlRequest, YouTubeOriginalHost};
use serde_json::Value;
use std::{io::Write, sync::atomic::AtomicBool, thread, time::{Duration, Instant}};
use tauri::{Manager, State, Webview};

const ENTRY: &str = "/.runtime/requirements-audit/20261117-cloud-goal-resume/actualgift-native-replay.html";
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
fn qa_actualgift_bootstrap(window: Webview, state: State<'_, Arc<QaState>>) -> Result<Value, String> {
    guard(&window, &state)?;
    Ok(json!({"job":state.job,"expectedRows":92,"durationSeconds":108.229}))
}
#[tauri::command]
fn qa_actualgift_describe(window: Webview, state: State<'_, Arc<QaState>>, job_id: String) -> Result<crate::cloud_recording::source::SourceDescription, String> {
    guard(&window, &state)?; job_allowed(&state, &job_id)?;
    Ok(state.authorization.description.clone())
}
#[tauri::command]
fn qa_actualgift_open(window: Webview, state: State<'_, Arc<QaState>>, job_id: String) -> Result<CloudReplayOpen, String> {
    guard(&window, &state)?; job_allowed(&state, &job_id)?;
    let mut leases = state.leases.lock().unwrap();
    if leases.len() >= 2 { return Err("QA_LEASE_LIMIT".into()); }
    let token = state.media.open_cloud_chat(&state.authorization).map_err(|_| "QA_SIDECAR_OPEN")?;
    leases.push(token.clone());
    Ok(CloudReplayOpen { provider:"soop".into(), token, cohort_id:state.authorization.cohort_id.clone(),
        description:state.authorization.description.clone(), session:None })
}
#[tauri::command]
fn qa_actualgift_page(window: Webview, state: State<'_, Arc<QaState>>, token: String, position_ms: u64, cursor: Option<String>, limit: usize) -> Result<replay_chat::ReplayChatPage, String> {
    guard(&window, &state)?; lease_allowed(&state, &token)?;
    if position_ms > 109_000 || limit > 120 || limit == 0 || cursor.as_ref().is_some_and(|v| v.len() > 1024) { return Err("QA_PAGE_BOUND".into()); }
    state.media.cloud_chat_page(&token, position_ms, cursor.as_deref(), limit).map_err(|_| "QA_PAGE".into())
}
#[tauri::command]
fn qa_actualgift_asset(window: Webview, state: State<'_, Arc<QaState>>, token: String, asset_id: String) -> Result<live_chat::ChatAssetRead, String> {
    guard(&window, &state)?; lease_allowed(&state, &token)?;
    state.media.cloud_chat_asset(&token, &asset_id).map_err(|_| "QA_ASSET".into())
}
#[tauri::command]
fn qa_actualgift_close(window: Webview, state: State<'_, Arc<QaState>>, provider: String, token: String) -> Result<(), String> {
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
const DOM: &str = r#"(()=>{const q=window.__atsumiActualGiftQa,h=document.querySelector('aside[data-provider="soop"]'),s=h?.shadowRoot,area=s?.querySelector('#chatArea'),imgs=Array.from(s?.querySelectorAll('.donation-container .custom-mobile-img img')??[]),row=s?.querySelector('.chat_memoyo'),notice=document.querySelector('.cloud-recording-capture-warning'),nr=notice?.getBoundingClientRect(),hr=h?.getBoundingClientRect();return {ready:q?.ready===true,dialog:!!document.querySelector('.cloud-recording-replay'),shadow:!!s,rows:row?.querySelectorAll('.platform-replay-chat-row').length??0,gifts:row?.querySelectorAll('.donation-container').length??0,customImages:imgs.length,customImagesLoaded:imgs.filter(i=>i.complete&&i.naturalWidth>0&&i.naturalHeight>0&&i.getAttribute('src')?.startsWith('data:image/')).length,loading:!!s?.querySelector('.soop-replay-status[role="status"]'),overflow:!!area&&area.scrollWidth>area.clientWidth+1,errors:document.querySelectorAll('.cloud-recording-error:not(.cloud-recording-capture-warning)').length,partialNotice:!!notice&&!!nr&&nr.width>0&&nr.height>0&&nr.top>=0&&nr.bottom<=innerHeight+1,chatHeight:hr?.height??0,chatWithinViewport:!!hr&&hr.height>200&&hr.top>=0&&hr.bottom<=innerHeight+1,hasEnabledSend:!!s?.querySelector('.btn_send:not(:disabled)')}})()"#;
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

const GIFT_RECORDING:&str="8fd00700-81d4-401f-a083-0d13fcf286ea";
const GIFT_CATALOG_SHA:&str="9224ca11d926311d8dc4f2f45fd7f32b4ae6c899f1b46ba3f23be39e59281cfe";
const GIFT_ASSET:&str="c301a923db42eca5ba685d03954ee3e49c985ed5ebb1dad0b4d740c539143a76";
fn gift_snapshot(host:&crate::cloud_recording::replay_receipt_actual::ActualGiftReceiptHost,directory:&Path)->Vec<Option<(usize,String)>>{
    let mut names=SIDECARS.to_vec();names.extend(["encoded-segments.jsonl","encoded-stop-diagnostic.json","encoded-stop.json","merge-attempts.jsonl"]);
    let mut result:Vec<_>=names.iter().map(|name|{let p=directory.join(name);if !p.exists(){return None;}let b=host.read(&p,1024*1024);Some((b.len(),actual::digest(&b)))}).collect();
    let b=host.read(&directory.join("chat-assets").join(GIFT_ASSET),64*1024);
    assert_eq!(actual::digest(&b),"2656ce3ae70a09e0d74f3e4472b81d6310bcd1bc033d2c3064a10f33ec388a8d","QA_GIFT_ASSET_CHANGED");
    result.push(Some((b.len(),actual::digest(&b))));result
}
fn gift_seek(app:&tauri::AppHandle,state:&QaState,main:&Webview,token:&str,seconds:f64,gifts:u64)->Result<Value,String>{
    control(app,token,"seek",Some(seconds))?;
    wait_until((Instant::now()+Duration::from_secs(8)).min(state.deadline),||{
        let c=clock(app,token)?;let ui=evaluate(main,DOM)?;
        Ok(c.fields.paused&&!c.fields.seeking&&c.fields.current_time.is_some_and(|t|(t-seconds).abs()<0.2)
            &&ui["ready"]==true&&ui["loading"]==false&&ui["gifts"]==gifts&&ui["errors"]==0)
    })?;
    Ok(json!({"requestedSeconds":seconds,"mediaSeconds":clock(app,token)?.fields.current_time,"ui":evaluate(main,DOM)?}))
}
fn exercise(app:&tauri::AppHandle,state:&QaState,output:&Path)->Result<Value,String>{
    let main=app.get_webview("main").ok_or("QA_MAIN")?;
    let ready=(Instant::now()+Duration::from_secs(60)).min(state.deadline);
    wait_until(ready,||Ok(evaluate(&main,DOM)?["ready"]==true&&!state.video_tokens.lock().unwrap().is_empty()&&!state.leases.lock().unwrap().is_empty()))?;
    let token=state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_VIDEO_TOKEN")?;
    let lease=state.leases.lock().unwrap().last().cloned().ok_or("QA_LEASE")?;
    wait_until(ready,||Ok(clock(app,&token).is_ok_and(|c|c.fields.video_id==state.job.video_id.as_deref().unwrap_or("")&&c.fields.duration.is_some_and(|d|(d-108.229).abs()<1.0))))?;
    let remote=app.webviews().into_values().find(|v|v.label().starts_with("youtube-original-vod-")).ok_or("QA_REMOTE_VIEW")?;
    let first=clock(app,&token)?;let mut progressed=false;
    wait_until(ready,||{let c=clock(app,&token)?;progressed=c.fields.current_time.zip(first.fields.current_time).is_some_and(|(a,b)|a>b+0.7);Ok(progressed&&evaluate(&remote,VIDEO)?["frames"].as_u64().is_some_and(|n|n>0))})?;
    control(app,&token,"pause",None)?;
    wait_until(ready,||Ok(clock(app,&token).is_ok_and(|v|v.fields.paused)))?;
    let mut samples=Vec::new();
    for (seconds,gifts) in [(0.0,0),(4.5,0),(5.5,1),(12.5,1),(13.5,2),(41.3,2),(42.3,3),(4.5,0),(42.3,3)]{
        samples.push(gift_seek(app,state,&main,&token,seconds,gifts)?);
    }
    wait_until((Instant::now()+Duration::from_secs(8)).min(state.deadline),||{let ui=evaluate(&main,DOM)?;Ok(ui["gifts"]==3&&ui["customImages"].as_u64().is_some_and(|n|n>0)&&ui["customImages"]==ui["customImagesLoaded"])})?;
    let presentation=evaluate(&main,DOM)?;
    if presentation["partialNotice"]!=true||presentation["overflow"]!=false||presentation["hasEnabledSend"]!=false||presentation["chatWithinViewport"]!=true{return Err("QA_GIFT_PRESENTATION".into());}
    let pause_before=clock(app,&token)?;thread::sleep(Duration::from_millis(800));
    let pause_after=clock(app,&token)?;
    let drift=(pause_after.fields.current_time.unwrap_or(-1.0)-pause_before.fields.current_time.unwrap_or(-2.0)).abs();
    if drift>0.15||evaluate(&main,DOM)?["gifts"]!=3{return Err("QA_GIFT_PAUSE_DRIFT".into());}
    capture(&main,&output.join("gift-native-chat.png"))?;capture(&remote,&output.join("gift-native-video.png"))?;
    // A real advancing clock crosses the first public event, without assigning UI time.
    gift_seek(app,state,&main,&token,4.0,0)?;
    let before_frames=evaluate(&remote,VIDEO)?["frames"].as_u64().unwrap_or(0);
    control(app,&token,"resume",None)?;
    wait_until((Instant::now()+Duration::from_secs(8)).min(state.deadline),||{let c=clock(app,&token)?;let ui=evaluate(&main,DOM)?;Ok(!c.fields.paused&&c.fields.current_time.is_some_and(|t|t>5.2&&t<9.0)&&ui["gifts"]==1&&evaluate(&remote,VIDEO)?["frames"].as_u64().is_some_and(|n|n>before_frames))})?;
    control(app,&token,"pause",None)?;
    if state.leases.lock().unwrap().as_slice()!=[lease.clone()]{return Err("QA_GIFT_LEASE_CHANGED".into());}
    evaluate(&main,"(()=>{window.__atsumiActualGiftQa.close();return true})()")?;
    wait_until((Instant::now()+Duration::from_secs(8)).min(state.deadline),||Ok(state.leases.lock().unwrap().is_empty()&&evaluate(&main,DOM)?["dialog"]==false))?;
    if state.media.cloud_chat_page(&lease,0,None,1).is_ok(){return Err("QA_GIFT_REVOKE".into());}
    evaluate(&main,"(()=>{window.__atsumiActualGiftQa.reopen();return true})()")?;
    wait_until((Instant::now()+Duration::from_secs(15)).min(state.deadline),||Ok(state.video_tokens.lock().unwrap().last().is_some_and(|t|t!=&token)&&!state.leases.lock().unwrap().is_empty()))?;
    let next=state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_GIFT_REOPEN")?;
    wait_until((Instant::now()+Duration::from_secs(15)).min(state.deadline),||Ok(clock(app,&next).is_ok()))?;
    control(app,&next,"pause",None)?;
    wait_until((Instant::now()+Duration::from_secs(5)).min(state.deadline),||Ok(clock(app,&next).is_ok_and(|c|c.fields.paused)))?;
    samples.push(gift_seek(app,state,&main,&next,42.3,3)?);
    wait_until((Instant::now()+Duration::from_secs(8)).min(state.deadline),||{let ui=evaluate(&main,DOM)?;Ok(ui["customImages"].as_u64().is_some_and(|n|n>0)&&ui["customImages"]==ui["customImagesLoaded"]&&ui["partialNotice"]==true)})?;
    let reopened=evaluate(&main,DOM)?;
    evaluate(&main,"(()=>{window.__atsumiActualGiftQa.close();return true})()")?;
    wait_until((Instant::now()+Duration::from_secs(5)).min(state.deadline),||Ok(state.leases.lock().unwrap().is_empty()))?;
    Ok(json!({"passed":true,"actualReceiptAndRemoteVideo":true,"realGiftEvents":3,"giftTimesMs":[4995,12995,41805],"boundarySamples":samples,"presentation":presentation,"reopened":reopened,"naturalFirstGiftCrossing":true,"pauseDriftSeconds":drift,"closeRevokesSidecar":true,"localCapturedAssetDecoded":true,"partialCaptureNotice":true,"timelineQuality":"receive-time-approximate","framePerfectSyncVerified":false,"audioAudibilityVerified":false,"localVideoBytesRead":0,"newUpload":false,"desktopInput":false,"fullClipPlayed":false}))
}

#[test]
#[ignore = "Actual native isolated unlisted SOOP gift replay; explicit scoped environment required"]
fn actual_soop_gift_native_remote_video_chat_and_capture() {
    assert_eq!(std::env::var("ATSUMI_SOOP_NATIVE_GIFT_REPLAY_QA").as_deref(), Ok("actual-unlisted-gift-readonly"), "QA_OPT_IN");
    let host = ActualReceiptHost::open_current_gift();
    let authorization = host.authorization(); let job = host.job_view();
    let catalog = host.catalog_path();
    let raw = host.read(&catalog, 16 * 1024); assert_eq!(actual::digest(&raw), GIFT_CATALOG_SHA, "QA_CATALOG");
    let source_job: MediaJob = serde_json::from_slice(&raw).unwrap();
    assert_eq!(fs::canonicalize(&source_job.download_root).unwrap(),fs::canonicalize(host.capture().join("media")).unwrap(),"QA_GIFT_SOURCE_ROOT");
    let source_value:Value=serde_json::from_slice(&raw).unwrap();
    assert_eq!(source_value["library"]["captureOutcome"]["state"],"interrupted","QA_GIFT_CAPTURE_STATE");
    assert!(authorization.description.coverage.capture_warning.is_some(),"QA_GIFT_CAPTURE_WARNING");
    let directory = storage::job_dir(&source_job.download_root, "soop", GIFT_RECORDING).unwrap();
    let before = gift_snapshot(&host, &directory);
    let video_before=fs::metadata(directory.join("media.mp4")).unwrap();
    assert_eq!(video_before.len(),16_150_161,"QA_GIFT_VIDEO_LENGTH");
    let media = no_workers(host.scratch(), source_job);
    media.validate_cloud_chat_source(&authorization).unwrap_or_else(|_| panic!("QA_SOURCE"));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let output = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(format!("actualgift-native-{run_id}"));
    fs::create_dir(&output).unwrap();
    let identity = format!("local.atsumi.cloudreplayqa.{run_id}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let url: tauri::Url = std::env::var("ATSUMI_SOOP_NATIVE_GIFT_REPLAY_QA_URL").unwrap_or_else(|_| format!("http://127.0.0.1:5181{ENTRY}")).parse().unwrap();
    assert!(url.scheme()=="http" && url.host_str()==Some("127.0.0.1") && url.port()==Some(5181) && url.path()==ENTRY
        && url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(), "QA_ENTRY");
    let state = Arc::new(QaState {media:media.clone(),authorization,job,leases:Mutex::new(Vec::new()),video_tokens:Mutex::new(Vec::new()),deadline:Instant::now()+Duration::from_secs(150),closed:AtomicBool::new(false)});
    let outcome = Arc::new(Mutex::new(None::<Result<Value,String>>));
    let outcome_worker = outcome.clone(); let worker_state = state.clone(); let output_worker = output.clone(); let profile_check = profile.clone();
    let old_args = std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--mute-audio");
    let mut context = tauri::generate_context!();
    context.config_mut().identifier=identity;
    context.config_mut().build.dev_url=Some(url.clone());
    context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app = tauri::Builder::default().any_thread().manage(state.clone()).manage(YouTubeOriginalHost::default())
        .invoke_handler(tauri::generate_handler![qa_actualgift_bootstrap,qa_actualgift_describe,qa_actualgift_open,qa_actualgift_page,qa_actualgift_asset,qa_actualgift_close,youtube_original_open,
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
    assert_eq!(gift_snapshot(&host,&directory),before,"QA_SOURCE_SIDECARS_CHANGED");
    let video_after=fs::metadata(directory.join("media.mp4")).unwrap();
    assert!(video_after.len()==video_before.len()&&video_after.modified().unwrap()==video_before.modified().unwrap(),"QA_GIFT_VIDEO_METADATA_CHANGED");
    assert_eq!(actual::digest(&host.read(&catalog,16*1024)),GIFT_CATALOG_SHA,"QA_CATALOG_CHANGED");
    assert!(media.inner.playback.lock().unwrap().is_empty() && media.inner.cloud_chat.lock().unwrap().is_empty() && media.inner.upload_leases.lock().unwrap().is_empty(),"QA_LEASE_REMAINS");
    { let saved=media.inner.state.lock().unwrap(); assert!(saved.queue.is_empty()&&saved.cancel.is_empty()&&!saved.shutting_down,"QA_WORK_STARTED"); }
    host.assert_unchanged(); drop(state); drop(media); let read_bytes=host.finish();
    // Only the fresh UUID profile may be cleaned; a busy WebView profile is retained.
    let cleaned = cleanup_profile(&profile,&format!("local.atsumi.cloudreplayqa.{run_id}"));
    let result=outcome.lock().unwrap().take().unwrap_or_else(||Err("QA_NO_RESULT".into()));
    let report=match &result {Ok(value)=>json!({"result":value,"originalSmallFileHashesUnchanged":true,"originalVideoMetadataUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes}),Err(code)=>json!({"passed":false,"errorCode":code,"originalSmallFileHashesUnchanged":true,"originalVideoMetadataUnchanged":true,"freshProfileCleaned":cleaned,"sourceAndScratchReadBytes":read_bytes})};
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

