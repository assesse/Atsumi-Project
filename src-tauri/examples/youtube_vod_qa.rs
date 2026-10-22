//! Isolated public official VOD playback -> seek -> natural end -> official replay.
//! No user profile, login inputs, media download, chat or recording is used.
use atsumi_lib::youtube::original::YouTubeOriginalHost;
use serde_json::{json,Value};
use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,AtomicI32,Ordering}},thread,time::{Duration,Instant}};
use tauri::{Manager,Webview};
static RESULT:AtomicI32=AtomicI32::new(2);
fn eval(view:&Webview,code:&str)->Value{
    let(tx,rx)=std::sync::mpsc::sync_channel(1);
    if view.eval_with_callback(code,move|value|{let _=tx.send(value);}).is_err(){return Value::Null;}
    rx.recv_timeout(Duration::from_secs(2)).ok().and_then(|s|serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}
fn inspect(view:&Webview)->Value{
    eval(view,r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video');let r=null;try{r=p?.getPlayerResponse?.()}catch{}const b=p?.getBoundingClientRect();return{readyState:v?.readyState,currentTime:v?.currentTime,duration:Number.isFinite(v?.duration)?v.duration:null,paused:v?.paused,ended:v?.ended,decodedFrames:v?.getVideoPlaybackQuality?.().totalVideoFrames,videoId:r?.videoDetails?.videoId,ad:!!p?.classList.contains('ad-showing'),nativeButtons:p?.querySelectorAll('.ytp-chrome-controls button')?.length??0,player:b?{width:b.width,height:b.height}:null,bridge:window.__atsumiYouTubeVod?.status?.()??null,hasCapture:!!window.__atsumiYouTubeEncodedCapture,hasLiveChat:!!document.querySelector('iframe#chatframe')}})()"#)
}
fn run(app:&tauri::AppHandle,video:String)->Result<Value,String>{
    let host=app.state::<YouTubeOriginalHost>();host.set_tray_muted(app,true);
    let opened=host.open_vod(app,video.clone(),uuid::Uuid::new_v4().to_string())?;let mut token=opened.token.clone();
    let result=(||->Result<Value,String>{
        host.viewport(app,&token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":540},"chat":null,"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"invalid fixture geometry")?)?;
        let view=app.webviews().values().find(|v|v.label().starts_with("youtube-original-vod-")).cloned().ok_or("watch-only document missing")?;
        let start=Instant::now();let mut baseline=None;let mut progressed=false;let mut facts=Value::Null;
        while start.elapsed()<Duration::from_secs(90){
            facts=inspect(&view);
            if facts["videoId"]==video&&facts["ad"]==false&&facts["paused"]==false{
                if let Some(time)=facts["currentTime"].as_f64(){let first=*baseline.get_or_insert(time);if time>first+2.{progressed=true;break;}}
            }
            thread::sleep(Duration::from_millis(500));
        }
        if !progressed{return Ok(json!({"kind":"youtube_vod_qa","success":false,"stage":"playback","facts":facts,"snapshot":host.snapshot(&token)?}));}
        let first=facts.clone();
        let seek=eval(&view,r#"(()=>{const p=document.getElementById('movie_player'),v=p?.querySelector('video');if(!v||!Number.isFinite(v.duration)||v.duration<3||typeof p?.seekTo!=='function')return false;p.seekTo(Math.max(0,v.duration-1.5),true);p.playVideo?.();return true})()"#)==true;
        let end=Instant::now();let mut ended=false;
        while seek&&end.elapsed()<Duration::from_secs(20){facts=inspect(&view);if facts["ended"]==true{ended=true;break;}thread::sleep(Duration::from_millis(250));}
        // The official play button is retained at end. Activate it, not a replacement video.
        let replay_clicked=if ended{eval(&view,r#"(()=>{const button=document.querySelector('#movie_player .ytp-play-button');if(!button)return false;button.click();return true})()"#)==true}else{false};
        let replay=Instant::now();let mut replayed=false;
        while replay_clicked&&replay.elapsed()<Duration::from_secs(15){facts=inspect(&view);if facts["ended"]==false&&facts["paused"]==false&&facts["currentTime"].as_f64().is_some_and(|t|t>0.2&&t<5.){replayed=true;break;}thread::sleep(Duration::from_millis(250));}
        let record_rejected=host.record(app,&token,true).is_err();
        let replacement=host.open_vod(app,video.clone(),uuid::Uuid::new_v4().to_string())?;
        let distinct_viewer_token=replacement.token!=token;
        host.close(app,&token)?;token=replacement.token;
        let replacement_retained=distinct_viewer_token&&app.get_webview(view.label()).is_some()&&host.snapshot(&token).is_ok();
        host.viewport(app,&token,serde_json::from_value(json!({"sequence":2,"player":null,"chat":null,"visible":false,"muted":true,"occlusions":[]})).map_err(|_|"invalid hidden geometry")?)?;
        thread::sleep(Duration::from_millis(300));
        let retained=app.get_webview(view.label()).is_some();let snapshot=host.snapshot(&token)?;
        let official=first["nativeButtons"].as_u64().unwrap_or(0)>3&&first["player"]["width"].as_f64().unwrap_or(0.)>900.;
        let no_capture=first["hasCapture"]==false&&!snapshot.capture_ready&&snapshot.recording_job.is_none()&&record_rejected;
        Ok(json!({"kind":"youtube_vod_qa","success":progressed&&official&&seek&&ended&&replayed&&retained&&no_capture&&replacement_retained,"isolated":true,"muted":true,"loginTested":false,"singleWatchOnlyDocument":app.webviews().keys().filter(|label|label.starts_with("youtube-original-vod-")).count()==1,"officialControls":official,"playbackProgressed":progressed,"seekAccepted":seek,"naturalEndObserved":ended,"officialReplayActivated":replay_clicked,"replayProgressed":replayed,"retainedWhileHidden":retained,"replacementSurvivedOldClose":replacement_retained,"noLiveCaptureAuthority":no_capture,"initialFacts":first,"finalFacts":facts,"snapshot":snapshot}))
    })();
    let _=host.close(app,&token);host.shutdown(app);result
}
fn main()->Result<(),Box<dyn std::error::Error>>{
    let video=std::env::args().nth(1).unwrap_or_else(||"jNQXAC9IVRw".into());
    if video.len()!=11||!video.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')){return Err("invalid public video ID".into());}
    let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let root=workspace.join(".runtime/youtube-vod-qa").join(uuid::Uuid::new_v4().to_string());std::fs::create_dir_all(root.join("profile"))?;
    std::env::set_var("APPDATA",root.join("profile"));
    let done=Arc::new(AtomicBool::new(false));let finished=done.clone();
    let mut context=tauri::generate_context!();context.config_mut().identifier="local.atsumi.youtube-vod-qa".into();context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().manage(YouTubeOriginalHost::default()).setup(move|app|{
        tauri::window::WindowBuilder::new(app,"main").title("Isolated YouTube VOD QA").inner_size(960.,540.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle=app.handle().clone();let deadline=handle.clone();let deadline_done=done.clone();
        thread::spawn(move||{thread::sleep(Duration::from_secs(150));if !deadline_done.load(Ordering::Acquire){eprintln!("YOUTUBE_VOD_QA_DEADLINE");deadline.exit(3);}});
        thread::spawn(move||{let result=run(&handle,video);let success=result.as_ref().is_ok_and(|r|r["success"]==true);match result{Ok(value)=>println!("{value}"),Err(error)=>println!("{}",json!({"kind":"youtube_vod_qa","success":false,"error":error}))};RESULT.store(if success{0}else{2},Ordering::Release);done.store(true,Ordering::Release);handle.exit(if success{0}else{2});});Ok(())
    }).build(context)?;
    app.run_return(|_,_|{});finished.store(true,Ordering::Release);if RESULT.load(Ordering::Acquire)!=0{return Err("YouTube official VOD flow not verified; inspect bounded result".into());}Ok(())
}
