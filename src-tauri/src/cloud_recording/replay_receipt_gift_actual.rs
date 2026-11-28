//! Separate ignored-test host for the actual captured SOOP gift recording.
//! Existing historical SOOP42 receipt pins remain unchanged in the parent.
use super::*;

pub(crate) const GIFT_RECORDING: &str = "8fd00700-81d4-401f-a083-0d13fcf286ea";
pub(crate) const GIFT_MEDIA_SHA: &str = "457778363cdd49782cdb8c4b9025521c35d76336ad10275d7af258f012022c44";
pub(crate) const GIFT_CATALOG_SHA: &str = "9224ca11d926311d8dc4f2f45fd7f32b4ae6c899f1b46ba3f23be39e59281cfe";
const CAPTURE: &str = ".runtime/soop-original-qa/baa5338478ee4c8cb1cdda57d278d918";
const STATE_BOUND: u64 = 64 * 1024;

pub(crate) struct ActualGiftReceiptHost {
    service: CloudService, source: PathBuf, capture: PathBuf, scratch: tempfile::TempDir,
    calls: Arc<AtomicUsize>, reads: AtomicUsize, state_sha: String, plan_sha: String,
    plan_id: String, job_id: String,
}
impl ActualGiftReceiptHost {
    pub(super) fn open() -> Self {
        assert_eq!(std::env::var("ATSUMI_SOOP_NATIVE_GIFT_REPLAY_QA").as_deref(),Ok("actual-unlisted-gift-readonly"),"QA_GIFT_OPT_IN");
        let source=PathBuf::from(std::env::var_os("APPDATA").expect("QA_APPDATA")).join(PROFILE);
        let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let capture=workspace.join(CAPTURE);
        let reads=AtomicUsize::new(0);
        let encrypted=read(&source.join("cloud-recording/state.bin"),STATE_BOUND,&reads);
        assert!(!encrypted.is_empty(),"QA_GIFT_STATE_EMPTY");
        let state_sha=digest(&encrypted);
        let scratch=tempfile::Builder::new().prefix("atsumi-soop-gift-receipt-").tempdir().expect("QA_GIFT_TEMP");
        let root=fs::canonicalize(scratch.path()).expect("QA_TEMP_PATH");
        let temp=fs::canonicalize(std::env::temp_dir()).expect("QA_TEMP_PATH");
        assert!(root.parent()==Some(temp.as_path())&&!root.starts_with(fs::canonicalize(workspace).unwrap()),"QA_TEMP_SCOPE");
        plain(&root);
        fs::create_dir(root.join("cloud-recording")).expect("QA_TEMP_CREATE");
        fs::create_dir(root.join("cloud-recording/plans")).expect("QA_TEMP_CREATE");
        write_new(&root.join("cloud-recording/state.bin"),&encrypted);
        let vault=vault::Vault::open(&root).unwrap_or_else(|_| panic!("QA_TEMP_VAULT"));
        let catalog=vault.load().unwrap_or_else(|_| panic!("QA_ACTUAL_DPAPI"));
        let candidates:Vec<_>=catalog.jobs.iter().filter(|j|j.view.source.provider=="soop"
            &&j.view.source.recording_id==GIFT_RECORDING&&!j.view.recording_plan).collect();
        assert_eq!(candidates.len(),1,"QA_GIFT_RECEIPT_AMBIGUOUS");
        let job=candidates[0];
        assert!(job.view.state=="ready"&&job.has_upload_receipt()&&job.view.visibility==CloudVisibility::Unlisted,"QA_GIFT_NOT_READY_UNLISTED");
        assert!(job.source_sha256.as_deref()==Some(GIFT_MEDIA_SHA)&&job.view.total_bytes==16_150_161
            &&(job.view.duration_seconds-108.229).abs()<0.000_001&&job.view.media_offset_seconds==0.0
            &&job.view.source.part_index.is_none(),"QA_GIFT_RECEIPT_IDENTITY");
        let plan_id=job.view.source.plan_id.clone().expect("QA_GIFT_PLAN");
        let unit_id=job.view.source.unit_id.as_deref().expect("QA_GIFT_UNIT");
        assert!(uuid::Uuid::parse_str(&plan_id).is_ok_and(|v|v.to_string()==plan_id)
            &&uuid::Uuid::parse_str(unit_id).is_ok_and(|v|v.to_string()==unit_id),"QA_GIFT_PLAN_ID");
        let job_id=job.view.id.clone();
        let plan=read(&source.join("cloud-recording/plans").join(format!("{plan_id}.json")),32*1024,&reads);
        let plan_sha=digest(&plan);
        write_new(&root.join("cloud-recording/plans").join(format!("{plan_id}.json")),&plan);
        assert_eq!(digest(&read(&source.join("cloud-recording/state.bin"),STATE_BOUND,&reads)),state_sha,"QA_GIFT_COPY_CHANGED");
        let source_catalog=read(&capture.join("data/platform-media/jobs").join(format!("{GIFT_RECORDING}.json")),16*1024,&reads);
        assert_eq!(digest(&source_catalog),GIFT_CATALOG_SHA,"QA_GIFT_ORIGINAL_CATALOG");
        let registered=read(&source.join("platform-media/jobs").join(format!("{GIFT_RECORDING}.json")),16*1024,&reads);
        assert_eq!(registered,source_catalog,"QA_GIFT_REGISTERED_SOURCE_CHANGED");
        let calls=Arc::new(AtomicUsize::new(0));
        let resolver_calls=calls.clone();let planner_calls=calls.clone();let browser_calls=calls.clone();
        let service=CloudService {
            inner:Mutex::new(Inner {catalog,account_error:None,authorizing:false,auth_workers:0,auth_generation:0,
                auth_cancel:None,active:None,worker_running:false,closing:false,persistence_error:None,
                preparation_progress_at:None,automatic_running:false,automatic_observations:Default::default(),automatic_error:None,live:Default::default()}),
            wake:Condvar::new(),threads:Mutex::new(Vec::new()),vault,
            resolver:Arc::new(move|_,_|{resolver_calls.fetch_add(1,Ordering::Relaxed);Err("QA_FORBIDDEN_SOURCE".into())}),
            planner:Some(Arc::new(move|_,_|{planner_calls.fetch_add(1,Ordering::Relaxed);Err("QA_FORBIDDEN_PLAN".into())})),
            open_browser:Arc::new(move|_|{browser_calls.fetch_add(1,Ordering::Relaxed);Err("QA_FORBIDDEN_BROWSER".into())}),
            transport:Arc::new(NoTransport(calls.clone())),
        };
        Self{service,source,capture,scratch,calls,reads,state_sha,plan_sha,plan_id,job_id}
    }
    pub(crate) fn scratch(&self)->&Path{self.scratch.path()}
    pub(crate) fn capture(&self)->&Path{&self.capture}
    pub(crate) fn catalog_path(&self)->PathBuf{self.capture.join("data/platform-media/jobs").join(format!("{GIFT_RECORDING}.json"))}
    pub(crate) fn read(&self,path:&Path,limit:u64)->Vec<u8>{
        assert!(path.starts_with(&self.source)||path.starts_with(&self.capture)||path.starts_with(self.scratch.path()),"QA_GIFT_READ_SCOPE");
        assert!(!matches!(path.extension().and_then(|s|s.to_str()),Some("mp4"|"m4s"|"webm"|"ts")),"QA_VIDEO_READ_FORBIDDEN");
        read(path,limit,&self.reads)
    }
    pub(crate) fn job_view(&self)->model::CloudJobSnapshot{
        self.service.inner.lock().unwrap().catalog.jobs.iter().find(|j|j.view.id==self.job_id).expect("QA_GIFT_JOB").view.clone()
    }
    pub(crate) fn authorization(&self)->replay::CloudReplayAuthorization{
        let auth=self.service.replay_authorization(self.scratch.path(),&self.job_id).unwrap_or_else(|_| panic!("QA_GIFT_AUTHORIZATION"));
        let inner=self.service.inner.lock().unwrap();let job=inner.catalog.jobs.iter().find(|j|j.view.id==self.job_id).unwrap();
        assert!(auth.source==job.view.source&&auth.owner_channel_id==job.channel_id&&auth.original_inputs.len()==1,"QA_GIFT_OWNER_PLAN");
        let input=&auth.original_inputs[0];
        assert!(input.sha256==GIFT_MEDIA_SHA&&input.bytes==16_150_161&&(input.duration_seconds-108.229).abs()<0.000_001
            &&input.media_offset_seconds==0.0&&input.part_index.is_none()&&auth.matches_recording_generation(""),"QA_GIFT_SOURCE_BINDING");
        assert_eq!(auth.description.parts.len(),1,"QA_GIFT_SINGLE_UNIT");
        auth
    }
    pub(crate) fn assert_unchanged(&self){
        assert_eq!(self.calls.load(Ordering::Relaxed),0,"QA_GIFT_FORBIDDEN_ACTION");
        assert!(self.service.threads.lock().unwrap().is_empty(),"QA_GIFT_WORKER");
        let inner=self.service.inner.lock().unwrap();
        assert!(!inner.authorizing&&inner.auth_workers==0&&inner.active.is_none()&&!inner.worker_running&&!inner.automatic_running&&!inner.closing&&inner.persistence_error.is_none(),"QA_GIFT_WORK_STATE");drop(inner);
        for root in [self.source.as_path(),self.scratch.path()]{
            assert_eq!(digest(&self.read(&root.join("cloud-recording/state.bin"),STATE_BOUND)),self.state_sha,"QA_GIFT_STATE_CHANGED");
            assert_eq!(digest(&self.read(&root.join("cloud-recording/plans").join(format!("{}.json",self.plan_id)),32*1024)),self.plan_sha,"QA_GIFT_PLAN_CHANGED");
        }
        assert_eq!(digest(&self.read(&self.catalog_path(),16*1024)),GIFT_CATALOG_SHA,"QA_GIFT_CATALOG_CHANGED");
        assert_eq!(digest(&self.read(&self.source.join("platform-media/jobs").join(format!("{GIFT_RECORDING}.json")),16*1024)),GIFT_CATALOG_SHA,"QA_GIFT_REGISTERED_CHANGED");
    }
    pub(crate) fn finish(self)->usize{
        self.assert_unchanged();let count=self.reads.load(Ordering::Relaxed);
        let root=fs::canonicalize(self.scratch.path()).expect("QA_TEMP_PATH");
        assert_eq!(root.parent(),Some(fs::canonicalize(std::env::temp_dir()).unwrap().as_path()),"QA_TEMP_CLEANUP_SCOPE");
        let Self{service,scratch,..}=self;drop(service);scratch.close().expect("QA_GIFT_TEMP_CLEANUP");assert!(!root.exists(),"QA_GIFT_TEMP_REMAINS");count
    }
}
