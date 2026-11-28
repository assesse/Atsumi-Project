//! Exact, manually selected SOOP gift verification. Test-only; never starts from the app.
//! Contract: requirements-audit/20261117-cloud-goal-resume/soop-gift-upload-native-pre-edit.json.
use super::super::*;
use crate::{
    platform_media::{MediaJob, PlatformMediaService, PlatformMediaTools},
    streaming::{browser_store, startup::DeferredBrowser},
};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, io::Write, sync::atomic::AtomicUsize};

type Checked<T> = Result<T, String>;
const ID: &str = "8fd00700-81d4-401f-a083-0d13fcf286ea";
const CAPTURE: &str = "baa5338478ee4c8cb1cdda57d278d918";
const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const BYTES: u64 = 16_150_161;
const DURATION: f64 = 108.229;
const MEDIA_SHA: &str = "457778363cdd49782cdb8c4b9025521c35d76336ad10275d7af258f012022c44";
const CATALOG_SHA: &str = "9224ca11d926311d8dc4f2f45fd7f32b4ae6c899f1b46ba3f23be39e59281cfe";
const MANIFEST_SHA: &str = "797794dd69e3c711c9c499739d3d09864ae0db5fb1dc5c43188efb57c429272a";
const OLD: [(&str, &str); 4] = [
    ("youtube", "38965dca-b875-4758-8b84-642507653bd1"),
    ("soop", "9011ac2c-8063-4e1a-a85a-3bc449879c8f"),
    ("soop", "241bc41d-d3e6-4bf9-a205-899f2c2d21d5"),
    ("chzzk", "6e82ac06ed1048c6b8201b29892ff659"),
];
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
fn audit() -> PathBuf {
    workspace().join(".runtime/requirements-audit/20261117-cloud-goal-resume")
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
            &audit().join("soop-gift-selected-source-files.json"),
            32 * 1024,
        )?;
        if digest(&bytes) != MANIFEST_SHA {
            return Err("QA_SOURCE_MANIFEST".into());
        }
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|_| "QA_SOURCE_MANIFEST")?;
        let entries = manifest["files"]
            .as_array()
            .filter(|v| v.len() == 31)
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
        || job.request_id != "e712b185-ad42-457d-95de-05db34da9a84"
        || job.state != "completed"
        || job.revision != 15
        || job.reference.provider != "soop"
        || job.reference.kind != "live"
        || job.reference.external_id != "hinqocorv"
        || job.broadcast_id.as_deref() != Some("298642901")
        || item.broadcast_id != job.broadcast_id
        || item.reference != job.reference
        || item.library_id != ID
        || item.revision != 15
        || item.bytes != BYTES
        || item.duration != DURATION
        || item.sha256 != MEDIA_SHA
        || item.modified_millis != 1794868179658
        || canonical(&job.download_root)? != canonical(&capture_root().join("media"))?
        || raw["library"]["captureOutcome"]["state"] != "interrupted"
        || raw["library"]["captureOutcome"]["reasons"] != json!(["source_interrupted"])
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
fn baseline(catalog: &Catalog, allow_selected: bool) -> Checked<(Vec<u8>, Vec<u8>)> {
    let owner = &catalog
        .grant
        .as_ref()
        .ok_or("QA_CONNECTED_GRANT_REQUIRED")?
        .channel_id;
    if catalog.client.is_none() || catalog.jobs.len() != 4 + usize::from(allow_selected) {
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
            || &job.channel_id != owner
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
                selected(&job.view.source) && &job.channel_id == owner && job.automatic.is_none()
            })
            .count()
            != 1
    {
        return Err("QA_SELECTED_JOB_COUNT".into());
    }
    Ok((
        serde_json::to_vec(&(old, &catalog.client)).map_err(|_| "QA_BASELINE_JSON")?,
        serde_json::to_vec(&catalog.automatic).map_err(|_| "QA_BASELINE_JSON")?,
    ))
}
struct BoundedTransport {
    inner: upload::GoogleTransport,
    deadline: Instant,
    calls: AtomicUsize,
    creates: AtomicUsize,
    bytes: AtomicUsize,
    session: Mutex<Option<String>>,
    video: Mutex<Option<String>>,
}
impl BoundedTransport {
    fn new() -> Checked<Self> {
        Ok(Self {
            inner: upload::GoogleTransport::new().map_err(code)?,
            deadline: Instant::now() + Duration::from_secs(600),
            calls: AtomicUsize::new(0),
            creates: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            session: Mutex::new(None),
            video: Mutex::new(None),
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
            if query != expected
                || body["status"]["privacyStatus"] != "unlisted"
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
        if create && self.creates.fetch_add(1, Ordering::SeqCst) != 0 {
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
        let response = self.inner.send(request)?;
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
        let description = source::get_plan(&d, units[0].plan_id.as_deref().ok_or("QA_PLAN_ID")?)?;
        let part = description.parts.first().ok_or("QA_PLAN_PART")?;
        if description.parts.len() != 1
            || part.source != units[0]
            || part.kind != "direct"
            || part.bytes != BYTES
            || (part.duration_seconds - DURATION).abs() > 0.000001
            || part.media_offset_seconds != 0.0
        {
            return Err("QA_PLAN_CHANGED".into());
        }
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
#[ignore = "One fixed real SOOP gift recording unlisted upload; explicit opt-in and exclusive QA vault"]
fn actual_soop_selected_gift_unlisted_upload() -> Checked<()> {
    if std::env::var("ATSUMI_SOOP_GIFT_UPLOAD_QA").as_deref() != Ok(ID) {
        return Err("QA_UPLOAD_OPT_IN".into());
    }
    let mut originals = Originals::open()?;
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(PROFILE);
    let job = selected_catalog(&data, &mut originals)?;
    plain(&data.join("cloud-recording/state.bin"))?;
    let vault = vault::Vault::open(&data).map_err(code)?;
    let catalog = vault.load().map_err(code)?;
    if catalog
        .jobs
        .iter()
        .any(|value| selected(&value.view.source))
    {
        return Err("QA_EXISTING_SOURCE_NO_RECREATE".into());
    }
    let expected = baseline(&catalog, false)?;
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
    let transport = Arc::new(BoundedTransport::new()?);
    let cloud = host(vault, catalog, &platform, &data, transport.clone());
    let drain = Drain {
        cloud: cloud.clone(),
        platform: platform.clone(),
    };
    let result = (|| -> Checked<Value> {
        cloud.enqueue_recording(reference()).map_err(code)?;
        let deadline = Instant::now() + Duration::from_secs(600);
        let mut previous = String::new();
        loop {
            let snapshot = cloud.snapshot().map_err(code)?;
            let current = snapshot
                .jobs
                .iter()
                .find(|value| selected(&value.source))
                .ok_or("QA_NEW_JOB_MISSING")?;
            if baseline(&cloud.lock().map_err(code)?.catalog, true)? != expected {
                return Err("QA_PRIOR_STATE_CHANGED".into());
            }
            if previous != current.state {
                println!("QA_SOOP_GIFT_STATE={}", current.state);
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
                    || current.visibility != CloudVisibility::Unlisted
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
                    json!({"ready":true,"visibility":"unlisted","actualEncryptedReceipt":true,"sourceSha256":MEDIA_SHA,"bytes":BYTES,"durationSeconds":DURATION,"mediaOffsetSeconds":0,"planId":auth.source.plan_id,"unitId":auth.source.unit_id,"jobId":current.id,"captureOutcomePreserved":"interrupted/source_interrupted","actualRemoteAvVerified":false}),
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
            if Instant::now() > deadline || audit().join("soop-gift-upload.cancel").exists() {
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
    let saved = cloud.vault.load().map_err(code)?;
    let created = saved.jobs.iter().any(|value| selected(&value.view.source));
    let prior_unchanged = baseline(&saved, created).map(|value| value == expected)?;
    let report = json!({"scope":"agent-fixed manual verification source; no automatic enrollment","result":result.as_ref().ok(),"errorCode":result.as_ref().err(),"normalDrain":stopped.is_ok(),"original31AndQACatalogUnchanged":unchanged.is_ok(),"priorFourJobsAndAutomaticConfigUnchanged":prior_unchanged,"createRequests":transport.creates.load(Ordering::SeqCst),"requests":transport.calls.load(Ordering::SeqCst),"sentMediaBytes":transport.bytes.load(Ordering::SeqCst),"captureOutcomePreserved":"interrupted/source_interrupted","actualRemoteAvVerified":false});
    let path = audit().join(format!(
        "soop-selected-gift-upload-result-{}.json",
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
    println!("QA_SOOP_GIFT_RESULT={}", path.display());
    stopped?;
    unchanged?;
    if !prior_unchanged {
        return Err("QA_PRIOR_STATE_CHANGED_AFTER".into());
    }
    result.map(|_| ())
}

#[test]
fn selected_gift_scope_rejects_other_sources_and_notification_defaults_without_network() {
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
    let transport = BoundedTransport::new().unwrap();
    // Accept the unchanged production insertion metadata without sending it.
    let valid = upload::Request {
        method: reqwest::Method::POST,
        url: "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status&notifySubscribers=false".into(),
        bearer: None,
        headers: vec![("X-Upload-Content-Length".into(), BYTES.to_string())],
        body: serde_json::to_vec(&json!({"status":{"privacyStatus":"unlisted"}})).unwrap(),
    };
    assert_eq!(transport.classify(&valid).unwrap(), (true, false));
    for (query, visibility) in [
        ("uploadType=resumable&part=snippet,status", "unlisted"),
        (
            "uploadType=resumable&part=snippet,status&notifySubscribers=true",
            "unlisted",
        ),
        (
            "uploadType=resumable&part=snippet,status&notifySubscribers=false",
            "public",
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
