//! Opt-in existing-QA Live consent and one read-only readiness request.
//! OAuth URLs and credentials stay in memory; no upload or Live resource writes.
use super::*;
use crate::cloud_recording::{catalog::Catalog, upload, vault, BrowserOpener, SourceResolver};
use reqwest::{Method, Url};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{atomic::AtomicUsize, Condvar, Mutex},
    time::Instant,
};

type Checked<T> = Result<T, String>;
const LIVE_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";

fn safe_code(error: CloudError) -> String {
    let code = error.code;
    if code.len() <= 64 && (code.starts_with("CLOUD_") || code.starts_with("QA_"))
        && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    { code } else { "QA_UNCLASSIFIED_ERROR".into() }
}

fn plain(path: &Path, limit: u64) -> Checked<()> {
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor).map_err(|_| "QA_STATE_PATH")?;
        if meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0 {
            return Err("QA_STATE_LINK".into());
        }
    }
    let meta = fs::metadata(path).map_err(|_| "QA_STATE")?;
    if !meta.is_file() || meta.len() > limit { return Err("QA_STATE_BOUND".into()); }
    Ok(())
}

fn state_bytes(path: &Path) -> Checked<Vec<u8>> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    plain(path, LIMIT)?;
    let file = fs::File::open(path).map_err(|_| "QA_STATE")?;
    let expected = file.metadata().map_err(|_| "QA_STATE")?.len();
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|_| "QA_STATE")?;
    if bytes.len() as u64 > LIMIT || bytes.len() as u64 != expected {
        return Err("QA_STATE_BOUND".into());
    }
    Ok(bytes)
}

fn preserved(catalog: &Catalog) -> Checked<Value> {
    let mut value = serde_json::to_value(catalog).map_err(|_| "QA_BASELINE")?;
    let object = value.as_object_mut().ok_or("QA_BASELINE")?;
    object.remove("live_grant");
    object.remove("live_generation");
    Ok(value)
}

fn query(url: &Url) -> Result<BTreeMap<String, String>, CloudError> {
    let mut result = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if result.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err(CloudError::protocol());
        }
    }
    Ok(result)
}

fn expected(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
}

fn once(counter: &AtomicUsize) -> Result<(), CloudError> {
    counter.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .map(|_| ()).map_err(|_| CloudError::protocol())
}

struct ConsentTransport {
    inner: upload::GoogleTransport,
    owner: String,
    deadline: Instant,
    auth_posts: AtomicUsize,
    refresh_posts: AtomicUsize,
    owner_gets: AtomicUsize,
    readiness_gets: AtomicUsize,
}
impl upload::Transport for ConsentTransport {
    fn send(&self, request: upload::Request) -> Result<upload::Response, CloudError> {
        if Instant::now() >= self.deadline { return Err(CloudError::cancelled()); }
        let url = Url::parse(&request.url).map_err(|_| CloudError::protocol())?;
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
            || url.fragment().is_some() || url.port().is_some()
        { return Err(CloudError::protocol()); }
        let is_owner = url.host_str() == Some("www.googleapis.com")
            && url.path() == "/youtube/v3/channels";
        if url.host_str() == Some("oauth2.googleapis.com") && url.path() == "/token" {
            if request.method != Method::POST || url.query().is_some() || request.bearer.is_some()
                || request.body.len() > 32768 || request.headers != vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())]
            { return Err(CloudError::protocol()); }
            let mut form_url = Url::parse("https://localhost/").map_err(|_| CloudError::protocol())?;
            form_url.set_query(Some(std::str::from_utf8(&request.body).map_err(|_| CloudError::protocol())?));
            let form = query(&form_url)?;
            let keys: Vec<_> = form.keys().map(String::as_str).collect();
            match form.get("grant_type").map(String::as_str) {
                Some("authorization_code") if keys == ["client_id", "client_secret", "code", "code_verifier", "grant_type", "redirect_uri"] => once(&self.auth_posts)?,
                Some("refresh_token") if keys == ["client_id", "client_secret", "grant_type", "refresh_token"] => once(&self.refresh_posts)?,
                _ => return Err(CloudError::protocol()),
            }
        } else {
            if request.method != Method::GET || !request.body.is_empty() || !request.headers.is_empty()
                || request.bearer.is_none()
            { return Err(CloudError::protocol()); }
            if is_owner && query(&url)? == expected(&[("part", "snippet"), ("mine", "true"), ("fields", "items(id,snippet/title)")]) {
                once(&self.owner_gets)?;
            } else if url.host_str() == Some("www.googleapis.com") && url.path() == "/youtube/v3/liveStreams"
                && query(&url)? == expected(&[("part", "status"), ("mine", "true"), ("maxResults", "1")])
            {
                once(&self.readiness_gets)?;
                // Upload transport deliberately excludes this endpoint, and Live
                // provisioning transport permits POST only. Keep both policies
                // unchanged; this ignored QA branch admits only the exact GET.
                let client = reqwest::blocking::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(8))
                    .build().map_err(|_| CloudError::protocol())?;
                let response = client.get(url)
                    .bearer_auth(request.bearer.as_deref().ok_or_else(CloudError::protocol)?)
                    .send().map_err(|_| CloudError::new("QA_READINESS_NETWORK", "Read-only request failed", false))?;
                let status = response.status().as_u16();
                let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()).map(|v| {
                        if v.starts_with("application/json") { "json" }
                        else if v.starts_with("text/html") { "html" } else { "other" }
                    }).unwrap_or("missing");
                let mut body = Vec::new();
                response.take(32769).read_to_end(&mut body).map_err(|_| CloudError::protocol())?;
                if body.len() > 32768 { return Err(CloudError::protocol()); }
                return Ok(upload::Response { status,
                    headers: vec![("content-type-class".into(),content_type.into())], body });
            }
            else { return Err(CloudError::protocol()); }
        }
        let response = upload::Transport::send(&self.inner, request)?;
        if response.body.len() > 32768 { return Err(CloudError::protocol()); }
        if is_owner && response.status == 200 {
            let value = response.json()?;
            if value["items"].as_array().is_none_or(|v| v.len() != 1)
                || value["items"][0]["id"].as_str() != Some(self.owner.as_str())
            { return Err(CloudError::new("QA_OWNER_MISMATCH", "Existing owner did not match", false)); }
        }
        Ok(response)
    }
}

fn official_consent_url(raw: &str) -> bool {
    if raw.len() > 16384 || raw.bytes().any(|b| b == b'\r' || b == b'\n') { return false; }
    let Ok(url) = Url::parse(raw) else { return false };
    let Ok(values) = query(&url) else { return false };
    url.scheme() == "https" && url.host_str() == Some("accounts.google.com")
        && url.path() == "/o/oauth2/v2/auth" && url.port().is_none()
        && url.username().is_empty() && url.password().is_none() && url.fragment().is_none()
        && values.get("scope").is_some_and(|v| v == LIVE_SCOPE)
        && values.get("response_type").is_some_and(|v| v == "code")
        && values.get("code_challenge_method").is_some_and(|v| v == "S256")
        && values.get("redirect_uri").and_then(|s| Url::parse(s).ok()).is_some_and(|u|
            u.scheme() == "http" && u.host_str() == Some("127.0.0.1") && u.port().is_some()
                && u.path() == "/oauth2/callback" && u.query().is_none() && u.fragment().is_none())
}

fn handoff_request(stream: &mut TcpStream, host: &str, path: &str) -> Checked<bool> {
    stream.set_read_timeout(Some(Duration::from_millis(500))).map_err(|_| "QA_HANDOFF")?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|_| "QA_HANDOFF")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut block = [0; 1024];
    while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
        if bytes.len() >= 8192 || Instant::now() >= deadline { return Ok(false); }
        let n = match stream.read(&mut block) { Ok(n) => n, Err(_) => return Ok(false) };
        if n == 0 { return Ok(false); }
        bytes.extend_from_slice(&block[..n]);
    }
    if bytes.len() > 8192 { return Ok(false); }
    let Ok(text) = std::str::from_utf8(&bytes) else { return Ok(false) };
    let Some((headers, body)) = text.split_once("\r\n\r\n") else { return Ok(false) };
    if !body.is_empty() { return Ok(false); }
    let mut lines = headers.split("\r\n");
    let line = lines.next().unwrap_or_default();
    if line != format!("GET {path} HTTP/1.1") && line != format!("GET {path} HTTP/1.0") { return Ok(false); }
    let mut hosts = 0;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else { return Ok(false) };
        if key.eq_ignore_ascii_case("host") {
            hosts += 1;
            if value.trim() != host { return Ok(false); }
        }
        if key.eq_ignore_ascii_case("transfer-encoding")
            || key.eq_ignore_ascii_case("content-length") && value.trim() != "0"
        { return Ok(false); }
    }
    Ok(hosts == 1)
}

struct AuthDrain(Option<Arc<CloudService>>);
impl AuthDrain {
    fn finish(mut self) -> Checked<()> {
        Self::drain(self.0.take().ok_or("QA_DRAIN")?)
    }
    fn drain(service: Arc<CloudService>) -> Checked<()> {
        service.request_live_shutdown().map_err(safe_code)?;
        let handles = std::mem::take(&mut *service.threads.lock().map_err(|_| "QA_THREADS")?);
        for handle in handles { handle.join().map_err(|_| "QA_AUTH_JOIN")?; }
        Ok(())
    }
}
impl Drop for AuthDrain {
    fn drop(&mut self) { if let Some(service) = self.0.take() { let _ = Self::drain(service); } }
}

fn host(vault: vault::Vault, catalog: Catalog, opener: BrowserOpener, transport: Arc<ConsentTransport>) -> Arc<CloudService> {
    let resolver: SourceResolver = Arc::new(|_, _| Err("QA_SOURCE_FORBIDDEN".into()));
    Arc::new(CloudService {
        inner: Mutex::new(Inner {
            catalog, account_error: None, authorizing: false, auth_workers: 0, auth_generation: 0,
            auth_cancel: None, active: None, worker_running: false, closing: false,
            persistence_error: None, preparation_progress_at: None, automatic_running: false,
            automatic_observations: Default::default(), automatic_error: None, live: Default::default(),
        }),
        wake: Condvar::new(), threads: Mutex::new(Vec::new()), vault, resolver, planner: None,
        open_browser: opener, transport,
    })
}

fn readiness(service: &CloudService, transport: &ConsentTransport) -> Checked<Value> {
    let owner = {
        let inner = service.lock().map_err(safe_code)?;
        current_owner(&inner).ok_or("QA_LIVE_GRANT_MISSING")?
    };
    // Exercise the production expiry/scope/owner fence, including its refresh if needed.
    let _authorization = service.live_authorization(&owner).map_err(safe_code)?;
    let token = service.lock().map_err(safe_code)?.catalog.live_grant.as_ref()
        .ok_or("QA_LIVE_GRANT_MISSING")?.grant.access_token.clone();
    let response = upload::Transport::send(transport, upload::Request {
        method: Method::GET,
        url: "https://www.googleapis.com/youtube/v3/liveStreams?part=status&mine=true&maxResults=1".into(),
        bearer: Some(token), headers: Vec::new(), body: Vec::new(),
    }).map_err(safe_code)?;
    let parsed = serde_json::from_slice::<Value>(&response.body).ok();
    let value = parsed.as_ref().unwrap_or(&Value::Null);
    let listed = value["items"].as_array().map(Vec::len);
    let shape = value["kind"] == "youtube#liveStreamListResponse" && listed.is_some_and(|v| v <= 1);
    let reason = value["error"]["errors"][0]["reason"].as_str().map(|v| match v {
        "liveStreamingNotEnabled" => "liveStreamingNotEnabled",
        "insufficientLivePermissions" => "insufficientLivePermissions",
        "insufficientPermissions" => "insufficientPermissions",
        "quotaExceeded" => "quotaExceeded",
        "authError" => "authError",
        _ => "unclassified_remote_error",
    });
    let body_kind = if response.body.is_empty() { "empty" } else if parsed.is_some() { "json" }
        else if response.body.iter().find(|v| !v.is_ascii_whitespace()) == Some(&b'<') { "html_or_markup" }
        else { "other" };
    Ok(json!({"httpStatus":response.status,"reason":reason,
        "readonlyListSucceeded":response.status==200 && shape,"returnedStreamCount":listed,
        "responseShapeValid":shape,"jsonParsed":parsed.is_some(),"bodyBytes":response.body.len(),
        "bodyKind":body_kind,"contentTypeClass":response.header("content-type-class").map_err(safe_code)?}))
}

#[test]
#[ignore = "Existing isolated Live grant only; one read-only status GET; no authorization or resource writes"]
fn actual_selected_account_existing_live_readiness() -> Checked<()> {
    if std::env::var("ATSUMI_LIVE_EXISTING_READINESS_QA").as_deref() != Ok("existing-live-grant-readonly-once") {
        return Err("QA_OPT_IN".into());
    }
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?)
        .join("local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510");
    let state = data.join("cloud-recording/state.bin");
    plain(&state, 8 * 1024 * 1024)?;
    plain(&data.join("cloud-recording/state.lock"), 1024)?;
    let vault = vault::Vault::open(&data).map_err(|_| "QA_VAULT_BUSY")?;
    let before = state_bytes(&state)?;
    let catalog = vault.load().map_err(safe_code)?;
    let owner = catalog.grant.as_ref().filter(|g| g.valid()).ok_or("QA_UPLOAD_OWNER_REQUIRED")?.channel_id.clone();
    let grant = catalog.live_grant.as_ref().filter(|g| g.valid() && g.grant.channel_id == owner)
        .ok_or("QA_EXISTING_LIVE_GRANT_REQUIRED")?;
    let scopes = grant.scopes.clone();
    let generation = catalog.live_generation;
    let baseline = preserved(&catalog)?;
    // The same selected trial has now finished. Refreshing its existing grant
    // must preserve the complete receipt; it must never create another trial.
    let known_receipts = match catalog.live_receipts.as_slice() {
        [] => true,
        [receipt] => receipt.view.selection_id == "b7f26403-51ca-413b-ad04-858b15bad6a7"
            && receipt.view.manager_joined && receipt.view.worker_joined && receipt.view.sidecar_joined
            && receipt.view.destination_channel_id.as_deref() == Some(owner.as_str()),
        _ => false,
    };
    if catalog.client.is_none() || !known_receipts || catalog.jobs.len() != 5
        || catalog.jobs.iter().any(|j| j.view.state != "ready" || j.channel_id != owner)
        || serde_json::to_value(&catalog.automatic).map_err(|_| "QA_BASELINE")?["rules"].as_array().is_none_or(|v| v.len() != 1)
    { return Err("QA_BASELINE".into()); }
    let transport = Arc::new(ConsentTransport {
        inner: upload::GoogleTransport::new().map_err(safe_code)?, owner: owner.clone(),
        deadline: Instant::now() + Duration::from_secs(60), auth_posts: AtomicUsize::new(0),
        refresh_posts: AtomicUsize::new(0), owner_gets: AtomicUsize::new(0), readiness_gets: AtomicUsize::new(0),
    });
    let service = host(vault, catalog, Arc::new(|_| Err("QA_BROWSER_FORBIDDEN".into())), transport.clone());
    let drain = AuthDrain(Some(service.clone()));
    let result = readiness(&service, &transport);
    let drained = drain.finish();
    let saved = service.vault.load().map_err(safe_code)?;
    let preserved_baseline = preserved(&saved)? == baseline && saved.live_generation == generation;
    let same_owner_scope = saved.live_grant.as_ref().is_some_and(|g|
        g.valid() && g.grant.channel_id == owner && g.scopes == scopes);
    let no_authorization = transport.auth_posts.load(Ordering::SeqCst) == 0
        && transport.owner_gets.load(Ordering::SeqCst) == 0;
    let bytes_unchanged = state_bytes(&state)? == before;
    let report = json!({"existingGrantReused":true,"readiness":result.as_ref().ok(),
        "errorCode":result.as_ref().err(),"normalDrain":drained.is_ok(),
        "preservedBaselineUnchanged":preserved_baseline,"sameOwnerScope":same_owner_scope,
        "stateBytesUnchanged":bytes_unchanged,"newAuthorization":false,"browserHandoffs":0,
        "authorizationRequests":transport.auth_posts.load(Ordering::SeqCst),
        "refreshTokenRequests":transport.refresh_posts.load(Ordering::SeqCst),
        "ownerRequests":transport.owner_gets.load(Ordering::SeqCst),
        "readinessRequests":transport.readiness_gets.load(Ordering::SeqCst),
        "liveResourceCreates":0,"liveResourceDeletes":0,"uploads":0,"liveCreationVerified":false});
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?
        .join(".runtime/requirements-audit/20261122-cloud-resume")
        .join(format!("existing-live-readiness-{}.json",uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&output).map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "QA_REPORT")?)
        .and_then(|_| file.sync_all()).map_err(|_| "QA_REPORT")?;
    println!("QA_EXISTING_LIVE_READINESS_RESULT={}",output.display());
    drained?;
    if !preserved_baseline || !same_owner_scope || !no_authorization
        || transport.refresh_posts.load(Ordering::SeqCst) == 0 && !bytes_unchanged
    { return Err("QA_UNEXPECTED_STATE_CHANGE".into()); }
    result.map(|_| ())
}

#[test]
#[ignore = "Existing isolated account explicit Live consent via one-use browser handoff, then readonly status"]
fn actual_selected_account_live_consent_and_readiness() -> Checked<()> {
    if std::env::var("ATSUMI_LIVE_CONSENT_QA").as_deref() != Ok("existing-qa-browser-consent-once") {
        return Err("QA_OPT_IN".into());
    }
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?)
        .join("local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510");
    let state = data.join("cloud-recording/state.bin");
    plain(&state, 8 * 1024 * 1024)?;
    plain(&data.join("cloud-recording/state.lock"), 1024)?;
    let vault = vault::Vault::open(&data).map_err(|_| "QA_VAULT_BUSY")?;
    let before = state_bytes(&state)?;
    let catalog = vault.load().map_err(safe_code)?;
    let owner = catalog.grant.as_ref().filter(|g| g.valid()).ok_or("QA_UPLOAD_OWNER_REQUIRED")?.channel_id.clone();
    if catalog.client.is_none() || catalog.live_grant.is_some() || !catalog.live_receipts.is_empty()
        || catalog.jobs.len() != 5 || catalog.jobs.iter().any(|j| j.view.state != "ready" || j.channel_id != owner)
        || serde_json::to_value(&catalog.automatic).map_err(|_| "QA_BASELINE")?["rules"].as_array().is_none_or(|v| v.len() != 1)
    { return Err("QA_BASELINE".into()); }
    let baseline = preserved(&catalog)?;
    let old_generation = catalog.live_generation;
    let deadline = Instant::now() + Duration::from_secs(600);
    let transport = Arc::new(ConsentTransport {
        inner: upload::GoogleTransport::new().map_err(safe_code)?, owner: owner.clone(), deadline,
        auth_posts: AtomicUsize::new(0), refresh_posts: AtomicUsize::new(0),
        owner_gets: AtomicUsize::new(0), readiness_gets: AtomicUsize::new(0),
    });
    let handoff = Arc::new(Mutex::new(None::<String>));
    let opened = Arc::new(AtomicUsize::new(0));
    let slot = handoff.clone();
    let open_count = opened.clone();
    let service = host(vault, catalog, Arc::new(move |url| {
        if !official_consent_url(url) || open_count.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err("QA_OPENER_SCOPE".into());
        }
        *slot.lock().map_err(|_| "QA_HANDOFF")? = Some(url.into());
        Ok(())
    }), transport.clone());
    let drain = AuthDrain(Some(service.clone()));
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "QA_HANDOFF_BIND")?;
    listener.set_nonblocking(true).map_err(|_| "QA_HANDOFF_BIND")?;
    let host = format!("127.0.0.1:{}", listener.local_addr().map_err(|_| "QA_HANDOFF_BIND")?.port());
    let path = format!("/start/{}", uuid::Uuid::new_v4().simple());
    let mut accept_count = 0usize;
    let mut redirected = false;
    let result = (|| -> Checked<Value> {
        service.authorize_live().map_err(safe_code)?;
        let mut announced = false;
        loop {
            if Instant::now() >= deadline { return Err("QA_AUTH_DEADLINE".into()); }
            if !announced && handoff.lock().map_err(|_| "QA_HANDOFF")?.is_some() {
                println!("QA_LIVE_BROWSER_START=http://{host}{path}");
                std::io::stdout().flush().map_err(|_| "QA_HANDOFF")?;
                announced = true;
            }
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    accept_count += 1;
                    if accept_count > 16 { return Err("QA_HANDOFF_LIMIT".into()); }
                    if peer.ip().is_loopback() && !redirected && handoff_request(&mut stream, &host, &path)? {
                        if let Some(url) = handoff.lock().map_err(|_| "QA_HANDOFF")?.take() {
                            let response = format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n");
                            stream.write_all(response.as_bytes()).map_err(|_| "QA_HANDOFF_WRITE")?;
                            redirected = true;
                        }
                    } else {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n");
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err("QA_HANDOFF_ACCEPT".into()),
            }
            let snapshot = service.live_snapshot().map_err(safe_code)?;
            if !snapshot.account.authorizing {
                if !snapshot.account.connected {
                    return Err(snapshot.account.error.map(safe_code).unwrap_or("QA_AUTHORIZATION_FAILED".into()));
                }
                return readiness(&service, &transport);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    drop(listener);
    *handoff.lock().map_err(|_| "QA_HANDOFF")? = None;
    let drained = drain.finish();
    let saved = service.vault.load().map_err(safe_code)?;
    let baseline_unchanged = preserved(&saved)? == baseline;
    let live_valid = saved.live_grant.as_ref().is_some_and(|g| g.valid() && g.grant.channel_id == owner);
    let intended_generation = if live_valid { old_generation.checked_add(1) == Some(saved.live_generation) }
        else { saved.live_generation == old_generation };
    let bytes_unchanged = state_bytes(&state)? == before;
    let report = json!({
        "authorizationSucceeded":live_valid,"sameOwnerVerified":live_valid,
        "liveScopeVerified":live_valid,"durableGrantVerified":live_valid,
        "preservedBaselineUnchanged":baseline_unchanged,"intendedGeneration":intended_generation,
        "stateBytesUnchanged":bytes_unchanged,"normalDrain":drained.is_ok(),
        "readiness":result.as_ref().ok(),"errorCode":result.as_ref().err(),
        "browserHandoffs":usize::from(redirected),"browserAccepts":accept_count,
        "authorizationTokenRequests":transport.auth_posts.load(Ordering::SeqCst),
        "refreshTokenRequests":transport.refresh_posts.load(Ordering::SeqCst),
        "ownerRequests":transport.owner_gets.load(Ordering::SeqCst),
        "readinessRequests":transport.readiness_gets.load(Ordering::SeqCst),
        "liveResourceCreates":0,"liveResourceDeletes":0,"uploads":0,
        "liveCreationVerified":false,"jobCount":saved.jobs.len(),"liveReceiptCount":saved.live_receipts.len(),
    });
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?
        .join(".runtime/requirements-audit/20261122-cloud-resume")
        .join(format!("live-consent-readiness-{}.json", uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&output).map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "QA_REPORT")?)
        .and_then(|_| file.sync_all()).map_err(|_| "QA_REPORT")?;
    println!("QA_LIVE_CONSENT_RESULT={}", output.display());
    drained?;
    if !baseline_unchanged || !intended_generation || !live_valid && !bytes_unchanged {
        return Err("QA_UNEXPECTED_STATE_CHANGE".into());
    }
    result.map(|_| ())
}
