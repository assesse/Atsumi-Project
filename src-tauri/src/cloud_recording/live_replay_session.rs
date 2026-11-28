//! A native selected archive owns its official player and immutable history.
//! No renderer-provided frames, source paths, offsets or timing proofs enter here.
use super::{CloudService, CloudError, live_archive::{ArchiveState,Identity},
    live_alignment::{self,AlignmentBinding,MeasuredAlignment,RemoteFrame,RemoteWindow},
    live_sidecar::{LiveHistoryLease,LiveHistoryFacts}};
use crate::youtube::original::{OwnedVodLease,NativeFrameWindow,YouTubeOriginalHost,VodClock,
    VodControlRequest,OriginalSnapshot};
use crate::platform_native_geometry::NativeViewport;
use serde::Serialize;
use std::sync::{Arc,Weak,Mutex,atomic::{AtomicBool,Ordering}};
use tauri::{AppHandle,Manager};
use uuid::Uuid;

pub(super) fn unavailable()->CloudError {CloudError::new("CLOUD_LIVE_REPLAY","보관 영상과 원본 기록의 연결을 확인하지 못했습니다.",false)}
fn waiting()->CloudError {CloudError::new("CLOUD_LIVE_REPLAY_PROCESSING","YouTube에서 영상을 처리 중입니다.",true)}
#[derive(Clone)]
struct BoundArchive {identity:Identity,proof:String,duration:f64}
fn selected(service:&CloudService,id:&str,revision:Option<u64>)->Result<BoundArchive,CloudError>{
    let inner=service.lock()?;
    if inner.closing{return Err(CloudError::cancelled());}
    let r=inner.catalog.live_receipts.iter().find(|r|r.view.selection_id==id&&r.valid()).ok_or_else(unavailable)?;
    if revision.is_some_and(|v|v!=r.view.revision){return Err(unavailable());}
    let identity=r.archive_identity().ok_or_else(unavailable)?;
    let record=r.archive.as_ref().filter(|a|a.valid()&&a.identity==identity).ok_or_else(waiting)?;
    if record.state!=ArchiveState::MetadataVerified{return Err(waiting());}
    let snapshot=record.snapshot();
    if snapshot.video_id.as_deref()!=Some(identity.broadcast_id.as_str())||snapshot.embeddable!=Some(true){return Err(unavailable());}
    let duration=snapshot.duration_seconds.filter(|d|d.is_finite()&&*d>0.&&*d<=604800.).ok_or_else(unavailable)?;
    Ok(BoundArchive{identity,proof:record.verified.as_ref().ok_or_else(unavailable)?.sha256.clone(),duration})
}
#[derive(Clone,Serialize)]
#[serde(rename_all="camelCase")]
pub struct Opened {pub token:String,pub selection_id:String,pub provider:String,pub video_id:String,pub duration_seconds:f64,pub player:OriginalSnapshot}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Snapshot {pub token:String,pub selection_id:String,pub provider:String,pub video_id:String,
    pub duration_seconds:f64,pub player:OriginalSnapshot,pub clock:Option<VodClock>,pub alignment_state:String,pub history_state:String,
    pub history_items:usize,pub untimed_items:usize,pub history_incomplete:bool,pub error:Option<CloudError>}
#[derive(Clone,Serialize)]
#[serde(rename_all="camelCase")]
pub struct RemoteInterval {pub lower_ticks90k:u64,pub upper_ticks90k:u64,
    pub presentation_uncertainty:&'static str,pub measurement:&'static str}
#[derive(Clone)]
pub(super) struct SourcePoint {pub transport90k:u64,pub numeric_uncertainty:u32}
pub(super) struct Row {pub id:String,pub value:serde_json::Value,pub source:Option<SourcePoint>,pub reason:Option<String>}
#[derive(Default)]
pub(super) struct Index {pub store:Option<super::live_replay_disk_index::DiskIndex>,pub ready:bool,pub partial:bool,
    pub error:Option<CloudError>}
pub(crate) struct LiveReplaySession {
    service:Weak<CloudService>,selected:BoundArchive,pub(super) history:Arc<LiveHistoryLease>,
    player:OwnedVodLease,pub(super) cancel:Arc<AtomicBool>,alignment:Mutex<Option<MeasuredAlignment>>,
    pub(super) index:Mutex<Index>,pub(super) opened:Opened,
    pub(super) measuring:AtomicBool,pub(super) measure_cancel:Arc<AtomicBool>,pub(super) measure_error:Mutex<Option<CloudError>>,
}
impl LiveReplaySession {
    pub(super) fn open(app:&AppHandle,service:&Arc<CloudService>,id:&str,revision:u64,
        token:String,cancel:Arc<AtomicBool>)->Result<Self,CloudError>{
        let bound=selected(service,id,Some(revision))?;
        if cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
        let history=Arc::new(service.open_live_history_cancellable(id,cancel.clone())?);
        let facts=history.facts();
        if !matches!(facts.provider.as_str(),"soop"|"chzzk"){return Err(unavailable());}
        // Native first-open sealing may advance the receipt revision. The UI
        // revision was checked before that preparation; immutable ownership and
        // archive proof must still match exactly after it.
        let again=selected(service,id,None)?;
        if again.identity!=bound.identity||again.proof!=bound.proof||again.duration!=bound.duration{return Err(unavailable());}
        if cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
        let player=app.try_state::<YouTubeOriginalHost>().ok_or_else(unavailable)?
            .open_owned_vod(app,bound.identity.broadcast_id.clone(),Uuid::new_v4()).map_err(|_|unavailable())?;
        let mut snapshot=player.snapshot().map_err(|_|unavailable())?;snapshot.token=token.clone();
        let opened=Opened{token,selection_id:id.into(),provider:facts.provider,video_id:bound.identity.broadcast_id.clone(),duration_seconds:bound.duration,player:snapshot};
        let value=Self{service:Arc::downgrade(service),selected:bound,history,player,cancel,
            alignment:Mutex::new(None),index:Mutex::new(Index::default()),opened,measuring:AtomicBool::new(false),measure_cancel:Arc::new(AtomicBool::new(false)),measure_error:Mutex::new(None)};
        value.valid()?;Ok(value)
    }
    pub(super) fn valid(&self)->Result<(),CloudError>{
        if self.cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
        let service=self.service.upgrade().ok_or_else(unavailable)?;
        let current=selected(&service,&self.selected.identity.selection_id,None)?;
        if current.identity!=self.selected.identity||current.proof!=self.selected.proof||current.duration!=self.selected.duration{return Err(unavailable());}
        Ok(())
    }
    pub(super) fn owner(&self)->Uuid {self.player.owner_id()}
    pub(super) fn facts(&self)->LiveHistoryFacts {self.history.facts()}
    pub(super) fn clock(&self)->Result<Option<VodClock>,CloudError>{
        self.valid()?;
        let mut clock=self.player.clock().map_err(|_|unavailable())?;
        if let Some(c)=clock.as_mut(){c.token=self.opened.token.clone();}
        Ok(clock)
    }
    pub(super) fn content_clock(&self)->Result<Option<VodClock>,CloudError>{
        Ok(self.clock()?.filter(|c|c.age_ms<=1500&&c.fields.valid&&c.fields.ad_state=="content"
            &&c.fields.video_id==self.selected.identity.broadcast_id&&c.fields.duration.is_some_and(|d|
                d.is_finite()&&(d-self.selected.duration).abs()<=1.)))
    }
    pub(super) fn snapshot(&self)->Result<Snapshot,CloudError>{
        self.valid()?;let mut player=self.player.snapshot().map_err(|_|unavailable())?;
        player.token=self.opened.token.clone();let clock=self.clock()?;
        let aligned=self.current_alignment(clock.as_ref())?;
        let mut index=self.index.lock().map_err(|_|unavailable())?;
        let (count,untimed)=match index.store.as_mut(){Some(store)=>store.counts(aligned.as_ref(),self.opened.duration_seconds)?,None=>(0,0)};
        let result=Snapshot{token:self.opened.token.clone(),selection_id:self.opened.selection_id.clone(),provider:self.opened.provider.clone(),
            video_id:self.opened.video_id.clone(),duration_seconds:self.opened.duration_seconds,player,clock,alignment_state:if self.measuring.load(Ordering::Acquire){"measuring"}else if aligned.is_some(){"measured"}else if self.measure_error.lock().map_err(|_|unavailable())?.is_some(){"failed"}else{"unknown"}.into(),
            history_state:if index.partial{"partial"}else if !index.ready{"preparing"}else{"ready"}.into(),
            history_items:count,untimed_items:untimed,history_incomplete:index.partial||self.facts().history_incomplete,error:self.measure_error.lock().map_err(|_|unavailable())?.clone().or_else(||index.error.clone())};
        drop(index);self.valid()?;
        let after=self.clock()?;let after_alignment=self.current_alignment(after.as_ref())?;
        let generation=|c:&VodClock|(c.document_epoch,c.fields.source_id.clone(),c.fields.source_generation,c.fields.seek_generation);
        if result.clock.as_ref().map(generation)!=after.as_ref().map(generation)
            ||aligned.as_ref().map(|a|(&a.binding,a.offset))!=after_alignment.as_ref().map(|a|(&a.binding,a.offset)){
            return Err(CloudError::new("CLOUD_LIVE_REPLAY_CHANGED","재생 위치가 바뀌었습니다.",true));
        }
        Ok(result)
    }
    pub(super) fn viewport(&self,value:NativeViewport)->Result<(),CloudError>{self.valid()?;if !value.visible||value.muted{self.measure_cancel.store(true,Ordering::Release);}self.player.viewport(value).map_err(|_|unavailable())}
    pub(super) fn control(&self,value:VodControlRequest)->Result<(),CloudError>{self.valid()?;self.measure_cancel.store(true,Ordering::Release);self.player.control(value).map_err(|_|unavailable())}
    pub(super) fn fullscreen(&self,value:bool)->Result<(),CloudError>{self.valid()?;self.player.fullscreen(value).map_err(|_|unavailable())}
    pub(super) fn controls(&self,value:bool)->Result<(),CloudError>{self.valid()?;self.player.controls(value).map_err(|_|unavailable())}
    pub(super) fn asset(&self,id:&str)->Result<Option<String>,CloudError>{self.valid()?;let value=self.history.asset(id)?;self.valid()?;Ok(value)}
    // Native coordinator-only. No command accepts observations or offsets.
    pub(super) fn sample(&self)->Result<NativeFrameWindow,CloudError>{
        self.valid()?;let value=self.player.sample_frames(&self.measure_cancel).map_err(|message|{
            let code=if message.starts_with("remote_frame_")&&message.len()<80&&message.bytes().all(|b|b.is_ascii_lowercase()||b==b'_'){format!("CLOUD_LIVE_{}",message.to_ascii_uppercase())}else{"CLOUD_LIVE_FRAME_UNAVAILABLE".into()};
            CloudError::new(&code,"공식 플레이어에서 동기화용 장면을 확인하지 못했습니다.",false)})?;self.valid()?;Ok(value)
    }
    pub(super) fn measure(&self,windows:Vec<NativeFrameWindow>)->Result<(),CloudError>{
        *self.alignment.lock().map_err(|_|unavailable())?=None;
        let clock=self.content_clock()?.ok_or_else(unavailable)?;
        let reference=self.history.references()?.ok_or_else(unavailable)?;
        let binding=AlignmentBinding{run_id:reference.references().run_id,source_id:reference.references().source_id,
            stream_id:reference.references().stream_id.ok_or_else(unavailable)?,video_id:self.opened.video_id.clone(),
            remote_source_id:Uuid::parse_str(&clock.fields.source_id).map_err(|_|unavailable())?,
            document_epoch:clock.document_epoch,source_generation:clock.fields.source_generation};
        if !(2..=3).contains(&windows.len()){return Err(unavailable());}
        let mut observed=Vec::with_capacity(windows.len());
        for w in windows {
            if !self.player.owns_capture(&w){return Err(unavailable());}
            observed.push(RemoteWindow{capture_id:w.capture_id,video_id:w.video_id,document_epoch:w.document_epoch,
                source_id:w.source_id,source_generation:w.source_generation,seek_generation:w.seek_generation,
                ad_observed:false,seeking_observed:false,buffering_observed:false,
                frames:w.frames.into_iter().map(|f|RemoteFrame{media_pts90k:f.media_pts90k,
                    uncertainty_ticks:f.uncertainty_ticks,feature:f.feature}).collect()});
        }
        let measured=live_alignment::align(reference.references(),reference.timing(),&binding,&observed).map_err(|_|unavailable())?;
        let after=self.content_clock()?.ok_or_else(unavailable)?;
        if !binding_matches(&measured,&after){return Err(unavailable());}
        self.valid()?;*self.alignment.lock().map_err(|_|unavailable())?=Some(measured);Ok(())
    }
    pub(super) fn current_alignment(&self,clock:Option<&VodClock>)->Result<Option<MeasuredAlignment>,CloudError>{
        let mut alignment=self.alignment.lock().map_err(|_|unavailable())?;
        let Some(measured)=alignment.as_ref()else{return Ok(None);};
        let Some(clock)=clock else{return Ok(None);};
        if !binding_matches(measured,clock){*alignment=None;return Ok(None);}
        if clock.fields.duration.is_none_or(|d|!d.is_finite()||(d-self.selected.duration).abs()>1.){*alignment=None;return Ok(None);}
        if clock.age_ms>1500||!clock.fields.valid||clock.fields.ad_state!="content"||clock.fields.seeking||clock.fields.buffering{return Ok(None);}
        Ok(Some(measured.clone()))
    }
    pub(super) fn clear_alignment(&self){if let Ok(mut value)=self.alignment.lock(){*value=None;}}
    pub(super) fn cancel_history(&self){self.cancel.store(true,Ordering::Release);self.history.cancel();}
    pub(super) fn force_close(&self){self.cancel_history();self.measure_cancel.store(true,Ordering::Release);self.player.cancel();}
    pub(super) fn close(self)->Result<(),CloudError>{self.cancel_history();self.player.close().map_err(|_|unavailable())}
}
fn binding_matches(measured:&MeasuredAlignment,clock:&VodClock)->bool{
    measured.binding.video_id==clock.fields.video_id&&measured.binding.document_epoch==clock.document_epoch
        &&measured.binding.remote_source_id.to_string()==clock.fields.source_id&&measured.binding.source_generation==clock.fields.source_generation
}
pub(super) fn project_interval(t:u64,n:u32,m:&MeasuredAlignment,duration:f64)->Result<Option<RemoteInterval>,CloudError>{
    project_bounds(t,n,m.offset.lower,m.offset.upper,duration,m.two_window_estimate)
}
fn project_bounds(t:u64,n:u32,offset_lower:i64,offset_upper:i64,duration:f64,two:bool)->Result<Option<RemoteInterval>,CloudError>{
    let lo=i128::from(t)-i128::from(n)-i128::from(offset_upper);
    let hi=i128::from(t)+i128::from(n)-i128::from(offset_lower);
    let end=(duration*90_000.).floor();
    if !end.is_finite()||end<0.||end>u64::MAX as f64||offset_lower>offset_upper{return Err(unavailable());}
    if lo<0||hi<lo||hi>i128::from(end as u64){return Ok(None);}
    Ok(Some(RemoteInterval{lower_ticks90k:lo as u64,upper_ticks90k:hi as u64,
        presentation_uncertainty:"unknown",measurement:if two{"two_window_estimate"}else{"visual_observation"}}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn projection_expands_negative_offsets_without_clamping(){
        let p=project_bounds(90000,2,-9000,-4500,10.,false).unwrap().unwrap();
        assert_eq!((p.lower_ticks90k,p.upper_ticks90k),(94498,99002));assert_eq!(p.presentation_uncertainty,"unknown");
        assert!(project_bounds(1,2,0,0,10.,false).unwrap().is_none());
        assert!(project_bounds(900000,2,0,0,10.,false).unwrap().is_none());
        assert!(project_bounds(90000,2,5,4,10.,false).is_err());
        assert!(project_bounds(90000,2,0,0,f64::NAN,false).is_err());
    }
}



