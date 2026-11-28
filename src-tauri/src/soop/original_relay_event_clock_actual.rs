//! One anonymous public SOOP source -> memory-only localhost HLS -> sealed history.
//! No record(), remote destination, account action or desktop input.
use super::*;
use crate::cloud_recording::live_relay::{NativeRelaySource, SelectionPhase};

fn source_facts(app: &AppHandle) -> Result<Value, String> {
    let views: Vec<_> = app.webviews().into_values().filter(|v|v.label().starts_with("soop-original-")).collect();
    require(views.len()==1,"qa_source_view_count")?;
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    views[0].eval_with_callback(r#"(()=>{const p=window.livePlayer,v=document.getElementById('livePlayer'),r=window.__atsumiSoopEncodedCapture?.getRelayStatus?.();
      const visible=e=>{if(!e)return false;const r=e.getBoundingClientRect(),s=getComputedStyle(e);return r.width>0&&r.height>0&&s.display!=='none'&&s.visibility!=='hidden'};
      let ad=null,main=null,live=null,m=null;try{const a=p?.externalInterface?.getAdInfo?.();if(typeof a?.isAdViewing==='boolean')ad=a.isAdViewing;
      if(typeof p?.getMainMedia==='function'){m=p.getMainMedia();main=m===v;}if(typeof p?.getLiveMainMedia==='function')live=p.getLiveMainMedia()===v;}catch{}
      const media=m instanceof HTMLMediaElement?m:null,q=window.__atsumiQaMainMediaPreparation;
      const mainMedia=media?{tagName:['VIDEO','AUDIO'].includes(media.tagName)?media.tagName:'OTHER',
        id:media.id==='livePlayer'?'livePlayer':media.id?'other':'empty',paused:media.paused,ready:media.readyState,
        currentTime:Number.isFinite(media.currentTime)?media.currentTime:null,ended:media.ended,
        error:Number.isInteger(media.error?.code)?media.error.code:null,visible:visible(media)}:null;
      const c=r?.runId?window.__atsumiSoopEncodedCapture?.getRelayClock?.(r.runId):null;
      return {ready:v?.readyState??null,paused:v?.paused??null,seeking:v?.seeking??null,time:v?.currentTime??null,
      muted:v?.muted??null,volume:v?.volume??null,frames:v?.getVideoPlaybackQuality?.().totalVideoFrames??null,
      videoVisible:visible(v),modalVisible:visible(document.getElementById('layer_high_quality')),
      adViewing:ad,mainMatches:main,liveMatches:live,mainMedia,
      mainPlayAttempts:Number.isInteger(q?.attempts)?q.attempts:0,
      mainPlayState:['pending','fulfilled','rejected','threw'].includes(q?.outcome)?q.outcome:null,chatSocketPresent:!!p?.chatSocket,
      observerInstalled:typeof window.__atsumiSoopChatClockObserver?.create==='function',
      sourceClockPresent:!!c?.sample,sourceClockPaused:c?.sample?.paused??null,sourceClockSeeking:c?.sample?.seeking??null,
      relayActive:r?.active===true,relayApproved:r?.nativeApproved===true};})()"#,
        move |value|{let _=tx.send(value);}).map_err(|_|"qa_source_dispatch")?;
    let text=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"qa_source_timeout")?;
    require(text.len()<=4096,"qa_source_facts_bound")?;
    serde_json::from_str(&text).map_err(|_|"qa_source_facts_shape".into())
}


const PREPARE_LOW_QUALITY: &str = r#"(()=>{
  const expected=__EXPECTED__,p=window.livePlayer,c=p?.liveAPI?.getResponse?.()?.CHANNEL;
  const actualPath='/' + expected.channel + '/' + expected.broadcast + '/embed';
  if(location.origin!=='https://play.sooplive.com'||location.pathname.replace(/\/$/,'')!==actualPath
    ||String(c?.BJID??'').toLowerCase()!==expected.channel||String(c?.BNO??'')!==expected.broadcast)
    return {clicked:false,reason:'source_changed'};
  const visible=e=>{if(!e)return false;const r=e.getBoundingClientRect(),s=getComputedStyle(e);
    return r.width>0&&r.height>0&&s.display!=='none'&&s.visibility!=='hidden'&&Number(s.opacity)!==0};
  const modal=document.getElementById('layer_high_quality');
  if(!visible(modal))return {clicked:false,reason:'modal_absent'};
  const buttons=Array.from(modal.querySelectorAll('button._btn_line'));
  if(buttons.length!==1)return {clicked:false,reason:'unknown_official_button'};
  const button=buttons[0],label=String(button.textContent??'').replace(/\s+/g,' ').trim();
  if(!(button instanceof HTMLButtonElement)||button.type!=='button'||button.disabled
    ||button.getAttribute('aria-disabled')==='true'||button.id==='install_streamer'||!visible(button)
    ||!['저화질 참여','Join in Low Quality'].includes(label))return {clicked:false,reason:'unknown_official_button'};
  button.click();
  return {clicked:true,reason:'official_low_quality_selected'};
})()"#;
fn choose_official_low_quality(app:&AppHandle,channel:&str,broadcast:&str)->Result<Value,String>{
    let views:Vec<_>=app.webviews().into_values().filter(|v|v.label().starts_with("soop-original-")).collect();
    require(views.len()==1,"qa_prepare_view_count")?;
    let script=PREPARE_LOW_QUALITY.replace("__EXPECTED__",&json!({"channel":channel,"broadcast":broadcast}).to_string());
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    views[0].eval_with_callback(&script,move|raw|{let _=tx.try_send(raw);}).map_err(|_|"qa_prepare_dispatch")?;
    let raw=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"qa_prepare_timeout")?;
    require(raw.len()<=1024,"qa_prepare_facts_bound")?;
    let result:Value=serde_json::from_str(&raw).map_err(|_|"qa_prepare_shape")?;
    require(result["clicked"].is_boolean()&&matches!(result["reason"].as_str(),Some("modal_absent"|"official_low_quality_selected"|"source_changed"|"unknown_official_button")),"qa_prepare_shape")?;
    Ok(result)
}
fn content_visible(facts:&Value)->bool{
    facts["adViewing"]==false&&facts["mainMatches"]==true&&facts["liveMatches"]==true
        &&facts["videoVisible"]==true&&facts["modalVisible"]==false
        &&facts["ready"].as_f64().is_some_and(|v|v>=3.)&&facts["seeking"]==false
}

const PREPARE_MAIN_MEDIA: &str = r#"(()=>{
  const expected=__EXPECTED__,p=window.livePlayer,initial=p?.liveAPI?.getResponse?.()?.CHANNEL;
  if(!p||typeof p.getMainMedia!=='function'||initial==null)
    return {attempted:false,reason:'source_unavailable'};
  if(typeof initial!=='object'||Array.isArray(initial))return {attempted:false,reason:'source_changed'};
  if(initial.BJID==null||initial.BJID===''||initial.BNO==null||initial.BNO==='')
    return {attempted:false,reason:'source_unavailable'};
  const bound=()=>{const c=p?.liveAPI?.getResponse?.()?.CHANNEL;return window.livePlayer===p
    &&location.origin==='https://play.sooplive.com'
    &&location.pathname.replace(/\/$/,'')==='/' + expected.channel + '/' + expected.broadcast + '/embed'
    &&typeof c?.BJID==='string'&&(typeof c?.BNO==='string'||(typeof c?.BNO==='number'&&Number.isSafeInteger(c.BNO)))
    &&c.BJID.toLowerCase()===expected.channel&&String(c.BNO)===expected.broadcast};
  if(!bound())return {attempted:false,reason:'source_changed'};
  const visible=e=>{const r=e.getBoundingClientRect(),s=getComputedStyle(e);return e.isConnected
    &&r.width>0&&r.height>0&&s.display!=='none'&&s.visibility!=='hidden'&&Number(s.opacity)!==0};
  const modal=document.getElementById('layer_high_quality');
  if(modal&&visible(modal))return {attempted:false,reason:'modal_visible'};
  const m=p?.getMainMedia?.();
  const eligible=()=>m instanceof HTMLMediaElement&&visible(m)&&m.paused&&!m.ended&&!m.seeking&&m.readyState>=2;
  if(!eligible())return {attempted:false,reason:'media_ineligible'};
  let q=window.__atsumiQaMainMediaPreparation;
  if(!q)q=window.__atsumiQaMainMediaPreparation={channel:expected.channel,broadcast:expected.broadcast,seen:new WeakSet(),attempts:0,outcome:null};
  if(q.channel!==expected.channel||q.broadcast!==expected.broadcast)return {attempted:false,reason:'source_changed'};
  if(q.seen.has(m))return {attempted:false,reason:'already_attempted'};
  if(q.attempts>=2)return {attempted:false,reason:'attempt_limit'};
  if(!bound()||p.getMainMedia()!==m)return {attempted:false,reason:'selection_changed'};
  m.muted=true;m.volume=0;
  if(!bound()||p.getMainMedia()!==m||!eligible())return {attempted:false,reason:'selection_changed'};
  q.seen.add(m);q.attempts++;q.outcome='pending';const attempt=q.attempts;
  try{Promise.resolve(m.play()).then(()=>{if(q.attempts===attempt)q.outcome='fulfilled'},()=>{if(q.attempts===attempt)q.outcome='rejected'})}
  catch{q.outcome='threw'}
  return {attempted:true,reason:'official_main_play_requested',attempt};
})()"#;
fn prepare_main_media(app:&AppHandle,channel:&str,broadcast:&str)->Result<Value,String>{
    let views:Vec<_>=app.webviews().into_values().filter(|v|v.label().starts_with("soop-original-")).collect();
    require(views.len()==1,"qa_main_view_count")?;
    let script=PREPARE_MAIN_MEDIA.replace("__EXPECTED__",&json!({"channel":channel,"broadcast":broadcast}).to_string());
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    views[0].eval_with_callback(&script,move|raw|{let _=tx.try_send(raw);}).map_err(|_|"qa_main_dispatch")?;
    let raw=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"qa_main_timeout")?;
    require(raw.len()<=1024,"qa_main_facts_bound")?;
    let result:Value=serde_json::from_str(&raw).map_err(|_|"qa_main_shape")?;
    require(result["attempted"].is_boolean()&&matches!(result["reason"].as_str(),Some("source_changed"|"source_unavailable"|"modal_visible"|"media_ineligible"|"already_attempted"|"attempt_limit"|"selection_changed"|"official_main_play_requested")),"qa_main_shape")?;
    Ok(result)
}

fn exercise_clock(app:&AppHandle,root:&Path,channel:&str,broadcast:&str,server:&server::Server,
    report:&mut Value,selected_slot:&Mutex<Option<Arc<LiveRelayCoordinator>>>) -> Result<(),String> {
    require(fs2::available_space(root).map_err(|_|"qa_disk_space")?>2*1024*1024*1024,"qa_disk_reserve")?;
    let startup=app.state::<Arc<crate::startup::Startup>>();
    crate::initialize_backend(app,&startup).map_err(|_|"qa_backend_init")?;
    let state=app.state::<AppState>();let media=state.platform_media(app)?;let host=app.state::<SoopOriginalHost>();
    host.set_tray_muted(app,true);
    let opened=host.open(app,media.clone(),channel.into(),uuid::Uuid::new_v4().to_string(),root.join("media"))
        .map_err(|_|"qa_public_source_open")?;
    require(opened.channel_id==channel&&opened.broadcast_id==broadcast,"qa_selected_broadcast_changed")?;
    let token=opened.token;
    host.viewport(app,&token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},
        "chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"qa_viewport_shape")?)?;
    let ready=Instant::now()+Duration::from_secs(90);
    let mut low_quality_attempted=false;
    report["mainPlaybackRequests"]=json!([]);
    let mut progressed_from=None::<(f64,f64)>;
    loop {
        let snapshot=host.snapshot(&token)?;
        require(snapshot.channel_id==channel&&snapshot.broadcast_id==broadcast,"qa_source_changed")?;
        let facts=source_facts(app)?;report["preparation"]=facts.clone();
        if facts["modalVisible"]==true&&!low_quality_attempted {
            low_quality_attempted=true;
            let chosen=choose_official_low_quality(app,channel,broadcast)?;
            report["officialLowQualitySelection"]=chosen.clone();
            require(chosen["clicked"]==true,"qa_official_low_quality_unconfirmed")?;
        }
        // Only the official player's currently selected media; allow ads to progress normally.
        if report["mainPlaybackRequests"].as_array().is_some_and(|r|r.len()<2) {
            let main=prepare_main_media(app,channel,broadcast)?;
            require(main["reason"]!="source_changed","qa_main_source_changed")?;
            if main["attempted"]==true {
                let requests=report["mainPlaybackRequests"].as_array_mut().ok_or("qa_main_requests")?;
                requests.push(json!({"request":main,"before":facts.clone()}));
            } else {report["lastMainPreparation"]=main;}
        }
        if snapshot.capture_ready&&snapshot.chat_visible&&content_visible(&facts) {
            report["quietPlaybackGuard"]=quiet_video(app,true)?;
            if facts["paused"]==false {
                if let Some((time,frames))=facts["time"].as_f64().zip(facts["frames"].as_f64()) {
                    if progressed_from.is_some_and(|(a,b)|time>a+0.7&&frames>b) {break;}
                    progressed_from.get_or_insert((time,frames));
                }
            } else {progressed_from=None;}
        } else {progressed_from=None;}
        require(Instant::now()<ready,"qa_official_content_not_ready")?;thread::sleep(Duration::from_millis(250));
    }
    host.set_tray_muted(app,false);
    report["visibleContentPrepared"]=json!(true);
    require(media.jobs(Some("soop")).is_empty()&&video_files(&root.join("media"))?==0,"qa_nonempty_baseline")?;
    let source=NativeRelaySource::Soop(host.select_relay_source(&token)?);
    let facts=source.facts();
    let selected=Arc::new(LiveRelayCoordinator::select_native(source.clone()));
    *lock(selected_slot)=Some(selected.clone());
    let owner=NativeDestinationOwner{channel_id:"UCabcdefghijklmnopqrstuv".into(),generation:1};
    selected.start(app,&host,&owner,PreparedLiveDestination::loopback_test(owner.clone(),server.target())).map_err(|_|"qa_relay_start")?;
    report["nativeSelectionStarted"]=serde_json::to_value(selected.status()).map_err(|_|"qa_selection_shape")?;
    report["samples"]=json!([]);let began=Instant::now();let mut next_sample=Instant::now();
    let first=source_facts(app)?;
    while began.elapsed()<Duration::from_secs(60) {
        require(source.current(app),"qa_native_source_changed")?;
        let snapshot=host.snapshot(&token)?;
        require(snapshot.channel_id==channel&&snapshot.broadcast_id==broadcast,"qa_broadcast_changed")?;
        let current=selected.status();
        require(current.fault.is_none()&&!current.manager_joined,"qa_relay_ended_early")?;
        if Instant::now()>=next_sample {
            let sample=source_facts(app)?;
            if !content_visible(&sample)||sample["paused"]!=false {report["visibleContentInterrupted"]=json!(true);}
            report["samples"].as_array_mut().ok_or("qa_samples")?.push(json!({"elapsedMs":began.elapsed().as_millis(),"facts":sample}));
            next_sample=Instant::now()+Duration::from_secs(10);
        }
        thread::sleep(Duration::from_millis(200));
    }
    let last=source_facts(app)?;
    report["observationMs"]=json!(began.elapsed().as_millis());
    report["sourceProgressed"]=json!(last["time"].as_f64().zip(first["time"].as_f64()).is_some_and(|(a,b)|a>b+30.)
        &&last["frames"].as_f64().zip(first["frames"].as_f64()).is_some_and(|(a,b)|a>b));
    report["lastSourceFacts"]=last;
    selected.stop(false);
    let deadline=Instant::now()+Duration::from_secs(180);
    let observed=loop {
        host.poll_relay_join(app);let observed=selected.status();
        if observed.manager_joined {break observed;}
        require(Instant::now()<deadline,"qa_native_join_deadline")?;thread::sleep(Duration::from_millis(50));
    };
    report["nativeSelectionJoined"]=serde_json::to_value(&observed).map_err(|_|"qa_join_shape")?;
    require(observed.state==SelectionPhase::Joined&&observed.fault.is_none()&&!observed.archive_ready,"qa_join_state")?;
    let relay=observed.relay.as_ref().ok_or("qa_join_relay")?;
    require(relay.worker_joined&&relay.manager_joined&&relay.sidecar.as_ref().is_some_and(|s|s.joined)
        &&relay.delivery.as_ref().is_some_and(|d|d.segment_acks>0&&d.end_list_acked),"qa_final_native_proof")?;
    report["http"]=server.snapshot();report["localVideoFiles"]=json!(video_files(&root.join("media"))?);
    report["localMediaJobs"]=json!(media.jobs(Some("soop")).len());
    let history=crate::cloud_recording::verify_actual_joined_history(&root.join("sealed-vault"),facts,&observed)
        .map_err(|error|{report["nativeHistoryErrorCode"]=json!(error.code);"qa_native_history_bridge"})?;
    report["history"]=serde_json::to_value(&history).map_err(|_|"qa_history_shape")?;
    report["http"]=server.snapshot();report["localVideoFiles"]=json!(video_files(&root.join("media"))?);
    report["localMediaJobs"]=json!(media.jobs(Some("soop")).len());
    require(report["localVideoFiles"]==0&&report["localMediaJobs"]==0&&report["sourceProgressed"]==true,"qa_diskless_source_progress")?;
    require(report["http"]["error"].is_null(),"qa_loopback_receiver_error")?;
    let objects=report["http"]["objects"].as_array().ok_or("qa_http_objects")?;
    require(objects.iter().filter(|v|v["endList"]==true).count()==1,"qa_one_end_ack")?;
    require(history.reopened&&history.events>0&&!history.archive_ready&&history.local_media_jobs==0,"qa_native_history")?;
    report["actualSourceUuidJoin"]=json!(history.source_event_identity_matches>0);
    report["actualAckMappedEvents"]=json!(history.source_positions);
    require(history.source_event_identity_matches>0,"qa_no_actual_uuid_matches")?;
    require(history.source_positions>0,"qa_no_usable_uuid_source_to_ack")?;
    require(report["visibleContentInterrupted"]!=true,"qa_visible_content_interrupted")?;
    Ok(())
}

#[test]
#[ignore="One approved public SOOP source, 60 seconds memory relay to localhost; no account/YouTube/recording"]
fn actual_soop_event_uuid_source_clock_ack_60s() {
    assert_eq!(std::env::var("ATSUMI_SOOP_EVENT_CLOCK_QA").as_deref(),Ok("public-source-memory-loopback-only"),"QA_OPT_IN");
    let channel=std::env::var("ATSUMI_SOOP_RELAY_QA_CHANNEL").expect("QA_CHANNEL_REQUIRED");
    assert!(matches!(channel.as_str(),"townboy"|"hinqocorv"),"QA_PREVIOUS_PUBLIC_SOURCE_ONLY");
    let broadcast=std::env::var("ATSUMI_SOOP_EVENT_CLOCK_QA_BROADCAST").expect("QA_BROADCAST_REQUIRED");
    assert!(!broadcast.is_empty()&&broadcast.len()<=24&&broadcast.bytes().all(|c|c.is_ascii_digit()),"QA_BROADCAST");
    let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run=uuid::Uuid::new_v4().simple().to_string();
    let root=workspace.join(".runtime/soop-event-clock-native-qa").join(&run);
    fs::create_dir_all(root.parent().unwrap()).unwrap();fs::create_dir(&root).unwrap();fs::create_dir(root.join("media")).unwrap();no_links(&root).unwrap();
    let identity=format!("local.atsumi.soopclockqa.{run}");
    let profile=PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);assert!(!profile.exists(),"QA_PROFILE_EXISTS");
    let server=Arc::new(server::Server::start().unwrap());let worker_server=server.clone();
    let selected=Arc::new(Mutex::new(None::<Arc<LiveRelayCoordinator>>));let worker_selected=selected.clone();
    let result=Arc::new(Mutex::new(json!({"passed":false,"isolated":true,"noCloud":true,"noDesktopInput":true,"noRecording":true})));
    let output=result.clone();let worker_root=root.clone();
    let old_args=std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    struct RestoreArgs(Option<std::ffi::OsString>);
    impl Drop for RestoreArgs {fn drop(&mut self){if let Some(value)=&self.0 {std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",value);}else{std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");}}}
    let mut quiet_args=old_args.clone().unwrap_or_default();quiet_args.push(" --mute-audio");
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",quiet_args);let _restore=RestoreArgs(old_args);
    let mut context=tauri::generate_context!();context.config_mut().identifier=identity;context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().any_thread()
        .manage(SoopOriginalHost::default()).manage(crate::youtube::original::YouTubeOriginalHost::default())
        .manage(Arc::new(crate::startup::Startup::default())).manage(Arc::new(crate::ui_download_events::DownloadEvents::default()))
        .setup(move|app|{
            if app.path().app_data_dir()?!=profile{return Err("QA_PROFILE_PATH".into());}
            tauri::window::WindowBuilder::new(app,"main").title("Atsumi source clock validation")
                .inner_size(1290.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle=app.handle().clone();thread::spawn(move||{
                let mut report=json!({"passed":false,"isolated":true,"noCloud":true,"noDesktopInput":true,"noRecording":true,"source":channel,"broadcast":broadcast});
                let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||exercise_clock(&handle,&worker_root,&channel,&broadcast,&worker_server,&mut report,&worker_selected)))
                    .unwrap_or_else(|_|Err("qa_worker_panic".into()));
                if outcome.is_err(){if let Some(selected)=lock(&worker_selected).as_ref(){selected.stop(true);}}
                report["passed"]=json!(outcome.is_ok());report["error"]=json!(outcome.err());*lock(&output)=report;
                if let Some(state)=handle.try_state::<AppState>() {
                    if let Ok(snapshot)=state.active_work_snapshot() {
                        if state.request_graceful_quit(handle.clone(),AppQuitRequest{expected_work_set_fingerprint:snapshot.work_set_fingerprint,
                            confirm_active_work:true,force_when_status_unknown:false}).is_ok_and(|r|r.accepted){return;}
                    }
                    lock(&output)["passed"]=json!(false);lock(&output)["quitError"]=json!("normal_quit_rejected");
                    let _=fs::write(worker_root.join("pending-result.json"),serde_json::to_vec_pretty(&*lock(&output)).unwrap());
                }else{handle.exit(2);}
            });Ok(())
        }).build(context).expect("QA_BUILD");
    app.run_return(|_,_|{});
    let mut report=lock(&result).clone();report["normalAppExitReturned"]=json!(true);report["httpAfterExit"]=server.snapshot();
    if let Some(selected)=lock(&selected).as_ref(){report["nativeAfterExit"]=serde_json::to_value(selected.status()).unwrap();}
    report["qualifications"]=json!(["Dispatch-time source coordinate, not source-sent/paint time. Presentation uncertainty remains unknown.",
        "Localhost ACK and actual UUID identity only; no YouTube archive origin or remote playback synchronization proof.",
        "Existing user profiles, videos, private OAuth QA and recordings were not opened or changed."]);
    fs::write(root.join("result.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("QA_SOOP_EVENT_CLOCK_RESULT={}",root.join("result.json").display());
    assert_eq!(report["passed"],true,"SOOP actual UUID/source/ACK flow remains unverified; inspect safe result");
}
