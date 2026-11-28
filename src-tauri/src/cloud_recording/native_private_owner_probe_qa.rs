//! Test-only existing isolated website-session probe. No upload or login actions.
//! Register under cloud_recording/tests.rs only after supervising review.
use std::{fs, io::Read, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Manager;

const IDENTITY: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
fn plain(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        let meta = fs::symlink_metadata(part).map_err(|_| "QA_PATH")?;
        if meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0 { return Err("QA_LINK".into()); }
    }
    Ok(())
}
fn home(url: &tauri::Url) -> bool {
    url.scheme()=="https" && url.host_str()==Some("www.youtube.com") && url.port().is_none()
        && url.username().is_empty() && url.password().is_none() && url.path()=="/" && url.query().is_none() && url.fragment().is_none()
}
fn observe(view: &tauri::WebviewWindow) -> Result<(Option<bool>, bool), String> {
    if !view.url().ok().as_ref().is_some_and(home) { return Err("QA_DOCUMENT".into()); }
    let started=Instant::now(); let (tx,rx)=std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(include_str!("../youtube/original_account.js"),move|raw| { let _=tx.try_send((raw.len()<=4096).then_some(raw)); }).map_err(|_|"QA_EVAL")?;
    let raw=rx.recv_timeout(Duration::from_secs(3)).map_err(|_|"QA_EVAL_TIMEOUT")?.ok_or("QA_EVAL_SIZE")?;
    if started.elapsed()>Duration::from_secs(2) || !view.url().ok().as_ref().is_some_and(home) {return Err("QA_STALE_OBSERVATION".into());}
    let value:Value=serde_json::from_str(&raw).map_err(|_|"QA_EVAL_JSON")?;
    Ok((value["authenticated"].as_bool(),value["interactive"].as_bool().unwrap_or(true)))
}

#[test]
#[ignore = "Existing isolated YouTube profile website-authentication observation only; no upload/login/input"]
fn actual_existing_isolated_private_owner_website_probe() -> Result<(), String> {
    if std::env::var("ATSUMI_PRIVATE_OWNER_PROBE_QA").as_deref()!=Ok(IDENTITY) { return Err("QA_OPT_IN".into()); }
    let appdata=PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?);
    let data=appdata.join(IDENTITY); let browser=data.join("youtube-original-profile");
    plain(&browser)?; if !browser.is_dir() {return Err("QA_EXISTING_BROWSER_PROFILE_REQUIRED".into());}
    let state=data.join("cloud-recording/state.bin"); plain(&state)?;
    let state=fs::canonicalize(&state).map_err(|_|"QA_CATALOG_CANONICAL")?; plain(&state)?;
    // Keep the exact existing catalog pinned against mutation; do not decrypt it.
    let mut pin=crate::streaming::browser_store::cleanup::verification_guard(&state).map_err(|_|"QA_CATALOG_PIN")?;
    let length=pin.metadata().map_err(|_|"QA_CATALOG_STAT")?.len();
    if length==0 || length>256*1024 {return Err("QA_CATALOG_BOUND".into());}
    let mut bytes=Vec::new(); pin.read_to_end(&mut bytes).map_err(|_|"QA_CATALOG_READ")?;
    let before=format!("{:x}",Sha256::digest(&bytes)); bytes.fill(0); drop(bytes);
    let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let output=workspace.join(".runtime/requirements-audit/20261122-cloud-resume").join(format!("private-owner-probe-{}",uuid::Uuid::new_v4().simple()));
    fs::create_dir(&output).map_err(|_|"QA_OUTPUT")?;
    let blocked=Arc::new(AtomicBool::new(false)); let navigated=blocked.clone();
    let initial_blank=Arc::new(AtomicBool::new(false)); let navigation_blank=initial_blank.clone();
    let home_started=AtomicBool::new(false);
    let result=Arc::new(Mutex::new(None::<Value>)); let worker_result=result.clone();
    let worker=Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>)); let setup_worker=worker.clone();
    let exit_seen=Arc::new(AtomicBool::new(false)); let run_exit_seen=exit_seen.clone();
    let profile_check=data.clone(); let mut context=tauri::generate_context!();
    context.config_mut().identifier=IDENTITY.into(); context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().any_thread().setup(move|app| {
        if app.path().app_data_dir()?!=profile_check {return Err("QA_PROFILE_IDENTITY".into());}
        let view=tauri::WebviewWindowBuilder::new(app,"qa-private-owner-probe",tauri::WebviewUrl::External("https://www.youtube.com/".parse().unwrap()))
            .data_directory(browser).browser_extensions_enabled(false).additional_browser_args(crate::browser_identity::BROWSER_ARGS)
            .title("Atsumi isolated website-session check").inner_size(800.,600.).position(-16000.,-16000.).visible(false).focused(false).skip_taskbar(true)
            .on_navigation(move|url|{
                if home(url){home_started.store(true,Ordering::Release);return true;}
                // Do not widen the allowlist for speculative WebView startup behavior.
                // A rejected initial blank is diagnostic, not a login redirect.
                if url.as_str()=="about:blank"&&!home_started.load(Ordering::Acquire){navigation_blank.store(true,Ordering::Release);return false;}
                navigated.store(true,Ordering::Release);false
            })
            .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).on_download(|_,_|false).build()?;
        let handle=app.handle().clone();
        let join=std::thread::spawn(move|| {
            let deadline=Instant::now()+Duration::from_secs(45); let mut samples=0; let mut stable=0; let mut last=None; let mut interactive=true; let mut errors=0;
            while Instant::now()<deadline && !blocked.load(Ordering::Acquire) {
                match observe(&view) {
                    Ok((value,prompt))=> {samples+=1; interactive=prompt; if value.is_some() && value==last && !prompt {stable+=1;} else {stable=0;} last=value; if stable>=2 {break;}},
                    Err(_)=>{errors+=1;stable=0;}
                }
                std::thread::sleep(Duration::from_millis(350));
            }
            let authenticated=last==Some(true)&&stable>=2&&!interactive&&!blocked.load(Ordering::Acquire);
            let status=if authenticated {"website_authenticated_owner_unverified"} else if last==Some(false)&&stable>=2 {"user_authentication_required"} else if blocked.load(Ordering::Acquire)||interactive&&last.is_some() {"interactive_or_consent_required"} else {"observation_unavailable"};
            let close_accepted=view.close().is_ok();
            let observed_at=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|d|u64::try_from(d.as_millis()).ok());
            *worker_result.lock().unwrap()=Some(json!({"status":status,"profileIdentity":IDENTITY,"observedAtMs":observed_at,"websiteAuthenticated":authenticated,"ownerChannelVerified":false,"privateVideoAccessVerified":false,"stableSamples":stable,"samples":samples,"observationErrors":errors,"blockedNavigation":blocked.load(Ordering::Acquire),"initialBlankRejected":initial_blank.load(Ordering::Acquire),"interactive":interactive,"uploads":0,"loginActions":0,"desktopInput":0,"credentialContentsRead":false,"profileCopied":false,"closeAccepted":close_accepted}));
            handle.exit(0);
        });
        *setup_worker.lock().map_err(|_|"QA_WORKER_LOCK")?=Some(join); Ok(())
    }).build(context).map_err(|_|"QA_APP_BUILD")?;
    let exit_code=app.run_return(move|_,event|{if matches!(event,tauri::RunEvent::Exit){run_exit_seen.store(true,Ordering::Release);}});
    let joined=worker.lock().map_err(|_|"QA_WORKER_LOCK")?.take().ok_or("QA_WORKER_MISSING")?.join().is_ok();
    use std::io::{Seek,SeekFrom}; pin.seek(SeekFrom::Start(0)).map_err(|_|"QA_CATALOG_SEEK")?;
    let mut after=Vec::new(); pin.read_to_end(&mut after).map_err(|_|"QA_CATALOG_READ")?;
    let same=after.len()as u64==length&&format!("{:x}",Sha256::digest(&after))==before;after.fill(0);
    let mut report=result.lock().unwrap().take().ok_or("QA_NO_OBSERVATION")?;
    let normal_exit=exit_code==0&&exit_seen.load(Ordering::Acquire)&&joined&&report["closeAccepted"]==true;
    report["originalCatalogUnchanged"]=json!(same);report["normalExit"]=json!(normal_exit);report["exitObserved"]=json!(exit_seen.load(Ordering::Acquire));report["workerJoined"]=json!(joined);report["profileRetained"]=json!(true);
    let target=output.join("result.json"); use std::io::Write;
    let mut file=fs::OpenOptions::new().create_new(true).write(true).open(&target).map_err(|_|"QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_|"QA_REPORT")?).and_then(|_|file.sync_all()).map_err(|_|"QA_REPORT")?;
    println!("QA_PRIVATE_OWNER_PROBE={}",target.display());
    if !same {return Err("QA_CATALOG_CHANGED".into());}
    if !normal_exit {return Err("QA_NORMAL_EXIT_UNCONFIRMED".into());} Ok(())
}
