//! Existing receipt only. The sole remote API surface is the production
//! archive GET allowlist; no source, upload, OAuth or Live mutation path runs.
use super::*;
use crate::cloud_recording::{catalog::Catalog, live_archive::{self, ArchiveReason, Record},
    model::{CloudError, now}, upload::{self, Transport}, vault, Inner};
use crate::interface::{AppQuitRequest, AppState};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, os::windows::fs::MetadataExt,
    path::{Path, PathBuf}, sync::{Condvar, Mutex, atomic::AtomicUsize}};
#[path = "native_live_alignment_failure_qa.rs"]
mod failure;
#[path = "native_live_owned_isolation_qa.rs"]
mod owned_isolation;

const SELECTION: &str = "b7f26403-51ca-413b-ad04-858b15bad6a7";
const QA_VAULT: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
fn held<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(|p| p.into_inner()) }
fn require(ok: bool, code: &'static str) -> Checked<()> { if ok { Ok(()) } else { Err(code.into()) } }
fn regular(path: &Path, limit: u64) -> Checked<()> {
    for p in path.ancestors() {
        let m=fs::symlink_metadata(p).map_err(|_|"QA_STATE_PATH")?;
        require(!m.file_type().is_symlink() && m.file_attributes() & 0x400 == 0,"QA_STATE_LINK")?;
    }
    let m=fs::metadata(path).map_err(|_|"QA_STATE_PATH")?;
    require(m.is_file() && m.len()<=limit,"QA_STATE_BOUND")
}
fn baseline(catalog: &Catalog) -> Checked<Value> {
    let mut value=serde_json::to_value(catalog).map_err(|_|"QA_BASELINE")?;
    let receipts=value["live_receipts"].as_array_mut().ok_or("QA_BASELINE")?;
    let receipt=receipts.iter_mut().find(|r|r["view"]["selectionId"]==SELECTION).ok_or("QA_SELECTION")?;
    let object=receipt.as_object_mut().ok_or("QA_BASELINE")?;
    object.remove("archive"); object.remove("updatedAt");
    let view=object.get_mut("view").and_then(Value::as_object_mut).ok_or("QA_BASELINE")?;
    view.remove("archiveVerification"); view.remove("revision");
    Ok(value)
}
struct DenyNetwork(AtomicUsize);
impl Transport for DenyNetwork {
    fn send(&self,_:upload::Request)->std::result::Result<upload::Response,CloudError>{
        self.0.fetch_add(1,Ordering::SeqCst);Err(CloudError::protocol())
    }
}
fn service(vault:vault::Vault,catalog:Catalog,transport:Arc<DenyNetwork>)->Arc<CloudService>{
    Arc::new(CloudService {
        inner:Mutex::new(Inner {catalog,account_error:None,authorizing:false,auth_workers:0,auth_generation:0,
            auth_cancel:None,active:None,worker_running:false,closing:false,persistence_error:None,
            preparation_progress_at:None,automatic_running:false,automatic_observations:Default::default(),
            automatic_error:None,live:Default::default()}),
        wake:Condvar::new(),threads:Mutex::new(Vec::new()),vault,
        resolver:Arc::new(|_,_|Err("QA_SOURCE_FORBIDDEN".into())),planner:None,
        open_browser:Arc::new(|_|Err("QA_AUTHORIZATION_FORBIDDEN".into())),transport,
    })
}

// The diagnostic projection runs before verify_round's processing predicate.
// Do not emit names, timestamps, URLs, tokens, metadata descriptions or bodies.
fn safe_enum(value:&Value,path:&str,allowed:&[&str])->Value{
    match value.pointer(path) {None|Some(Value::Null)=>Value::Null,
        Some(Value::String(v)) if allowed.contains(&v.as_str())=>json!(v),_=>json!("other")}
}
fn response_facts(resource:&str,response:&upload::Response,expected:&str)->Value{
    let parsed=serde_json::from_slice::<Value>(&response.body).ok();
    let items=parsed.as_ref().and_then(|v|v["items"].as_array());
    let exact=items.is_some_and(|v|v.len()==1 && v[0]["id"]==expected);
    let mut out=json!({"resource":resource,"httpStatus":response.status,
        "responseSha256":format!("{:x}",Sha256::digest(&response.body)),"responseBytes":response.body.len(),
        "itemCount":items.map(Vec::len),"exactId":exact});
    if exact {
        let item=&items.unwrap()[0];
        if resource=="liveBroadcasts" {
            out["lifecycle"]=safe_enum(item,"/status/lifeCycleStatus",&["complete","live","liveStarting","testing","testStarting","ready","created","revoked"]);
            out["recording"]=safe_enum(item,"/status/recordingStatus",&["notRecording","recording","recorded"]);
        } else {
            out["upload"]=safe_enum(item,"/status/uploadStatus",&["deleted","failed","processed","rejected","uploaded"]);
            out["processing"]=safe_enum(item,"/processingDetails/processingStatus",&["failed","processing","succeeded","terminated"]);
            out["processingDetailsPresent"]=json!(item.get("processingDetails").is_some_and(|v|!v.is_null()));
            out["processingStatusPresent"]=json!(item.pointer("/processingDetails/processingStatus").is_some_and(|v|!v.is_null()));
            out["embeddable"]=item.pointer("/status/embeddable").and_then(Value::as_bool).map_or(Value::Null,|v|json!(v));
            let raw=item.pointer("/contentDetails/duration").and_then(Value::as_str);
            let safe=raw.filter(|v|v.len()<=64 && v.starts_with('P') && v.bytes().all(|b|b.is_ascii_digit()||b"PTDHMS.".contains(&b)));
            out["duration"]=safe.map_or(Value::Null,|v|json!(v));
            out["durationSeconds"]=safe.and_then(upload::iso_duration).filter(|v|v.is_finite()&&*v>=0.).map_or(Value::Null,|v|json!(v));
            out["durationMalformed"]=json!(raw.is_some()&&safe.and_then(upload::iso_duration).is_none());
        }
    }
    out
}
struct DiagnosticTarget { identity:Identity, video_sha256:String, duration:f64 }
fn diagnostic_duration(response:&upload::Response,video:&str,owner:&str)->Option<(String,f64)>{
    if response.status!=200{return None;}
    let body:Value=serde_json::from_slice(&response.body).ok()?;
    let items=body["items"].as_array()?;if items.len()!=1{return None;}
    let v=&items[0];
    if v["id"]!=video||v["kind"]!="youtube#video"||v["snippet"]["channelId"]!=owner
        ||v["status"]["privacyStatus"]!="unlisted"||v["status"]["embeddable"]!=true
        ||v["status"]["uploadStatus"]!="uploaded"||v["processingDetails"]["processingStatus"]!="processing"{return None;}
    let duration=upload::iso_duration(v["contentDetails"]["duration"].as_str()?)?;
    if !duration.is_finite()||!(5.0..=120.0).contains(&duration){return None;}
    Some((format!("{:x}",Sha256::digest(&response.body)),duration))
}
struct ObservedGet {inner:live_archive::GoogleArchiveTransport,video:String,owner:String,end:Instant,
    facts:Mutex<Vec<Value>>,attempts:AtomicUsize,diagnostic:Mutex<Option<(String,f64)>>}
impl Transport for ObservedGet {
    fn send(&self,request:upload::Request)->std::result::Result<upload::Response,CloudError>{
        // Production transport has a 10s total request timeout. Reserve that
        // time before each GET so the pair never runs past this QA's 60s limit.
        if self.end.saturating_duration_since(Instant::now())<Duration::from_secs(11) {return Err(CloudError::protocol());}
        let url=reqwest::Url::parse(&request.url).map_err(|_|CloudError::protocol())?;
        let resource=match url.path(){"/youtube/v3/liveBroadcasts"=>"liveBroadcasts","/youtube/v3/videos"=>"videos",_=>return Err(CloudError::protocol())};
        if request.method!=reqwest::Method::GET || !url.query_pairs().any(|(k,v)|k=="id"&&v==self.video)
            || self.attempts.fetch_add(1,Ordering::SeqCst)>=8 {return Err(CloudError::protocol());}
        let response=self.inner.send(request)?;
        held(&self.facts).push(response_facts(resource,&response,&self.video));
        if resource=="videos" {*held(&self.diagnostic)=diagnostic_duration(&response,&self.video,&self.owner);}
        Ok(response)
    }
}
fn save_archive(service:&CloudService,expected:&Value,identity:&Identity,generation:u64,record:Record)->Checked<()> {
    let mut inner=service.lock().map_err(|_|"QA_CATALOG")?;
    require(!inner.closing && inner.catalog.live_generation==generation && baseline(&inner.catalog)?==*expected,"QA_OWNER_CHANGED")?;
    require(inner.catalog.live_grant.as_ref().is_some_and(|g|g.valid()&&g.grant.channel_id==identity.owner_channel_id),"QA_OWNER_CHANGED")?;
    let mut candidate=inner.catalog.clone();
    let receipt=candidate.live_receipts.iter_mut().find(|r|r.view.selection_id==SELECTION).ok_or("QA_SELECTION")?;
    require(receipt.valid() && receipt.archive_identity().as_ref()==Some(identity) && record.valid() && &record.identity==identity,"QA_ARCHIVE_IDENTITY")?;
    receipt.view.archive_verification=Some(record.snapshot());receipt.archive=Some(record);
    receipt.changed().map_err(|_|"QA_REVISION")?;
    require(receipt.valid(),"QA_RECEIPT_INVALID")?;
    service.publish(&mut inner,candidate).map_err(|_|"QA_SAVE".to_owned())
}
fn metadata(service:&CloudService,expected:&Value,report:&mut Value,diagnostic:&mut Option<DiagnosticTarget>)->Checked<bool>{
    let end=Instant::now()+Duration::from_secs(60);
    let (identity,generation,auth,mut record)={
        let inner=service.lock().map_err(|_|"QA_CATALOG")?;
        let receipt=inner.catalog.live_receipts.iter().find(|r|r.view.selection_id==SELECTION).filter(|r|r.valid()).ok_or("QA_SELECTION")?;
        let identity=receipt.archive_identity().ok_or("QA_JOINED_RECEIPT_REQUIRED")?;
        let grant=inner.catalog.live_grant.as_ref().filter(|g|g.valid()&&g.grant.channel_id==identity.owner_channel_id
            &&g.grant.expires_at>now().saturating_add(65_000)).ok_or("QA_FRESH_LIVE_GRANT_REQUIRED")?;
        let generation=inner.catalog.live_generation;
        let auth=live_archive::Authorization::new(grant.grant.access_token.clone(),grant.grant.channel_id.clone(),generation)
            .map_err(|_|"QA_AUTHORIZATION")?;
        let record=match receipt.archive.as_ref(){Some(prior) if now()<prior.deadline_at=>{
            let mut r=prior.clone();r.resume(generation,now()).map_err(|_|"QA_ARCHIVE_BUDGET")?;r},
            prior=>Record::begin(identity.clone(),generation,now(),prior).map_err(|_|"QA_ARCHIVE_BUDGET")?};
        (identity,generation,auth,record)
    };
    let transport=ObservedGet{inner:live_archive::GoogleArchiveTransport::new().map_err(|_|"QA_GET_TRANSPORT")?,
        video:identity.broadcast_id.clone(),owner:identity.owner_channel_id.clone(),end,facts:Mutex::new(Vec::new()),
        attempts:AtomicUsize::new(0),diagnostic:Mutex::new(None)};
    let result=(||->Checked<bool>{
        save_archive(service,expected,&identity,generation,record.clone())?;
        loop {
            if end.saturating_duration_since(Instant::now())<Duration::from_secs(22){break;}
            if let Some(due)=record.retry_at.filter(|due|*due>now()){
                thread::sleep(Duration::from_millis(due.saturating_sub(now()).min(200)));continue;
            }
            record.start_round(now()).map_err(|_|"QA_ARCHIVE_BUDGET")?;
            save_archive(service,expected,&identity,generation,record.clone())?;
            let round=live_archive::verify_round(&transport,&auth,&identity,||{
                record.request(now())?;
                save_archive(service,expected,&identity,generation,record.clone()).map_err(|_|ArchiveReason::SaveFailed)
            });
            record.apply(round,now()).map_err(|_|"QA_ARCHIVE_RESPONSE")?;
            save_archive(service,expected,&identity,generation,record.clone())?;
            if record.state!=ArchiveState::Pending {return Ok(record.state==ArchiveState::MetadataVerified);}
        }
        report["archiveObservationLimitReached"]=json!(true);
        report["lastRemoteObservation"]=json!(record.snapshot());
        record.stop(ArchiveReason::Cancelled); // This bounded observer stopped; not a remote failure.
        save_archive(service,expected,&identity,generation,record.clone())?;
        Ok(false)
    })();
    report["metadataResponses"]=json!(*held(&transport.facts));
    report["archive"]=json!(record.snapshot());
    report["metadataGetRequests"]=json!(transport.attempts.load(Ordering::SeqCst));
    if result.as_ref().is_ok_and(|ready|!*ready) {
        if let (Some(facts),Some((hash,duration)))=(record.last.as_ref(),held(&transport.diagnostic).as_ref()) {
            if facts.lifecycle.as_deref()==Some("complete")&&facts.recording.as_deref()==Some("recorded")
                &&facts.upload.as_deref()==Some("uploaded")&&facts.processing.as_deref()==Some("processing")
                &&facts.embeddable==Some(true)&&facts.video_sha256.as_ref()==Some(hash){
                *diagnostic=Some(DiagnosticTarget{identity,video_sha256:hash.clone(),duration:*duration});
            }
        }
    }
    result
}
fn diagnostic_identity(service:&CloudService,target:&DiagnosticTarget)->Checked<()> {
    let inner=service.lock().map_err(|_|"QA_DIAGNOSTIC_CATALOG")?;
    let receipt=inner.catalog.live_receipts.iter().find(|r|r.view.selection_id==SELECTION).filter(|r|r.valid()).ok_or("QA_DIAGNOSTIC_RECEIPT")?;
    require(receipt.archive_identity().as_ref()==Some(&target.identity),"QA_DIAGNOSTIC_IDENTITY")?;
    let record=receipt.archive.as_ref().filter(|r|r.valid()&&r.identity==target.identity).ok_or("QA_DIAGNOSTIC_RECORD")?;
    require(record.last.as_ref().is_some_and(|f|f.video_sha256.as_ref()==Some(&target.video_sha256)
        &&f.lifecycle.as_deref()==Some("complete")&&f.recording.as_deref()==Some("recorded")
        &&f.upload.as_deref()==Some("uploaded")&&f.processing.as_deref()==Some("processing")&&f.embeddable==Some(true)),"QA_DIAGNOSTIC_CHANGED")
}
fn processing_playback(app:&AppHandle,service:&CloudService,target:&DiagnosticTarget,root:&Path,evidence:&mut Value)->Checked<Value>{
    *evidence=json!({"failedStage":"receipt_identity","diagnosticOnly":true,"screenshotSaved":false});
    diagnostic_identity(service,target)?;
    let cancel=AtomicBool::new(false);let end=Instant::now()+Duration::from_secs(60);
    evidence["failedStage"]=json!("open_owned_qa_document");
    let owned=OwnedVod::open(app,&target.identity.broadcast_id)?;
    let video=&target.identity.broadcast_id;
    let started=Instant::now();let mut stage="initial_content_clock";
    let outcome=(||->Checked<Value>{
    let initial=wait_clock(app,&owned.token,video,(Instant::now()+Duration::from_secs(35)).min(end),&cancel,
        |c|c.fields.duration.is_some_and(|d|d.is_finite()&&d>0.&&d<=120.&&(d-target.duration).abs()<=1.)&&!c.fields.seeking)?;
    stage="mute_owned_qa_document";owned.quiet()?;
    stage="resume";control(app,&owned.token,video,"resume",None)?;
    stage="playing_content_clock";
    let playing=wait_clock(app,&owned.token,video,(Instant::now()+Duration::from_secs(10)).min(end),&cancel,
        |c|!c.fields.paused&&!c.fields.seeking&&!c.fields.buffering&&!c.fields.ended)?;
    stage="sample_native_frames";
    let native=app.state::<YouTubeOriginalHost>().vod_sample_frames(app,&owned.token,video,&cancel).map_err(frame_error)?;
    stage="validate_frame_boundary";
    let after=clock(app,&owned.token,video)?;
    require(native.video_id==*video&&native.document_epoch==initial.document_epoch&&native.document_epoch==after.document_epoch
        &&native.source_id.to_string()==initial.fields.source_id&&native.source_id.to_string()==after.fields.source_id
        &&native.source_generation==initial.fields.source_generation&&native.source_generation==after.fields.source_generation
        &&native.seek_generation==playing.fields.seek_generation&&native.seek_generation==after.fields.seek_generation
        &&native.frames.len()>=8&&native.frames.windows(2).all(|p|p[1].media_pts90k>p[0].media_pts90k)
        &&!after.fields.paused&&!after.fields.seeking&&!after.fields.buffering,"QA_DIAGNOSTIC_FRAME_BOUNDARY")?;
    stage="pause";let pause=control(app,&owned.token,video,"pause",None)?;
    stage="paused_content_clock";
    let paused=wait_clock(app,&owned.token,video,end,&cancel,|c|c.fields.paused&&c.fields.sequence>pause.fields.sequence)?;
    let to=(target.duration/2.).min(target.duration-2.);
    stage="seek";
    let seek=control(app,&owned.token,video,"seek",Some(to))?;
    stage="sought_content_clock";
    let sought=wait_clock(app,&owned.token,video,end,&cancel,|c|c.fields.sequence>seek.fields.sequence&&!c.fields.seeking
        &&c.fields.current_time.is_some_and(|t|(t-to).abs()<=0.35))?;
    require(sought.document_epoch==initial.document_epoch&&sought.fields.source_id==initial.fields.source_id
        &&sought.fields.source_generation==initial.fields.source_generation,"QA_DIAGNOSTIC_NAVIGATION")?;
    stage="revalidate_receipt_identity";diagnostic_identity(service,target)?;
    Ok(json!({"actualPlayingFramesObserved":true,"nativeFrameCount":native.frames.len(),
        "firstMediaPts90k":native.frames.first().map(|f|f.media_pts90k),"lastMediaPts90k":native.frames.last().map(|f|f.media_pts90k),
        "clockDurationSeconds":initial.fields.duration,"pauseConfirmed":paused.fields.paused,
        "seekConfirmed":true,"seekTargetSeconds":to,"observedSeekSeconds":sought.fields.current_time,
        "diagnosticOnly":true,"processingStillUnverified":true,"metadataVerified":false,"measuredAlignment":false,
        "receiptPromoted":false,"remoteAnchor":"unknown","synchronizedReplayVerified":false,"pixelsSaved":false}))
    })();
    if outcome.is_err(){
        let elapsed=started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        *evidence=failure::observe(&owned,video,stage,target.duration,root);
        evidence["elapsedMsBeforeEvidence"]=json!(elapsed);
    }else{*evidence=json!({"completedPlaybackStages":true,"diagnosticOnly":true,"screenshotSaved":false});}
    let closed=owned.close();evidence["ownedDocumentClosed"]=json!(closed.is_ok());
    if let Err(code)=closed {evidence["closeError"]=json!(code);if outcome.is_ok(){evidence["failedStage"]=json!("close_owned_qa_document");return Err("QA_ALIGNMENT_CLOSE".into());}}
    outcome
}
fn chat_facts(service:&Arc<CloudService>)->Checked<Value>{
    let lease=service.open_live_history(SELECTION).map_err(|_|"QA_HISTORY")?;
    let refs=lease.references().map_err(|_|"QA_REFERENCES")?.ok_or("QA_REFERENCES_MISSING")?;
    let windows=refs.references().windows.len();
    let mut cursor=None::<String>;let mut pages=0;let mut total=0;let mut mapped=0;
    let mut reasons=BTreeMap::<String,usize>::new();let mut low=None::<u64>;let mut high=None::<u64>;
    let mut max_numeric=0;let mut done=false;
    while pages<64 {
        let page=lease.source_chat_page(cursor.as_deref(),120).map_err(|_|"QA_CHAT_PAGE")?;
        pages+=1;total+=page.items.len();
        for item in page.items {if let Some(p)=item.position {mapped+=1;low=Some(low.map_or(p.transport_ticks90k,|v|v.min(p.transport_ticks90k)));
            high=Some(high.map_or(p.transport_ticks90k,|v|v.max(p.transport_ticks90k)));max_numeric=max_numeric.max(p.numeric_uncertainty_ticks90k);
            require(p.presentation_uncertainty=="unknown","QA_CHAT_PRECISION")?;
        } else {*reasons.entry(item.unmapped_reason.unwrap_or("unknown").to_owned()).or_default()+=1;}}
        let Some(next)=page.cursor else{done=true;break;};require(cursor.as_ref()!=Some(&next),"QA_CHAT_CURSOR")?;cursor=Some(next);
    }
    Ok(json!({"pages":pages,"events":total,"mapped":mapped,"unmapped":total-mapped,"unmappedReasons":reasons,
        "allPagesRead":done,"transportMin90k":low,"transportMax90k":high,"numericUncertaintyTicks90k":max_numeric,
        "presentationUncertainty":"unknown","referenceWindows":windows,"remoteAnchor":"unknown","chatSynchronizationVerified":false}))
}
fn write_report(path:&Path,value:&Value)->Checked<()> {
    let mut f=fs::OpenOptions::new().write(true).create(true).truncate(true).open(path).map_err(|_|"QA_REPORT")?;
    f.write_all(&serde_json::to_vec_pretty(value).map_err(|_|"QA_REPORT")?).and_then(|_|f.sync_all()).map_err(|_|"QA_REPORT".into())
}

#[test]
#[ignore="Exact existing selected Live receipt metadata GET and fresh owned VOD alignment; no new relay"]
fn actual_existing_selected_live_readonly_alignment()->Checked<()> {
    require(std::env::var("ATSUMI_EXISTING_LIVE_ALIGNMENT_QA").as_deref()==Ok("b7f26403-existing-readonly"),"QA_OPT_IN")?;
    let appdata=PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?);
    let qa_data=appdata.join(QA_VAULT);regular(&qa_data.join("cloud-recording/state.bin"),8*1024*1024)?;
    regular(&qa_data.join("cloud-recording/state.lock"),1024)?;
    let vault=vault::Vault::open(&qa_data).map_err(|_|"QA_VAULT_BUSY")?;
    let catalog=vault.load().map_err(|_|"QA_VAULT")?;
    require(catalog.live_receipts.iter().filter(|r|r.view.selection_id==SELECTION).count()==1,"QA_SELECTION")?;
    let expected=baseline(&catalog)?;
    let deny=Arc::new(DenyNetwork(AtomicUsize::new(0)));let service=service(vault,catalog,deny.clone());
    let id=Uuid::new_v4().simple().to_string();let identity=format!("local.atsumi.livealignmentqa.{id}");
    let data=appdata.join(&identity);require(!data.exists(),"QA_FRESH_PROFILE")?;
    let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let root=workspace.join(".runtime/existing-live-alignment-qa").join(id);
    fs::create_dir_all(root.parent().ok_or("QA_ROOT")?).map_err(|_|"QA_ROOT")?;fs::create_dir(&root).map_err(|_|"QA_ROOT")?;
    let output=Arc::new(Mutex::new(json!({"selectionId":SELECTION,"trialCompleted":false,"remoteAnchor":"unknown",
        "freshViewerProfile":true,"existingQaVaultInPlace":true,"credentialCopies":0,"sourceOpenAttempts":0,
        "startCalls":0,"remoteMutations":0,"refreshRequests":0,"desktopInput":false,"archiveObservationSeconds":60,
        "synchronizedReplayVerified":false,"measuredAlignment":false,"ordinaryProfileAccess":false})));
    let slots=Arc::new(Mutex::new(None::<thread::JoinHandle<()>>));let setup_slots=slots.clone();
    let worker_output=output.clone();let worker_service=service.clone();let worker_root=root.clone();let worker_expected=expected.clone();
    let mut context=tauri::generate_context!();context.config_mut().identifier=identity;
    context.config_mut().app.windows.clear();context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().any_thread()
        .manage(crate::soop::original::SoopOriginalHost::default()).manage(YouTubeOriginalHost::default())
        .manage(Arc::new(crate::startup::Startup::default())).manage(Arc::new(crate::ui_download_events::DownloadEvents::default()))
        .setup(move|app|{
            if app.path().app_data_dir()?!=data{return Err("QA_PROFILE_PATH".into());}
            tauri::window::WindowBuilder::new(app,"main").title("Atsumi existing archive verification")
                .inner_size(960.,720.).position(-16000.,-16000.).focused(false).skip_taskbar(true).build()?;
            let handle=app.handle().clone();
            *held(&setup_slots)=Some(thread::spawn(move||{
                let mut report=held(&worker_output).clone();
                let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Checked<()>{
                    let startup=handle.state::<Arc<crate::startup::Startup>>();
                    crate::initialize_backend(&handle,&startup).map_err(|_|"QA_BACKEND")?;
                    report["sourceChat"]=chat_facts(&worker_service)?;
                    let mut diagnostic=None;
                    let ready=metadata(&worker_service,&worker_expected,&mut report,&mut diagnostic)?;
                    report["metadataVerified"]=json!(ready);
                    write_report(&worker_root.join("pending-result.json"),&report)?;
                    if ready {
                        let proof=verify_selected_live_alignment(&handle,&worker_service,SELECTION,&AtomicBool::new(false))?;
                        report["measuredAlignment"]=proof["measuredAlignment"].clone();report["alignment"]=proof;
                    } else {
                        report["alignmentSkippedReason"]=json!("archive_metadata_not_verified");
                        if let Some(target)=diagnostic {
                            let mut evidence=Value::Null;
                            let probe=processing_playback(&handle,&worker_service,&target,&worker_root,&mut evidence);
                            report["processingPlaybackEvidence"]=evidence;
                            report["processingPlaybackError"]=json!(probe.as_ref().err());
                            if let Ok(proof)=probe {report["processingPlayback"]=proof;}
                        } else {report["processingPlaybackSkipped"]=json!("native_diagnostic_identity_unavailable");}
                    }
                    if std::env::var("ATSUMI_OWNED_VOD_ISOLATION_QA").ok().as_deref()==Some("existing-soop42-readonly") {
                        let isolated=owned_isolation::exercise(&handle,&worker_service);
                        report["ownedIsolationError"]=json!(isolated.as_ref().err());
                        if let Ok(proof)=isolated.as_ref(){report["ownedIsolation"]=proof.clone();}
                        isolated?;
                    }
                    Ok(())
                })).unwrap_or_else(|_|Err("QA_WORKER_PANIC".into()));
                report["exerciseErrorCode"]=json!(outcome.as_ref().err());
                report["trialCompleted"]=json!(outcome.is_ok());
                // The helper's owned VOD closes before this normal app drain.
                let drained=worker_service.request_live_shutdown().is_ok() && worker_service.wait_live_history().is_ok();
                report["cloudHistoryDrained"]=json!(drained);
                if !drained {report["trialCompleted"]=json!(false);}
                *held(&worker_output)=report;
                let _=write_report(&worker_root.join("pending-result.json"),&held(&worker_output));
                if let Some(state)=handle.try_state::<AppState>() {
                    let answer=state.active_work_snapshot().map_err(|_|()).and_then(|s|state.request_graceful_quit(handle.clone(),AppQuitRequest{
                        expected_work_set_fingerprint:s.work_set_fingerprint,confirm_active_work:true,force_when_status_unknown:false}).map_err(|_|()));
                    match answer {Ok(a) if a.accepted=>held(&worker_output)["normalQuitAccepted"]=json!(true),
                        _=>{held(&worker_output)["trialCompleted"]=json!(false);held(&worker_output)["quitError"]=json!("QA_NORMAL_QUIT_REJECTED");}}
                } else {held(&worker_output)["backendNeverInitialized"]=json!(true);handle.exit(2);}
            }));Ok(())
        }).build(context).map_err(|_|"QA_BUILD")?;
    app.run_return(|_,_|{});
    let joined=held(&slots).take().is_some_and(|h|h.join().is_ok());
    let saved=service.vault.load().map_err(|_|"QA_VAULT")?;let preserved=baseline(&saved)?==expected;
    let mut report=held(&output).clone();report["qaObserverJoined"]=json!(joined);report["normalAppExitReturned"]=json!(true);
    report["nonArchiveCatalogPreserved"]=json!(preserved);report["forbiddenTransportCalls"]=json!(deny.0.load(Ordering::SeqCst));
    if !joined||!preserved||deny.0.load(Ordering::SeqCst)!=0||report["normalQuitAccepted"]!=true {report["trialCompleted"]=json!(false);}
    write_report(&root.join("result.json"),&report)?;println!("QA_EXISTING_LIVE_ALIGNMENT_RESULT={}",root.join("result.json").display());
    require(report["trialCompleted"]==true,"QA_TRIAL_UNVERIFIED")
}

#[test]
fn safe_metadata_projection_retains_duration_before_processing_and_no_payload(){
    let body=json!({"items":[{"id":"abcdefghijk","snippet":{"title":"PRIVATE_TEXT","channelId":"PRIVATE_OWNER"},
        "status":{"uploadStatus":"processed","embeddable":true},"contentDetails":{"duration":"PT52S"}}]});
    let response=upload::Response{status:200,headers:Vec::new(),body:serde_json::to_vec(&body).unwrap()};
    let facts=response_facts("videos",&response,"abcdefghijk");
    assert_eq!(facts["durationSeconds"],52.);assert_eq!(facts["processing"],Value::Null);
    assert_eq!(facts["processingStatusPresent"],false);assert_eq!(facts["upload"],"processed");
    assert!(!facts.to_string().contains("PRIVATE"));assert_eq!(facts["responseSha256"].as_str().unwrap().len(),64);
    assert!(response_facts("videos",&response,"otherid1234").get("duration").is_none());
}

#[test]
fn processing_diagnostic_duration_requires_same_owner_unlisted_exact_video(){
    let mut body=json!({"items":[{"id":"abcdefghijk","kind":"youtube#video","snippet":{"channelId":"UCowner"},
        "status":{"privacyStatus":"unlisted","uploadStatus":"uploaded","embeddable":true},
        "processingDetails":{"processingStatus":"processing"},"contentDetails":{"duration":"PT59S"}}]});
    let response=|v:&Value|upload::Response{status:200,headers:Vec::new(),body:serde_json::to_vec(v).unwrap()};
    let original=response(&body);let valid=diagnostic_duration(&original,"abcdefghijk","UCowner").unwrap();
    assert_eq!(valid.1,59.);assert_eq!(valid.0,format!("{:x}",Sha256::digest(&original.body)));
    assert!(diagnostic_duration(&original,"otherid1234","UCowner").is_none());
    assert!(diagnostic_duration(&original,"abcdefghijk","UCother").is_none());
    body["items"][0]["status"]["privacyStatus"]=json!("public");
    assert!(diagnostic_duration(&response(&body),"abcdefghijk","UCowner").is_none());
    body["items"][0]["status"]["privacyStatus"]=json!("unlisted");
    body["items"][0]["processingDetails"]=Value::Null;
    assert!(diagnostic_duration(&response(&body),"abcdefghijk","UCowner").is_none());
}
