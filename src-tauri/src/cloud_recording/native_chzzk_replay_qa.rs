//! Opt-in actual CHZZK part11 remote video/donation replay. Test-only child module.
//! Pre-edit: actual60-native-replay-pre-edit.json. No upload, recovery or original video read.
use super::*;
use crate::cloud_recording::{CloudJobSnapshot, replay::{CloudReplayAuthorization, CloudReplayOpen}};
use crate::streaming::replay::{self as chat_replay, ReplayService};
use crate::youtube::original::{OriginalSnapshot, VodClock, VodControlRequest, YouTubeOriginalHost};
use serde_json::{json, Value};
use std::{fs, io::Write, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
use tauri::{Manager, State, Webview};
use tauri::http::{Method, Request, StatusCode};

const ENTRY: &str = "/.runtime/requirements-audit/20261117-cloud-goal-resume/actual60-native-replay.html";
struct QaState {
    replay: ReplayService,
    authorization: CloudReplayAuthorization,
    job: CloudJobSnapshot,
    leases: Mutex<Vec<String>>,
    video_tokens: Mutex<Vec<String>>,
    deadline: Instant,
    closed: AtomicBool,
}
fn guard(window: &Webview, state: &QaState) -> Checked<tauri::AppHandle> {
    let app=window.app_handle(); let url=window.url().map_err(|_|"QA_WINDOW")?;
    if state.closed.load(Ordering::Acquire)||Instant::now()>=state.deadline||url.path()!=ENTRY
        || !crate::frontend_origin::trusted_main(window.label(),&url,true,app.config().build.dev_url.as_ref()) {return Err("QA_SCOPE".into());}
    Ok(app.clone())
}
fn job_allowed(state:&QaState,id:&str)->Checked<()> {if id==state.job.id{Ok(())}else{Err("QA_JOB".into())}}
#[tauri::command]
fn qa_actual60_bootstrap(window:Webview,state:State<'_,Arc<QaState>>)->Checked<Value> {
    guard(&window,&state)?;Ok(json!({"job":state.job}))
}
#[tauri::command]
fn qa_actual60_describe(window:Webview,state:State<'_,Arc<QaState>>,job_id:String)->Checked<source::SourceDescription> {
    guard(&window,&state)?;job_allowed(&state,&job_id)?;Ok(state.authorization.description.clone())
}
fn media_denied(service:&ReplayService,token:&str)->Checked<()> {
    for method in [Method::GET,Method::HEAD] {for suffix in ["","/part/0"] {
        let request=Request::builder().method(method.clone()).uri(format!("http://atsumi-replay.localhost/{token}{suffix}"))
            .header("Range","bytes=0-15").body(vec![]).map_err(|_|"QA_PROTOCOL_REQUEST")?;
        if service.media_response(&request,"main").status()!=StatusCode::FORBIDDEN{return Err("QA_LOCAL_VIDEO_ALLOWED".into());}
    }}Ok(())
}
#[tauri::command]
fn qa_actual60_open(window:Webview,state:State<'_,Arc<QaState>>,job_id:String)->Checked<CloudReplayOpen> {
    guard(&window,&state)?;job_allowed(&state,&job_id)?;
    let mut leases=state.leases.lock().map_err(|_|"QA_LOCK")?;
    if leases.len()>=2{return Err("QA_LEASE_LIMIT".into());}
    let session=state.replay.open_cloud_chat(&state.authorization).map_err(|_|"QA_SIDECAR_OPEN")?;
    let valid=session.mime_type=="application/x-atsumi-remote-chat"&&session.parts.is_empty()
        && !session.warnings.is_empty()&&(session.duration_seconds-(OFFSET+DURATION)).abs()<0.000001;
    if !valid||media_denied(&state.replay,&session.token).is_err(){let _=state.replay.close(&session.token);return Err("QA_SIDECAR_DESCRIPTOR".into());}
    leases.push(session.token.clone());
    Ok(CloudReplayOpen{provider:"chzzk".into(),token:session.token.clone(),cohort_id:state.authorization.cohort_id.clone(),
        description:state.authorization.description.clone(),session:Some(session)})
}
#[tauri::command]
fn qa_actual60_close(window:Webview,state:State<'_,Arc<QaState>>,provider:String,token:String)->Checked<()> {
    guard(&window,&state)?;if provider!="chzzk"{return Err("QA_PROVIDER".into());}
    let mut leases=state.leases.lock().map_err(|_|"QA_LOCK")?;
    if !leases.iter().any(|v|v==&token){return Err("QA_LEASE".into());}
    state.replay.close(&token).map_err(|_|"QA_CLOSE")?;leases.retain(|v|v!=&token);Ok(())
}
// Only the fixed-video admission wrapper differs; actual native receiver/clock/controls are production.
#[tauri::command]
async fn youtube_original_open(window:Webview,state:State<'_,Arc<QaState>>,channel_id:String,request_id:String,playback_kind:Option<String>)->Checked<OriginalSnapshot> {
    let app=guard(&window,&state)?;
    if playback_kind.as_deref()!=Some("vod")||state.job.video_id.as_deref()!=Some(channel_id.as_str()){return Err("QA_VIDEO".into());}
    let owned=state.inner().clone();
    tauri::async_runtime::spawn_blocking(move||{
        let value=app.state::<YouTubeOriginalHost>().open_vod(&app,channel_id,request_id)?;
        owned.video_tokens.lock().unwrap().push(value.token.clone());Ok(value)
    }).await.map_err(|_|"QA_OPEN_WORKER".to_owned())?
}

fn evaluate(view: &Webview, script: &str) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script, move |raw| { let _ = tx.try_send((raw.len() <= 8192).then_some(raw)); }).map_err(|_| "QA_EVAL")?;
    let raw = rx.recv_timeout(Duration::from_secs(3)).map_err(|_| "QA_EVAL_TIMEOUT")?.ok_or("QA_EVAL_BOUND")?;
    serde_json::from_str(&raw).map_err(|_| "QA_EVAL_JSON".into())
}
const DOM: &str = r#"(()=>{const q=window.__atsumiActual60Qa,h=document.querySelector('aside.recording-replay-chat'),s=h?.shadowRoot,rows=[...(s?.querySelectorAll('.recording-replay-message')??[])],donations=rows.filter(n=>n.dataset.notice==='donation'),area=s?.querySelector('.recording-replay-chat-scroll'),busy=area?.getAttribute('aria-busy'),indexBuilding=!!s?.querySelector('.recording-replay-chat-status[role="status"]'),visible=n=>{const a=n.getBoundingClientRect(),b=area?.getBoundingClientRect();return !!b&&a.width>0&&a.height>0&&a.bottom>b.top&&a.top<b.bottom&&getComputedStyle(n).display!=='none'};return {ready:q?.ready===true,dialog:!!document.querySelector('.cloud-recording-replay'),shadow:!!s,chatBusy:busy??null,indexBuilding,chatReady:busy==='false'&&!indexBuilding,rows:rows.length,donations:donations.length,visibleDonations:donations.filter(visible).length,donationHasContent:donations.every(n=>(n.textContent?.trim().length??0)>0),overflow:!!area&&area.scrollWidth>area.clientWidth+1,warningVisible:!!s?.querySelector('.recording-replay-warning'),alerts:document.querySelectorAll('[role="alert"]').length+(s?.querySelectorAll('[role="alert"]').length??0)}})()"#;
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

fn seek_and_observe(app:&tauri::AppHandle,token:&str,main:&Webview,deadline:Instant,seconds:f64,count:u64)->Checked<Value> {
    let before=clock(app,token)?;
    control(app,token,"seek",Some(seconds))?;
    wait_until(deadline,||{
        let observed=clock(app,token).is_ok_and(|c|c.document_epoch==before.document_epoch
            &&c.fields.source_id==before.fields.source_id&&c.fields.source_generation==before.fields.source_generation
            &&c.fields.sequence>before.fields.sequence&&c.fields.paused&&c.fields.current_time.is_some_and(|t|(t-seconds).abs()<0.35));
        if !observed{return Ok(false);}
        let dom=evaluate(main,DOM)?;
        Ok(dom["shadow"]==true&&dom["chatReady"]==true&&dom["donations"].as_u64()==Some(count)&&dom["alerts"]==0
            &&(count==0||dom["visibleDonations"].as_u64()==Some(count)))
    })?;evaluate(main,DOM)
}
fn exercise(app:&tauri::AppHandle,state:&QaState,output:&Path)->Checked<Value> {
    let main=app.get_webview("main").ok_or("QA_MAIN")?;
    let ready=(Instant::now()+Duration::from_secs(75)).min(state.deadline);
    wait_until(ready,||Ok(evaluate(&main,DOM)?["ready"]==true&&!state.video_tokens.lock().unwrap().is_empty()))?;
    let token=state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_VIDEO_TOKEN")?;
    let mut first=None;let mut progressed=false;
    wait_until(ready,||{
        if let Ok(c)=clock(app,&token){
            if c.fields.video_id!=state.job.video_id.as_deref().unwrap_or("")||c.fields.duration.is_none_or(|v|(v-DURATION).abs()>1.0){return Err("QA_REMOTE_IDENTITY".into());}
            if let Some(t)=c.fields.current_time{let start=*first.get_or_insert(t);progressed|=t>start+0.7;}
        }Ok(progressed)
    })?;
    let remote=app.webviews().into_values().find(|v|v.label().starts_with("youtube-original-vod-")).ok_or("QA_REMOTE_VIEW")?;
    let video=evaluate(&remote,VIDEO)?;
    if video["frames"].as_u64().unwrap_or(0)==0||video["ad"]!=false||video["capture"]!=false||video["chat"]!=false{return Err("QA_REMOTE_DECODE".into());}
    control(app,&token,"pause",None)?;
    let actions=(Instant::now()+Duration::from_secs(60)).min(state.deadline);
    wait_until(actions,||Ok(clock(app,&token).is_ok_and(|v|v.fields.paused)))?;
    let paused=clock(app,&token)?;let before=paused.fields.current_time.ok_or("QA_PAUSE_TIME")?;
    thread::sleep(Duration::from_millis(800));
    wait_until(actions,||Ok(clock(app,&token).is_ok_and(|c|c.document_epoch==paused.document_epoch&&c.fields.source_id==paused.fields.source_id
        &&c.fields.source_generation==paused.fields.source_generation&&c.fields.sequence>paused.fields.sequence&&c.fields.paused)))?;
    let after=clock(app,&token)?.fields.current_time.ok_or("QA_PAUSE_TIME")?;
    if (after-before).abs()>0.15{return Err("QA_PAUSE_DRIFT".into());}
    // Only the React renderer owns replay generations; these controls never call chat_at.
    let before_event=seek_and_observe(app,&token,&main,actions,17.5,0)?;
    let after_event=seek_and_observe(app,&token,&main,actions,19.5,1)?;
    let backward=seek_and_observe(app,&token,&main,actions,17.5,0)?;
    let forward=seek_and_observe(app,&token,&main,actions,19.5,1)?;
    if forward["overflow"]!=false||forward["donationHasContent"]!=true{return Err("QA_DONATION_PRESENTATION".into());}
    capture(&main,&output.join("actual60-native-chat.png"))?;capture(&remote,&output.join("actual60-native-video.png"))?;
    let old_leases=state.leases.lock().unwrap().clone();
    evaluate(&main,"(()=>{window.__atsumiActual60Qa.close();return true})()")?;
    wait_until(actions,||Ok(state.leases.lock().unwrap().is_empty()&&evaluate(&main,DOM)?["dialog"]==false))?;
    if old_leases.iter().any(|token|state.replay.timeline(token,None).is_ok()){return Err("QA_REVOKE".into());}
    evaluate(&main,"(()=>{window.__atsumiActual60Qa.reopen();return true})()")?;
    wait_until(actions,||Ok(state.video_tokens.lock().unwrap().last().is_some_and(|v|v!=&token)&&!state.leases.lock().unwrap().is_empty()))?;
    let reopened=state.video_tokens.lock().unwrap().last().cloned().ok_or("QA_REOPEN")?;
    wait_until(actions,||Ok(clock(app,&reopened).is_ok()))?;control(app,&reopened,"pause",None)?;
    wait_until(actions,||Ok(clock(app,&reopened).is_ok_and(|c|c.fields.paused)))?;
    let reopened_dom=seek_and_observe(app,&reopened,&main,actions,19.5,1)?;
    evaluate(&main,"(()=>{window.__atsumiActual60Qa.close();return true})()")?;
    wait_until(actions,||Ok(state.leases.lock().unwrap().is_empty()))?;
    Ok(json!({"passed":true,"actualRemoteVideoProgressed":progressed,"decodedFramesObserved":video["frames"],
        "actualNativeClock":true,"actualNativeProductionReact":true,"pauseDriftSeconds":(after-before).abs(),
        "beforeEvent":before_event,"afterEvent":after_event,"backward":backward,"forward":forward,"reopened":reopened_dom,
        "donationPartSecondsFromPinnedMapper":DONATION,"observedSeekSeconds":[17.5,19.5],"actualDonationSeekVerified":true,
        "closeRevokesSidecar":true,"reopen":true,"sourceOffsetSeconds":OFFSET,"parentComplete":false,
        "localVideoBytesRead":0,"desktopInput":false,"audioAudibilityVerified":false}))
}
struct StopReplay(ReplayService);
impl Drop for StopReplay{fn drop(&mut self){self.0.shutdown_and_wait();}}
struct BrowserArgs(Option<std::ffi::OsString>);
impl Drop for BrowserArgs{fn drop(&mut self){if let Some(v)=self.0.take(){std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",v)}else{std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")}}}

#[test]
#[ignore="Actual ready unlisted CHZZK part11 native replay; explicit scoped environment required"]
fn actual_chzzk60_native_remote_video_donation_and_capture()->Checked<()> {
    let receipt=load_ready_receipt()?;let originals=Originals::open()?;
    let run_id=uuid::Uuid::new_v4().simple().to_string();
    let output=workspace().join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(format!("actual60-native-{run_id}"));
    fs::create_dir(&output).map_err(|_|"QA_OUTPUT")?;
    let scratch=tempfile::Builder::new().prefix("atsumi-chzzk-native-").tempdir().map_err(|_|"QA_SCRATCH")?;
    fs::create_dir_all(scratch.path().join("streaming/browser")).map_err(|_|"QA_SCRATCH")?;
    fs::write(scratch.path().join("streaming/browser/catalog.jsonl"),originals.selected_catalog_line()?).map_err(|_|"QA_CATALOG_COPY")?;
    let url:tauri::Url=std::env::var("ATSUMI_CHZZK_NATIVE_REPLAY_QA_URL").unwrap_or_else(|_|format!("http://127.0.0.1:5181{ENTRY}")).parse().map_err(|_|"QA_ENTRY")?;
    if url.scheme()!="http"||url.host_str()!=Some("127.0.0.1")||url.port()!=Some(5181)||url.path()!=ENTRY
        ||!url.username().is_empty()||url.password().is_some()||url.query().is_some()||url.fragment().is_some(){return Err("QA_ENTRY".into());}
    let replay=ReplayService::new(scratch.path(),Arc::new(Mutex::new(BrowserCaptureStore::new_deferred(scratch.path()).map_err(|_|"QA_DEFERRED_STORE")?))).with_development_url(Some(url.clone()));
    let stop=StopReplay(replay.clone());
    let identity=format!("local.atsumi.chzzkreplayqa.{run_id}");
    let profile=PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(&identity);
    if profile.exists(){return Err("QA_PROFILE_EXISTS".into());}
    let state=Arc::new(QaState{replay:replay.clone(),authorization:receipt.authorization.clone(),job:receipt.job.clone(),
        leases:Mutex::new(vec![]),video_tokens:Mutex::new(vec![]),deadline:Instant::now()+Duration::from_secs(150),closed:AtomicBool::new(false)});
    let outcome=Arc::new(Mutex::new(None::<Checked<Value>>));
    let worker=Arc::new(Mutex::new(None));
    let result:Checked<Value>=(||{
        let _args=BrowserArgs(std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"));
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS","--mute-audio");
        let mut context=tauri::generate_context!();context.config_mut().identifier=identity.clone();context.config_mut().build.dev_url=Some(url.clone());
        context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
        let worker_state=state.clone();let output_worker=output.clone();let outcome_worker=outcome.clone();let worker_slot=worker.clone();let profile_check=profile.clone();
        let app=tauri::Builder::default().any_thread().manage(state.clone()).manage(replay.clone()).manage(YouTubeOriginalHost::default())
            .register_asynchronous_uri_scheme_protocol("atsumi-replay",|context,request,responder|{
                let label=context.webview_label().to_owned();
                if label!="main"{responder.respond(crate::replay_protocol_failure(StatusCode::FORBIDDEN));return;}
                let Some(service)=context.app_handle().try_state::<ReplayService>().map(|v|v.inner().clone())else{responder.respond(crate::replay_protocol_failure(StatusCode::NOT_FOUND));return;};
                let Some(permit)=crate::ReplayIoPermit::acquire()else{responder.respond(crate::replay_protocol_failure(StatusCode::SERVICE_UNAVAILABLE));return;};
                tauri::async_runtime::spawn_blocking(move||{let _permit=permit;responder.respond(service.media_response(&request,&label));});
            })
            .invoke_handler(tauri::generate_handler![qa_actual60_bootstrap,qa_actual60_describe,qa_actual60_open,qa_actual60_close,youtube_original_open,
                chat_replay::replay_chat_at,chat_replay::replay_chat_page,chat_replay::replay_chat_search,chat_replay::replay_timeline,
                crate::youtube::original_commands::youtube_original_snapshot,crate::youtube::original_commands::youtube_original_vod_clock,crate::youtube::original_commands::youtube_original_vod_control,
                crate::youtube::original_commands::youtube_original_viewport,crate::youtube::original_commands::youtube_original_close,crate::youtube::original_commands::youtube_original_controls,crate::youtube::original_commands::youtube_original_fullscreen])
            .setup(move|app|{
                if app.path().app_data_dir()?!=profile_check{return Err("QA_PROFILE_PATH".into());}
                tauri::WebviewWindowBuilder::new(app,"main",tauri::WebviewUrl::External(url)).data_directory(profile_check.join("main-webview")).title("Atsumi CHZZK native replay verification")
                    .inner_size(1280.,760.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
                let handle=app.handle().clone();
                *worker_slot.lock().unwrap()=Some(thread::spawn(move||{
                    let value=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||exercise(&handle,&worker_state,&output_worker))).unwrap_or_else(|_|Err("QA_WORKER_PANIC".into()));
                    if value.is_err(){
                        let main=handle.get_webview("main");let remote=handle.webviews().into_values().find(|v|v.label().starts_with("youtube-original-vod-"));
                        let facts=json!({"main":main.as_ref().and_then(|v|evaluate(v,DOM).ok()),"video":remote.as_ref().and_then(|v|evaluate(v,VIDEO).ok())});
                        let _=fs::write(output_worker.join("failure-facts.json"),serde_json::to_vec_pretty(&facts).unwrap());
                        if let Some(v)=main{let _=capture(&v,&output_worker.join("failure-main.png"));}if let Some(v)=remote{let _=capture(&v,&output_worker.join("failure-video.png"));}
                    }
                    worker_state.closed.store(true,Ordering::Release);
                    for token in worker_state.leases.lock().unwrap().drain(..){let _=worker_state.replay.close(&token);}
                    let receiver=handle.state::<YouTubeOriginalHost>();for token in worker_state.video_tokens.lock().unwrap().drain(..){let _=receiver.close(&handle,&token);}
                    receiver.shutdown(&handle);worker_state.replay.shutdown_and_wait();
                    *outcome_worker.lock().unwrap()=Some(value);handle.exit(0);
                }));Ok(())
            }).build(context).map_err(|_|"QA_NATIVE_BUILD")?;
        app.run_return(|_,_|{});
        if let Some(handle)=worker.lock().unwrap().take(){handle.join().map_err(|_|"QA_WORKER_JOIN")?;}
        outcome.lock().unwrap().take().unwrap_or_else(||Err("QA_NO_RESULT".into()))
    })();
    drop(stop);
    let originals_check=originals.assert_unchanged();let receipt_check=receipt.assert_unchanged();
    let lease_empty=state.leases.lock().unwrap().is_empty();drop(state);drop(replay);
    let receipt_cleanup=receipt.finish();let scratch_cleanup=scratch.close().map_err(|_|"QA_SCRATCH_CLEANUP");
    let cleaned=cleanup_profile(&profile,&identity);
    let report=json!({"result":result.as_ref().ok(),"errorCode":result.as_ref().err(),"sourceHashesUnchanged":originals_check.is_ok(),
        "encryptedReceiptUnchanged":receipt_check.is_ok(),"receiptScratchCleaned":receipt_cleanup.is_ok(),"replayScratchCleaned":scratch_cleanup.is_ok(),
        "freshProfileCleaned":cleaned,"sidecarLeasesEmpty":lease_empty,"sourceMediaBytesRead":0,"newUpload":false,
        "scope":"Actual fixed receipt + production native remote video/clock + original CHZZK chat/donation. Muted isolated test shell; incomplete parent unchanged."});
    fs::write(output.join("result.json"),serde_json::to_vec_pretty(&report).map_err(|_|"QA_RESULT_JSON")?).map_err(|_|"QA_RESULT_WRITE")?;
    println!("QA_NATIVE_RESULT={}",output.join("result.json").display());
    originals_check?;receipt_check?;receipt_cleanup?;scratch_cleanup?;
    if !lease_empty{return Err("QA_LEASE_REMAINS".into());}result.map(|_|())
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
        || !identity.starts_with("local.atsumi.chzzkreplayqa.") {return false;}
    for path in profile.ancestors() {
        let Ok(meta)=fs::symlink_metadata(path) else{return false;};
        if meta.file_type().is_symlink() || meta.file_attributes()&0x400!=0 {return false;}
    }
    fs::remove_dir_all(&resolved).is_ok()
}
