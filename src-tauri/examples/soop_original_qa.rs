//! Actual isolated SOOP host -> same-receiver archive -> verified library reopen.
//! No account login, cookies, existing Atsumi data, chat text or UI automation.
use atsumi_lib::{platform_media::{PlatformMediaService, PlatformMediaTools},soop::original::SoopOriginalHost};
use serde_json::{json,Value};
use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,AtomicI32,Ordering}},thread,time::{Duration,Instant}};
use tauri::{Manager,Webview};
static RESULT:AtomicI32=AtomicI32::new(2);
fn inspect(view:&Webview)->Value {
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    let code=r#"(()=>{const v=document.getElementById('livePlayer'),p=document.getElementById('player'),c=document.getElementById('chatting_area');const box=e=>{const r=e?.getBoundingClientRect();return r?{x:r.x,y:r.y,width:r.width,height:r.height}:null};return {readyState:v?.readyState,currentTime:v?.currentTime,decodedFrames:v?.getVideoPlaybackQuality?.().totalVideoFrames,paused:v?.paused,player:box(p),chat:box(c),capture:window.__atsumiSoopEncodedCapture?.getDiagnostics?.()??null}})()"#;
    if view.eval_with_callback(code,move|value|{let _=tx.send(value);}).is_err(){return Value::Null;}
    rx.recv_timeout(Duration::from_secs(2)).ok().and_then(|v|serde_json::from_str(&v).ok()).unwrap_or(Value::Null)
}
fn run(app:&tauri::AppHandle,root:&std::path::Path,channel:String)->Result<Value,String>{
    let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let ff=workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let media=PlatformMediaService::new(root.join("data"),PlatformMediaTools{yt_dlp:workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"),deno:workspace.join(".runtime/platform-tools/bin/deno.exe"),ffmpeg:ff.join("ffmpeg.exe"),ffprobe:ff.join("ffprobe.exe")},None)?;
    let host=app.state::<SoopOriginalHost>();
    host.set_tray_muted(app,true);
    let first=host.open(app,media.clone(),channel,uuid::Uuid::new_v4().to_string(),root.join("media"))?;
    let token=first.token.clone();
    host.viewport(app,&token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"test viewport")?)?;
    let result=(||->Result<Value,String>{
        let start=Instant::now();let mut last=first;let mut initial=None;let mut progressed=false;let mut facts=Value::Null;
        while start.elapsed()<Duration::from_secs(60){
            last=host.snapshot(&token)?;
            if let Some(view)=app.webviews().values().find(|v|v.label().starts_with("soop-original-")){facts=inspect(view);}
            if let Some(t)=facts["currentTime"].as_f64(){let base=*initial.get_or_insert(t);progressed|=t>base+2.;}
            if last.capture_ready&&last.chat_visible&&progressed{break;}
            thread::sleep(Duration::from_secs(1));
        }
        if !last.capture_ready||!last.chat_visible||!progressed{return Ok(json!({"kind":"soop_original_qa","success":false,"stage":"playback_capture_ready","snapshot":last,"facts":facts,"progressed":progressed}));}
        host.record(app,&token,true)?;
        let start=Instant::now();let job=loop{let s=host.snapshot(&token)?;if let Some(id)=s.recording_job{break id;}if start.elapsed()>Duration::from_secs(20){return Err("Native record acknowledgement timed out".into());}thread::sleep(Duration::from_millis(500));};
        thread::sleep(Duration::from_secs(20));
        host.record(app,&token,false)?;
        let stop=Instant::now();let final_job=loop{let j=media.jobs(Some("soop")).into_iter().find(|j|j.job_id==job).ok_or("recording job disappeared")?;if matches!(j.state.as_str(),"completed"|"failed"|"interrupted"){break j;}if stop.elapsed()>Duration::from_secs(90){break j;}thread::sleep(Duration::from_secs(1));};
        let reopened=if final_job.library.is_some(){match media.open(&job){Ok(p)=>{media.close(&p.token);true},Err(_)=>false}}else{false};
        let current=host.snapshot(&token)?;
        Ok(json!({"kind":"soop_original_qa","success":final_job.state=="completed"&&reopened,"isolated":true,"muted":true,"loginTested":false,"singleOfficialReceiver":true,"originalChatVisible":last.chat_visible,"playbackProgressed":progressed,"viewStillPlaying":current.state=="playing","job":final_job,"reopenedThroughLibrary":reopened,"facts":facts}))
    })();
    let _=host.close(app,&token);host.shutdown(app);media.shutdown();
    result
}
fn main()->Result<(),Box<dyn std::error::Error>>{
    let channel=std::env::args().nth(1).unwrap_or_else(||"phonics1".into());
    if channel.is_empty()||channel.len()>40||!channel.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')){return Err("invalid public channel id".into());}
    let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let root=workspace.join(".runtime/soop-original-qa").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(root.join("media"))?;std::fs::create_dir_all(root.join("profile"))?;
    std::env::set_var("APPDATA",root.join("profile"));
    let done=Arc::new(AtomicBool::new(false));let finished=done.clone();
    let mut context=tauri::generate_context!();context.config_mut().identifier="local.atsumi.soop-original-qa".into();context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().manage(SoopOriginalHost::default()).setup(move|app|{
        tauri::window::WindowBuilder::new(app,"main").title("Isolated SOOP original QA").inner_size(1290.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle=app.handle().clone();let deadline=handle.clone();let deadline_done=done.clone();
        thread::spawn(move||{thread::sleep(Duration::from_secs(220));if !deadline_done.load(Ordering::Acquire){eprintln!("SOOP_ORIGINAL_QA_DEADLINE");deadline.exit(3);}});
        thread::spawn(move||{let result=run(&handle,&root,channel);let success=result.as_ref().is_ok_and(|r|r["success"]==true);match result{Ok(v)=>println!("{v}"),Err(e)=>println!("{}",json!({"kind":"soop_original_qa","success":false,"error":e}))};RESULT.store(if success{0}else{2},Ordering::Release);done.store(true,Ordering::Release);handle.exit(if success{0}else{2});});Ok(())
    }).build(context)?;
    app.run_return(|_,_|{});finished.store(true,Ordering::Release);
    if RESULT.load(Ordering::Acquire)!=0{return Err("SOOP original flow not verified; inspect bounded result".into());}Ok(())
}
