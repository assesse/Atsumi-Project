//! Explicit, connection-only QA. Never run as a production launcher.
//! Uses real CloudService OAuth/DPAPI; does not test the picker or replay UI.
//! A browser operator opens the private URL file; this program opens no browser.
#[cfg(all(windows, debug_assertions))]
mod qa {
    use atsumi_lib::cloud_recording::{CloudError, CloudService, CloudSnapshot, SourceResolver};
    use serde_json::{json, Value};
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::windows::{fs::MetadataExt, process::CommandExt},
        path::{Component, Path, PathBuf},
        process::Command,
        sync::{atomic::{AtomicBool, Ordering}, Arc},
        time::{Duration, Instant},
    };

    const APPROVED_ID: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
    const AUDIT_DIR: &str = ".runtime/requirements-audit/20261116-cloud-recording/cloud-ui-qa";
    const DEADLINE: Duration = Duration::from_secs(300);
    static CANCEL: AtomicBool = AtomicBool::new(false);
    type Checked<T> = Result<T, &'static str>;

    pub struct Args { identity: String, client: PathBuf }
    pub fn args(values: Vec<String>) -> Checked<Args> {
        if values.len() != 5 || values[0] != "--qa-identity" || values[2] != "--client-json" || values[4] != "--confirm-connect" {
            return Err("QA_ARGUMENTS");
        }
        if values[1] != APPROVED_ID { return Err("QA_IDENTITY"); }
        let client = PathBuf::from(&values[3]);
        if !client.is_absolute() || client.components().any(|part| matches!(part, Component::ParentDir)) {
            return Err("QA_CLIENT_PATH");
        }
        Ok(Args { identity: values[1].clone(), client })
    }

    fn plain(path: &Path) -> Checked<fs::Metadata> {
        let metadata = fs::symlink_metadata(path).map_err(|_| "QA_PATH_MISSING")?;
        for ancestor in path.ancestors() {
            let info = fs::symlink_metadata(ancestor).map_err(|_| "QA_PATH_MISSING")?;
            if info.file_type().is_symlink() || info.file_attributes() & 0x400 != 0 { return Err("QA_LINKED_PATH"); }
        }
        Ok(metadata)
    }
    fn bounded_json(path: &Path) -> Checked<Value> {
        let info = plain(path)?;
        if !info.is_file() || info.len() > 128 * 1024 { return Err("QA_RECEIPT"); }
        let mut bytes = Vec::new();
        File::open(path).map_err(|_| "QA_RECEIPT")?.take(128 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "QA_RECEIPT")?;
        if bytes.len() > 128 * 1024 { return Err("QA_RECEIPT"); }
        serde_json::from_slice(&bytes).map_err(|_| "QA_RECEIPT")
    }
    fn profile(workspace: &Path, app_data: &Path, identity: &str) -> Checked<PathBuf> {
        if identity != APPROVED_ID { return Err("QA_IDENTITY"); }
        let data = app_data.join(identity);
        let qa = workspace.join(".runtime/platform-app-qa").join(identity);
        if !plain(&data)?.is_dir() || !plain(&qa.join("media"))?.is_dir()
            || !plain(&data.join("atsumi-next.sqlite3"))?.is_file() { return Err("QA_PROFILE"); }
        if data.join("cloud-recording").try_exists().map_err(|_| "QA_PROFILE")? { return Err("QA_CLOUD_ALREADY_EXISTS"); }
        let receipt = bounded_json(&qa.join("cloud-real-copy-receipt.json"))?;
        if receipt["version"] != 2 || receipt["identity"] != identity || receipt["mode"] != "Seed"
            || receipt["originalFilesUnchanged"] != true || receipt["mediaAndSidecarsByteIdentical"] != true
            || receipt["catalogMetadataMatchesExpected"] != true || receipt["credentialsOrProfilesCopied"] != false
            || receipt["uploadRequested"] != false || receipt["sourceOrCopyDeleted"] != false {
            return Err("QA_SEED_RECEIPT");
        }
        Ok(data)
    }
    fn system_tool(name: &str) -> Checked<PathBuf> {
        let root = std::env::var_os("SystemRoot").ok_or("QA_SYSTEM_ROOT")?;
        let path = PathBuf::from(root).join("System32").join(name);
        if !plain(&path)?.is_file() { return Err("QA_SYSTEM_TOOL"); }
        Ok(path)
    }
    fn current_sid() -> Checked<String> {
        let result = Command::new(system_tool("whoami.exe")?).args(["/user", "/fo", "csv", "/nh"])
            .creation_flags(0x08000000).output().map_err(|_| "QA_PRIVATE_ACL")?;
        if !result.status.success() || result.stdout.len() > 4096 { return Err("QA_PRIVATE_ACL"); }
        let text = String::from_utf8(result.stdout).map_err(|_| "QA_PRIVATE_ACL")?;
        let sid = text.trim().rsplit(',').next().unwrap_or("").trim_matches('"');
        if !sid.starts_with("S-1-") || sid.len() > 184 || !sid.bytes().all(|b| b == b'S' || b == b'-' || b.is_ascii_digit()) { return Err("QA_PRIVATE_ACL"); }
        Ok(sid.into())
    }
    fn private_directory(parent: &Path) -> Checked<PathBuf> {
        if !plain(parent)?.is_dir() { return Err("QA_PRIVATE_DIRECTORY"); }
        let directory = parent.join(format!("connect-private-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&directory).map_err(|_| "QA_PRIVATE_DIRECTORY")?;
        let grant = format!("*{}:(OI)(CI)F", current_sid()?);
        // The fresh directory has no explicit inherited-policy overrides.
        // Remove inherited ACEs before placing any authorization material here.
        let result = Command::new(system_tool("icacls.exe")?).arg(&directory)
            .args(["/inheritance:r", "/grant:r", &grant, "*S-1-5-18:(OI)(CI)F"])
            .creation_flags(0x08000000).output().map_err(|_| "QA_PRIVATE_ACL")?;
        if !result.status.success() { return Err("QA_PRIVATE_ACL"); }
        Ok(directory)
    }
    fn valid_auth_url(raw: &str) -> bool {
        if raw.len() > 8192 { return false; }
        let Ok(url) = reqwest::Url::parse(raw) else { return false; };
        if url.scheme() != "https" || url.host_str() != Some("accounts.google.com") || url.port().is_some()
            || url.path() != "/o/oauth2/v2/auth" || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() { return false; }
        let values: Vec<_> = url.query_pairs().collect();
        let one = |name: &str| { let mut found = values.iter().filter(|(key, _)| key == name); let value = found.next(); if found.next().is_some() { None } else { value.map(|(_, value)| value.as_ref()) } };
        let Some(redirect) = one("redirect_uri").and_then(|value| reqwest::Url::parse(value).ok()) else { return false; };
        one("response_type") == Some("code") && one("code_challenge_method") == Some("S256")
            && one("state").is_some_and(|value| value.len() >= 20 && value.len() <= 512)
            && one("code_challenge").is_some_and(|value| value.len() == 43 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
            && redirect.scheme() == "http" && redirect.host_str() == Some("127.0.0.1") && redirect.port().is_some_and(|value| value > 0)
            && redirect.path() == "/oauth2/callback" && redirect.username().is_empty() && redirect.password().is_none() && redirect.query().is_none() && redirect.fragment().is_none()
    }
    fn write_url(path: &Path, raw: &str) -> Checked<()> {
        if !valid_auth_url(raw) { return Err("QA_AUTH_URL"); }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(|_| "QA_AUTH_FILE")?;
        file.write_all(raw.as_bytes()).and_then(|_| file.sync_all()).map_err(|_| "QA_AUTH_FILE")
    }
    fn blocked_resolver() -> SourceResolver { Arc::new(|_, _| Err("QA_CONNECTION_ONLY_UPLOAD_DISABLED".into())) }
    fn safe_code(error: &CloudError) -> String {
        if error.code.len() <= 64 && !error.code.is_empty() && error.code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') { error.code.clone() }
        else { "CLOUD_OPERATION_FAILED".into() }
    }
    fn status(stage: &str, snapshot: &CloudSnapshot) {
        println!("{}", json!({"stage":stage,"configured":snapshot.account.configured,"connected":snapshot.account.connected,"authorizing":snapshot.account.authorizing,"channelBound":snapshot.account.channel_id.is_some(),"jobCount":snapshot.jobs.len(),"errorCode":snapshot.account.error.as_ref().map(safe_code),"uploadTested":false,"appUiTested":false,"replayTested":false}));
        let _ = std::io::stdout().flush();
    }
    #[link(name = "kernel32")]
    extern "system" { fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32; }
    unsafe extern "system" fn cancel_signal(kind: u32) -> i32 {
        if kind <= 1 { CANCEL.store(true, Ordering::Release); 1 } else { 0 }
    }
    struct SignalGuard;
    impl Drop for SignalGuard { fn drop(&mut self) { unsafe { SetConsoleCtrlHandler(Some(cancel_signal), 0); } } }

    pub fn run(args: Args) -> Result<(), String> {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
        let app_data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?);
        let data = profile(workspace, &app_data, &args.identity)?;
        let client_info = plain(&args.client)?;
        if !client_info.is_file() || client_info.len() > 65536 { return Err("QA_CLIENT_FILE".into()); }
        let private = private_directory(&workspace.join(AUDIT_DIR))?;
        let authorization = private.join("authorization-url.txt");
        let cancel_file = private.join("cancel.request");
        if unsafe { SetConsoleCtrlHandler(Some(cancel_signal), 1) } == 0 { return Err("QA_CANCEL_HANDLER".into()); }
        let _signal = SignalGuard;
        let output_path = authorization.clone();
        let service = CloudService::new(data, blocked_resolver(), Arc::new(move |url| write_url(&output_path, url).map_err(str::to_owned))).map_err(|error| safe_code(&error))?;
        let outcome = (|| -> Result<&'static str, String> {
            let initial = service.snapshot().map_err(|error| safe_code(&error))?;
            if initial.account.configured || initial.account.connected || !initial.jobs.is_empty() { return Err("QA_CLOUD_NOT_EMPTY".into()); }
            let configured = service.import_client_path(&args.client).map_err(|error| safe_code(&error))?;
            status("configured", &configured);
            let authorizing = service.authorize().map_err(|error| safe_code(&error))?;
            println!("{}", json!({"stage":"authorization_files","identity":args.identity,"authorizationUrlFile":authorization,"cancelFile":cancel_file,"timeoutSeconds":DEADLINE.as_secs()}));
            status("authorizing", &authorizing);
            let deadline = Instant::now() + DEADLINE;
            loop {
                if CANCEL.load(Ordering::Acquire) || cancel_file.try_exists().map_err(|_| "QA_CANCEL_FILE")? { return Ok("cancelled"); }
                if Instant::now() >= deadline { return Ok("timeout"); }
                let snapshot = service.snapshot().map_err(|error| safe_code(&error))?;
                if !snapshot.jobs.is_empty() { return Err("QA_UPLOAD_FORBIDDEN".into()); }
                if snapshot.account.connected { status("connected", &snapshot); return Ok("connected"); }
                if !snapshot.account.authorizing { status("authorization_failed", &snapshot); return Err(snapshot.account.error.as_ref().map(safe_code).unwrap_or("QA_NOT_CONNECTED".into())); }
                std::thread::sleep(Duration::from_millis(200));
            }
        })();
        // No forced termination. Existing bounded HTTP calls may still be draining.
        service.shutdown_and_wait().map_err(|error| safe_code(&error))?;
        let snapshot = service.snapshot().map_err(|error| safe_code(&error))?;
        status("drained", &snapshot);
        match outcome {
            Ok("connected") => Ok(()),
            Ok(reason) => Err(format!("QA_{}", reason.to_ascii_uppercase())),
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn argv(identity: &str) -> Vec<String> { ["--qa-identity", identity, "--client-json", "C:\\synthetic\\client.json", "--confirm-connect"].into_iter().map(str::to_owned).collect() }
        fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
            let root = tempfile::tempdir().unwrap();
            let workspace = root.path().join("workspace"); let appdata = root.path().join("appdata");
            fs::create_dir_all(appdata.join(APPROVED_ID)).unwrap();
            fs::write(appdata.join(APPROVED_ID).join("atsumi-next.sqlite3"), b"synthetic-only").unwrap();
            let qa = workspace.join(".runtime/platform-app-qa").join(APPROVED_ID); fs::create_dir_all(qa.join("media")).unwrap();
            fs::write(qa.join("cloud-real-copy-receipt.json"), serde_json::to_vec(&json!({"version":2,"identity":APPROVED_ID,"mode":"Seed","originalFilesUnchanged":true,"mediaAndSidecarsByteIdentical":true,"catalogMetadataMatchesExpected":true,"credentialsOrProfilesCopied":false,"uploadRequested":false,"sourceOrCopyDeleted":false})).unwrap()).unwrap();
            (root, workspace, appdata)
        }
        #[test] fn explicit_exact_identity_and_absolute_client_are_required() {
            assert!(args(argv(APPROVED_ID)).is_ok());
            for identity in ["local.atsumi.next", "local.atsumi.platformqa.00000000000000000000000000000000", "../local.atsumi.next"] { assert!(args(argv(identity)).is_err()); }
            let mut missing = argv(APPROVED_ID); missing.pop(); assert!(args(missing).is_err());
            let mut relative = argv(APPROVED_ID); relative[3] = "client.json".into(); assert!(args(relative).is_err());
        }
        #[test] fn existing_isolated_seed_is_required_and_cloud_reuse_is_refused() {
            let (_root, workspace, appdata) = fixture();
            assert_eq!(profile(&workspace, &appdata, APPROVED_ID).unwrap(), appdata.join(APPROVED_ID));
            assert!(profile(&workspace, &appdata, "local.atsumi.next").is_err());
            assert!(profile(&workspace, &workspace.join("missing"), APPROVED_ID).is_err());
            fs::create_dir(appdata.join(APPROVED_ID).join("cloud-recording")).unwrap();
            assert_eq!(profile(&workspace, &appdata, APPROVED_ID).unwrap_err(), "QA_CLOUD_ALREADY_EXISTS");
        }
        #[test] fn metadata_or_identity_receipt_failure_is_not_accepted() {
            let (_root, workspace, appdata) = fixture(); let path = workspace.join(".runtime/platform-app-qa").join(APPROVED_ID).join("cloud-real-copy-receipt.json");
            let original = bounded_json(&path).unwrap();
            for (key, value) in [("identity", json!("local.atsumi.next")), ("version", json!(1)), ("catalogMetadataMatchesExpected", json!(false)), ("credentialsOrProfilesCopied", json!(true))] {
                let mut changed = original.clone(); changed[key] = value; fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
                assert_eq!(profile(&workspace, &appdata, APPROVED_ID).unwrap_err(), "QA_SEED_RECEIPT");
            }
        }
        fn url() -> String { format!("https://accounts.google.com/o/oauth2/v2/auth?response_type=code&code_challenge_method=S256&state={}&code_challenge={}&redirect_uri=http%3A%2F%2F127.0.0.1%3A43123%2Foauth2%2Fcallback", "a".repeat(64), "b".repeat(43)) }
        #[test] fn url_boundary_and_create_new_prevent_redirect_or_overwrite() {
            let value = url(); assert!(valid_auth_url(&value));
            for bad in [value.replace("accounts.google.com", "evil.test"), value.replace("127.0.0.1", "localhost"), format!("{value}&state=other"), format!("{value}#fragment")] { assert!(!valid_auth_url(&bad)); }
            let root = tempfile::tempdir().unwrap(); let path = root.path().join("synthetic-url.txt");
            write_url(&path, &value).unwrap(); assert!(write_url(&path, &value).is_err()); assert_eq!(fs::read_to_string(path).unwrap(), value);
        }
        #[test] fn error_output_has_no_provider_text() {
            let error = CloudError::new("bad https://secret.test", "token=hidden", false); assert_eq!(safe_code(&error), "CLOUD_OPERATION_FAILED");
            assert_eq!(safe_code(&CloudError::new("CLOUD_AUTH_DENIED", "hidden", false)), "CLOUD_AUTH_DENIED");
        }
        #[test] fn every_source_is_rejected_without_reading_any_media() {
            use atsumi_lib::cloud_recording::{CloudSourceRef, SourceResolveContext};
            let context = SourceResolveContext { cancel: Arc::new(AtomicBool::new(false)), progress: Arc::new(|_| {}) };
            for provider in ["youtube", "soop", "chzzk"] {
                let source = CloudSourceRef { provider: provider.into(), recording_id: "synthetic".into(), part_index: None, plan_id: None, unit_id: None };
                assert!(blocked_resolver()(&source, &context).is_err());
            }
        }
    }
}

#[cfg(all(windows, debug_assertions))]
fn main() {
    let result = qa::args(std::env::args().skip(1).collect()).map_err(str::to_owned).and_then(qa::run);
    if let Err(code) = result { eprintln!("{}", serde_json::json!({"status":"not_connected","errorCode":code,"uploadTested":false,"appUiTested":false,"replayTested":false})); std::process::exit(2); }
}
#[cfg(not(all(windows, debug_assertions)))]
fn main() { eprintln!("QA_REQUIRES_WINDOWS_DEBUG_BUILD"); std::process::exit(2); }
