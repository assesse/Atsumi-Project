//! Actual isolated YouTube host -> same-receiver archive -> verified library reopen.
//! No account login, cookies, existing Atsumi data, chat text or UI automation.
use atsumi_lib::{platform_media::{PlatformMediaService, PlatformMediaTools},youtube::original::YouTubeOriginalHost};
use serde_json::{json,Value};
use std::{path::PathBuf,sync::{Arc,atomic::{AtomicBool,AtomicI32,Ordering}},thread,time::{Duration,Instant}};
use tauri::{Manager,Webview};
static RESULT:AtomicI32=AtomicI32::new(2);
fn inspect(view:&Webview)->Value {
    let (tx,rx)=std::sync::mpsc::sync_channel(1);
    let code=r#"(()=>{const v=document.querySelector('#movie_player video'),p=document.getElementById('movie_player'),c=document.querySelector('yt-live-chat-app');const box=e=>{const r=e?.getBoundingClientRect();return r?{x:r.x,y:r.y,width:r.width,height:r.height}:null};const bar=window.ytInitialData?.contents?.twoColumnWatchNextResults?.conversationBar,renderer=bar?.liveChatRenderer,notice=bar?.conversationBarRenderer?.availabilityMessage?.messageRenderer?.text;let text=null;if(typeof notice?.simpleText==='string'&&notice.simpleText.length<=256)text=notice.simpleText;else if(Array.isArray(notice?.runs)&&notice.runs.length>0&&notice.runs.length<=8&&notice.runs.every(r=>typeof r?.text==='string'&&r.text.length<=256)){const joined=notice.runs.map(r=>r.text).join('');if(joined.length<=256)text=joined;}let matchingIframe=false;try{const src=document.querySelector('iframe#chatframe')?.src;if(typeof src==='string'&&src.length<=4096){const u=new URL(src),id=new URLSearchParams(location.search).get('v');matchingIframe=u.origin==='https://www.youtube.com'&&!u.username&&!u.password&&!u.port&&!u.hash&&u.pathname==='/live_chat'&&u.searchParams.getAll('v').length===1&&u.searchParams.get('v')===id;}}catch{}const chatEvidence={rendererPresent:!!renderer&&typeof renderer==='object'&&!Array.isArray(renderer),noticeKind:text==='Chat is disabled for this live stream.'?'disabled_en':text==='채팅을 사용할 수 없는 실시간 스트림입니다.'?'disabled_ko':notice==null?'absent':'other',matchingIframe};return {readyState:v?.readyState,currentTime:v?.currentTime,decodedFrames:v?.getVideoPlaybackQuality?.().totalVideoFrames,paused:v?.paused,player:box(p),chat:box(c),chatEvidence,status:window.__atsumiYouTubeOriginal?.status?.()??null,capture:window.__atsumiYouTubeEncodedCapture?.getDiagnostics?.()??null}})()"#;
    if view.eval_with_callback(code,move|value|{let _=tx.send(value);}).is_err(){return Value::Null;}
    rx.recv_timeout(Duration::from_secs(2)).ok().and_then(|v|serde_json::from_str(&v).ok()).unwrap_or(Value::Null)
}
fn source_chat_matches(expect_no_chat:bool,visible:bool,facts:&Value)->bool{
    let available=facts["status"]["chatDescriptor"]["chatAvailable"].as_bool();
    if expect_no_chat {available==Some(false)}else{visible&&available==Some(true)}
}
fn archive_matches(expect_no_chat:bool,source_matches:bool,state:Option<&str>)->bool{
    source_matches&&if expect_no_chat{matches!(state,Some("partial"|"unknown"))}else{state==Some("complete")}
}
fn run(app:&tauri::AppHandle,root:&std::path::Path,channel:String,expect_no_chat:bool)->Result<Value,String>{
    let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let ff=workspace.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let media=PlatformMediaService::new(root.join("data"),PlatformMediaTools{yt_dlp:workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"),deno:workspace.join(".runtime/platform-tools/bin/deno.exe"),ffmpeg:ff.join("ffmpeg.exe"),ffprobe:ff.join("ffprobe.exe")},None)?;
    let host=app.state::<YouTubeOriginalHost>();
    host.set_tray_muted(app,true);
    let first=host.open(app,media.clone(),channel,uuid::Uuid::new_v4().to_string(),root.join("media"))?;
    let token=first.token.clone();
    host.viewport(app,&token,serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_|"test viewport")?)?;
    let result=(||->Result<Value,String>{
        let start=Instant::now();let mut last=first;let mut initial=None;let mut progressed=false;let mut facts=Value::Null;
        while start.elapsed()<Duration::from_secs(60){
            last=host.snapshot(&token)?;
            if let Some(view)=app.webviews().values().find(|v|v.label().starts_with("youtube-original-")&&!v.label().starts_with("youtube-original-chat-")){facts=inspect(view);}
            if let Some(t)=facts["currentTime"].as_f64(){let base=*initial.get_or_insert(t);progressed|=t>base+2.;}
            if last.capture_ready&&source_chat_matches(expect_no_chat,last.chat_visible,&facts)&&progressed{break;}
            thread::sleep(Duration::from_secs(1));
        }
        if !last.capture_ready||!source_chat_matches(expect_no_chat,last.chat_visible,&facts)||!progressed{return Ok(json!({"kind":"youtube_original_qa","success":false,"stage":"playback_capture_ready","expectedSourceChat":if expect_no_chat{"unavailable"}else{"available"},"snapshot":last,"facts":facts,"progressed":progressed}));}
        host.record(app,&token,true)?;
        let start=Instant::now();let job=loop{let s=host.snapshot(&token)?;if let Some(id)=s.recording_job{break id;}if s.error.is_some()||start.elapsed()>Duration::from_secs(20)||media.jobs(Some("youtube")).iter().any(|job|matches!(job.state.as_str(),"failed"|"interrupted")){
            let current_facts=app.webviews().values().find(|v|v.label().starts_with("youtube-original-")&&!v.label().starts_with("youtube-original-chat-")).map(inspect).unwrap_or(Value::Null);
            return Ok(json!({"kind":"youtube_original_qa","success":false,"stage":"record_ack","snapshot":s,"facts":current_facts,"jobs":media.jobs(Some("youtube"))}));
        }thread::sleep(Duration::from_millis(500));};
        thread::sleep(Duration::from_secs(20));
        host.record(app,&token,false)?;
        let stop=Instant::now();let final_job=loop{let j=media.jobs(Some("youtube")).into_iter().find(|j|j.job_id==job).ok_or("recording job disappeared")?;if matches!(j.state.as_str(),"completed"|"failed"|"interrupted"){break j;}if stop.elapsed()>Duration::from_secs(90){break j;}thread::sleep(Duration::from_secs(1));};
        let reopened=if final_job.library.is_some(){match media.open(&job){Ok(p)=>{media.close(&p.token);true},Err(_)=>false}}else{false};
        let current=host.snapshot(&token)?;
        let final_facts=app.webviews().values().find(|v|v.label().starts_with("youtube-original-")&&!v.label().starts_with("youtube-original-chat-")).map(inspect).unwrap_or(Value::Null);
        let video_verified=final_job.state=="completed"&&reopened;
        let archive=final_job.library.as_ref().and_then(|item|item.chat_archive.as_ref()).and_then(|chat|serde_json::to_value(chat).ok());
        let archive_state=archive.as_ref().and_then(|chat|chat["state"].as_str());
        let chat_complete=archive_state==Some("complete");
        let source_matches=source_chat_matches(expect_no_chat,last.chat_visible,&facts)&&source_chat_matches(expect_no_chat,current.chat_visible,&final_facts);
        let expected_chat_verified=archive_matches(expect_no_chat,source_matches,archive_state);
        Ok(json!({"kind":"youtube_original_qa","success":video_verified&&expected_chat_verified,"expectedSourceChat":if expect_no_chat{"unavailable"}else{"available"},"expectedChatVerified":expected_chat_verified,"videoVerified":video_verified,"chatComplete":chat_complete,"chatArchiveState":archive_state,"isolated":true,"muted":true,"loginTested":false,"singleOfficialReceiver":true,"originalChatVisible":last.chat_visible,"playbackProgressed":progressed,"viewStillPlaying":current.state=="playing","job":final_job,"reopenedThroughLibrary":reopened,"facts":facts,"finalFacts":final_facts}))
    })();
    let _=host.close(app,&token);host.shutdown(app);media.shutdown();
    result
}
fn main()->Result<(),Box<dyn std::error::Error>>{
    let mut channel=None;let mut expect_no_chat=false;
    for arg in std::env::args().skip(1){if arg=="--expect-no-chat"&&!expect_no_chat{expect_no_chat=true;}else if !arg.starts_with('-')&&channel.is_none(){channel=Some(arg);}else{return Err("usage: youtube_original_qa [video-id] [--expect-no-chat]".into());}}
    let channel=channel.unwrap_or_else(||if expect_no_chat{"T4XZmMPQ9Kw"}else{"cswFQYam8Ic"}.into());
    if channel.len()!=11||!channel.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')){return Err("invalid public channel id".into());}
    let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned();
    let root=workspace.join(".runtime/youtube-original-qa").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(root.join("media"))?;std::fs::create_dir_all(root.join("profile"))?;
    std::env::set_var("APPDATA",root.join("profile"));
    let done=Arc::new(AtomicBool::new(false));let finished=done.clone();
    let mut context=tauri::generate_context!();context.config_mut().identifier="local.atsumi.youtube-original-qa".into();context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().manage(YouTubeOriginalHost::default()).setup(move|app|{
        tauri::window::WindowBuilder::new(app,"main").title("Isolated YouTube original QA").inner_size(1290.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
        let handle=app.handle().clone();let deadline=handle.clone();let deadline_done=done.clone();
        thread::spawn(move||{thread::sleep(Duration::from_secs(220));if !deadline_done.load(Ordering::Acquire){eprintln!("YouTube_ORIGINAL_QA_DEADLINE");deadline.exit(3);}});
        thread::spawn(move||{let result=run(&handle,&root,channel,expect_no_chat);let success=result.as_ref().is_ok_and(|r|r["success"]==true);match result{Ok(v)=>println!("{v}"),Err(e)=>println!("{}",json!({"kind":"youtube_original_qa","success":false,"expectedSourceChat":if expect_no_chat{"unavailable"}else{"available"},"error":e}))};RESULT.store(if success{0}else{2},Ordering::Release);done.store(true,Ordering::Release);handle.exit(if success{0}else{2});});Ok(())
    }).build(context)?;
    app.run_return(|_,_|{});finished.store(true,Ordering::Release);
    if RESULT.load(Ordering::Acquire)!=0{return Err("YouTube original flow not verified; inspect bounded result".into());}Ok(())
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn explicit_no_chat_never_accepts_unknown_descriptor_or_claims_complete(){
        let available=json!({"status":{"chatDescriptor":{"chatAvailable":true}}});let unavailable=json!({"status":{"chatDescriptor":{"chatAvailable":false}}});
        assert!(source_chat_matches(false,true,&available));assert!(!source_chat_matches(false,false,&unavailable));
        assert!(source_chat_matches(true,false,&unavailable));assert!(!source_chat_matches(true,false,&Value::Null));assert!(source_chat_matches(true,true,&unavailable));
        let unknown=json!({"status":{"chatDescriptor":{"chatAvailable":null}}});assert!(!source_chat_matches(true,true,&unknown));assert!(!source_chat_matches(false,true,&unknown));
        assert!(archive_matches(false,true,Some("complete")));assert!(!archive_matches(false,true,Some("partial")));
        assert!(archive_matches(true,true,Some("partial")));assert!(!archive_matches(true,true,Some("complete")));assert!(!archive_matches(true,false,Some("unknown")));assert!(!archive_matches(true,true,None));
    }
}
