//! Exact, manually selected SOOP membership PRIVATE upload verification. Test-only; never starts from the app.
//! Contract: requirements-audit/20261122-cloud-resume/private-owner-probe-preedit.json.
use super::super::*;
use crate::{
    platform_media::{MediaJob, PlatformMediaService, PlatformMediaTools},
    streaming::{browser_store, startup::DeferredBrowser},
};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, io::Write, sync::atomic::AtomicUsize};

type Checked<T> = Result<T, String>;
const ID: &str = "3bae70dd-ca33-401b-aa95-a508bab9ffc4";
const CAPTURE: &str = "0d19e1f866b84ef8a2edef3980bb923b";
const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const BYTES: u64 = 17_633_383;
const DURATION: f64 = 118.177334;
const MEDIA_SHA: &str = "08c0087446f4485dd11151f10f31b3396409aaad3055ae0ff02fa153e8c9a106";
const CATALOG_SHA: &str = "f1d987d6f78644f79587bbf5ec08a3567fd46e37f9eeb5102fe1bb04f284ed9f";
const MANIFEST_SHA: &str = "bf892399d2cdd62a0bb856c6975cbb5a23edfcef9cc00f28c4e809b81440b978";
const OLD: [(&str, &str); 5] = [
    ("youtube", "38965dca-b875-4758-8b84-642507653bd1"),
    ("soop", "9011ac2c-8063-4e1a-a85a-3bc449879c8f"),
    ("soop", "241bc41d-d3e6-4bf9-a205-899f2c2d21d5"),
    ("chzzk", "6e82ac06ed1048c6b8201b29892ff659"),
    ("soop", "8fd00700-81d4-401f-a083-0d13fcf286ea"),
];
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
fn reference() -> CloudSourceRef {
    CloudSourceRef {
        provider: "soop".into(),
        recording_id: ID.into(),
        part_index: None,
        plan_id: None,
        unit_id: None,
    }
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
        || raw["library"]["captureOutcome"]["reasons"] != json!(["source_interrupted", "audio_coverage_tail"])
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
fn baseline(catalog: &Catalog, owner: &str, allow_selected: bool) -> Checked<(Vec<u8>, Vec<u8>)> {
    if catalog.grant.as_ref().is_some_and(|grant| grant.channel_id != owner) {
        return Err("QA_OWNER_CHANGED".into());
    }
    if catalog.client.is_none() || catalog.jobs.len() != OLD.len() + usize::from(allow_selected) {
        return Err("QA_BASELINE_COUNT".into());
    }
    let mut old = Vec::new();
    for (provider, id) in OLD {
        let found = catalog
            .jobs
            .iter()
            .filter(|job| {
                job.view.source.provider == provider && job.view.source.recording_id == id
            })
            .collect::<Vec<_>>();
        if found.len() != 1 {
            return Err("QA_BASELINE_SOURCE".into());
        }
        let job = found[0];
        if job.view.state != "ready"
            || !job.has_upload_receipt()
            || job.view.visibility != CloudVisibility::Unlisted
            || job.channel_id != owner
            || (provider == "chzzk"
                && (job.view.source.part_index != Some(11) || job.view.source.plan_id.is_some()))
        {
            return Err("QA_BASELINE_READY".into());
        }
        old.push(job);
    }
    let automatic = serde_json::to_value(&catalog.automatic).map_err(|_| "QA_AUTOMATIC")?;
    let rules = automatic["rules"]
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or("QA_AUTOMATIC_COUNT")?;
    if rules[0]["target"]["kind"] != "recording"
        || rules[0]["target"]["provider"] != "soop"
        || rules[0]["target"]["recordingId"] != OLD[2].1
    {
        return Err("QA_AUTOMATIC_TARGET".into());
    }
    if allow_selected
        && catalog
            .jobs
            .iter()
            .filter(|job| {
                selected(&job.view.source) && job.channel_id == owner && job.automatic.is_none()
            })
            .count()
            != 1
    {
        return Err("QA_SELECTED_JOB_COUNT".into());
    }
    Ok((
        serde_json::to_vec(&(old, &catalog.client, &catalog.live_grant, catalog.live_generation, &catalog.live_receipts)).map_err(|_| "QA_BASELINE_JSON")?,
        serde_json::to_vec(&catalog.automatic).map_err(|_| "QA_BASELINE_JSON")?,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UploadMode {
    AuthenticatedWorkflow,
    UploadOnly,
    ResumeExisting,
}
impl UploadMode {
    fn opt_in(self) -> &'static str {
        match self {
            Self::AuthenticatedWorkflow => "ATSUMI_SOOP_PRIVATE_UPLOAD_QA",
            Self::UploadOnly => "ATSUMI_SOOP_PRIVATE_UPLOAD_ONLY_QA",
            Self::ResumeExisting => "ATSUMI_SOOP_PRIVATE_RESUME_QA",
        }
    }
    fn observation(self) -> &'static str {
        match self {
            Self::AuthenticatedWorkflow => "required_fresh_probe",
            Self::UploadOnly | Self::ResumeExisting => "not_requested_upload_only",
        }
    }
    fn admit(self, opt_in: Option<&str>, observe: impl FnOnce() -> Checked<()>) -> Checked<()> {
        if opt_in != Some(ID) {
            return Err("QA_UPLOAD_OPT_IN".into());
        }
        match self {
            Self::AuthenticatedWorkflow => observe(),
            Self::UploadOnly | Self::ResumeExisting => Ok(()),
        }
    }
}

// Preserve this fresh prerequisite for the original authenticated workflow.
// It is never remote playback/owner authority or an upload-only requirement.
fn require_recent_existing_profile_observation() -> Checked<()> {
    let path = PathBuf::from(std::env::var_os("ATSUMI_PRIVATE_OWNER_PROBE_RESULT").ok_or("QA_WEBSITE_PROBE_REQUIRED")?);
    let path = canonical(&path)?;
    let root = canonical(&audit())?;
    let parent = path.parent().ok_or("QA_WEBSITE_PROBE_PATH")?;
    let directory = parent.file_name().and_then(|s|s.to_str()).ok_or("QA_WEBSITE_PROBE_PATH")?;
    if parent.parent()!=Some(root.as_path()) || path.file_name().and_then(|s|s.to_str())!=Some("result.json")
        || !directory.strip_prefix("private-owner-probe-").is_some_and(|id|uuid::Uuid::parse_str(id).is_ok()) { return Err("QA_WEBSITE_PROBE_PATH".into()); }
    let bytes = read(&path,16*1024)?;
    let observed:Value=serde_json::from_slice(&bytes).map_err(|_|"QA_WEBSITE_PROBE_JSON")?;
    let at=observed["observedAtMs"].as_u64().ok_or("QA_WEBSITE_PROBE_TIME")?;
    let now=model::now();
    if observed["profileIdentity"]!=PROFILE || observed["status"]!="website_authenticated_owner_unverified"
        || observed["websiteAuthenticated"]!=true || observed["normalExit"]!=true
        || observed["originalCatalogUnchanged"]!=true || observed["workerJoined"]!=true
        || observed["blockedNavigation"]!=false || observed["interactive"]!=false
        || at>now || now-at>30*60*1000 { return Err("QA_WEBSITE_AUTHENTICATION_REQUIRED".into()); }
    Ok(())
}

struct BoundedTransport {
    inner: upload::GoogleTransport,
    deadline: Instant,
    calls: AtomicUsize,
    creates: AtomicUsize,
    allow_create: bool,
    bytes: AtomicUsize,
    session: Mutex<Option<String>>,
    video: Mutex<Option<String>>,
    token_failure: Mutex<Option<(u16, &'static str)>>,
}
impl BoundedTransport {
    fn new(existing: Option<&Job>) -> Checked<Self> {
        Ok(Self {
            inner: upload::GoogleTransport::new().map_err(code)?,
            deadline: Instant::now() + Duration::from_secs(600),
            calls: AtomicUsize::new(0),
            creates: AtomicUsize::new(0),
            allow_create: existing.is_none_or(|job| job.session_url.is_none() && job.view.video_id.is_none() && job.view.uploaded_bytes == 0),
            bytes: AtomicUsize::new(0),
            session: Mutex::new(existing.and_then(|job| job.session_url.clone())),
            video: Mutex::new(existing.and_then(|job| job.view.video_id.clone())),
            token_failure: Mutex::new(None),
        })
    }
    fn classify(&self, request: &upload::Request) -> Result<(bool, bool), CloudError> {
        let url = reqwest::Url::parse(&request.url).map_err(|_| CloudError::protocol())?;
        if url.scheme() != "https"
            || url.username() != ""
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return Err(CloudError::protocol());
        }
        let create = request.method == reqwest::Method::POST
            && url.host_str() == Some("www.googleapis.com")
            && url.path() == "/upload/youtube/v3/videos";
        let put = request.method == reqwest::Method::PUT
            && self.session.lock().unwrap().as_deref() == Some(request.url.as_str());
        let video = self.video.lock().unwrap().clone();
        let get = request.method == reqwest::Method::GET
            && url.host_str() == Some("www.googleapis.com")
            && url.path() == "/youtube/v3/videos"
            && video.is_some()
            && url
                .query_pairs()
                .filter(|(key, _)| key == "id")
                .map(|(_, value)| value.into_owned())
                .collect::<Vec<_>>()
                == video.into_iter().collect::<Vec<_>>();
        let refresh = request.method == reqwest::Method::POST
            && url.as_str() == "https://oauth2.googleapis.com/token"
            && request
                .body
                .split(|b| *b == b'&')
                .any(|part| part == b"grant_type=refresh_token");
        if create {
            let body: Value =
                serde_json::from_slice(&request.body).map_err(|_| CloudError::protocol())?;
            let mut query = url
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect::<Vec<_>>();
            query.sort();
            let mut expected = vec![
                ("uploadType".into(), "resumable".into()),
                ("part".into(), "snippet,status".into()),
                ("notifySubscribers".into(), "false".into()),
            ];
            expected.sort();
            if !self.allow_create || query != expected
                || body["status"]["privacyStatus"] != "private"
                || !request.headers.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("X-Upload-Content-Length") && v == &BYTES.to_string()
                })
            {
                return Err(CloudError::protocol());
            }
        } else if !put && !get && !refresh {
            return Err(CloudError::new(
                "QA_NETWORK_SCOPE",
                "Outside selected verification",
                false,
            ));
        }
        Ok((create, put))
    }
}
impl upload::Transport for BoundedTransport {
    fn send(&self, request: upload::Request) -> Result<upload::Response, CloudError> {
        let (create, put) = self.classify(&request)?;
        if Instant::now() > self.deadline || self.calls.fetch_add(1, Ordering::SeqCst) >= 100 {
            return Err(CloudError::new(
                "QA_NETWORK_BOUND",
                "Verification bound reached",
                false,
            ));
        }
        if create && self.creates.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err(CloudError::new(
                "QA_DUPLICATE_CREATE",
                "One upload only",
                false,
            ));
        }
        if put
            && (request.body.len() > upload::CHUNK_BYTES
                || self
                    .bytes
                    .fetch_add(request.body.len(), Ordering::SeqCst)
                    .saturating_add(request.body.len())
                    > 2 * BYTES as usize)
        {
            return Err(CloudError::protocol());
        }
        let token_request = request.url == "https://oauth2.googleapis.com/token";
        let response = self.inner.send(request)?;
        if token_request && response.status != 200 {
            let body = response.json().ok();
            let reason = match body.as_ref().and_then(|value| value["error"].as_str()) {
                Some("invalid_grant") => "invalid_grant",
                Some("invalid_client") => "invalid_client",
                Some("unauthorized_client") => "unauthorized_client",
                Some("access_denied") => "access_denied",
                _ => "other",
            };
            *self.token_failure.lock().unwrap() = Some((response.status, reason));
        }
        if create && matches!(response.status, 200 | 201) {
            *self.session.lock().unwrap() = Some(
                response
                    .header("location")?
                    .filter(|value| upload::valid_session_url(value))
                    .ok_or_else(CloudError::protocol)?
                    .into(),
            );
        }
        if put && matches!(response.status, 200 | 201) {
            *self.video.lock().unwrap() = Some(
                response.json()?["id"]
                    .as_str()
                    .filter(|value| model::video_id(value))
                    .ok_or_else(CloudError::protocol)?
                    .into(),
            );
        }
        Ok(response)
    }
}
fn verify_direct_part(data: &Path, unit: &CloudSourceRef) -> Checked<()> {
    let description = source::get_plan(data, unit.plan_id.as_deref().ok_or("QA_PLAN_ID")?)?;
    let [part] = description.parts.as_slice() else { return Err("QA_PLAN_PART".into()) };
    if &part.source != unit || part.kind != "direct" || part.bytes != BYTES
        || (part.duration_seconds - DURATION).abs() > 0.000001 || part.media_offset_seconds != 0.0 {
        return Err("QA_PLAN_CHANGED".into());
    }
    Ok(())
}

fn check_resume_job(job: &Job, owner: &str) -> Checked<()> {
    let view = &job.view;
    if !selected(&view.source) || view.source.plan_id.is_none() || view.recording_plan
        || job.channel_id != owner || job.automatic.is_some() || view.visibility != CloudVisibility::Private
        || job.source_sha256.as_deref() != Some(MEDIA_SHA) || view.total_bytes != BYTES
        || (view.duration_seconds - DURATION).abs() > 0.000001 || view.media_offset_seconds != 0.0
        || view.uploaded_bytes > 0 && job.session_url.is_none() && !job.has_upload_receipt() {
        return Err("QA_RESUME_IDENTITY".into());
    }
    if view.state == "ready" && job.has_upload_receipt() { return Ok(()); }
    if view.state == "waiting_auth" && view.error.as_ref().is_some_and(|error|
        matches!(error.code.as_str(), "CLOUD_AUTH_REQUIRED" | "CLOUD_REAUTH_REQUIRED")) {
        return Ok(());
    }
    Err("QA_RESUME_STATE".into())
}

fn host(
    vault: vault::Vault,
    catalog: Catalog,
    platform: &PlatformMediaService,
    data: &Path,
    transport: Arc<dyn upload::Transport>,
) -> Arc<CloudService> {
    let p = platform.clone();
    let d = data.to_owned();
    let resolver: SourceResolver = Arc::new(move |reference, context| {
        if !selected(reference) || reference.plan_id.is_none() {
            return Err("QA_SOURCE_SCOPE".into());
        }
        source::resolve(&DeferredBrowser::default(), &p, &d, reference, context)
    });
    let p = platform.clone();
    let d = data.to_owned();
    let planner: PlanResolver = Arc::new(move |source_ref, context| {
        if *source_ref != reference() {
            return Err("QA_PLAN_SCOPE".into());
        }
        let units = source::prepare_plan(&DeferredBrowser::default(), &p, &d, source_ref, context)?;
        if units.len() != 1 || !selected(&units[0]) {
            return Err("QA_PLAN_UNITS".into());
        }
        verify_direct_part(&d, &units[0])?;
        Ok(units)
    });
    Arc::new(CloudService {
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
        resolver,
        planner: Some(planner),
        open_browser: Arc::new(|_| Err("QA_AUTHORIZATION_FORBIDDEN".into())),
        transport,
    })
}
struct Drain {
    cloud: Arc<CloudService>,
    platform: PlatformMediaService,
}
impl Drop for Drain {
    fn drop(&mut self) {
        let _ = self.cloud.shutdown_and_wait();
        self.platform.shutdown();
    }
}
struct PlatformGuard(PlatformMediaService);
impl Drop for PlatformGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

#[test]
#[ignore = "One fixed real SOOP membership recording Private upload; explicit reviewed opt-in and existing isolated website profile"]
fn actual_soop_selected_membership_private_upload() -> Checked<()> {
    selected_private_upload(UploadMode::AuthenticatedWorkflow)
}

#[test]
#[ignore = "One exact SOOP118 Private upload only; separately reviewed opt-in, no website/replay authentication claim"]
fn actual_soop_selected_membership_private_upload_only() -> Checked<()> {
    selected_private_upload(UploadMode::UploadOnly)
}

#[test]
#[ignore = "Resume the one existing SOOP118 Private job after same-owner authorization, or verify its ready receipt"]
fn actual_soop_selected_membership_resume_existing_private_upload() -> Checked<()> {
    selected_private_upload(UploadMode::ResumeExisting)
}

fn selected_private_upload(mode: UploadMode) -> Checked<()> {
    let opt_in = std::env::var(mode.opt_in()).ok();
    mode.admit(opt_in.as_deref(), require_recent_existing_profile_observation)?;
    let mut originals = Originals::open()?;
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(PROFILE);
    let job = selected_catalog(&data, &mut originals)?; // no automatic import
    plain(&data.join("cloud-recording/state.bin"))?;
    let vault = vault::Vault::open(&data).map_err(code)?;
    let catalog = vault.load().map_err(code)?;
    let existing = catalog.jobs.iter().find(|value| selected(&value.view.source));
    match (mode, existing) {
        (UploadMode::ResumeExisting, None) => return Err("QA_EXISTING_JOB_REQUIRED".into()),
        (UploadMode::UploadOnly | UploadMode::AuthenticatedWorkflow, Some(_)) => return Err("QA_EXISTING_SOURCE_NO_RECREATE".into()),
        _ => {}
    }
    let owner = catalog.grant.as_ref().ok_or("QA_CONNECTED_GRANT_REQUIRED")?.channel_id.clone();
    let expected = baseline(&catalog, &owner, existing.is_some())?;
    if let Some(entry) = existing {
        check_resume_job(entry, &owner)?;
        verify_direct_part(&data, &entry.view.source)?;
    }
    let resume = existing.map(|entry| (entry.view.id.clone(), entry.view.state == "ready"));
    let bin = workspace()
        .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let tools = PlatformMediaTools {
        yt_dlp: workspace().join(".runtime/platform-tools/bin/yt-dlp.exe"),
        deno: workspace().join(".runtime/platform-tools/bin/deno.exe"),
        ffmpeg: bin.join("ffmpeg.exe"),
        ffprobe: bin.join("ffprobe.exe"),
    };
    let platform =
        PlatformMediaService::new(data.clone(), tools, None).map_err(|_| "QA_PLATFORM_START")?;
    let platform_guard = PlatformGuard(platform.clone());
    if !platform.active_ids().is_empty()
        || platform
            .jobs(Some("soop"))
            .iter()
            .find(|value| value.job_id == ID)
            .is_none_or(|value| {
                value.library.as_ref().map(|v| &v.sha256) != job.library.as_ref().map(|v| &v.sha256)
            })
    {
        return Err("QA_PLATFORM_CATALOG".into());
    }
    let transport = Arc::new(BoundedTransport::new(existing)?);
    let cloud = host(vault, catalog, &platform, &data, transport.clone());
    let drain = Drain {
        cloud: cloud.clone(),
        platform: platform.clone(),
    };
    let result = (|| -> Checked<Value> {
        if let Some((id, ready)) = &resume {
            if !ready { cloud.resume(id).map_err(code)?; }
        } else {
            cloud.enqueue_recording_with_visibility(reference(), CloudVisibility::Private).map_err(code)?;
        }
        let deadline = Instant::now() + Duration::from_secs(600);
        let mut previous = String::new();
        loop {
            let snapshot = cloud.snapshot().map_err(code)?;
            let current = snapshot
                .jobs
                .iter()
                .find(|value| selected(&value.source))
                .ok_or("QA_NEW_JOB_MISSING")?;
            if resume.as_ref().is_some_and(|(id, _)| &current.id != id) { return Err("QA_RESUME_ID_CHANGED".into()); }
            if baseline(&cloud.lock().map_err(code)?.catalog, &owner, true)? != expected {
                return Err("QA_PRIOR_STATE_CHANGED".into());
            }
            if previous != current.state {
                println!("QA_SOOP_PRIVATE_STATE={}", current.state);
                previous = current.state.clone();
            }
            if current.state == "ready" {
                let auth = cloud
                    .replay_authorization(&data, &current.id)
                    .map_err(code)?;
                if auth.source_sha256 != MEDIA_SHA
                    || auth.bytes != BYTES
                    || (auth.duration_seconds - DURATION).abs() > 0.000001
                    || auth.media_offset_seconds != 0.0
                    || current.visibility != CloudVisibility::Private
                    || auth.original_inputs.len() != 1
                    || auth.original_inputs[0].sha256 != MEDIA_SHA
                    || auth.source.plan_id.is_none()
                    || auth.source.unit_id.is_none()
                {
                    return Err("QA_READY_RECEIPT_MISMATCH".into());
                }
                platform
                    .validate_cloud_chat_source(&auth)
                    .map_err(|_| "QA_NATIVE_SIDECAR_ADMISSION")?;
                let token = platform
                    .open_cloud_chat(&auth)
                    .map_err(|_| "QA_NATIVE_SIDECAR_OPEN")?;
                platform.close_cloud_chat(&token);
                return Ok(
                    json!({"ready":true,"visibility":"private","actualEncryptedReceipt":true,"sourceSha256":MEDIA_SHA,"bytes":BYTES,"durationSeconds":DURATION,"mediaOffsetSeconds":0,"planId":auth.source.plan_id,"unitId":auth.source.unit_id,"jobId":current.id,"captureOutcomePreserved":"interrupted/source_interrupted,audio_coverage_tail","actualRemoteAvVerified":false}),
                );
            }
            if matches!(
                current.state.as_str(),
                "failed" | "paused" | "retry_wait" | "waiting_auth"
            ) {
                return Err(current
                    .error
                    .clone()
                    .map(code)
                    .unwrap_or("QA_UPLOAD_STOPPED".into()));
            }
            if Instant::now() > deadline || audit().join("soop-private-upload.cancel").exists() {
                return Err("QA_UPLOAD_DEADLINE_OR_CANCEL".into());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    })();
    let stopped = cloud.shutdown_and_wait().map_err(code);
    platform.shutdown();
    drop(drain);
    drop(platform_guard);
    let unchanged = originals.unchanged();
    let prior_unchanged = cloud.vault.load().map_err(code).and_then(|saved| {
        let created = saved.jobs.iter().any(|value| selected(&value.view.source));
        baseline(&saved, &owner, created).map(|value| value == expected)
    });
    let report = json!({
        "scope":"agent-fixed manual verification source; no automatic enrollment",
        "existingJob":resume.as_ref().map(|(id, _)| id), "websiteObservation":mode.observation(),
        "actualPrivatePlaybackVerified":false, "actualRemoteAvVerified":false,
        "result":result.as_ref().ok(), "errorCode":result.as_ref().err(), "normalDrain":stopped.is_ok(),
        "original37AndQACatalogUnchanged":unchanged.is_ok(), "priorFiveJobsAutomaticAndLiveUnchanged":prior_unchanged.as_ref().ok(),
        "finalCatalogError":prior_unchanged.as_ref().err(), "oauthHttpFailure":*transport.token_failure.lock().unwrap(),
        "createRequests":transport.creates.load(Ordering::SeqCst), "requests":transport.calls.load(Ordering::SeqCst),
        "sentMediaBytes":transport.bytes.load(Ordering::SeqCst), "captureOutcomePreserved":"interrupted/source_interrupted,audio_coverage_tail"
    });
    let report_kind = match mode { UploadMode::ResumeExisting => "resume", UploadMode::UploadOnly => "upload-only", UploadMode::AuthenticatedWorkflow => "upload" };
    let path = audit().join(format!(
        "soop-selected-private-{report_kind}-result-{}.json",
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "QA_REPORT")?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "QA_REPORT")?;
    println!("QA_SOOP_PRIVATE_RESULT={}", path.display());
    stopped?;
    unchanged?;
    if !prior_unchanged? {
        return Err("QA_PRIOR_STATE_CHANGED_AFTER".into());
    }
    result.map(|_| ())
}

#[test]
fn private_upload_modes_require_distinct_opt_in_and_preserve_strict_website_probe() {
    assert_eq!(UploadMode::AuthenticatedWorkflow.opt_in(), "ATSUMI_SOOP_PRIVATE_UPLOAD_QA");
    assert_eq!(UploadMode::UploadOnly.opt_in(), "ATSUMI_SOOP_PRIVATE_UPLOAD_ONLY_QA");
    assert_eq!(UploadMode::ResumeExisting.opt_in(), "ATSUMI_SOOP_PRIVATE_RESUME_QA");
    for mode in [UploadMode::AuthenticatedWorkflow, UploadMode::UploadOnly, UploadMode::ResumeExisting] {
        for value in [None, Some(""), Some(OLD[1].1)] {
            assert_eq!(mode.admit(value, || panic!("probe before explicit source admission")).unwrap_err(), "QA_UPLOAD_OPT_IN");
        }
    }
    assert_eq!(UploadMode::AuthenticatedWorkflow.admit(Some(ID), || Err("QA_WEBSITE_PROBE_REQUIRED".into())).unwrap_err(), "QA_WEBSITE_PROBE_REQUIRED");
    let observed = std::cell::Cell::new(false);
    UploadMode::AuthenticatedWorkflow.admit(Some(ID), || { observed.set(true); Ok(()) }).unwrap();
    assert!(observed.get());
    UploadMode::UploadOnly.admit(Some(ID), || panic!("upload-only must not inspect or fabricate a website probe")).unwrap();
    assert_eq!(UploadMode::UploadOnly.observation(), "not_requested_upload_only");
}

#[test]
fn private_resume_recognizes_only_the_selected_owner_state_and_reuses_durable_handles() {
    let owner = super::OWNER;
    let source = CloudSourceRef { plan_id: Some(uuid::Uuid::new_v4().to_string()),
        unit_id: Some(uuid::Uuid::new_v4().to_string()), ..reference() };
    let mut job: Job = serde_json::from_value(json!({
        "view":{"id":uuid::Uuid::new_v4(),"source":source,"title":"fixture","state":"waiting_auth",
            "visibility":"private","uploadedBytes":0,"totalBytes":BYTES,"durationSeconds":DURATION,
            "mediaOffsetSeconds":0,"error":CloudError::auth(),"createdAt":1,"updatedAt":2},
        "channel_id":owner,"source_sha256":MEDIA_SHA,"session_url":null,"attempts":0
    })).unwrap();
    assert!(check_resume_job(&job, owner).is_ok());
    assert!(check_resume_job(&job, "different channel").is_err());
    job.view.visibility = CloudVisibility::Unlisted;
    assert!(check_resume_job(&job, owner).is_err());
    job.view.visibility = CloudVisibility::Private;
    job.view.state = "paused".into();
    assert!(check_resume_job(&job, owner).is_err());
    job.view.state = "waiting_auth".into();
    job.view.uploaded_bytes = 1;
    assert!(check_resume_job(&job, owner).is_err());
    let session = "https://www.googleapis.com/upload/youtube/v3/videos?upload_id=fixture";
    job.session_url = Some(session.into());
    assert!(check_resume_job(&job, owner).is_ok());
    let transport = BoundedTransport::new(Some(&job)).unwrap();
    assert!(!transport.allow_create);
    let mut probe = upload::Request { method:reqwest::Method::PUT, url:session.into(), bearer:None, headers:vec![], body:vec![] };
    assert_eq!(transport.classify(&probe).unwrap(), (false, true));
    probe.url.push_str("-other");
    assert!(transport.classify(&probe).is_err());
    job.view.state = "ready".into();job.view.uploaded_bytes = BYTES;job.session_url = None;
    job.view.video_id = Some("AAAAAAAAAAA".into());
    assert!(check_resume_job(&job, owner).is_ok());
    let transport = BoundedTransport::new(Some(&job)).unwrap();
    assert!(!transport.allow_create);
    let mut get = upload::Request { method:reqwest::Method::GET, url:"https://www.googleapis.com/youtube/v3/videos?id=AAAAAAAAAAA".into(), bearer:None, headers:vec![], body:vec![] };
    assert_eq!(transport.classify(&get).unwrap(), (false, false));
    get.url.pop();get.url.push('B');
    assert!(transport.classify(&get).is_err());
}

#[test]
fn private_selected_scope_rejects_other_sources_and_notification_defaults_without_network() {
    assert!(selected(&reference()));
    let mut wrong = reference();
    wrong.recording_id = OLD[1].1.into();
    assert!(!selected(&wrong));
    let mut wrong = reference();
    wrong.provider = "youtube".into();
    assert!(!selected(&wrong));
    let mut wrong = reference();
    wrong.part_index = Some(0);
    assert!(!selected(&wrong));
    let transport = BoundedTransport::new(None).unwrap();
    // Accept the unchanged production insertion metadata without sending it.
    let valid = upload::Request {
        method: reqwest::Method::POST,
        url: "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status&notifySubscribers=false".into(),
        bearer: None,
        headers: vec![("X-Upload-Content-Length".into(), BYTES.to_string())],
        body: serde_json::to_vec(&json!({"status":{"privacyStatus":"private"}})).unwrap(),
    };
    assert_eq!(transport.classify(&valid).unwrap(), (true, false));
    for (query, visibility) in [
        ("uploadType=resumable&part=snippet,status", "private"),
        (
            "uploadType=resumable&part=snippet,status&notifySubscribers=true",
            "private",
        ),
        (
            "uploadType=resumable&part=snippet,status&notifySubscribers=false",
            "public",
        ),
        (
            "uploadType=resumable&part=snippet,status&notifySubscribers=false",
            "unlisted",
        ),
    ] {
        let request = upload::Request {
            method: reqwest::Method::POST,
            url: format!("https://www.googleapis.com/upload/youtube/v3/videos?{query}"),
            bearer: None,
            headers: vec![("X-Upload-Content-Length".into(), BYTES.to_string())],
            body: serde_json::to_vec(
                &json!({"status":{"privacyStatus":visibility,"selfDeclaredMadeForKids":false}}),
            )
            .unwrap(),
        };
        assert!(transport.classify(&request).is_err());
    }
    let request = upload::Request {
        method: reqwest::Method::GET,
        url: "https://www.googleapis.com/youtube/v3/videos?id=AAAAAAAAAAA".into(),
        bearer: None,
        headers: vec![],
        body: vec![],
    };
    assert!(transport.classify(&request).is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(transport.creates.load(Ordering::SeqCst), 0);
}
