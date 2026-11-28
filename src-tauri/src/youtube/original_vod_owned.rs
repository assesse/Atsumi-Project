//! One native-only VOD document. A private registry grants the lease access;
//! the public registry only counts/drains its Arc<Session>, never its token.
use super::*;
use uuid::Uuid;

/// No Clone/Deserialize or exposed raw token. Dropping/closing this lease only
/// disposes its exact document. The caller still joins its own worker threads.
pub(crate) struct OwnedVodLease {
    app:AppHandle,host:YouTubeOriginalHost,private:WatchHost,session:Arc<Session>,
    owner:Uuid,cancelled:AtomicBool,captures:Mutex<Vec<NativeFrameWindow>>,
}
impl WatchHost {
    /// Parent holds admission and enforces the combined four-document bound.
    pub(crate) fn open_owned(&self,app:&AppHandle,host:YouTubeOriginalHost,video:String,owner:Uuid)->Result<OwnedVodLease,String>{
        if owner.is_nil()||lock(&self.registry).sessions.values().any(|s|s.owned_by==Some(owner)){return Err(STALE.into());}
        let private=WatchHost::default();
        let opened=private.open_inner(app,host.clone(),video,Uuid::new_v4().to_string(),true,Some(owner))?;
        let session=private.session(&opened.token)?;
        // Only the document is published globally. Public operations require a
        // token mapping, which deliberately exists solely in the private host.
        lock(&self.registry).sessions.insert(session.token.clone(),session.clone());
        Ok(OwnedVodLease{app:app.clone(),host,private,session,owner,cancelled:AtomicBool::new(false),captures:Mutex::new(Vec::new())})
    }
}
impl OwnedVodLease {
    pub(crate) fn owner_id(&self)->Uuid{self.owner}
    fn valid(&self)->Result<(),String>{
        if self.cancelled.load(Ordering::Acquire)||self.host.closing.load(Ordering::Acquire)
            ||self.session.closed.load(Ordering::Acquire)||self.session.identity_blocked.load(Ordering::Acquire)
            ||self.session.owned_by!=Some(self.owner){return Err(STALE.into());}
        let global=lock(&self.host.watch_only.registry);
        if !global.sessions.get(&self.session.token).is_some_and(|s|Arc::ptr_eq(s,&self.session)){return Err(STALE.into());}
        Ok(())
    }
    pub(crate) fn snapshot(&self)->Result<OriginalSnapshot,String>{
        self.valid()?;let mut result=self.private.snapshot(&self.session.token)?;result.token.clear();Ok(result)
    }
    pub(crate) fn clock(&self)->Result<Option<VodClock>,String>{
        self.valid()?;Ok(self.private.clock(&self.session.token)?.map(|mut c|{c.token.clear();c}))
    }
    pub(crate) fn viewport(&self,viewport:NativeViewport)->Result<(),String>{
        self.valid()?;self.private.viewport(&self.app,&self.session.token,viewport,self.host.tray_muted.load(Ordering::Acquire))
    }
    pub(crate) fn control(&self,request:VodControlRequest)->Result<(),String>{
        self.valid()?;self.private.control(&self.app,&self.session.token,request)
    }
    pub(crate) fn fullscreen(&self,enabled:bool)->Result<(),String>{
        self.valid()?;self.private.fullscreen(&self.app,&self.session.token,enabled)
    }
    pub(crate) fn controls(&self,can_close:bool)->Result<(),String>{
        self.valid()?;self.private.controls(&self.app,&self.session.token,can_close)
    }
    pub(crate) fn sample_frames(&self,cancel:&AtomicBool)->Result<NativeFrameWindow,String>{
        self.valid()?;
        // Native alignment uses at most 24 bounded attempts. Keep only compact
        // observations so a derived contiguous subset can retain provenance.
        if lock(&self.captures).len()>=24{return Err("remote_frame_capture_bound".into());}
        let window=self.private.sample_frames(&self.app,&self.session.token,&self.session.video,cancel)?;
        self.valid()?;
        let mut captures=lock(&self.captures);
        if captures.len()>=24{return Err("remote_frame_capture_bound".into());}
        captures.push(window.clone());Ok(window)
    }
    pub(crate) fn owns_capture(&self,window:&NativeFrameWindow)->bool{
        if self.valid().is_err(){return false;}
        let Ok(Some(clock))=self.clock()else{return false;};
        if window.document_epoch!=clock.document_epoch||window.video_id!=clock.fields.video_id
            ||window.source_id.to_string()!=clock.fields.source_id||window.source_generation!=clock.fields.source_generation{return false;}
        lock(&self.captures).iter().any(|original|same_capture_subset(original,window))
    }
    /// Terminal cancellation closes this document and makes in-flight native
    /// sampling observe closed/stale. It is not evidence the sampler has joined.
    pub(crate) fn cancel(&self){self.dispose();}
    fn dispose(&self){
        if self.cancelled.swap(true,Ordering::AcqRel){return;}
        let _admission=lock(&self.host.admission);
        let owned=lock(&self.host.watch_only.registry).sessions.get(&self.session.token)
            .is_some_and(|s|Arc::ptr_eq(s,&self.session)&&s.owned_by==Some(self.owner));
        if owned{self.host.watch_only.dispose(&self.app,&self.session);}
        lock(&self.captures).clear();
    }
    pub(crate) fn close(self)->Result<(),String>{self.dispose();Ok(())}
}
impl Drop for OwnedVodLease {fn drop(&mut self){self.dispose();}}

fn same_capture_subset(a:&NativeFrameWindow,b:&NativeFrameWindow)->bool{
    if a.capture_id!=b.capture_id||a.video_id!=b.video_id||a.document_epoch!=b.document_epoch||a.source_id!=b.source_id
        ||a.source_generation!=b.source_generation||a.seek_generation!=b.seek_generation||b.frames.is_empty()
        ||b.frames.len()>a.frames.len(){return false;}
    a.frames.windows(b.frames.len()).any(|slice|slice.iter().zip(&b.frames).all(|(x,y)|
        x.media_pts90k==y.media_pts90k&&x.uncertainty_ticks==y.uncertainty_ticks&&x.feature==y.feature))
}

#[cfg(test)]
#[path="original_vod_owned_tests.rs"]
mod tests;
