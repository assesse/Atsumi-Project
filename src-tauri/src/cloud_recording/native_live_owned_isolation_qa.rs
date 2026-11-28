//! Proposed child of native_live_alignment_runner_qa; no registration yet.
//! Uses the existing native SOOP42 ready receipt, never a renderer/video URL input.
use super::*;
use crate::youtube::original::OwnedVodLease;

fn receipt_video(service:&CloudService)->Checked<String>{
    let inner=service.lock().map_err(|_|"QA_ISOLATION_CATALOG")?;
    let job=inner.catalog.jobs.iter().find(|j|j.view.id=="e0d2937d-74bb-48c7-97fe-c558b66bfb32")
        .filter(|j|j.has_upload_receipt()&&j.view.state=="ready"&&!j.view.recording_plan
            &&j.view.visibility==crate::cloud_recording::model::CloudVisibility::Unlisted
            &&j.view.source.provider=="soop"&&j.view.source.recording_id=="241bc41d-d3e6-4bf9-a205-899f2c2d21d5"
            &&(j.view.duration_seconds-42.102521).abs()<0.01).ok_or("QA_ISOLATION_EXISTING_RECEIPT")?;
    job.view.video_id.clone().ok_or("QA_ISOLATION_VIDEO".into())
}
fn visible(sequence:u64,x:f64)->NativeViewport{NativeViewport{sequence,player:Some(Rect{x,y:0.,width:480.,height:270.}),
    chat:None,visible:true,muted:false,occlusions:Vec::new()}}
fn owned_clock(lease:&OwnedVodLease,video:&str)->Checked<VodClock>{
    lease.clock().map_err(|_|"QA_ISOLATION_OWNED_CLOCK")?
        .filter(|c|c.age_ms<=1500&&c.fields.valid&&c.fields.ad_state=="content"&&c.fields.video_id==video)
        .ok_or("QA_ISOLATION_OWNED_CONTENT_CLOCK".into())
}
fn owned_control(lease:&OwnedVodLease,video:&str,action:&str,seconds:Option<f64>)->Checked<VodClock>{
    let c=owned_clock(lease,video)?;
    lease.control(VodControlRequest{request_id:Uuid::new_v4().to_string(),document_epoch:c.document_epoch,
        source_id:c.fields.source_id.clone(),source_generation:c.fields.source_generation,action:action.into(),seconds,rate:None})
        .map_err(|_|"QA_ISOLATION_OWNED_CONTROL")?;Ok(c)
}
fn wait_owned(lease:&OwnedVodLease,video:&str,end:Instant,predicate:impl Fn(&VodClock)->bool)->Checked<VodClock>{
    loop{check(end,&AtomicBool::new(false))?;if let Ok(c)=owned_clock(lease,video){if predicate(&c){return Ok(c);}}
        thread::sleep(Duration::from_millis(50));}
}
fn source_unchanged(a:&VodClock,b:&VodClock)->bool{
    a.document_epoch==b.document_epoch&&a.fields.source_id==b.fields.source_id&&a.fields.source_generation==b.fields.source_generation
}
fn held_position(a:&VodClock,b:&VodClock,seconds:f64)->bool{
    source_unchanged(a,b)&&b.fields.paused&&!b.fields.seeking&&b.fields.current_time.is_some_and(|t|(t-seconds).abs()<=0.35)
        &&a.fields.seek_generation==b.fields.seek_generation
}
fn silence_view(view:&Webview)->Checked<()>{
    let(tx,rx)=std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(r#"(()=>{const v=document.querySelector('#movie_player video');if(!v)return false;const q=()=>{if(v.volume!==0)v.volume=0;if(!v.muted)v.muted=true};v.addEventListener('volumechange',q);q();return v.muted&&v.volume===0})()"#,
        move|s|{let _=tx.try_send(s);}).map_err(|_|"QA_ISOLATION_SILENCE")?;
    require(rx.recv_timeout(Duration::from_secs(3)).ok().as_deref()==Some("true"),"QA_ISOLATION_SILENCE")
}
fn open_exclusive(app:&AppHandle,video:&str,end:Instant)->Checked<OwnedVodLease>{
    let old:std::collections::HashSet<_>=app.webviews().into_keys().collect();
    let lease=app.state::<YouTubeOriginalHost>().open_owned_vod(app,video.into(),Uuid::new_v4())
        .map_err(|_|"QA_ISOLATION_EXCLUSIVE_OPEN")?;
    let mut v=visible(1,480.);v.muted=true;lease.viewport(v).map_err(|_|"QA_ISOLATION_VIEWPORT")?;
    let views:Vec<_>=app.webviews().into_values().filter(|v|!old.contains(v.label())&&v.label().starts_with("youtube-original-vod-")).collect();
    require(views.len()==1,"QA_ISOLATION_EXACT_DOCUMENT")?;
    let c=wait_owned(&lease,video,(Instant::now()+Duration::from_secs(35)).min(end),|c|c.fields.duration.is_some_and(|d|(d-42.102521).abs()<=1.))?;
    require(c.token.is_empty(),"QA_ISOLATION_OPAQUE_LEASE")?;silence_view(&views[0])?;
    lease.viewport(visible(2,480.)).map_err(|_|"QA_ISOLATION_VIEWPORT")?;Ok(lease)
}

/// Caller owns a fresh QA app and worker-free native service. This is additive
/// diagnostic viewing; it neither updates the upload nor admits Live replay.
pub(super) fn exercise(app:&AppHandle,service:&CloudService)->Checked<Value>{
    let video=receipt_video(service)?;qa_profile(app)?;
    let end=Instant::now()+Duration::from_secs(120);let cancel=AtomicBool::new(false);
    let host=app.state::<YouTubeOriginalHost>();
    let ordinary=OwnedVod::open(app,&video)?;
    wait_clock(app,&ordinary.token,&video,(Instant::now()+Duration::from_secs(35)).min(end),&cancel,
        |c|c.fields.duration.is_some_and(|d|(d-42.102521).abs()<=1.))?;
    ordinary.quiet()?;host.viewport(app,&ordinary.token,visible(3,0.)).map_err(|_|"QA_ISOLATION_VIEWPORT")?;
    control(app,&ordinary.token,&video,"pause",None)?;
    control(app,&ordinary.token,&video,"seek",Some(5.))?;
    let ordinary_five=wait_clock(app,&ordinary.token,&video,end,&cancel,|c|c.fields.paused&&!c.fields.seeking&&c.fields.current_time.is_some_and(|t|(t-5.).abs()<=0.35))?;
    let exclusive=open_exclusive(app,&video,end)?;
    require(app.webviews().keys().filter(|s|s.starts_with("youtube-original-vod-")).count()==2,"QA_ISOLATION_TWO_DOCUMENTS")?;
    let ec=owned_control(&exclusive,&video,"resume",None)?;
    let moving=wait_owned(&exclusive,&video,end,|c|source_unchanged(&ec,c)&&!c.fields.paused&&!c.fields.buffering
        &&matches!((ec.fields.current_time,c.fields.current_time),(Some(a),Some(b)) if b>a+0.3))?;
    require(held_position(&ordinary_five,&clock(app,&ordinary.token,&video)?,5.),"QA_ISOLATION_OWNED_RESUME_AFFECTED_ORDINARY")?;
    owned_control(&exclusive,&video,"pause",None)?;
    owned_control(&exclusive,&video,"seek",Some(15.))?;
    let owned_fifteen=wait_owned(&exclusive,&video,end,|c|c.fields.paused&&!c.fields.seeking&&c.fields.current_time.is_some_and(|t|(t-15.).abs()<=0.35))?;
    require(source_unchanged(&moving,&owned_fifteen)&&held_position(&ordinary_five,&clock(app,&ordinary.token,&video)?,5.),"QA_ISOLATION_OWNED_SEEK_AFFECTED_ORDINARY")?;
    let oc=control(app,&ordinary.token,&video,"resume",None)?;
    wait_clock(app,&ordinary.token,&video,end,&cancel,|c|source_unchanged(&oc,c)&&!c.fields.paused&&!c.fields.buffering
        &&c.fields.current_time.is_some_and(|t|t>5.3))?;
    require(held_position(&owned_fifteen,&owned_clock(&exclusive,&video)?,15.),"QA_ISOLATION_ORDINARY_RESUME_AFFECTED_OWNED")?;
    control(app,&ordinary.token,&video,"pause",None)?;control(app,&ordinary.token,&video,"seek",Some(25.))?;
    let ordinary_twentyfive=wait_clock(app,&ordinary.token,&video,end,&cancel,|c|c.fields.paused&&!c.fields.seeking&&c.fields.current_time.is_some_and(|t|(t-25.).abs()<=0.35))?;
    require(held_position(&owned_fifteen,&owned_clock(&exclusive,&video)?,15.),"QA_ISOLATION_ORDINARY_SEEK_AFFECTED_OWNED")?;
    exclusive.close().map_err(|_|"QA_ISOLATION_OWNED_CLOSE")?;
    control(app,&ordinary.token,&video,"resume",None)?;
    wait_clock(app,&ordinary.token,&video,end,&cancel,|c|source_unchanged(&ordinary_twentyfive,c)&&!c.fields.paused&&!c.fields.buffering
        &&c.fields.current_time.is_some_and(|t|t>25.3))?;
    control(app,&ordinary.token,&video,"pause",None)?;
    let replacement=open_exclusive(app,&video,end)?;
    owned_control(&replacement,&video,"pause",None)?;owned_control(&replacement,&video,"seek",Some(10.))?;
    let ten=wait_owned(&replacement,&video,end,|c|c.fields.paused&&!c.fields.seeking&&c.fields.current_time.is_some_and(|t|(t-10.).abs()<=0.35))?;
    ordinary.close()?;owned_control(&replacement,&video,"resume",None)?;
    wait_owned(&replacement,&video,end,|c|source_unchanged(&ten,c)&&!c.fields.paused&&!c.fields.buffering&&c.fields.current_time.is_some_and(|t|t>10.3))?;
    replacement.close().map_err(|_|"QA_ISOLATION_OWNED_CLOSE")?;
    let close_end=(Instant::now()+Duration::from_secs(3)).min(end);
    while app.webviews().keys().any(|s|s.starts_with("youtube-original-vod-")){check(close_end,&cancel)?;thread::sleep(Duration::from_millis(25));}
    require(receipt_video(service)?==video,"QA_ISOLATION_RECEIPT_CHANGED")?;
    Ok(json!({"passed":true,"sameVideoSeparateNativeDocuments":true,"ownedResumeSeekDidNotMoveOrdinary":true,
        "ordinaryResumeSeekDidNotMoveOwned":true,"ordinaryProgressedAfterOwnedClose":true,"ownedProgressedAfterOrdinaryClose":true,
        "allQaDocumentsClosed":true,"newUploads":0,"newLiveSources":0,"remoteMutations":0,"metadataVerifiedByThisProbe":false,
        "privateUploadReplayVerified":false,"synchronizedLiveReplayVerified":false,"desktopInput":false}))
}
