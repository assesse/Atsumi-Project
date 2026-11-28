//! Failure evidence for the exact, fresh-profile QA player only. No authority or readiness output.
use super::*;

const DOM: &str = r#"(()=>{const p=document.querySelector('#movie_player'),v=p?.querySelector('video'),r=p?.getBoundingClientRect();let state=null,response=null;try{state=p?.getPlayerState?.();response=p?.getPlayerResponse?.()??window.ytInitialPlayerResponse}catch{};const declared=Number(response?.videoDetails?.lengthSeconds),ps=response?.playabilityStatus?.status;return {
documentReady:document.readyState==='complete'?2:document.readyState==='interactive'?1:0,
responsePresent:!!response,exactResponseVideo:response?.videoDetails?.videoId===new URLSearchParams(location.search).get('v'),
declaredLengthSeconds:Number.isFinite(declared)?declared:null,playabilityStatus:['OK','ERROR','UNPLAYABLE','LOGIN_REQUIRED','LIVE_STREAM_OFFLINE'].includes(ps)?ps:'UNKNOWN',
playerPresent:!!p,videoPresent:!!v,readyState:v?.readyState??null,networkState:v?.networkState??null,
mediaErrorCode:v?.error?.code??null,currentTime:Number.isFinite(v?.currentTime)?v.currentTime:null,
duration:Number.isFinite(v?.duration)?v.duration:null,paused:v?.paused??null,seeking:v?.seeking??null,
ended:v?.ended??null,muted:v?.muted??null,sourcePresent:!!v?.currentSrc,videoWidth:v?.videoWidth??null,
videoHeight:v?.videoHeight??null,playerState:Number.isInteger(state)&&[-1,0,1,2,3,5].includes(state)?state:null,
adShowing:!!p?.classList.contains('ad-showing'),innerWidth:innerWidth,innerHeight:innerHeight,
playerX:r?.x??null,playerY:r?.y??null,playerWidth:r?.width??null,playerHeight:r?.height??null,
visiblePasswordInput:!!Array.from(document.querySelectorAll('input[type=password]')).find(e=>e.getClientRects().length)
}})()"#;

fn project_dom(raw: &str) -> Checked<Value> {
    require(raw.len() <= 4096, "QA_DIAGNOSTIC_DOM_BOUND")?;
    let value: Value = serde_json::from_str(raw).map_err(|_| "QA_DIAGNOSTIC_DOM_JSON")?;
    let object = value.as_object().ok_or("QA_DIAGNOSTIC_DOM_SHAPE")?;
    let bools = ["playerPresent","videoPresent","paused","seeking","ended","muted","sourcePresent","adShowing","visiblePasswordInput","responsePresent","exactResponseVideo"];
    let numbers = ["documentReady","readyState","networkState","mediaErrorCode","currentTime","duration","videoWidth","videoHeight","playerState","innerWidth","innerHeight","playerX","playerY","playerWidth","playerHeight","declaredLengthSeconds"];
    require(object.len() == bools.len()+numbers.len()+1 && object.keys().all(|k| bools.contains(&k.as_str())||numbers.contains(&k.as_str())||k=="playabilityStatus"), "QA_DIAGNOSTIC_DOM_KEYS")?;
    require(bools.iter().all(|k| value[*k].is_boolean()||value[*k].is_null())
        && numbers.iter().all(|k| value[*k].is_null()||value[*k].as_f64().is_some_and(|v|v.is_finite()&&v.abs()<=1_000_000.))
        && matches!(value["playabilityStatus"].as_str(),Some("OK"|"ERROR"|"UNPLAYABLE"|"LOGIN_REQUIRED"|"LIVE_STREAM_OFFLINE"|"UNKNOWN")),"QA_DIAGNOSTIC_DOM_VALUES")?;
    Ok(value)
}
fn exact_watch(owned:&OwnedVod, video:&str)->bool {
    owned.view.url().ok().is_some_and(|u| u.scheme()=="https" && matches!(u.host_str(),Some("www.youtube.com"|"youtube.com"))
        && u.path()=="/watch" && u.query_pairs().filter(|(k,_)|k=="v").map(|(_,v)|v.into_owned()).collect::<Vec<_>>()==[video])
}
fn player_only(value:&Value)->bool {
    value["playerPresent"]==true && value["visiblePasswordInput"]==false
        && matches!((value["innerWidth"].as_f64(),value["innerHeight"].as_f64(),value["playerX"].as_f64(),value["playerY"].as_f64(),value["playerWidth"].as_f64(),value["playerHeight"].as_f64()),
            (Some(w),Some(h),Some(x),Some(y),Some(pw),Some(ph)) if w>=100.&&h>=100.&&w<=2048.&&h<=1536.&&x.abs()<=2.&&y.abs()<=2.&&pw>=w-2.&&ph>=h-2.)
}
fn capture(owned:&OwnedVod,video:&str,path:&Path)->Checked<()> {
    use webview2_com::{CapturePreviewCompletedHandler, Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG};
    use windows::Win32::{System::Com::{STATFLAG_NONAME,STATSTG,STREAM_SEEK_SET},UI::Shell::SHCreateMemStream};
    require(exact_watch(owned,video),"QA_DIAGNOSTIC_FOREIGN_DOCUMENT")?;
    let(tx,rx)=std::sync::mpsc::sync_channel(1);
    owned.view.with_webview(move|platform|unsafe {
        let Some(stream)=SHCreateMemStream(None)else{let _=tx.try_send(Err("QA_CAPTURE_STREAM"));return;};
        let data=stream.clone();let completed=tx.clone();
        let callback=CapturePreviewCompletedHandler::create(Box::new(move|status|{
            let result=(||->std::result::Result<Vec<u8>,&'static str>{
                status.map_err(|_|"QA_CAPTURE")?;let mut stat=STATSTG::default();
                data.Stat(&mut stat,STATFLAG_NONAME).map_err(|_|"QA_CAPTURE_STAT")?;
                if stat.cbSize<24||stat.cbSize>16*1024*1024{return Err("QA_CAPTURE_BOUND");}
                data.Seek(0,STREAM_SEEK_SET,None).map_err(|_|"QA_CAPTURE_SEEK")?;
                let mut bytes=vec![0;stat.cbSize as usize];let mut read=0;
                data.Read(bytes.as_mut_ptr().cast(),bytes.len() as u32,Some(&mut read)).ok().map_err(|_|"QA_CAPTURE_READ")?;
                if read as usize!=bytes.len()||!bytes.starts_with(b"\x89PNG\r\n\x1a\n")||&bytes[12..16]!=b"IHDR"{return Err("QA_CAPTURE_PNG");}
                let width=u32::from_be_bytes(bytes[16..20].try_into().unwrap());let height=u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                if width==0||height==0||width>2048||height>1536{return Err("QA_CAPTURE_DIMENSIONS");}Ok(bytes)
            })();let _=completed.try_send(result);Ok(())
        }));
        if platform.controller().CoreWebView2().and_then(|c|c.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,&stream,&callback)).is_err(){let _=tx.try_send(Err("QA_CAPTURE_DISPATCH"));}
    }).map_err(|_|"QA_CAPTURE_DISPATCH")?;
    let bytes=rx.recv_timeout(Duration::from_secs(6)).map_err(|_|"QA_CAPTURE_TIMEOUT")?.map_err(str::to_owned)?;
    require(exact_watch(owned,video),"QA_DIAGNOSTIC_FOREIGN_DOCUMENT")?;
    let mut f=fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|_|"QA_CAPTURE_CREATE")?;
    f.write_all(&bytes).and_then(|_|f.sync_all()).map_err(|_|"QA_CAPTURE_SAVE".into())
}

pub(super) fn observe(owned:&OwnedVod,video:&str,stage:&str,expected_duration:f64,root:&Path)->Value {
    let host=owned.app.state::<YouTubeOriginalHost>();
    let mut evidence=json!({"failedStage":stage,"expectedDurationSeconds":expected_duration,"diagnosticOnly":true,
        "metadataVerified":false,"synchronizedReplayVerified":false,"screenshotSaved":false});
    match host.snapshot(&owned.token) {
        Ok(s)=>{evidence["nativeSnapshot"]=json!({"state":match s.state.as_str(){"loading"=>"loading","playing"=>"playing","ended"=>"ended","error"=>"error",_=>"unknown"},
            "exactVideo":s.broadcast_id==video,"authenticated":s.authenticated,"playability":s.playability,"errorPresent":s.error.is_some(),"captureReady":s.capture_ready});},
        Err(_)=>evidence["snapshotError"]=json!("QA_DIAGNOSTIC_SNAPSHOT")
    }
    match host.vod_clock(&owned.token) {
        Ok(Some(c))=>{let f=&c.fields;evidence["nativeClock"]=json!({"available":true,"ageMs":c.age_ms,"documentEpoch":c.document_epoch,
            "valid":f.valid,"exactVideo":f.video_id==video,"sourceIdentityPresent":Uuid::parse_str(&f.source_id).is_ok(),
            "sourceGeneration":f.source_generation,"seekGeneration":f.seek_generation,"sequence":f.sequence,
            "currentTimeSeconds":f.current_time,"durationSeconds":f.duration,"durationMatchesMetadata":f.duration.is_some_and(|d|(d-expected_duration).abs()<=1.),
            "paused":f.paused,"seeking":f.seeking,"buffering":f.buffering,"ended":f.ended,"playbackRate":f.playback_rate,
            "adState":match f.ad_state.as_str(){"ad"=>"ad","content"=>"content",_=>"unknown"}});},
        Ok(None)=>evidence["nativeClock"]=json!({"available":false,"reason":"absent_stale_or_rejected_by_native_owner"}),
        Err(_)=>evidence["clockError"]=json!("QA_DIAGNOSTIC_CLOCK")
    }
    if !exact_watch(owned,video){evidence["domError"]=json!("QA_DIAGNOSTIC_FOREIGN_DOCUMENT");return evidence;}
    let(tx,rx)=std::sync::mpsc::sync_channel(1);
    let dom=owned.view.eval_with_callback(DOM,move|s|{let _=tx.try_send(s);}).map_err(|_|"QA_DIAGNOSTIC_DOM_DISPATCH".to_owned())
        .and_then(|_|rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"QA_DIAGNOSTIC_DOM_TIMEOUT".to_owned())).and_then(|s|project_dom(&s));
    match dom {
        Ok(value)=>{let screenshot_allowed=player_only(&value);evidence["dom"]=value;
            if screenshot_allowed {
                match capture(owned,video,&root.join("processing-player-failure.png")){
                    Ok(())=>{evidence["screenshotSaved"]=json!(true);evidence["screenshotFile"]=json!("processing-player-failure.png");},
                    Err(code)=>evidence["screenshotError"]=json!(code)
                }
            }else{evidence["screenshotSkipped"]=json!("player_does_not_cover_exact_owned_view");}
        },
        Err(code)=>evidence["domError"]=json!(code)
    }
    evidence
}

#[test]
fn failure_dom_rejects_text_and_foreign_fields_without_returning_payload(){
    assert!(project_dom(r#"{"title":"PRIVATE_ACCOUNT","duration":59}"#).is_err());
    let mut value=json!({});
    for key in ["playerPresent","videoPresent","paused","seeking","ended","muted","sourcePresent","adShowing","visiblePasswordInput","responsePresent","exactResponseVideo"]{value[key]=json!(false);}
    for key in ["documentReady","readyState","networkState","mediaErrorCode","currentTime","duration","videoWidth","videoHeight","playerState","innerWidth","innerHeight","playerX","playerY","playerWidth","playerHeight","declaredLengthSeconds"]{value[key]=Value::Null;}
    value["playabilityStatus"]=json!("UNKNOWN");
    assert!(project_dom(&value.to_string()).is_ok());
    value["playabilityStatus"]=json!("PRIVATE_ACCOUNT");assert!(project_dom(&value.to_string()).is_err());value["playabilityStatus"]=json!("OK");
    value["duration"]=json!("PRIVATE_URL");assert!(project_dom(&value.to_string()).is_err());
    value["duration"]=json!(59);value["url"]=json!("PRIVATE_URL");assert!(project_dom(&value.to_string()).is_err());
}
#[test]
fn failure_capture_requires_full_player_without_visible_password(){
    let mut value=json!({"playerPresent":true,"visiblePasswordInput":false,"innerWidth":960,"innerHeight":540,
        "playerX":0,"playerY":0,"playerWidth":960,"playerHeight":540});
    assert!(player_only(&value));value["playerHeight"]=json!(400);assert!(!player_only(&value));
    value["playerHeight"]=json!(540);value["visiblePasswordInput"]=json!(true);assert!(!player_only(&value));
}
