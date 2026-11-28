//! One read-only owner-status request. No live creation, token refresh or grant change.
use super::super::*;
use serde_json::{json, Value};
use std::{fs, io::Write, os::windows::fs::MetadataExt};

#[test]
#[ignore = "Existing isolated account read-only live-status query; exact opt-in"]
fn actual_selected_account_readonly_live_readiness() -> Result<(), String> {
    if std::env::var("ATSUMI_LIVE_READINESS_QA").as_deref() != Ok("existing-qa-readonly-once") {
        return Err("QA_OPT_IN".into());
    }
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?)
        .join("local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510");
    let state = data.join("cloud-recording/state.bin");
    for path in state.ancestors() {
        let metadata = fs::symlink_metadata(path).map_err(|_| "QA_STATE_PATH")?;
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 {
            return Err("QA_STATE_LINK".into());
        }
    }
    if fs::metadata(&state).map_err(|_| "QA_STATE")?.len() > 8 * 1024 * 1024
        || !data.join("cloud-recording/state.lock").is_file() { return Err("QA_STATE_BOUND".into()); }
    let vault = vault::Vault::open(&data).map_err(|_| "QA_VAULT_BUSY")?;
    let before = fs::read(&state).map_err(|_| "QA_STATE")?;
    let catalog = vault.load().map_err(|_| "QA_VAULT")?;
    let grant = catalog.grant.as_ref().filter(|g| g.valid() && g.expires_at > now() + 30_000).ok_or("QA_FRESH_GRANT_REQUIRED")?;
    let automatic = serde_json::to_value(&catalog.automatic).map_err(|_| "QA_BASELINE")?;
    if catalog.jobs.len() != 5 || catalog.jobs.iter().any(|j| j.view.state != "ready"
        || j.channel_id != grant.channel_id || j.view.visibility != CloudVisibility::Unlisted)
        || automatic["rules"].as_array().is_none_or(|rules| rules.len() != 1) { return Err("QA_BASELINE".into()); }
    let client = reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(8))
        .build().map_err(|_| "QA_CLIENT")?;
    let response = client.get("https://www.googleapis.com/youtube/v3/liveStreams")
        .query(&[("part", "status"), ("mine", "true"), ("maxResults", "1")])
        .bearer_auth(&grant.access_token).send().map_err(|_| "QA_NETWORK")?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response.take(16385).read_to_end(&mut bytes).map_err(|_| "QA_RESPONSE")?;
    if bytes.len() > 16384 { return Err("QA_RESPONSE_BOUND".into()); }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "QA_RESPONSE_JSON")?;
    let reason = value["error"]["errors"][0]["reason"].as_str().map(|raw| match raw {
        "liveStreamingNotEnabled" => "liveStreamingNotEnabled",
        "insufficientLivePermissions" => "insufficientLivePermissions",
        "insufficientPermissions" => "insufficientPermissions",
        "quotaExceeded" => "quotaExceeded",
        "authError" => "authError",
        _ => "unclassified_remote_error",
    });
    let listed = value["items"].as_array().map(Vec::len);
    if status == 200 && (value["kind"] != "youtube#liveStreamListResponse" || listed.is_none_or(|n|n>1)) {
        return Err("QA_RESPONSE_SHAPE".into());
    }
    if fs::read(&state).map_err(|_| "QA_STATE")? != before { return Err("QA_STATE_CHANGED".into()); }
    let report = json!({"httpStatus":status,"reason":reason,"readonlyListSucceeded":status==200,
        "returnedStreamCount":listed,"writePermissionVerified":false,"liveCreationVerified":false,
        "networkRequests":1,"httpMethod":"GET","accountMutations":0,"oauthRefresh":false,
        "existingReadyJobs":5,"existingAutomaticRules":1,"encryptedStateUnchanged":true});
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?
        .join(".runtime/requirements-audit/20261117-cloud-goal-resume")
        .join(format!("relay-readiness-result-{}.json",uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&output).map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_|"QA_REPORT")?)
        .and_then(|_|file.sync_all()).map_err(|_|"QA_REPORT")?;
    println!("QA_LIVE_READINESS_RESULT={}",output.display());
    Ok(())
}

struct LocalFactsTransport(std::sync::atomic::AtomicUsize);
impl upload::Transport for LocalFactsTransport {
    fn send(&self, _: upload::Request) -> Result<upload::Response, CloudError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CloudError::protocol())
    }
}

fn local_facts_plain_file(path: &Path, limit: u64) -> Result<(), String> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| "QA_STATE_PATH")?;
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 {
            return Err("QA_STATE_LINK".into());
        }
    }
    let metadata = fs::metadata(path).map_err(|_| "QA_STATE")?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("QA_STATE_BOUND".into());
    }
    Ok(())
}

fn local_facts_state_bytes(path: &Path) -> Result<Vec<u8>, String> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    local_facts_plain_file(path, LIMIT)?;
    let file = File::open(path).map_err(|_| "QA_STATE")?;
    let expected = file.metadata().map_err(|_| "QA_STATE")?.len();
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "QA_STATE")?;
    if bytes.len() as u64 > LIMIT || bytes.len() as u64 != expected {
        return Err("QA_STATE_BOUND".into());
    }
    Ok(bytes)
}

#[test]
#[ignore = "Existing isolated account local grant facts only; exact opt-in; no network"]
fn actual_selected_account_local_grant_facts() -> Result<(), String> {
    if std::env::var("ATSUMI_LIVE_LOCAL_FACTS_QA").as_deref()
        != Ok("existing-qa-local-only")
    {
        return Err("QA_OPT_IN".into());
    }
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?)
        .join("local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510");
    let state = data.join("cloud-recording/state.bin");
    // These files must already exist. Vault::open must not create a new profile.
    local_facts_plain_file(&state, 8 * 1024 * 1024)?;
    local_facts_plain_file(&data.join("cloud-recording/state.lock"), 1024)?;
    let vault = vault::Vault::open(&data).map_err(|_| "QA_VAULT_BUSY")?;
    let before = local_facts_state_bytes(&state)?;
    let catalog = vault.load().map_err(|_| "QA_VAULT")?;
    let transport = Arc::new(LocalFactsTransport(std::sync::atomic::AtomicUsize::new(0)));
    let forbidden_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let resolver_calls = forbidden_calls.clone();
    let browser_calls = forbidden_calls.clone();
    // Normal constructors recover/save old jobs. This read-only host deliberately
    // skips recovery, workers, schedulers, authorization and every mutation path.
    let service = CloudService {
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
        resolver: Arc::new(move |_, _| {
            resolver_calls.fetch_add(1, Ordering::SeqCst);
            Err("QA_SOURCE_FORBIDDEN".into())
        }),
        planner: None,
        open_browser: Arc::new(move |_| {
            browser_calls.fetch_add(1, Ordering::SeqCst);
            Err("QA_BROWSER_FORBIDDEN".into())
        }),
        transport: transport.clone(),
    };
    let upload = service.snapshot().map_err(|_| "QA_SNAPSHOT")?;
    let live = service.live_snapshot().map_err(|_| "QA_SNAPSHOT")?;
    let mut report = {
        let inner = service.lock().map_err(|_| "QA_SNAPSHOT")?;
        let upload_grant = inner.catalog.grant.as_ref();
        let live_grant = inner.catalog.live_grant.as_ref();
        let current = now();
        json!({
            "configured": upload.account.configured && live.account.configured,
            "uploadConnected": upload.account.connected,
            "uploadGrantValid": upload_grant.is_some_and(|g| g.valid()),
            "uploadFresh": upload_grant.is_some_and(|g| g.valid() && g.expires_at > current.saturating_add(30_000)),
            "liveConnected": live.account.connected,
            "liveGrantValid": live_grant.is_some_and(|g| g.valid()),
            "liveScopeVerified": live_grant.is_some_and(|g| g.valid()),
            "liveFresh": live_grant.is_some_and(|g| g.valid() && g.grant.expires_at > current.saturating_add(60_000)),
            "sameOwnerIfBothPresent": upload_grant.zip(live_grant).map(|(u, l)| u.channel_id == l.grant.channel_id),
            "jobCount": upload.jobs.len(),
            "automaticRuleCount": upload.automatic.rules.len(),
            "liveReceiptCount": live.operations.len(),
        })
    };
    let network_calls = transport.0.load(Ordering::SeqCst);
    let worker_threads = service.threads.lock().map_err(|_| "QA_THREADS")?.len();
    if network_calls != 0 || worker_threads != 0 || forbidden_calls.load(Ordering::SeqCst) != 0 {
        return Err("QA_LOCAL_ONLY_VIOLATION".into());
    }
    if local_facts_state_bytes(&state)? != before {
        return Err("QA_STATE_CHANGED".into());
    }
    report["stateBytesUnchanged"] = json!(true);
    report["networkCalls"] = json!(network_calls);
    report["workerThreads"] = json!(worker_threads);
    // Dropping this worker-free host only releases its exclusive vault lock.
    drop(service);
    let output = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().ok_or("QA_WORKSPACE")?
        .join(".runtime/requirements-audit/20261122-cloud-resume")
        .join(format!("live-local-grant-facts-{}.json", uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true)
        .open(&output).map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "QA_REPORT")?)
        .and_then(|_| file.sync_all()).map_err(|_| "QA_REPORT")?;
    println!("QA_LIVE_LOCAL_FACTS_RESULT={}", output.display());
    Ok(())
}
