//! Test-only exact ready Private receipt + pinned originals; no workers, API or copied vault.
use super::*;
use crate::{platform_media::MediaJob, streaming::browser_store};
use serde_json::Value;
use std::{collections::HashSet, fs, sync::atomic::AtomicUsize};
type Checked<T> = Result<T, String>;
const ID: &str = "3bae70dd-ca33-401b-aa95-a508bab9ffc4";
const CAPTURE: &str = "0d19e1f866b84ef8a2edef3980bb923b";
const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const BYTES: u64 = 17_633_383;
const DURATION: f64 = 118.177334;
const MEDIA_SHA: &str = "08c0087446f4485dd11151f10f31b3396409aaad3055ae0ff02fa153e8c9a106";
const CATALOG_SHA: &str = "f1d987d6f78644f79587bbf5ec08a3567fd46e37f9eeb5102fe1bb04f284ed9f";
const MANIFEST_SHA: &str = "bf892399d2cdd62a0bb856c6975cbb5a23edfcef9cc00f28c4e809b81440b978";
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
fn audit() -> PathBuf {
    workspace().join(".runtime/requirements-audit/20261122-cloud-resume")
}
fn capture_root() -> PathBuf {
    workspace().join(".runtime/soop-original-qa").join(CAPTURE)
}
fn selected(value: &CloudSourceRef) -> bool {
    value.provider == "soop"
        && value.recording_id == ID
        && value.part_index.is_none()
        && value.valid()
}
fn code(error: CloudError) -> String {
    if error.code.len() <= 64
        && error
            .code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        error.code
    } else {
        "QA_CLOUD_ERROR".into()
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn plain(path: &Path) -> Checked<()> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        let meta = fs::symlink_metadata(part).map_err(|_| "QA_PATH")?;
        if meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0 {
            return Err("QA_LINK".into());
        }
    }
    Ok(())
}
fn canonical(path: &Path) -> Checked<PathBuf> {
    plain(path)?;
    let value = fs::canonicalize(path).map_err(|_| "QA_CANONICAL")?;
    plain(&value)?;
    Ok(value)
}
fn read(path: &Path, limit: u64) -> Checked<Vec<u8>> {
    let path = canonical(path)?;
    let mut file =
        browser_store::cleanup::verification_guard(&path).map_err(|_| "QA_READ_GUARD")?;
    let length = file.metadata().map_err(|_| "QA_READ_STAT")?.len();
    if length > limit {
        return Err("QA_READ_BOUND".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "QA_READ")?;
    if bytes.len() as u64 != length {
        return Err("QA_READ_CHANGED".into());
    }
    Ok(bytes)
}
struct Held {
    path: PathBuf,
    file: File,
    bytes: u64,
    sha: String,
    _directories: Vec<File>,
}
#[derive(Default)]
struct Originals {
    files: Vec<Held>,
}
impl Originals {
    fn hold(&mut self, path: &Path, bytes: u64, sha: &str) -> Checked<()> {
        if !valid_sha(sha) || bytes > 32 * 1024 * 1024 {
            return Err("QA_EXPECTED_FILE".into());
        }
        let path = canonical(path)?;
        let directories =
            browser_store::cleanup::pin_directories(path.parent().ok_or("QA_PARENT")?)
                .map_err(|_| "QA_DIRECTORY_GUARD")?;
        let mut file =
            browser_store::cleanup::verification_guard(&path).map_err(|_| "QA_FILE_GUARD")?;
        if file.metadata().map_err(|_| "QA_STAT")?.len() != bytes
            || hash_source(&mut file, &AtomicBool::new(false)).map_err(code)? != sha
        {
            return Err("QA_ORIGINAL_CHANGED".into());
        }
        self.files.push(Held {
            path,
            file,
            bytes,
            sha: sha.into(),
            _directories: directories,
        });
        Ok(())
    }
    fn open() -> Checked<Self> {
        let bytes = read(
            &audit().join("private-upload-118-source-files.json"),
            32 * 1024,
        )?;
        if digest(&bytes) != MANIFEST_SHA {
            return Err("QA_SOURCE_MANIFEST".into());
        }
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|_| "QA_SOURCE_MANIFEST")?;
        let entries = manifest["files"]
            .as_array()
            .filter(|v| v.len() == 37)
            .ok_or("QA_SOURCE_FILE_COUNT")?;
        let mut paths = HashSet::new();
        let mut result = Self::default();
        for entry in entries {
            let name = entry["relative"].as_str().ok_or("QA_SOURCE_PATH")?;
            if name.len() > 240
                || name.contains('\\')
                || !Path::new(name)
                    .components()
                    .all(|p| matches!(p, std::path::Component::Normal(_)))
                || !paths.insert(name)
            {
                return Err("QA_SOURCE_PATH".into());
            }
            result.hold(
                &capture_root().join(name),
                entry["bytes"].as_u64().ok_or("QA_SOURCE_BYTES")?,
                entry["sha256"].as_str().ok_or("QA_SOURCE_SHA")?,
            )?;
        }
        Ok(result)
    }
    fn unchanged(&mut self) -> Checked<()> {
        for held in &mut self.files {
            if canonical(&held.path)? != held.path
                || held.file.metadata().map_err(|_| "QA_STAT")?.len() != held.bytes
                || hash_source(&mut held.file, &AtomicBool::new(false)).map_err(code)? != held.sha
            {
                return Err("QA_ORIGINAL_CHANGED_AFTER".into());
            }
        }
        Ok(())
    }
}
fn selected_catalog(data: &Path, pins: &mut Originals) -> Checked<MediaJob> {
    let original = read(
        &capture_root().join(format!("data/platform-media/jobs/{ID}.json")),
        1024 * 1024,
    )?;
    if digest(&original) != CATALOG_SHA {
        return Err("QA_SOURCE_CATALOG".into());
    }
    let job: MediaJob = serde_json::from_slice(&original).map_err(|_| "QA_SOURCE_JOB")?;
    let item = job.library.as_ref().ok_or("QA_SOURCE_LIBRARY")?;
    let raw: Value = serde_json::from_slice(&original).map_err(|_| "QA_SOURCE_JOB")?;
    if job.job_id != ID
        || job.request_id != "d31ecfbc-dfe5-473e-8c1a-be146e4acb78"
        || job.state != "completed"
        || job.revision != 16
        || job.reference.provider != "soop"
        || job.reference.kind != "live"
        || job.reference.external_id != "hinqocorv"
        || job.broadcast_id.as_deref() != Some("298642901")
        || item.broadcast_id != job.broadcast_id
        || item.reference != job.reference
        || item.library_id != ID
        || item.revision != 16
        || item.bytes != BYTES
        || item.duration != DURATION
        || item.sha256 != MEDIA_SHA
        || item.modified_millis != 1794872019667
        || canonical(&job.download_root)? != canonical(&capture_root().join("media"))?
        || raw["library"]["captureOutcome"]["state"] != "interrupted"
        || raw["library"]["captureOutcome"]["reasons"]
            != serde_json::json!(["source_interrupted", "audio_coverage_tail"])
    {
        return Err("QA_SOURCE_IDENTITY".into());
    }
    let journal = data.join("platform-media/jobs");
    let mut files = fs::read_dir(&journal)
        .map_err(|_| "QA_CATALOG_DIRECTORY")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "QA_CATALOG_DIRECTORY")?;
    if files.len() > 128 {
        return Err("QA_CATALOG_BOUND".into());
    }
    files.sort_by_key(|entry| entry.file_name());
    let mut selected_count = 0;
    for entry in files {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let bytes = read(&path, 1024 * 1024)?;
        pins.hold(&path, bytes.len() as u64, &digest(&bytes))?;
        let rows = if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".batch.json"))
        {
            serde_json::from_slice::<Vec<MediaJob>>(&bytes).map_err(|_| "QA_CATALOG_JSON")?
        } else {
            vec![serde_json::from_slice::<MediaJob>(&bytes).map_err(|_| "QA_CATALOG_JSON")?]
        };
        if rows.len() > 100 || rows.iter().any(|row| row.active()) {
            return Err("QA_EXISTING_PLATFORM_WORK".into());
        }
        for row in rows {
            if row.job_id == ID {
                selected_count += 1;
                if path.file_name().and_then(|name| name.to_str()) != Some(&format!("{ID}.json"))
                    || bytes != original
                {
                    return Err("QA_IMPORTED_CATALOG_CHANGED".into());
                }
            } else if row.request_id == job.request_id {
                return Err("QA_DUPLICATE_REQUEST".into());
            }
        }
    }
    if selected_count != 1 {
        return Err("QA_IMPORTED_CATALOG_COUNT".into());
    }
    Ok(job)
}

struct NoNetwork(Arc<AtomicUsize>);
impl upload::Transport for NoNetwork {
    fn send(&self, _: upload::Request) -> Result<upload::Response, CloudError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CloudError::protocol())
    }
}
fn service(
    vault: vault::Vault,
    catalog: Catalog,
    transport: Arc<dyn upload::Transport>,
) -> CloudService {
    CloudService {
        inner: Mutex::new(Inner {
            catalog,
            account_error: None,
            authorizing: false,
            auth_workers: 0,
            auth_generation: 0,
            auth_cancel: None,
            active: None,
            worker_running: false,
            closing: false,
            persistence_error: None,
            preparation_progress_at: None,
            automatic_running: false,
            automatic_observations: Default::default(),
            automatic_error: None,
            live: Default::default(),
        }),
        wake: Condvar::new(),
        threads: Mutex::new(Vec::new()),
        vault,
        resolver: Arc::new(|_, _| Err("QA_SOURCE_ACTION_FORBIDDEN".into())),
        planner: None,
        open_browser: Arc::new(|_| Err("QA_AUTHORIZATION_FORBIDDEN".into())),
        transport,
    }
}
pub(crate) struct PrivateReceiptHost {
    service: CloudService,
    originals: Originals,
    data: PathBuf,
    source_job: MediaJob,
    job: CloudJobSnapshot,
    authorization: replay::CloudReplayAuthorization,
    calls: Arc<AtomicUsize>,
}
impl PrivateReceiptHost {
    pub(crate) fn open() -> Checked<Self> {
        if std::env::var("ATSUMI_SOOP_PRIVATE_REPLAY_QA").as_deref() != Ok(ID) {
            return Err("QA_PRIVATE_REPLAY_OPT_IN".into());
        }
        let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(PROFILE);
        let profile = data.join("youtube-original-profile");
        plain(&profile)?;
        if !profile.is_dir() {
            return Err("QA_EXISTING_BROWSER_PROFILE_REQUIRED".into());
        }
        let mut originals = Originals::open()?;
        let source_job = selected_catalog(&data, &mut originals)?;
        // Pin the existing ciphertext in place. No copy/decryption outside the production Vault.
        let state = data.join("cloud-recording/state.bin");
        let mut bytes = read(&state, 8 * 1024 * 1024)?;
        let state_len = bytes.len() as u64;
        let state_sha = digest(&bytes);
        bytes.fill(0);
        drop(bytes);
        originals.hold(&state, state_len, &state_sha)?;
        let vault = vault::Vault::open(&data).map_err(code)?;
        let catalog = vault.load().map_err(code)?;
        if catalog.jobs.iter().any(|j| {
            matches!(
                j.view.state.as_str(),
                "queued" | "validating" | "uploading" | "processing" | "retry_wait"
            )
        }) {
            return Err("QA_EXISTING_CLOUD_WORK".into());
        }
        let selected_jobs = catalog
            .jobs
            .iter()
            .filter(|j| selected(&j.view.source))
            .collect::<Vec<_>>();
        if selected_jobs.len() != 1 {
            return Err("QA_EXACT_PRIVATE_RECEIPT_REQUIRED".into());
        }
        let selected = selected_jobs[0];
        if selected.view.visibility != CloudVisibility::Private
            || selected.view.state != "ready"
            || !selected.has_upload_receipt()
            || selected.automatic.is_some()
            || selected.view.source.plan_id.is_none()
            || selected.view.source.unit_id.is_none()
            || catalog
                .grant
                .as_ref()
                .is_none_or(|grant| grant.channel_id != selected.channel_id)
            || selected
                .view
                .video_id
                .as_deref()
                .is_none_or(|v| !model::video_id(v))
        {
            return Err("QA_PRIVATE_RECEIPT_IDENTITY".into());
        }
        let job = selected.view.clone();
        let plan = job.source.plan_id.as_deref().ok_or("QA_PLAN_REQUIRED")?;
        if !uuid::Uuid::parse_str(plan).is_ok_and(|v| v.to_string() == plan) {
            return Err("QA_PLAN_ID".into());
        }
        let plan_path = data
            .join("cloud-recording/plans")
            .join(format!("{plan}.json"));
        let plan_bytes = read(&plan_path, 1024 * 1024)?;
        originals.hold(&plan_path, plan_bytes.len() as u64, &digest(&plan_bytes))?;
        let calls = Arc::new(AtomicUsize::new(0));
        let service = service(vault, catalog, Arc::new(NoNetwork(calls.clone())));
        let authorization = service.replay_authorization(&data, &job.id).map_err(code)?;
        if authorization.source_sha256 != MEDIA_SHA
            || authorization.bytes != BYTES
            || authorization.duration_seconds != DURATION
            || authorization.media_offset_seconds != 0.
            || authorization.original_inputs.len() != 1
            || authorization.original_inputs[0].sha256 != MEDIA_SHA
            || authorization.original_inputs[0].bytes != BYTES
            || authorization.source != job.source
            || authorization.description.parts.len() != 1
        {
            return Err("QA_PRIVATE_SOURCE_BINDING".into());
        }
        Ok(Self {
            service,
            originals,
            data,
            source_job,
            job,
            authorization,
            calls,
        })
    }
    pub(crate) fn data(&self) -> &Path {
        &self.data
    }
    pub(crate) fn source_job(&self) -> MediaJob {
        self.source_job.clone()
    }
    pub(crate) fn job(&self) -> CloudJobSnapshot {
        self.job.clone()
    }
    pub(crate) fn authorization(&self) -> replay::CloudReplayAuthorization {
        self.authorization.clone()
    }
    pub(crate) fn unchanged(&mut self) -> Checked<()> {
        self.originals.unchanged()?;
        if self.calls.load(Ordering::SeqCst) != 0
            || !self
                .service
                .threads
                .lock()
                .map_err(|_| "QA_THREADS")?
                .is_empty()
        {
            return Err("QA_ACTION_STARTED".into());
        }
        let inner = self.service.lock().map_err(code)?;
        if inner.authorizing
            || inner.auth_workers != 0
            || inner.active.is_some()
            || inner.worker_running
            || inner.automatic_running
            || inner.closing
            || inner.persistence_error.is_some()
        {
            return Err("QA_WORK_STATE_CHANGED".into());
        }
        Ok(())
    }
}
