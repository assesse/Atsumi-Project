//! Opaque public replay tokens own one native player, history lease and worker.
//! Cancellation precedes a real join/drain; no idle replay blocks normal exit.
use super::{CloudService,CloudError,live_replay_session::{LiveReplaySession,Opened,unavailable}};
use std::{collections::{HashMap,VecDeque},sync::{Arc,Mutex,Condvar,atomic::{AtomicBool,Ordering}},thread::JoinHandle};
use tauri::{AppHandle,Listener,Emitter};
use uuid::Uuid;
const MAX_SESSIONS:usize=4;
const MAX_CLOSED:usize=64;
const MAX_CANCELLED_OPENS:usize=64;
#[derive(Clone,PartialEq,Eq)]
struct Request {selection:String,revision:u64,id:String}
struct CancelledOpen {request:Request,observed:bool}
struct State {opening:bool,closing:bool,closed:bool,active:usize,session:Option<Arc<LiveReplaySession>>,workers:Vec<JoinHandle<()>>,error:Option<CloudError>}
struct Slot {request:Request,token:String,cancel:Arc<AtomicBool>,state:Mutex<State>,wake:Condvar}
#[derive(Default)]
struct Entries {closing:bool,sessions:HashMap<String,Arc<Slot>>,closed:VecDeque<(String,Request)>,cancelled_opens:VecDeque<CancelledOpen>,listeners:Option<(AppHandle,Vec<tauri::EventId>)>}
#[derive(Default)]
pub(super) struct LiveReplayRegistry {entries:Mutex<Entries>}
struct Activity {slot:Arc<Slot>,session:Option<Arc<LiveReplaySession>>}
impl Activity {fn session(&self)->&LiveReplaySession {self.session.as_deref().expect("active session")}}
impl Drop for Activity {fn drop(&mut self){
    // Release the session capability BEFORE notifying a waiting closer.
    self.session.take();
    if let Ok(mut state)=self.slot.state.lock(){state.active=state.active.saturating_sub(1);self.slot.wake.notify_all();}
}}
fn canonical_id(value:&str)->bool {Uuid::parse_str(value).is_ok_and(|id|!id.is_nil()&&id.to_string()==value)}
fn request(selection:&str,revision:u64,id:&str)->Result<Request,CloudError>{
    if !canonical_id(selection)||!canonical_id(id){return Err(unavailable());}
    Ok(Request{selection:selection.into(),revision,id:id.into()})
}
impl Entries {
    fn admit_open_request(&mut self,request:&Request)->Result<(),CloudError>{
        if self.closing{return Err(CloudError::cancelled());}
        if let Some((_,closed))=self.closed.iter().find(|(_,r)|r.id==request.id){
            return Err(if closed==request{CloudError::cancelled()}else{unavailable()});
        }
        if let Some(cancelled)=self.cancelled_opens.iter_mut().find(|c|c.request.id==request.id){
            if cancelled.request!=*request{return Err(unavailable());}
            cancelled.observed=true;
            return Err(CloudError::cancelled());
        }
        Ok(())
    }
    fn remember_cancelled_open(&mut self,request:Request)->Result<(),CloudError>{
        if let Some(cancelled)=self.cancelled_opens.iter().find(|c|c.request.id==request.id){
            return if cancelled.request==request{Ok(())}else{Err(unavailable())};
        }
        if self.closing{return Err(CloudError::cancelled());}
        if self.cancelled_opens.len()>=MAX_CANCELLED_OPENS{
            // A delayed IPC has no safe time bound. Never evict its accepted
            // cancellation before the matching open actually observes it.
            let consumed=self.cancelled_opens.iter().position(|c|c.observed).ok_or_else(||
                CloudError::new("CLOUD_LIVE_REPLAY_CANCEL_LIMIT","이전 재생 요청을 마무리 중입니다. 잠시 후 다시 시도해 주세요.",true))?;
            self.cancelled_opens.remove(consumed);
        }
        self.cancelled_opens.push_back(CancelledOpen{request,observed:false});Ok(())
    }
}
impl LiveReplayRegistry {
    fn listeners(self:&Arc<Self>,app:&AppHandle)->Result<(),CloudError>{
        let mut entries=self.entries.lock().map_err(|_|unavailable())?;
        if entries.listeners.is_some(){return Ok(());}
        let mut ids=Vec::new();
        for (event,kind) in [("youtube-owned-vod:close","close"),("youtube-owned-vod:fullscreen","fullscreen")] {
            let weak=Arc::downgrade(self);let app=app.clone();let emitter=app.clone();
            ids.push(app.listen(event,move|event|{
                if event.payload().len()>512{return;}
                let Ok(value)=serde_json::from_str::<serde_json::Value>(event.payload())else{return;};
                let Some(owner)=value.get("owner").and_then(|v|v.as_str()).and_then(|s|Uuid::parse_str(s).ok())else{return;};
                let Some(registry)=weak.upgrade()else{return;};
                let Some((token,video))=registry.public_owner(owner)else{return;};
                if kind=="close"{let _=emitter.emit_to("main","cloud-live-replay:action",serde_json::json!({"token":token,"action":"viewer_close"}));}
                else if let Some(enabled)=value.get("enabled").and_then(|v|v.as_bool()){
                    let _=emitter.emit_to("main","cloud-live-replay:fullscreen",serde_json::json!({"token":token,"channelId":video,"enabled":enabled}));
                }
            }));
        }
        entries.listeners=Some((app.clone(),ids));Ok(())
    }
    fn public_owner(&self,owner:Uuid)->Option<(String,String)>{
        let entries=self.entries.lock().ok()?;
        if entries.closing{return None;}
        for slot in entries.sessions.values(){
            let state=slot.state.lock().ok()?;
            if state.closing||state.closed{continue;}
            if let Some(session)=state.session.as_ref().filter(|s|s.owner()==owner){
                return Some((slot.token.clone(),session.opened.video_id.clone()));
            }
        }
        None
    }
    pub(super) fn open(self:&Arc<Self>,app:&AppHandle,service:&Arc<CloudService>,selection:&str,revision:u64,id:&str)->Result<Opened,CloudError>{
        let request=request(selection,revision,id)?;self.listeners(app)?;
        let slot={
            let mut entries=self.entries.lock().map_err(|_|unavailable())?;
            entries.admit_open_request(&request)?;
            if let Some(slot)=entries.sessions.values().find(|s|s.request.id==request.id){
                if slot.request!=request{return Err(unavailable());}
                let state=slot.state.lock().map_err(|_|unavailable())?;
                if state.closing||state.closed||slot.cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
                return state.session.as_ref().map(|s|s.opened.clone()).ok_or_else(||CloudError::new("CLOUD_LIVE_REPLAY_OPENING","재생 창을 준비 중입니다.",true));
            }
            if entries.sessions.len()>=MAX_SESSIONS{return Err(CloudError::new("CLOUD_LIVE_REPLAY_LIMIT","재생 창을 닫은 뒤 다시 열어 주세요.",false));}
            let token=Uuid::new_v4().to_string();
            let slot=Arc::new(Slot{request,token:token.clone(),cancel:Arc::new(AtomicBool::new(false)),
                state:Mutex::new(State{opening:true,closing:false,closed:false,active:0,session:None,workers:Vec::new(),error:None}),wake:Condvar::new()});
            entries.sessions.insert(token,slot.clone());slot
        };
        // Opening is tracked separately, including failure and cancellation.
        let result=LiveReplaySession::open(app,service,selection,revision,slot.token.clone(),slot.cancel.clone());
        let published=(||{
            let session=Arc::new(result?);
            let mut state=slot.state.lock().map_err(|_|unavailable())?;
            if state.closing||slot.cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
            let worker_session=session.clone();
            let worker=std::thread::Builder::new().name("atsumi-live-replay-index".into()).spawn(move||worker_session.build_index()).map_err(|_|unavailable())?;
            let opened=session.opened.clone();state.workers.push(worker);state.session=Some(session);Ok(opened)
        })();
        {let mut state=slot.state.lock().map_err(|_|unavailable())?;state.opening=false;slot.wake.notify_all();}
        if published.is_err(){let _=self.close(&slot.token);}
        published
    }
    pub(super) fn cancel_open(&self,selection:&str,revision:u64,id:&str)->Result<(),CloudError>{
        let request=request(selection,revision,id)?;
        let token={
            let mut entries=self.entries.lock().map_err(|_|unavailable())?;
            if let Some(slot)=entries.sessions.values().find(|s|s.request.id==request.id){
                if slot.request!=request{return Err(unavailable());}
                // Raise the same flag used by history preparation before
                // waiting for opening/worker/command drain in normal close.
                slot.cancel.store(true,Ordering::Release);slot.token.clone()
            }else if let Some((_,closed))=entries.closed.iter().find(|(_,r)|r.id==request.id){
                return if *closed==request{Ok(())}else{Err(unavailable())};
            }else{return entries.remember_cancelled_open(request);}
        };
        self.close(&token)
    }
    fn begin(&self,token:&str)->Result<Activity,CloudError>{
        if !canonical_id(token){return Err(unavailable());}
        let slot={let entries=self.entries.lock().map_err(|_|unavailable())?;
            if entries.closing{return Err(CloudError::cancelled());}
            entries.sessions.get(token).cloned().ok_or_else(unavailable)?};
        let mut state=slot.state.lock().map_err(|_|unavailable())?;
        if state.opening||state.closing||state.closed||slot.cancel.load(Ordering::Acquire){return Err(CloudError::cancelled());}
        let session=state.session.clone().ok_or_else(unavailable)?;
        state.active=state.active.checked_add(1).ok_or_else(unavailable)?;drop(state);
        Ok(Activity{slot,session:Some(session)})
    }
    pub(super) fn with<T>(&self,token:&str,action:impl FnOnce(&LiveReplaySession)->Result<T,CloudError>)->Result<T,CloudError>{
        let active=self.begin(token)?;let result=action(active.session())?;active.session().valid()?;Ok(result)
    }
    pub(super) fn close(&self,token:&str)->Result<(),CloudError>{
        if !canonical_id(token){return Err(unavailable());}
        let slot={let entries=self.entries.lock().map_err(|_|unavailable())?;
            match entries.sessions.get(token){Some(s)=>s.clone(),None if entries.closed.iter().any(|(t,_)|t==token)=>return Ok(()),None=>return Err(unavailable())}};
        slot.cancel.store(true,Ordering::Release);
        let worker={
            let mut state=slot.state.lock().map_err(|_|unavailable())?;
            if let Some(session)=&state.session{session.measure_cancel.store(true,Ordering::Release);session.cancel_history();}
            if state.closing {
                while !state.closed{state=slot.wake.wait(state).map_err(|_|unavailable())?;}
                return state.error.clone().map_or(Ok(()),Err);
            }
            state.closing=true;
            while state.opening{state=slot.wake.wait(state).map_err(|_|unavailable())?;}
            std::mem::take(&mut state.workers)
        };
        let mut failure=None;for worker in worker{if worker.join().is_err(){failure=Some(unavailable());}}
        let session={let mut state=slot.state.lock().map_err(|_|unavailable())?;
            while state.active>0{state=slot.wake.wait(state).map_err(|_|unavailable())?;}state.session.take()};
        if let Some(session)=session{
            match Arc::try_unwrap(session){Ok(session)=>{if let Err(e)=session.close(){failure=Some(e);}},Err(session)=>{session.force_close();failure=Some(unavailable());}}
        }
        {let mut state=slot.state.lock().map_err(|_|unavailable())?;state.error=failure.clone();state.closed=true;slot.wake.notify_all();}
        {let mut entries=self.entries.lock().map_err(|_|unavailable())?;
            if entries.sessions.get(token).is_some_and(|s|Arc::ptr_eq(s,&slot)){entries.sessions.remove(token);entries.closed.push_back((token.into(),slot.request.clone()));while entries.closed.len()>MAX_CLOSED{entries.closed.pop_front();}}}
        failure.map_or(Ok(()),Err)
    }
    pub(super) fn measure(&self,token:&str)->Result<super::live_replay_session::Snapshot,CloudError>{
        let active=self.begin(token)?;
        let session=active.session.as_ref().ok_or_else(unavailable)?.clone();
        session.valid()?;
        if session.measuring.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return session.snapshot();}
        let mut state=active.slot.state.lock().map_err(|_|unavailable())?;
        if state.closing||active.slot.cancel.load(Ordering::Acquire){session.measuring.store(false,Ordering::Release);return Err(CloudError::cancelled());}
        let mut i=0;
        while i<state.workers.len(){
            if state.workers[i].is_finished(){
                if state.workers.swap_remove(i).join().is_err(){session.measuring.store(false,Ordering::Release);return Err(unavailable());}
            }else{i+=1;}
        }
        // Index worker plus at most a few finishing measurement threads. A
        // repeated failed request cannot retain an unbounded handle list.
        if state.workers.len()>=4{session.measuring.store(false,Ordering::Release);return Err(CloudError::new("CLOUD_LIVE_REPLAY_BUSY","이전 확인 작업을 마무리 중입니다.",true));}
        session.measure_cancel.store(false,Ordering::Release);
        *session.measure_error.lock().map_err(|_|unavailable())?=None;
        let worker_session=session.clone();
        match std::thread::Builder::new().name("atsumi-live-replay-measure".into()).spawn(move||worker_session.measure_worker()){
            Ok(worker)=>state.workers.push(worker),
            Err(_)=>{session.measuring.store(false,Ordering::Release);return Err(unavailable());}
        }
        drop(state);session.snapshot()
    }
    pub(super) fn active_ids(&self)->Vec<String>{
        let Ok(entries)=self.entries.lock()else{return vec!["live-replay:status-unavailable".into()];};
        entries.sessions.values().filter_map(|s|s.state.lock().ok().and_then(|v|
            (v.opening||v.active>0||v.workers.iter().any(|w|!w.is_finished())||(v.closing&&!v.closed)).then(||format!("live-replay:{}",s.token)))).collect()
    }
    pub(super) fn shutdown(&self)->Result<(),CloudError>{
        let (tokens,listeners)={let mut entries=self.entries.lock().map_err(|_|unavailable())?;entries.closing=true;
            for slot in entries.sessions.values(){slot.cancel.store(true,Ordering::Release);if let Ok(state)=slot.state.lock(){if let Some(session)=&state.session{session.measure_cancel.store(true,Ordering::Release);session.cancel_history();}}}
            (entries.sessions.keys().cloned().collect::<Vec<_>>(),entries.listeners.take())};
        if let Some((app,ids))=listeners{for id in ids{app.unlisten(id);}}
        let mut failure=None;for token in tokens{if let Err(e)=self.close(&token){failure=Some(e);}}failure.map_or(Ok(()),Err)
    }
}
impl CloudService {
    pub(super) fn live_replays(&self)->Result<Arc<LiveReplayRegistry>,CloudError>{Ok(self.lock()?.live.replays.clone())}
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn request_identity_is_exact_and_tokens_cannot_be_paths(){
        let selection=Uuid::new_v4().to_string();let id=Uuid::new_v4().to_string();
        let a=request(&selection,1,&id).unwrap();assert!(a!=request(&selection,2,&id).unwrap());
        assert!(a!=request(&Uuid::new_v4().to_string(),1,&id).unwrap());
        assert!(request("../catalog",1,&id).is_err());assert!(request(&selection,1,&Uuid::nil().to_string()).is_err());
    }
    #[test]fn cancel_before_open_is_exact_idempotent_and_consumed_only_by_matching_open(){
        let host=LiveReplayRegistry::default();
        let req=request(&Uuid::new_v4().to_string(),7,&Uuid::new_v4().to_string()).unwrap();
        for _ in 0..2{host.cancel_open(&req.selection,req.revision,&req.id).unwrap();}
        assert!(host.cancel_open(&req.selection,8,&req.id).is_err());
        assert!(host.cancel_open(&Uuid::new_v4().to_string(),7,&req.id).is_err());
        let mut entries=host.entries.lock().unwrap();
        assert_eq!(entries.cancelled_opens.len(),1);assert!(entries.sessions.is_empty());
        let mut changed=req.clone();changed.revision+=1;
        assert!(entries.admit_open_request(&changed).is_err());
        assert!(!entries.cancelled_opens[0].observed);
        changed=req.clone();changed.selection=Uuid::new_v4().to_string();
        assert!(entries.admit_open_request(&changed).is_err());
        assert!(!entries.cancelled_opens[0].observed);
        for _ in 0..2{assert_eq!(entries.admit_open_request(&req).unwrap_err().code,CloudError::cancelled().code);}
        assert!(entries.cancelled_opens[0].observed);assert!(entries.sessions.is_empty());
        let unrelated=request(&req.selection,req.revision,&Uuid::new_v4().to_string()).unwrap();
        assert!(entries.admit_open_request(&unrelated).is_ok());
    }
    #[test]fn unconsumed_open_cancellations_are_not_evicted_at_capacity(){
        let host=LiveReplayRegistry::default();let selection=Uuid::new_v4().to_string();
        let requests=(0..MAX_CANCELLED_OPENS).map(|_|request(&selection,1,&Uuid::new_v4().to_string()).unwrap()).collect::<Vec<_>>();
        for req in &requests{host.cancel_open(&req.selection,req.revision,&req.id).unwrap();}
        let extra=request(&selection,1,&Uuid::new_v4().to_string()).unwrap();
        assert_eq!(host.cancel_open(&extra.selection,extra.revision,&extra.id).unwrap_err().code,"CLOUD_LIVE_REPLAY_CANCEL_LIMIT");
        host.cancel_open(&requests[0].selection,1,&requests[0].id).unwrap();
        {
            let mut entries=host.entries.lock().unwrap();assert_eq!(entries.cancelled_opens.len(),MAX_CANCELLED_OPENS);
            assert!(entries.cancelled_opens.iter().all(|c|!c.observed));
            assert_eq!(entries.admit_open_request(&requests[0]).unwrap_err().code,CloudError::cancelled().code);
        }
        host.cancel_open(&extra.selection,extra.revision,&extra.id).unwrap();
        let mut entries=host.entries.lock().unwrap();assert_eq!(entries.cancelled_opens.len(),MAX_CANCELLED_OPENS);
        assert!(!entries.cancelled_opens.iter().any(|c|c.request==requests[0]));
        for req in requests.iter().skip(1).chain(std::iter::once(&extra)){
            assert_eq!(entries.admit_open_request(req).unwrap_err().code,CloudError::cancelled().code);
        }
    }
    #[test]fn cancel_open_raises_flag_before_open_join_and_preserves_other_slot(){
        use std::{sync::mpsc,time::{Duration,Instant}};
        let host=Arc::new(LiveReplayRegistry::default());
        let req=request(&Uuid::new_v4().to_string(),7,&Uuid::new_v4().to_string()).unwrap();
        let token=Uuid::new_v4().to_string();let other_token=Uuid::new_v4().to_string();
        let (release,wait)=mpsc::channel();let worker=std::thread::spawn(move||{wait.recv().unwrap();});
        let slot=Arc::new(Slot{request:req.clone(),token:token.clone(),cancel:Arc::new(AtomicBool::new(false)),
            state:Mutex::new(State{opening:true,closing:false,closed:false,active:1,session:None,workers:vec![worker],error:None}),wake:Condvar::new()});
        let other=Arc::new(Slot{request:request(&req.selection,req.revision,&Uuid::new_v4().to_string()).unwrap(),token:other_token.clone(),cancel:Arc::new(AtomicBool::new(false)),
            state:Mutex::new(State{opening:false,closing:false,closed:false,active:0,session:None,workers:vec![],error:None}),wake:Condvar::new()});
        {let mut entries=host.entries.lock().unwrap();entries.sessions.insert(token.clone(),slot.clone());entries.sessions.insert(other_token.clone(),other.clone());}
        assert!(host.cancel_open(&req.selection,req.revision+1,&req.id).is_err());
        assert!(host.cancel_open(&Uuid::new_v4().to_string(),req.revision,&req.id).is_err());
        assert!(!slot.cancel.load(Ordering::Acquire));
        let activity=Activity{slot:slot.clone(),session:None};
        let (done,receive)=mpsc::channel();let closer=host.clone();let copied=req.clone();
        let join=std::thread::spawn(move||{done.send(closer.cancel_open(&copied.selection,copied.revision,&copied.id)).unwrap();});
        let until=Instant::now()+Duration::from_secs(2);
        while !slot.cancel.load(Ordering::Acquire){assert!(Instant::now()<until);std::thread::yield_now();}
        assert!(receive.recv_timeout(Duration::from_millis(25)).is_err());
        {let mut state=slot.state.lock().unwrap();state.opening=false;slot.wake.notify_all();}
        assert!(receive.recv_timeout(Duration::from_millis(25)).is_err());
        release.send(()).unwrap();assert!(receive.recv_timeout(Duration::from_millis(25)).is_err());
        drop(activity);assert!(receive.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());join.join().unwrap();
        assert!(!other.cancel.load(Ordering::Acquire));assert!(host.entries.lock().unwrap().sessions.contains_key(&other_token));
        host.cancel_open(&req.selection,req.revision,&req.id).unwrap();
        assert!(host.cancel_open(&req.selection,req.revision+1,&req.id).is_err());
        {let mut entries=host.entries.lock().unwrap();assert_eq!(entries.admit_open_request(&req).unwrap_err().code,CloudError::cancelled().code);
            let mut wrong=req.clone();wrong.selection=Uuid::new_v4().to_string();assert!(entries.admit_open_request(&wrong).is_err());}
        host.close(&other_token).unwrap();host.shutdown().unwrap();
        host.cancel_open(&req.selection,req.revision,&req.id).unwrap();
        assert!(host.cancel_open(&req.selection,req.revision,&Uuid::new_v4().to_string()).is_err());
    }
    #[test]fn empty_shutdown_and_closed_token_are_idempotent(){
        let host=LiveReplayRegistry::default();let token=Uuid::new_v4().to_string();
        let req=request(&Uuid::new_v4().to_string(),1,&Uuid::new_v4().to_string()).unwrap();
        host.entries.lock().unwrap().closed.push_back((token.clone(),req));
        assert!(host.close(&token).is_ok());assert!(host.shutdown().is_ok());assert!(host.shutdown().is_ok());assert!(host.active_ids().is_empty());
        assert!(host.close(&Uuid::new_v4().to_string()).is_err());
    }
    #[test]fn close_waits_for_actual_worker_join_and_command_drop(){
        use std::{sync::mpsc,time::Duration};
        let host=Arc::new(LiveReplayRegistry::default());let token=Uuid::new_v4().to_string();
        let (release,wait)=mpsc::channel();let worker=std::thread::spawn(move||{wait.recv().unwrap();});
        let slot=Arc::new(Slot{request:request(&Uuid::new_v4().to_string(),1,&Uuid::new_v4().to_string()).unwrap(),token:token.clone(),
            cancel:Arc::new(AtomicBool::new(false)),state:Mutex::new(State{opening:false,closing:false,closed:false,active:1,session:None,workers:vec![worker],error:None}),wake:Condvar::new()});
        host.entries.lock().unwrap().sessions.insert(token.clone(),slot.clone());
        let activity=Activity{slot:slot.clone(),session:None};
        let (done,receive)=mpsc::channel();let closer=host.clone();let closed_token=token.clone();
        let join=std::thread::spawn(move||{done.send(closer.close(&closed_token)).unwrap();});
        let until=std::time::Instant::now()+Duration::from_secs(2);
        while !slot.cancel.load(Ordering::Acquire){assert!(std::time::Instant::now()<until);std::thread::yield_now();}
        assert!(receive.recv_timeout(Duration::from_millis(25)).is_err());
        release.send(()).unwrap();
        assert!(receive.recv_timeout(Duration::from_millis(25)).is_err());
        drop(activity);assert!(receive.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());join.join().unwrap();
        assert!(host.active_ids().is_empty());assert!(host.close(&token).is_ok());
    }
}

