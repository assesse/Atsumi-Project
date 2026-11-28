//! Same-frame, selected-document samples for native archive alignment.
//! Pixels are transient RAM only; callers receive compact gradients and clocks.
use super::*;
use crate::cloud_recording::live_fingerprint::{feature, FrameFeature, HEIGHT, WIDTH};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(crate) struct NativeFrame {
    pub media_pts90k: u64,
    pub uncertainty_ticks: u32,
    pub feature: FrameFeature,
}
#[derive(Clone, Debug)]
pub(crate) struct NativeFrameWindow {
    pub capture_id: Uuid,
    pub video_id: String,
    pub document_epoch: u64,
    pub source_id: Uuid,
    pub source_generation: u64,
    pub seek_generation: u64,
    pub frames: Vec<NativeFrame>,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct WireFrame { pts90k:u64, uncertainty_ticks:u32, gray:Vec<u8> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct WireWindow {
    request_id:Uuid, video_id:String, document_epoch:u64, source_id:Uuid,
    source_generation:u64, seek_generation:u64, error:Option<String>, frames:Vec<WireFrame>,
}
fn content(clock:&VodClock)->bool {
    clock.fields.valid && clock.fields.ad_state=="content" && !clock.fields.paused
        && !clock.fields.seeking && !clock.fields.buffering && !clock.fields.ended
}
fn same(first:&VodClock,next:&VodClock)->bool {
    content(next) && first.document_epoch==next.document_epoch
        &&first.fields.video_id==next.fields.video_id&&first.fields.source_id==next.fields.source_id
        &&first.fields.source_generation==next.fields.source_generation
        &&first.fields.seek_generation==next.fields.seek_generation
}
fn call(view:&Webview,script:String)->Result<Value,String> {
    let(tx,rx)=std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(script,move|value|{let _=tx.try_send(value);}).map_err(|_|"remote_frame_document")?;
    let raw=rx.recv_timeout(Duration::from_secs(1)).map_err(|_|"remote_frame_callback")?;
    if raw.len()>160_000{return Err("remote_frame_bound".into());}
    serde_json::from_str(&raw).map_err(|_|"remote_frame_response".into())
}
struct SampleDrop { view:Webview, id:Uuid }
impl Drop for SampleDrop {
    fn drop(&mut self){let _=self.view.eval(format!("window.__atsumiYouTubeVod?.takeFrameSample({},true)",json!(self.id)));}
}
fn decode(value:Value,id:Uuid,first:&VodClock)->Result<NativeFrameWindow,String> {
    let wire:WireWindow=serde_json::from_value(value).map_err(|_|"remote_frame_response")?;
    if let Some(error)=wire.error.as_deref(){return Err(match error {
        "frame_pixels_unavailable"=>"remote_frame_pixels_unavailable",
        "frame_timeout"=>"remote_frame_timeout",
        "frame_source_changed"|"frame_page_closed"=>"remote_frame_source_changed",
        "frame_clock"=>"remote_frame_clock",
        "frame_cancelled"=>"remote_frame_cancelled",
        _=>"remote_frame_unavailable",
    }.into());}
    if wire.request_id!=id||wire.video_id!=first.fields.video_id||wire.document_epoch!=first.document_epoch
        ||wire.source_id.to_string()!=first.fields.source_id||wire.source_generation!=first.fields.source_generation
        ||wire.seek_generation!=first.fields.seek_generation{return Err("remote_frame_source_changed".into());}
    let duration=first.fields.duration.ok_or("remote_frame_clock")?;
    if !(16..=48).contains(&wire.frames.len()){return Err("remote_frame_count".into());}
    let mut frames:Vec<NativeFrame>=Vec::with_capacity(wire.frames.len());
    for raw in wire.frames {
        if raw.gray.len()!=WIDTH*HEIGHT||raw.pts90k>54_432_090_000||!(450..=22500).contains(&raw.uncertainty_ticks)
            ||raw.pts90k as f64/90_000.>duration+1.
            ||frames.last().is_some_and(|last|raw.pts90k<=last.media_pts90k||raw.pts90k-last.media_pts90k>90_000){return Err("remote_frame_clock".into());}
        frames.push(NativeFrame{media_pts90k:raw.pts90k,uncertainty_ticks:raw.uncertainty_ticks,feature:feature(&raw.gray)?});
    }
    let span=frames.last().unwrap().media_pts90k-frames[0].media_pts90k;
    if !(135_000..=270_000).contains(&span){return Err("remote_frame_span".into());}
    Ok(NativeFrameWindow{capture_id:id,video_id:wire.video_id,document_epoch:wire.document_epoch,
        source_id:wire.source_id,source_generation:wire.source_generation,seek_generation:wire.seek_generation,frames})
}
/// Blocking native worker only. No seek, unmute, network or renderer IPC.
pub(super) fn sample(host:&WatchHost,app:&AppHandle,token:&str,video:&str,cancel:&AtomicBool)->Result<NativeFrameWindow,String>{
    let session=host.session(token)?;
    if session.video!=video||session.identity_blocked.load(Ordering::Acquire){return Err("remote_frame_identity".into());}
    if session.frame_busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("remote_frame_busy".into());}
    let _busy=ControlBusy(&session.frame_busy);
    let first=clock_snapshot(&session,token).filter(content).ok_or("remote_frame_not_ready")?;
    if cancel.load(Ordering::Acquire){return Err("remote_frame_cancelled".into());}
    let view=app.get_webview(&session.label).ok_or("remote_frame_document")?;
    let id=Uuid::new_v4();
    let payload=json!({"requestId":id,"documentEpoch":first.document_epoch,"sourceId":first.fields.source_id,
        "sourceGeneration":first.fields.source_generation,"seekGeneration":first.fields.seek_generation,
        "expiresAtMs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|"remote_frame_clock")?.as_millis()+2500});
    let _cleanup=SampleDrop{view:view.clone(),id};
    if call(&view,format!("window.__atsumiYouTubeVod?.beginFrameSample({payload})"))?!=true{return Err("remote_frame_not_ready".into());}
    let deadline=Instant::now()+Duration::from_secs(8);
    loop {
        if cancel.load(Ordering::Acquire){return Err("remote_frame_cancelled".into());}
        if Instant::now()>=deadline{return Err("remote_frame_timeout".into());}
        if !Arc::ptr_eq(&session,&host.session(token)?)||session.identity_blocked.load(Ordering::Acquire)
            ||session.epoch.load(Ordering::Acquire)!=first.document_epoch
            ||!view.url().ok().is_some_and(|url|policy::matches_document(&url,video,false)){return Err("remote_frame_source_changed".into());}
        let current=clock_snapshot(&session,token).ok_or("remote_frame_stale")?;
        if !same(&first,&current){return Err("remote_frame_source_changed".into());}
        let value=call(&view,format!("window.__atsumiYouTubeVod?.takeFrameSample({})",json!(id)))?;
        if value.get("pending")==Some(&Value::Bool(true)) {
            std::thread::sleep(Duration::from_millis(50));continue;
        }
        let next=clock_snapshot(&session,token).ok_or("remote_frame_stale")?;
        if cancel.load(Ordering::Acquire)||Instant::now()>=deadline{return Err("remote_frame_cancelled".into());}
        if !same(&first,&next)||session.closed.load(Ordering::Acquire)||session.identity_blocked.load(Ordering::Acquire)
            ||session.epoch.load(Ordering::Acquire)!=first.document_epoch||!Arc::ptr_eq(&session,&host.session(token)?)
            ||!view.url().ok().is_some_and(|url|policy::matches_document(&url,video,false)){return Err("remote_frame_source_changed".into());}
        return decode(value,id,&first);
    }
}

#[cfg(test)]mod tests{
    use super::*;
    fn fixture()->(Value,Uuid,VodClock){
        let id=Uuid::new_v4();let source=Uuid::new_v4();
        let clock=VodClock{token:"native-test".into(),document_epoch:2,age_ms:0,fields:VodClockFields{video_id:"abcdefghijk".into(),source_id:source.to_string(),source_generation:3,sequence:5,seek_generation:4,current_time:Some(1.),duration:Some(100.),paused:false,seeking:false,playback_rate:1.,buffering:false,ended:false,ad_state:"content".into(),valid:true}};
        let wire=json!({"requestId":id,"videoId":"abcdefghijk","documentEpoch":2,"sourceId":source,"sourceGeneration":3,"seekGeneration":4,"error":null,"frames":(0..25).map(|i|json!({"pts90k":90_000+i*7500,"uncertaintyTicks":4500,"gray":vec![100u8;WIDTH*HEIGHT]})).collect::<Vec<_>>()});(wire,id,clock)
    }
    #[test]fn sample_rejects_crossed_document_seek_and_missing_pixels(){
        let(value,id,clock)=fixture();assert_eq!(decode(value.clone(),id,&clock).unwrap().frames.len(),25);
        for(field,replacement)in[("documentEpoch",json!(3)),("seekGeneration",json!(5)),("sourceId",json!(Uuid::new_v4())),("videoId",json!("other-video")),("error",json!("unavailable"))]{let mut bad=value.clone();bad[field]=replacement;assert!(decode(bad,id,&clock).is_err());}
        let mut bad=value.clone();bad["frames"][1]["pts90k"]=bad["frames"][0]["pts90k"].clone();assert!(decode(bad,id,&clock).is_err());
        let mut bad=value;bad["frames"][0]["gray"]=json!([0]);assert!(decode(bad,id,&clock).is_err());
    }
    #[test]fn content_clock_cannot_cross_ad_buffer_or_seek(){
        let(_,_,first)=fixture();assert!(same(&first,&first));
        let mut changed=first.clone();changed.fields.seeking=true;assert!(!same(&first,&changed));
        changed=first.clone();changed.fields.buffering=true;assert!(!same(&first,&changed));
        changed=first.clone();changed.fields.ad_state="ad".into();assert!(!same(&first,&changed));
        changed=first.clone();changed.fields.source_generation+=1;assert!(!same(&first,&changed));
    }
}
