//! Fixed manual validation upload only. Never compiled into the production app.
//! Gate: requirements-audit/20261117-cloud-goal-resume/chzzk-upload-source-gate.json.
use super::super::*;
use crate::streaming::browser_store::{self, BrowserCaptureStore, BrowserRecording, BrowserRecordingStatus, BrowserMergeStatus};
use serde_json::{json, Value};
use std::{fs, io::Write, sync::atomic::AtomicUsize};

#[path = "native_chzzk_replay_qa.rs"]
mod replay_qa;

pub(super) type Checked<T> = Result<T, String>;
pub(super) const RECORDING: &str = "6e82ac06ed1048c6b8201b29892ff659";
pub(super) const OFFSET: f64 = 660.1371041666571;
pub(super) const DURATION: f64 = 60.00664583333128;
pub(super) const DONATION: f64 = 18.403003999999783;
const BYTES: u64 = 61_411_740;
const MEDIA_SHA: &str = "e03adaf0e107aa028640329bf7c6cae918e4624f89a1c0591a69573b4c82f2a3";
const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const OLD: [(&str, &str); 3] = [("youtube","38965dca-b875-4758-8b84-642507653bd1"),("soop","9011ac2c-8063-4e1a-a85a-3bc449879c8f"),("soop","241bc41d-d3e6-4bf9-a205-899f2c2d21d5")];
fn code(e: CloudError)->String { if e.code.len()<=64 && e.code.bytes().all(|v|v.is_ascii_uppercase()||v.is_ascii_digit()||v==b'_') {e.code} else {"QA_CLOUD_ERROR".into()} }
fn digest(v:&[u8])->String {format!("{:x}",Sha256::digest(v))}
fn valid_digest(v:&str)->bool {v.len()==64&&v.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
fn reference()->CloudSourceRef {CloudSourceRef{provider:"chzzk".into(),recording_id:RECORDING.into(),part_index:Some(11),plan_id:None,unit_id:None}}
fn workspace()->PathBuf {Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_owned()}
fn profile()->Checked<PathBuf> {Ok(PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(PROFILE))}
fn plain(path:&Path)->Checked<()> {
    use std::os::windows::fs::MetadataExt;
    for p in path.ancestors() {let m=fs::symlink_metadata(p).map_err(|_|"QA_PATH")?;if m.file_type().is_symlink()||m.file_attributes()&0x400!=0{return Err("QA_LINK".into());}}
    Ok(())
}
fn canonical_guard_path(path:&Path)->Checked<PathBuf> {
    // The production guards deliberately require a canonical Windows path.
    // Reject reparse ancestors before resolving so canonicalization cannot hide them.
    plain(path)?;
    let canonical=fs::canonicalize(path).map_err(|e|format!("QA_CANONICAL_OS_{}",e.raw_os_error().unwrap_or(0)))?;
    plain(&canonical)?;
    Ok(canonical)
}
fn bounded(path:&Path,limit:u64)->Checked<Vec<u8>> {
    let path=canonical_guard_path(path)?;let mut f=browser_store::cleanup::verification_guard(&path).map_err(|_|"QA_FILE_GUARD")?;
    let len=f.metadata().map_err(|_|"QA_STAT")?.len();if len>limit{return Err("QA_FILE_BOUND".into());}
    let mut v=Vec::new();(&mut f).take(limit+1).read_to_end(&mut v).map_err(|_|"QA_READ")?;
    if v.len() as u64!=len{return Err("QA_READ_CHANGED".into());}Ok(v)
}
struct Original {path:PathBuf,file:File,sha:String,bytes:Vec<u8>,_directories:Vec<File>}
pub(super) struct Originals {files:Vec<Original>}
impl Originals {
    pub(super) fn open()->Checked<Self> {
        let local=PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join("local.atsumi.next");
        let root=local.join("ssd-recordings/CHZZK/BrowserCapture").join(RECORDING);
        let archive=PathBuf::from("D:/Astumi/CHZZK/BrowserCapture").join(RECORDING);
        let mut saved=Self{files:vec![]};
        let expected_files=[
            (root.join("recording.json"),"74d4cb34515cd78f8face205ca87f97fef5c07d29e5ec543cf45629ba35872eb"),
            (root.join("progressive.json"),"211705f2680834dbcdf4b144646d68d2910aef314e1f026d4b5666238a29ebb1"),
            (root.join("chat.jsonl"),"b300ed24b5e40eb6dabc2b1c1c84a308dd183ec530c2e24a83b3cb3bf4aac35e"),
            (root.join("segments.jsonl"),"a25771ead7d0295657b6e55837d676088484ed15b50b66c2270b9856fcd0d7a7"),
            (root.join("ssd-ranges.json"),"25a5f94a01941afe8caee977b9ef22b0238d454134a55810ad58d3f1a6808acb"),
            (archive.join("ssd-ranges.json"),"25a5f94a01941afe8caee977b9ef22b0238d454134a55810ad58d3f1a6808acb"),
            (archive.join("ssd-archive-owner.json"),"d5fd9eea481ac9ca492cb4df05cfd6507adf47ffcca93388c9da3e2727703400"),
            (archive.join("merged-01c92a302acd423c8ef036493f2bafb6.timeline.jsonl"),"99d4e82e647bc9639cfb24bcb29f6b0636d72fd68a891fbd44e2b7c4c8b6600d"),
            (archive.join("merged-01c92a302acd423c8ef036493f2bafb6.cleanup.json"),"59a421fea55cd537f9f67e49576f3fc4c123c21122dc64b8bf65c0978822ffe2"),
            (root.join("viewer-metrics.jsonl"),"0163637fc21e058ab52a545a5e6c496494ee37e0e48fe627eeff563c358b6565"),
            (root.join("channel-profile.json"),"bde3bc53251d16fef9201743ba9d7196a7e1d596bdd7b901bd118d0b9a92cdf5"),
            (local.join("streaming/browser/catalog.jsonl"),"39411d1280415d0491843114cf19f0c81f4c8d977092370435933a0e0902f5c6"),
        ];
        if expected_files.iter().any(|(_,sha)|!valid_digest(sha))||!valid_digest(MEDIA_SHA){return Err("QA_EXPECTED_SHA_FORMAT".into());}
        for (path,sha) in expected_files {saved.hold(path,sha)?;}
        let value:Value=serde_json::from_slice(&saved.named("channel-profile.json")?.bytes).map_err(|_|"QA_PROFILE_JSON")?;
        let id=value["imageAssetId"].as_str().filter(|v|v.len()==64&&v.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))).ok_or("QA_PROFILE_ASSET")?;
        saved.hold(root.join("replay-assets").join(format!("{id}.json")),"dcab1a44feef4a03b48411412b04e34f8958a9ac6909761c0a8607e29841ea42")?;
        let record:BrowserRecording=serde_json::from_slice(&saved.named("recording.json")?.bytes).map_err(|_|"QA_RECORD")?;
        if record.id!=RECORDING||record.status!=BrowserRecordingStatus::Stopped||record.preserve_sources||record.segment_count!=310
            ||record.merge.as_ref().is_none_or(|v|v.status!=BrowserMergeStatus::Queued){return Err("QA_PARENT_CHANGED".into());}
        saved.selected_catalog_line()?;Ok(saved)
    }
    fn hold(&mut self,path:PathBuf,sha:&str)->Checked<()> {
        if !valid_digest(sha){return Err("QA_EXPECTED_SHA_FORMAT".into());}
        let path=canonical_guard_path(&path)?;let directories=browser_store::cleanup::pin_directories(path.parent().ok_or("QA_PARENT")?).map_err(|_|"QA_DIRECTORY_GUARD")?;
        let mut file=browser_store::cleanup::verification_guard(&path).map_err(|_|"QA_ORIGINAL_GUARD")?;
        if file.metadata().map_err(|_|"QA_ORIGINAL_STAT")?.len()>2*1024*1024{return Err("QA_ORIGINAL_BOUND".into());}
        let mut bytes=Vec::new();file.read_to_end(&mut bytes).map_err(|_|"QA_ORIGINAL_READ")?;
        file.seek(SeekFrom::Start(0)).map_err(|_|"QA_ORIGINAL_SEEK")?;
        if digest(&bytes)!=sha{return Err("QA_ORIGINAL_CHANGED".into());}
        self.files.push(Original{path,file,sha:sha.into(),bytes,_directories:directories});Ok(())
    }
    fn named(&self,name:&str)->Checked<&Original> {self.files.iter().find(|v|v.path.file_name().and_then(|s|s.to_str())==Some(name)).ok_or("QA_ORIGINAL_MISSING".into())}
    pub(super) fn selected_catalog_line(&self)->Checked<Vec<u8>> {
        let mut found=None;
        for line in self.named("catalog.jsonl")?.bytes.split_inclusive(|b|*b==b'\n') {
            if line.iter().all(u8::is_ascii_whitespace){continue;}
            let row:Value=serde_json::from_slice(line).map_err(|_|"QA_CATALOG_JSON")?;
            if row["id"]==RECORDING&&found.replace(line.to_vec()).is_some(){return Err("QA_DUPLICATE_CATALOG".into());}
        }
        let line=found.ok_or("QA_CATALOG_ENTRY")?;
        if digest(&line)!="755ffbd9ca7f640d0afa9079543cd27513caf3c29705fcb2bdcaf76d1c8b5631"{return Err("QA_CATALOG_LINE_CHANGED".into());}Ok(line)
    }
    pub(super) fn assert_unchanged(&self)->Checked<()> {
        for v in &self.files {if v.file.metadata().map_err(|_|"QA_ORIGINAL_STAT")?.len()!=v.bytes.len() as u64||digest(&bounded(&v.path,2*1024*1024)?)!=v.sha{return Err("QA_ORIGINAL_CHANGED".into());}}Ok(())
    }
}
fn source_authorization(owner:&str)->replay::CloudReplayAuthorization {
    let source=reference();
    replay::CloudReplayAuthorization{job_id:uuid::Uuid::new_v4().to_string(),source:source.clone(),owner_channel_id:owner.into(),cohort_id:"d".repeat(64),source_sha256:MEDIA_SHA.into(),bytes:BYTES,duration_seconds:DURATION,media_offset_seconds:OFFSET,
        original_inputs:vec![replay::CloudReplayInput{index:11,part_index:Some(11),sha256:MEDIA_SHA.into(),bytes:BYTES,duration_seconds:DURATION,media_offset_seconds:OFFSET,modified_nanos:None}],plan_fingerprint:None,input_parts:true,
        description:source::SourceDescription{source:source.clone(),title:"CHZZK selected validation part 12".into(),plan_id:None,coverage:source::SourceCoverage{state:"incomplete".into(),expected_segments:Some(310),covered_segments:Some(5),missing_from_segment:None,reason:Some("Selected verification range only; parent remains incomplete.".into()),capture_warning:None},preparation_required:false,duration_seconds:OFFSET+DURATION,bytes:BYTES,
            parts:vec![source::SourcePart{source,unit_index:0,part_index:Some(11),duration_seconds:DURATION,media_offset_seconds:OFFSET,bytes:BYTES,kind:"direct".into()}]}}
}
fn selected_file(originals:&Originals,scratch:&Path,owner:&str)->Checked<(CloudSource,File)> {
    fs::create_dir_all(scratch.join("streaming/browser")).map_err(|_|"QA_SCRATCH")?;
    fs::write(scratch.join("streaming/browser/catalog.jsonl"),originals.selected_catalog_line()?).map_err(|_|"QA_SCRATCH")?;
    let store=BrowserCaptureStore::new_deferred(scratch).map_err(|_|"QA_READONLY_STORE")?;
    // This authorization validates pinned LOCAL proof only; it is not an upload receipt.
    let held=store.cloud_chat_source(&source_authorization(owner),&AtomicBool::new(false)).map_err(|_|"QA_LOCAL_PROOF")?;
    let parts=browser_store::parts::load(&held.recording).map_err(|_|"QA_PARTS")?;
    let part=parts.get(11).ok_or("QA_PART_INDEX")?;
    if part.first!=55||part.end!=60||part.bytes!=BYTES||(part.offset-OFFSET).abs()>0.000001||(part.duration-DURATION).abs()>0.000001{return Err("QA_PART_IDENTITY".into());}
    let path=browser_store::archive::offload::Resolver::new(&held.recording).and_then(|r|r.resolve(&part.file)).map_err(|_|"QA_MEDIA_RECEIPT")?;
    let expected=PathBuf::from("D:/Astumi/CHZZK/BrowserCapture").join(RECORDING).join("merged-01c92a302acd423c8ef036493f2bafb6.mp4");
    let path=canonical_guard_path(&path)?;
    if path!=canonical_guard_path(&expected)?{return Err("QA_MEDIA_PATH".into());}
    let mut file=browser_store::cleanup::verification_guard(&path).map_err(|_|"QA_MEDIA_PIN")?;
    if file.metadata().map_err(|_|"QA_MEDIA_STAT")?.len()!=BYTES{return Err("QA_MEDIA_BYTES".into());}
    if hash_source(&mut file,&AtomicBool::new(false)).map_err(code)?!=MEDIA_SHA||!held.guard.valid(){return Err("QA_MEDIA_SHA".into());}
    file.seek(SeekFrom::Start(0)).map_err(|_|"QA_MEDIA_SEEK")?;
    let check=source::pin_file(&file)?;
    Ok((CloudSource{file,source:reference(),title:"Atsumi CHZZK donation synchronization verification · part 12".into(),duration_seconds:DURATION,media_offset_seconds:OFFSET,lease:Box::new(held.guard)},check))
}
fn baseline(catalog:&Catalog,allow_selected:bool)->Checked<(Vec<u8>,Vec<u8>)> {
    let owner=catalog.grant.as_ref().ok_or("QA_CONNECTED_GRANT_REQUIRED")?.channel_id.as_str();
    if catalog.client.is_none()||catalog.jobs.len()!=3+usize::from(allow_selected){return Err("QA_BASELINE_JOB_COUNT".into());}
    let mut old=Vec::new();
    for (provider,id) in OLD {
        let matches=catalog.jobs.iter().filter(|j|j.view.source.provider==provider&&j.view.source.recording_id==id).collect::<Vec<_>>();
        if matches.len()!=1{return Err("QA_BASELINE_SOURCE".into());}let j=matches[0];
        if j.view.state!="ready"||!j.has_upload_receipt()||j.view.visibility!=CloudVisibility::Unlisted||j.channel_id!=owner{return Err("QA_BASELINE_READY".into());}
        old.push(j);
    }
    let auto=serde_json::to_value(&catalog.automatic).map_err(|_|"QA_AUTOMATIC")?;
    let rules=auto["rules"].as_array().filter(|v|v.len()==1).ok_or("QA_AUTOMATIC_COUNT")?;
    if rules[0]["target"]["kind"]!="recording"||rules[0]["target"]["provider"]!="soop"||rules[0]["target"]["recordingId"]!=OLD[2].1{return Err("QA_AUTOMATIC_TARGET".into());}
    if allow_selected&&catalog.jobs.iter().filter(|j|j.view.source==reference()&&j.channel_id==owner&&j.automatic.is_none()).count()!=1{return Err("QA_SELECTED_JOB".into());}
    Ok((serde_json::to_vec(&old).map_err(|_|"QA_BASELINE_JSON")?,serde_json::to_vec(&catalog.automatic).map_err(|_|"QA_BASELINE_JSON")?))
}
struct BoundedTransport {inner:upload::GoogleTransport,deadline:Instant,calls:AtomicUsize,creates:AtomicUsize,bytes:AtomicUsize,session:Mutex<Option<String>>,video:Mutex<Option<String>>}
impl upload::Transport for BoundedTransport {
    fn send(&self,request:upload::Request)->Result<upload::Response,CloudError> {
        if Instant::now()>self.deadline||self.calls.fetch_add(1,Ordering::SeqCst)>=100{return Err(CloudError::new("QA_NETWORK_BOUND","Verification request bound reached",false));}
        let url=reqwest::Url::parse(&request.url).map_err(|_|CloudError::protocol())?;
        let is_create=request.method==reqwest::Method::POST&&url.host_str()==Some("www.googleapis.com")&&url.path()=="/upload/youtube/v3/videos";
        let is_put=request.method==reqwest::Method::PUT&&self.session.lock().unwrap().as_deref()==Some(request.url.as_str());
        let expected_video=self.video.lock().unwrap().clone();
        let is_get=request.method==reqwest::Method::GET&&url.host_str()==Some("www.googleapis.com")&&url.path()=="/youtube/v3/videos"
            &&expected_video.is_some()&&url.query_pairs().filter(|(k,_)|k=="id").map(|(_,v)|v.into_owned()).collect::<Vec<_>>()==expected_video.into_iter().collect::<Vec<_>>();
        let refresh=request.method==reqwest::Method::POST&&url.as_str()=="https://oauth2.googleapis.com/token"
            &&request.body.split(|b|*b==b'&').any(|value|value==b"grant_type=refresh_token");
        if is_create {
            let data:Value=serde_json::from_slice(&request.body).map_err(|_|CloudError::protocol())?;
            let mut query=url.query_pairs().map(|(k,v)|(k.into_owned(),v.into_owned())).collect::<Vec<_>>();query.sort();
            let mut expected=vec![("uploadType".to_owned(),"resumable".to_owned()),("part".to_owned(),"snippet,status".to_owned()),("notifySubscribers".to_owned(),"false".to_owned())];expected.sort();
            if self.creates.fetch_add(1,Ordering::SeqCst)!=0||data["status"]["privacyStatus"]!="unlisted"
                ||query!=expected
                ||!request.headers.iter().any(|(k,v)|k.eq_ignore_ascii_case("X-Upload-Content-Length")&&v==&BYTES.to_string()){return Err(CloudError::protocol());}
        } else if is_put {
            if request.body.len()>upload::CHUNK_BYTES||self.bytes.fetch_add(request.body.len(),Ordering::SeqCst)+request.body.len()>2*BYTES as usize{return Err(CloudError::protocol());}
        } else if !is_get&&!refresh{return Err(CloudError::new("QA_NETWORK_SCOPE","Out of verification scope",false));}
        let response=self.inner.send(request)?;
        if is_create&&matches!(response.status,200|201){let session=response.header("location")?.filter(|v|upload::valid_session_url(v)).ok_or_else(CloudError::protocol)?;*self.session.lock().unwrap()=Some(session.into());}
        if is_put&&matches!(response.status,200|201){let value=response.json()?;let id=value["id"].as_str().filter(|v|model::video_id(v)).ok_or_else(CloudError::protocol)?;*self.video.lock().unwrap()=Some(id.into());}
        Ok(response)
    }
}
fn host(vault:vault::Vault,catalog:Catalog,resolver:SourceResolver,transport:Arc<dyn upload::Transport>)->Arc<CloudService> {
    Arc::new(CloudService{inner:Mutex::new(Inner{catalog,account_error:None,authorizing:false,auth_workers:0,auth_generation:0,auth_cancel:None,active:None,worker_running:false,closing:false,persistence_error:None,preparation_progress_at:None,automatic_running:false,automatic_observations:Default::default(),automatic_error:None,live:Default::default()}),wake:Condvar::new(),threads:Mutex::new(Vec::new()),vault,resolver,planner:None,open_browser:Arc::new(|_|Err("QA_AUTHORIZATION_FORBIDDEN".into())),transport})
}
struct Drain(Arc<CloudService>);
impl Drop for Drain {fn drop(&mut self){let _=self.0.shutdown_and_wait();}}

#[test]
fn canonical_paths_preserve_original_directory_and_file_guards()->Checked<()> {
    let scratch=tempfile::Builder::new().prefix("atsumi-chzzk-guard-test-").tempdir().map_err(|_|"QA_SCRATCH")?;
    let path=scratch.path().join("guard-fixture.json");
    let bytes=b"read-only guard fixture";
    fs::write(&path,bytes).map_err(|_|"QA_SCRATCH")?;
    let mut originals=Originals{files:vec![]};
    originals.hold(path.clone(),&digest(bytes))?;
    assert_eq!(originals.files[0].path,fs::canonicalize(&path).unwrap());
    assert_eq!(bounded(&path,1024)?,bytes);
    originals.assert_unchanged()?;
    drop(originals);
    Ok(())
}

#[test]
#[ignore="Exactly one fixed CHZZK part11 unlisted upload; explicit opt-in and exclusive QA vault required"]
fn actual_chzzk_selected_part11_unlisted_upload()->Checked<()> {
    if std::env::var("ATSUMI_CHZZK_SELECTED_UPLOAD_QA").as_deref()!=Ok("6e82ac06ed1048c6b8201b29892ff659:11"){return Err("QA_UPLOAD_OPT_IN".into());}
    let data=profile()?;plain(&data.join("cloud-recording/state.bin"))?;
    let vault=vault::Vault::open(&data).map_err(code)?;let initial=vault.load().map_err(code)?;
    if initial.jobs.iter().any(|j|j.view.source.provider=="chzzk"){return Err("QA_EXISTING_CHZZK_NO_RECREATE".into());}
    let expected=baseline(&initial,false)?;
    let originals=Originals::open()?;let scratch=tempfile::Builder::new().prefix("atsumi-chzzk-upload-proof-").tempdir().map_err(|_|"QA_SCRATCH")?;
    let (source,mut media_check)=selected_file(&originals,scratch.path(),&initial.grant.as_ref().unwrap().channel_id)?;
    let source_slot=Arc::new(Mutex::new(Some(source)));let resolve=source_slot.clone();
    let transport=Arc::new(BoundedTransport{inner:upload::GoogleTransport::new().map_err(code)?,deadline:Instant::now()+Duration::from_secs(600),calls:AtomicUsize::new(0),creates:AtomicUsize::new(0),bytes:AtomicUsize::new(0),session:Mutex::new(None),video:Mutex::new(None)});
    let cloud=host(vault,initial,Arc::new(move |r,c|{if *r!=reference()||c.cancel.load(Ordering::Acquire){return Err("QA_SOURCE_SCOPE".into());}resolve.lock().unwrap().take().ok_or("QA_SOURCE_ALREADY_USED".into())}),transport.clone());
    let drain=Drain(cloud.clone());
    let result=(||->Checked<Value>{
        cloud.enqueue(reference()).map_err(code)?;let deadline=Instant::now()+Duration::from_secs(600);let mut previous=String::new();
        loop {
            let snapshot=cloud.snapshot().map_err(code)?;let job=snapshot.jobs.iter().find(|j|j.source==reference()).ok_or("QA_NEW_JOB_MISSING")?;
            {let inner=cloud.lock().map_err(code)?;if baseline(&inner.catalog,true)?!=expected{return Err("QA_EXISTING_STATE_CHANGED".into());}}
            if job.state!=previous {println!("QA_CHZZK_STATE={}",job.state);previous=job.state.clone();}
            if job.state=="ready" {
                let auth=cloud.replay_authorization(&data,&job.id).map_err(code)?;
                if auth.source_sha256!=MEDIA_SHA||auth.bytes!=BYTES||(auth.duration_seconds-DURATION).abs()>0.000001||(auth.media_offset_seconds-OFFSET).abs()>0.000001||auth.description.coverage.state!="incomplete"||job.visibility!=CloudVisibility::Unlisted{return Err("QA_READY_RECEIPT_MISMATCH".into());}
                return Ok(json!({"ready":true,"visibility":"unlisted","actualEncryptedReceipt":true,"ownerDurationProcessingValidated":true,"selectedPartIndex":11,"bytes":BYTES,"durationSeconds":DURATION,"offsetSeconds":OFFSET,"parentComplete":false,"actualRemoteAvVerified":false}));
            }
            if matches!(job.state.as_str(),"failed"|"paused"|"retry_wait"|"waiting_auth"){return Err(job.error.clone().map(code).unwrap_or("QA_UPLOAD_STOPPED".into()));}
            if Instant::now()>deadline||workspace().join(".runtime/requirements-audit/20261117-cloud-goal-resume/chzzk-upload.cancel").exists(){return Err("QA_UPLOAD_DEADLINE_OR_CANCEL".into());}
            std::thread::sleep(Duration::from_millis(300));
        }
    })();
    let stopped=cloud.shutdown_and_wait().map_err(code);drop(drain);
    originals.assert_unchanged()?;
    if hash_source(&mut media_check,&AtomicBool::new(false)).map_err(code)?!=MEDIA_SHA{return Err("QA_MEDIA_CHANGED_AFTER_UPLOAD".into());}
    let saved=cloud.vault.load().map_err(code)?;
    let created=saved.jobs.iter().any(|j|j.view.source==reference());
    if baseline(&saved,created)?!=expected{return Err("QA_EXISTING_STATE_CHANGED_AFTER_UPLOAD".into());}
    let report=json!({"scope":"agent-fixed single validation part, not an automatic selection","result":result.as_ref().ok(),"errorCode":result.as_ref().err(),"normalDrain":stopped.is_ok(),"original13SmallFilesAndMediaShaUnchanged":true,"oldThreeReadyJobsAndAutomaticConfigUnchanged":true,"createRequests":transport.creates.load(Ordering::SeqCst),"requests":transport.calls.load(Ordering::SeqCst),"sentMediaBytes":transport.bytes.load(Ordering::SeqCst),"actualRemoteAvVerified":false});
    let path=workspace().join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(format!("chzzk-selected-upload-result-{}.json",uuid::Uuid::new_v4().simple()));
    fs::write(&path,serde_json::to_vec_pretty(&report).map_err(|_|"QA_REPORT")?).map_err(|_|"QA_REPORT")?;
    println!("QA_CHZZK_RESULT={}",path.display());stopped?;result.map(|_|())
}

struct NoTransport;
impl upload::Transport for NoTransport {fn send(&self,_:upload::Request)->Result<upload::Response,CloudError>{Err(CloudError::protocol())}}
pub(super) struct ReadyReceipt {
    pub authorization:replay::CloudReplayAuthorization,pub job:CloudJobSnapshot,
    source:PathBuf,sha:String,_service:Arc<CloudService>,scratch:tempfile::TempDir,
}
impl ReadyReceipt {
    pub(super) fn assert_unchanged(&self)->Checked<()> {
        if digest(&bounded(&self.source,32*1024)?)!=self.sha{return Err("QA_RECEIPT_CHANGED".into());}
        let state=self._service.lock().map_err(code)?;
        if state.active.is_some()||state.worker_running||state.automatic_running||state.authorizing||state.closing||!self._service.threads.lock().unwrap().is_empty(){return Err("QA_RECEIPT_WORK_STARTED".into());}Ok(())
    }
    pub(super) fn finish(self)->Checked<()> {
        self.assert_unchanged()?;let Self{_service,scratch,..}=self;drop(_service);
        let root=fs::canonicalize(scratch.path()).map_err(|_|"QA_CLEANUP")?;let temp=fs::canonicalize(std::env::temp_dir()).map_err(|_|"QA_CLEANUP")?;
        if root.parent()!=Some(temp.as_path()){return Err("QA_CLEANUP_BOUNDARY".into());}plain(&root)?;scratch.close().map_err(|_|"QA_CLEANUP".into())
    }
}
pub(super) fn load_ready_receipt()->Checked<ReadyReceipt> {
    if std::env::var("ATSUMI_CHZZK_NATIVE_REPLAY_QA").as_deref()!=Ok("actual-unlisted-readonly"){return Err("QA_REPLAY_OPT_IN".into());}
    let source=profile()?.join("cloud-recording/state.bin");let encrypted=bounded(&source,32*1024)?;let sha=digest(&encrypted);
    let scratch=tempfile::Builder::new().prefix("atsumi-chzzk-receipt-readonly-").tempdir().map_err(|_|"QA_SCRATCH")?;
    fs::create_dir(scratch.path().join("cloud-recording")).map_err(|_|"QA_SCRATCH")?;
    fs::write(scratch.path().join("cloud-recording/state.bin"),&encrypted).map_err(|_|"QA_SCRATCH")?;
    if digest(&bounded(&source,32*1024)?)!=sha{return Err("QA_RECEIPT_CHANGED_DURING_COPY".into());}
    let vault=vault::Vault::open(scratch.path()).map_err(code)?;let catalog=vault.load().map_err(code)?;baseline(&catalog,true)?;
    let selected=catalog.jobs.iter().find(|j|j.view.source==reference()).ok_or("QA_RECEIPT_MISSING")?;
    if selected.view.state!="ready"||selected.view.visibility!=CloudVisibility::Unlisted||!selected.has_upload_receipt(){return Err("QA_RECEIPT_NOT_READY_UNLISTED".into());}
    let job=selected.view.clone();let service=host(vault,catalog,Arc::new(|_,_|Err("QA_SOURCE_FORBIDDEN".into())),Arc::new(NoTransport));
    let authorization=service.replay_authorization(scratch.path(),&job.id).map_err(code)?;
    if authorization.source_sha256!=MEDIA_SHA||authorization.bytes!=BYTES||(authorization.duration_seconds-DURATION).abs()>0.000001||(authorization.media_offset_seconds-OFFSET).abs()>0.000001||authorization.original_inputs.len()!=1||authorization.original_inputs[0].part_index!=Some(11){return Err("QA_RECEIPT_SOURCE".into());}
    Ok(ReadyReceipt{authorization,job,source,sha,_service:service,scratch})
}

#[test]
fn selected_upload_rejects_notification_default_public_and_other_video_without_network() {
    use upload::Transport;
    for (query,visibility) in [
        ("uploadType=resumable&part=snippet,status","unlisted"),
        ("uploadType=resumable&part=snippet,status&notifySubscribers=true","unlisted"),
        ("uploadType=resumable&part=snippet,status&notifySubscribers=false","public"),
        ("uploadType=resumable&part=snippet,status&notifySubscribers=false&notifySubscribers=false","unlisted"),
    ] {
        let transport=BoundedTransport{inner:upload::GoogleTransport::new().unwrap(),deadline:Instant::now()+Duration::from_secs(1),calls:AtomicUsize::new(0),creates:AtomicUsize::new(0),bytes:AtomicUsize::new(0),session:Mutex::new(None),video:Mutex::new(None)};
        let result=transport.send(upload::Request{method:reqwest::Method::POST,url:format!("https://www.googleapis.com/upload/youtube/v3/videos?{query}"),bearer:None,headers:vec![("X-Upload-Content-Length".into(),BYTES.to_string())],body:serde_json::to_vec(&json!({"status":{"privacyStatus":visibility}})).unwrap()});
        assert!(result.is_err());assert!(transport.session.lock().unwrap().is_none());
        let result=transport.send(upload::Request{method:reqwest::Method::GET,url:"https://www.googleapis.com/youtube/v3/videos?id=AAAAAAAAAAA".into(),bearer:None,headers:vec![],body:vec![]});
        assert_eq!(result.err().unwrap().code,"QA_NETWORK_SCOPE");
    }
}
