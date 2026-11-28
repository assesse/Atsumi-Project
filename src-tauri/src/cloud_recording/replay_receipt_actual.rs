//! Test-only, no-worker host for the fixed actual encrypted QA receipt.
//! Never construct this against an app profile; only Vault::load sees a temp copy.
use super::*;
use std::{fs, io::Write, sync::atomic::AtomicUsize};

#[path = "replay_receipt_gift_actual.rs"]
mod gift_actual;
pub(crate) use gift_actual::ActualGiftReceiptHost;

pub(crate) const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
pub(crate) const RECORDING: &str = "241bc41d-d3e6-4bf9-a205-899f2c2d21d5";
const JOB: &str = "e0d2937d-74bb-48c7-97fe-c558b66bfb32";
const PLAN: &str = "660273f9-99dc-8432-ad0f-fcc789dcd164";
const UNIT: &str = "d20ca341-df3e-8e01-9d1e-ac13734869d4";
const STATE_SHA: &str = "983486ce2547f5d41f9eded5d179dc8f56db9e1dffeab725079ee2037e9765c5";
const PLAN_SHA: &str = "f8a1ab7f4ce4a28dfe475fe8281711a346c54b3064bb53e16fe800f885da9bb5";
pub(crate) const MEDIA_SHA: &str = "5c5fcddd3294ae9ffe3b7b3063e8c8a1a46cecc7b6a7a004629f9e715b825f75";
const READ_LIMIT: usize = 4 * 1024 * 1024;

struct NoTransport(Arc<AtomicUsize>);
impl upload::Transport for NoTransport {
    fn send(&self, _request: upload::Request) -> Result<upload::Response, CloudError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(CloudError::protocol())
    }
}
pub(crate) fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn plain(path: &Path) {
    use std::os::windows::fs::MetadataExt;
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor).unwrap_or_else(|_| panic!("QA_PATH_STAT"));
        assert!(!meta.file_type().is_symlink() && meta.file_attributes() & 0x400 == 0, "QA_LINKED_PATH");
    }
}
fn read(path: &Path, limit: u64, total: &AtomicUsize) -> Vec<u8> {
    plain(path);
    let mut file = File::open(path).unwrap_or_else(|_| panic!("QA_FILE_READ"));
    let meta = file.metadata().unwrap_or_else(|_| panic!("QA_FILE_STAT"));
    assert!(meta.is_file() && meta.len() <= limit, "QA_READ_BOUND");
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes).unwrap_or_else(|_| panic!("QA_FILE_READ"));
    assert!(bytes.len() as u64 <= limit && bytes.len() as u64 == meta.len(), "QA_READ_CHANGED");
    let previous = total.fetch_add(bytes.len(), Ordering::Relaxed);
    assert!(previous + bytes.len() <= READ_LIMIT, "QA_TOTAL_READ_BOUND");
    bytes
}
fn write_new(path: &Path, bytes: &[u8]) {
    plain(path.parent().unwrap());
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)
        .unwrap_or_else(|_| panic!("QA_SCRATCH_CREATE"));
    file.write_all(bytes).unwrap_or_else(|_| panic!("QA_SCRATCH_WRITE"));
}
pub(crate) struct ActualReceiptHost {
    service: CloudService,
    source: PathBuf,
    scratch: tempfile::TempDir,
    calls: Arc<AtomicUsize>,
    reads: AtomicUsize,
    source_state_sha: String,
    pub(crate) grant_initially_present: bool,
}
impl ActualReceiptHost {
    pub(crate) fn open_current_gift() -> ActualGiftReceiptHost {
        ActualGiftReceiptHost::open()
    }
    pub(crate) fn open() -> Self {
        Self::open_snapshot(false)
    }
    pub(crate) fn open_current() -> Self {
        Self::open_snapshot(true)
    }
    fn open_snapshot(current_semantic: bool) -> Self {
        if current_semantic {
            assert!(std::env::var("ATSUMI_SOOP_CURRENT_RECEIPT_QA").as_deref() == Ok("read-only-current-semantic"), "QA_CURRENT_OPT_IN_REQUIRED");
        } else {
            assert!(std::env::var("ATSUMI_SOOP_RECEIPT_QA").as_deref() == Ok("read-only-encrypted-copy"), "QA_OPT_IN_REQUIRED");
        }
        let source = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_else(|| panic!("QA_APPDATA"))).join(PROFILE);
        let reads = AtomicUsize::new(0);
        let encrypted = read(&source.join("cloud-recording/state.bin"), 16 * 1024, &reads);
        let plan = read(&source.join("cloud-recording/plans").join(format!("{PLAN}.json")), 16 * 1024, &reads);
        let source_state_sha = digest(&encrypted);
        if !current_semantic {
            assert!(encrypted.len() == 3496 && source_state_sha == STATE_SHA, "QA_ENCRYPTED_SOURCE_CHANGED");
        } else {
            assert!(!encrypted.is_empty() && encrypted.len() <= 16 * 1024, "QA_CURRENT_CIPHERTEXT_BOUND");
        }
        assert!(plan.len() == 953 && digest(&plan) == PLAN_SHA, "QA_PLAN_SOURCE_CHANGED");
        let scratch = tempfile::Builder::new().prefix("atsumi-soop-receipt-readonly-").tempdir()
            .unwrap_or_else(|_| panic!("QA_SCRATCH_CREATE"));
        let root = fs::canonicalize(scratch.path()).unwrap_or_else(|_| panic!("QA_SCRATCH_PATH"));
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| panic!("QA_TEMP_PATH"));
        let workspace = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
            .unwrap_or_else(|_| panic!("QA_WORKSPACE_PATH"));
        assert!(root.parent() == Some(temp.as_path()) && !root.starts_with(&workspace), "QA_SCRATCH_NOT_OS_TEMP");
        plain(&root);
        fs::create_dir(root.join("cloud-recording")).unwrap_or_else(|_| panic!("QA_SCRATCH_CREATE"));
        fs::create_dir(root.join("cloud-recording/plans")).unwrap_or_else(|_| panic!("QA_SCRATCH_CREATE"));
        write_new(&root.join("cloud-recording/state.bin"), &encrypted);
        write_new(&root.join("cloud-recording/plans").join(format!("{PLAN}.json")), &plan);
        assert!(digest(&read(&source.join("cloud-recording/state.bin"), 16 * 1024, &reads)) == source_state_sha, "QA_SOURCE_CHANGED_DURING_COPY");
        let vault = vault::Vault::open(&root).unwrap_or_else(|_| panic!("QA_SCRATCH_VAULT_OPEN"));
        let catalog = vault.load().unwrap_or_else(|_| panic!("QA_ACTUAL_DPAPI_LOAD"));
        let job = catalog.jobs.iter().find(|job| job.view.id == JOB).unwrap_or_else(|| panic!("QA_ACTUAL_JOB_MISSING"));
        assert!(job.view.state == "ready" && !job.view.recording_plan && job.has_upload_receipt(), "QA_ACTUAL_RECEIPT_NOT_READY");
        assert!(job.view.visibility == CloudVisibility::Unlisted, "QA_ACTUAL_RECEIPT_NOT_UNLISTED");
        assert!(job.view.source.provider == "soop" && job.view.source.recording_id == RECORDING
            && job.view.source.plan_id.as_deref() == Some(PLAN)
            && job.view.source.unit_id.as_deref() == Some(UNIT) && job.view.source.part_index.is_none(), "QA_ACTUAL_RECEIPT_SOURCE");
        assert!(job.source_sha256.as_deref() == Some(MEDIA_SHA) && job.view.total_bytes == 6_912_943
            && (job.view.duration_seconds - 42.102521).abs() < 0.000_001
            && job.view.media_offset_seconds == 0.0, "QA_ACTUAL_RECEIPT_MEDIA");
        let grant_initially_present = catalog.grant.is_some();
        let calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls = calls.clone();
        let planner_calls = calls.clone();
        let browser_calls = calls.clone();
        // Deliberately bypass constructors: no recovery, persistence or workers.
        let service = CloudService {
            inner: Mutex::new(Inner {
                catalog, account_error: None, authorizing: false, auth_workers: 0,
                auth_generation: 0, auth_cancel: None, active: None, worker_running: false,
                closing: false, persistence_error: None, preparation_progress_at: None,
                automatic_running: false, automatic_observations: Default::default(), automatic_error: None,
                live: Default::default(),
            }),
            wake: Condvar::new(), threads: Mutex::new(Vec::new()), vault,
            resolver: Arc::new(move |_, _| { resolver_calls.fetch_add(1, Ordering::Relaxed); Err("QA_FORBIDDEN_SOURCE".into()) }),
            planner: Some(Arc::new(move |_, _| { planner_calls.fetch_add(1, Ordering::Relaxed); Err("QA_FORBIDDEN_PLAN".into()) })),
            open_browser: Arc::new(move |_| { browser_calls.fetch_add(1, Ordering::Relaxed); Err("QA_FORBIDDEN_BROWSER".into()) }),
            transport: Arc::new(NoTransport(calls.clone())),
        };
        Self { service, source, scratch, calls, reads, source_state_sha, grant_initially_present }
    }
    pub(crate) fn ciphertext_sha256(&self) -> &str { &self.source_state_sha }
    pub(crate) fn job_view(&self) -> model::CloudJobSnapshot {
        self.service.inner.lock().unwrap().catalog.jobs.iter()
            .find(|job|job.view.id==JOB).unwrap_or_else(||panic!("QA_ACTUAL_JOB_MISSING"))
            .view.clone()
    }
    pub(crate) fn ready_provider_counts(&self) -> (usize, usize, usize) {
        let inner=self.service.inner.lock().unwrap();
        let count=|provider:&str|inner.catalog.jobs.iter().filter(|job|
            job.view.source.provider==provider && job.view.state=="ready"
            && !job.view.recording_plan && job.has_upload_receipt()).count();
        (count("soop"),count("chzzk"),count("youtube"))
    }
    pub(crate) fn profile(&self) -> &Path { &self.source }
    pub(crate) fn scratch(&self) -> &Path { self.scratch.path() }
    pub(crate) fn read(&self, path: &Path, limit: u64) -> Vec<u8> {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let media_root = workspace.join(".runtime/platform-app-qa").join(PROFILE).join("media");
        assert!(path.starts_with(&self.source) || path.starts_with(&media_root) || path.starts_with(self.scratch.path()), "QA_READ_PATH_SCOPE");
        assert!(path.file_name().and_then(|v| v.to_str()).is_none_or(|v| v != "media.mp4"), "QA_VIDEO_READ_FORBIDDEN");
        read(path, limit, &self.reads)
    }
    pub(crate) fn authorization(&self) -> replay::CloudReplayAuthorization {
        let value = self.service.replay_authorization(self.scratch.path(), JOB)
            .unwrap_or_else(|_| panic!("QA_NATIVE_RECEIPT_AUTHORIZATION"));
        let inner = self.service.inner.lock().unwrap();
        let actual = inner.catalog.jobs.iter().find(|job| job.view.id == JOB).unwrap();
        assert!(value.owner_channel_id == actual.channel_id && value.source == actual.view.source, "QA_OWNER_SOURCE_BINDING");
        assert!(value.original_inputs.len() == 1 && value.original_inputs[0].sha256 == MEDIA_SHA
            && value.original_inputs[0].modified_nanos == Some(1_792_072_473_618_217_000)
            && value.original_inputs[0].bytes == 6_912_943 && value.matches_recording_generation(""), "QA_PLAN_INPUT_BINDING");
        assert!(value.description.coverage.state == "complete" && value.description.parts.len() == 1, "QA_PLAN_DESCRIPTOR");
        value
    }
    pub(crate) fn without_current_grant(&self) -> replay::CloudReplayAuthorization {
        // Only the loaded in-memory copy is changed. Never disconnect or publish.
        self.service.inner.lock().unwrap().catalog.grant.take();
        self.authorization()
    }
    pub(crate) fn assert_unchanged(&self) {
        assert!(self.calls.load(Ordering::Relaxed) == 0, "QA_FORBIDDEN_ACTION_CALLED");
        assert!(self.service.threads.lock().unwrap().is_empty(), "QA_BACKGROUND_THREAD_CREATED");
        let inner = self.service.inner.lock().unwrap();
        assert!(!inner.authorizing && inner.auth_workers == 0 && inner.active.is_none()
            && !inner.worker_running && !inner.automatic_running && !inner.closing
            && inner.persistence_error.is_none(), "QA_WORK_STATE_CHANGED");
        drop(inner);
        for root in [&self.source, &self.scratch.path().to_owned()] {
            assert!(digest(&self.read(&root.join("cloud-recording/state.bin"), 16 * 1024)) == self.source_state_sha, "QA_ENCRYPTED_CATALOG_MUTATED");
            assert!(digest(&self.read(&root.join("cloud-recording/plans").join(format!("{PLAN}.json")), 16 * 1024)) == PLAN_SHA, "QA_PLAN_MUTATED");
        }
    }
    pub(crate) fn finish(self) -> usize {
        self.assert_unchanged();
        fn tally(path: &Path, files: &mut Vec<PathBuf>, total: &mut u64) {
            plain(path);
            for entry in fs::read_dir(path).unwrap_or_else(|_| panic!("QA_SCRATCH_LIST")) {
                let entry = entry.unwrap_or_else(|_| panic!("QA_SCRATCH_LIST"));
                let meta = entry.metadata().unwrap_or_else(|_| panic!("QA_SCRATCH_STAT"));
                if meta.is_dir() { tally(&entry.path(), files, total); }
                else { plain(&entry.path()); *total += meta.len(); files.push(entry.path()); }
            }
        }
        let mut files = Vec::new(); let mut total = 0;
        tally(self.scratch.path(), &mut files, &mut total);
        let expected = ["cloud-recording/state.bin".to_owned(), "cloud-recording/state.lock".to_owned(),
            format!("cloud-recording/plans/{PLAN}.json")];
        assert!(files.len() == 3 && total <= 1024 * 1024 && files.iter().all(|path|
            expected.iter().any(|value| self.scratch.path().join(value) == *path)), "QA_UNEXPECTED_SCRATCH_WRITE");
        let reads = self.reads.load(Ordering::Relaxed);
        let root = fs::canonicalize(self.scratch.path()).unwrap_or_else(|_| panic!("QA_SCRATCH_PATH"));
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| panic!("QA_TEMP_PATH"));
        assert!(root.parent() == Some(temp.as_path()), "QA_CLEANUP_NOT_CONFINED");
        // Release the scratch vault lock, then remove only this verified TempDir.
        let Self { service, scratch, .. } = self;
        drop(service);
        scratch.close().unwrap_or_else(|_| panic!("QA_SCRATCH_CLEANUP"));
        assert!(!root.exists(), "QA_SCRATCH_LEFT_BEHIND");
        reads
    }
}

