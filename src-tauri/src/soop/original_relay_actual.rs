//! Opt-in real Wry/AppState trial. No account, cloud request, desktop input or originals.
#[path = "original_relay_hls_actual.rs"]
mod hls;
use super::*;
use crate::interface::{AppQuitRequest, AppState};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn facts(app: &AppHandle) -> Result<Value, String> {
    let views: Vec<_> = app
        .webviews()
        .into_values()
        .filter(|view| view.label().starts_with("soop-original-"))
        .collect();
    if views.len() != 1 {
        return Err("receiver_count".into());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    views[0].eval_with_callback(r#"(()=>{const v=document.getElementById('livePlayer');const q=v?.getVideoPlaybackQuality?.();return {time:v?.currentTime,ready:v?.readyState,paused:v?.paused,rate:v?.playbackRate,frames:q?.totalVideoFrames,dropped:q?.droppedVideoFrames,bufferEnd:v?.buffered?.length?v.buffered.end(v.buffered.length-1):null,chatRows:document.querySelectorAll('#chatting_area .chatting-list-item').length,relay:window.__atsumiSoopEncodedCapture?.getRelayStatus?.()}})()"#,
        move |value| { let _ = tx.send(value); }).map_err(|_| "inspect_dispatch")?;
    serde_json::from_str(
        &rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| "inspect_timeout")?,
    )
    .map_err(|_| "inspect_shape".into())
}

fn window(app: &AppHandle, seconds: u64) -> Result<Value, String> {
    let before = facts(app)?;
    thread::sleep(Duration::from_secs(seconds));
    let after = facts(app)?;
    let progressed = after["time"]
        .as_f64()
        .zip(before["time"].as_f64())
        .is_some_and(|(a, b)| a > b + 1.)
        && after["frames"]
            .as_u64()
            .zip(before["frames"].as_u64())
            .is_some_and(|(a, b)| a > b)
        && before["rate"] == 1.0
        && after["rate"] == 1.0;
    Ok(json!({"before":before,"after":after,"progressed":progressed}))
}

fn video_files(root: &Path) -> Result<usize, String> {
    let mut paths = vec![root.to_path_buf()];
    let mut seen = 0;
    let mut videos = 0;
    while let Some(path) = paths.pop() {
        for entry in fs::read_dir(path).map_err(|_| "media_read")? {
            let entry = entry.map_err(|_| "media_entry")?;
            seen += 1;
            if seen > 4096 {
                return Err("media_bound".into());
            }
            let info = fs::symlink_metadata(entry.path()).map_err(|_| "media_metadata")?;
            use std::os::windows::fs::MetadataExt;
            if info.file_type().is_symlink() || info.file_attributes() & 0x400 != 0 {
                return Err("media_link".into());
            }
            if info.is_dir() {
                paths.push(entry.path());
            } else if entry
                .path()
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| matches!(v, "mp4" | "m4s" | "ts" | "webm" | "mkv"))
            {
                videos += 1;
            }
        }
    }
    Ok(videos)
}

fn exercise(
    app: &AppHandle,
    root: &Path,
    channel: String,
    evidence: &mut Value,
) -> Result<(), String> {
    let startup = app.state::<Arc<crate::startup::Startup>>();
    crate::initialize_backend(app, &startup).map_err(|_| "backend_init")?;
    let state = app.state::<AppState>();
    let media = state.platform_media(app)?;
    let host = app.state::<SoopOriginalHost>();
    host.set_tray_muted(app, true);
    let opened = host.open(
        app,
        media.clone(),
        channel,
        uuid::Uuid::new_v4().to_string(),
        root.join("media"),
    )?;
    let token = opened.token;
    host.viewport(app, &token, serde_json::from_value(json!({"sequence":1,"player":{"x":0,"y":0,"width":960,"height":720},"chat":{"x":970,"y":0,"width":320,"height":720},"visible":true,"muted":true,"occlusions":[]})).map_err(|_| "viewport_shape")?)?;
    let deadline = Instant::now() + Duration::from_secs(65);
    loop {
        let snapshot = host.snapshot(&token)?;
        if snapshot.capture_ready
            && snapshot.chat_visible
            && facts(app)?["ready"].as_u64().unwrap_or(0) >= 3
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("source_not_ready".into());
        }
        thread::sleep(Duration::from_millis(500));
    }
    evidence["baseline"] = window(app, 4)?;
    if evidence["baseline"]["progressed"] != true {
        return Err("baseline_stalled".into());
    }
    let before_work = state.active_work_snapshot().map_err(|_| "work_before")?;
    let before_files = video_files(&root.join("media"))?;
    let before_jobs = media.jobs(Some("soop")).len();
    if before_files != 0 || before_jobs != 0 {
        return Err("relay_baseline_has_recording".into());
    }
    let run = host.start_relay_inspector(app, &token)?;
    let during_work = state.active_work_snapshot().map_err(|_| "work_during")?;
    evidence["relayOnly"] = window(app, 8)?;
    evidence["relayOnlyVideoFiles"] = json!(video_files(&root.join("media"))? - before_files);
    evidence["relayOnlyJobs"] = json!(media.jobs(Some("soop")).len() - before_jobs);
    evidence["workFingerprintChanged"] =
        json!(before_work.work_set_fingerprint != during_work.work_set_fingerprint);
    evidence["receiverCount"] = json!(during_work.live_receivers.as_ref().map(|r| r.active_count));
    evidence["downloads"] = json!(during_work.downloads.active_count);
    if evidence["relayOnly"]["progressed"] != true
        || evidence["relayOnlyVideoFiles"] != 0
        || evidence["relayOnlyJobs"] != 0
        || evidence["workFingerprintChanged"] != true
        || evidence["receiverCount"] != 1
    {
        return Err("relay_only_invariant".into());
    }
    host.record(app, &token, true)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let job = loop {
        if let Some(job) = host.snapshot(&token)?.recording_job {
            break job;
        }
        if Instant::now() >= deadline {
            return Err("archive_start_timeout".into());
        }
        thread::sleep(Duration::from_millis(250));
    };
    evidence["archiveOverlap"] = window(app, 8)?;
    let before_archive_stop = host
        .relay_inspection(&token)?
        .ok_or("relay_progress_missing")?;
    evidence["relayBeforeArchiveStop"] =
        serde_json::to_value(&before_archive_stop).map_err(|_| "inspection_shape")?;
    host.record(app, &token, false)?;
    evidence["afterArchiveStop"] = window(app, 6)?;
    let after_archive_stop = host
        .relay_inspection(&token)?
        .ok_or("relay_progress_missing")?;
    evidence["relayAfterArchiveStop"] =
        serde_json::to_value(&after_archive_stop).map_err(|_| "inspection_shape")?;
    if after_archive_stop.output_bytes <= before_archive_stop.output_bytes
        || after_archive_stop.video_samples <= before_archive_stop.video_samples
        || after_archive_stop.audio_samples <= before_archive_stop.audio_samples
    {
        return Err("relay_did_not_advance_after_archive_stop".into());
    }
    if evidence["afterArchiveStop"]["after"]["relay"]["active"] != true
        || evidence["afterArchiveStop"]["after"]["relay"]["nativeApproved"] != true
    {
        return Err("relay_stopped_with_archive".into());
    }
    host.stop_relay_inspector(app, &token, false)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        host.poll_relay_join(app);
        if let Some(inspection) = host.relay_inspection(&token)? {
            evidence["relay"] =
                serde_json::to_value(&inspection).map_err(|_| "inspection_shape")?;
            if inspection.manager_joined {
                break;
            }
        }
        if Instant::now() >= deadline {
            return Err("relay_join_timeout".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    let deadline = Instant::now() + Duration::from_secs(50);
    let archive = loop {
        let row = media
            .jobs(Some("soop"))
            .into_iter()
            .find(|j| j.job_id == job)
            .ok_or("archive_missing")?;
        if matches!(
            row.state.as_str(),
            "completed" | "failed" | "interrupted" | "cancelled"
        ) {
            break row;
        }
        if Instant::now() >= deadline {
            return Err("archive_finish_timeout".into());
        }
        thread::sleep(Duration::from_millis(250));
    };
    let reopened = media
        .open(&job)
        .map(|p| {
            media.close(&p.token);
            true
        })
        .unwrap_or(false);
    evidence["archive"] =
        json!({"state":archive.state,"reopened":reopened,"jobs":media.jobs(Some("soop")).len()});
    let relay = &evidence["relay"];
    if relay["runId"] != run
        || relay["workerJoined"] != true
        || relay["codecAdmitted"] != true
        || relay["interrupted"] != false
        || relay["outputBytes"].as_u64().unwrap_or(0) == 0
        || evidence["archiveOverlap"]["progressed"] != true
        || evidence["afterArchiveStop"]["progressed"] != true
        || archive.state != "completed"
        || !reopened
    {
        return Err("overlap_or_join_invariant".into());
    }
    // Leave another owned run active: the normal application drain must join it.
    let second_run = host.start_relay_inspector(app, &token)?;
    evidence["beforeQuit"] = window(app, 4)?;
    let second = host
        .relay_inspection(&token)?
        .ok_or("second_relay_missing")?;
    evidence["secondRelayBeforeQuit"] =
        serde_json::to_value(&second).map_err(|_| "inspection_shape")?;
    let quit_work = state
        .active_work_snapshot()
        .map_err(|_| "work_before_quit")?;
    evidence["beforeQuitReceiverCount"] =
        json!(quit_work.live_receivers.as_ref().map(|r| r.active_count));
    evidence["secondRunChangedFingerprint"] =
        json!(quit_work.work_set_fingerprint != during_work.work_set_fingerprint);
    if evidence["beforeQuit"]["progressed"] != true
        || evidence["beforeQuit"]["after"]["relay"]["active"] != true
        || evidence["beforeQuit"]["after"]["relay"]["nativeApproved"] != true
        || second.run_id != second_run
        || second.output_bytes == 0
        || second.manager_joined
        || evidence["beforeQuitReceiverCount"] != 1
        || evidence["secondRunChangedFingerprint"] != true
    {
        return Err("second_relay_not_active_before_quit".into());
    }
    evidence["quitWithActiveRelay"] = json!(true);
    Ok(())
}

#[test]
#[ignore = "Explicit isolated public SOOP receiver trial; no account or YouTube transmission"]
fn actual_soop_relay_same_receiver_archive_and_graceful_quit() {
    assert_eq!(
        std::env::var("ATSUMI_SOOP_RELAY_QA").as_deref(),
        Ok("public-receiver-no-cloud"),
        "QA_OPT_IN"
    );
    let channel = std::env::var("ATSUMI_SOOP_RELAY_QA_CHANNEL").expect("QA_CHANNEL_REQUIRED");
    assert!(
        !channel.is_empty()
            && channel.len() <= 40
            && channel
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
        "QA_CHANNEL"
    );
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let root = workspace.join(".runtime/soop-relay-native-qa").join(&run);
    fs::create_dir_all(root.parent().unwrap()).unwrap();
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("media")).unwrap();
    let identity = format!("local.atsumi.sooprelayqa.{run}");
    let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
    assert!(!profile.exists(), "QA_PROFILE_EXISTS");
    let result = Arc::new(Mutex::new(
        json!({"passed":false,"isolated":true,"noCloud":true}),
    ));
    let output = result.clone();
    let worker_root = root.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = identity;
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default()
        .any_thread()
        .manage(SoopOriginalHost::default())
        .manage(crate::youtube::original::YouTubeOriginalHost::default())
        .manage(Arc::new(crate::startup::Startup::default()))
        .manage(Arc::new(
            crate::ui_download_events::DownloadEvents::default(),
        ))
        .setup(move |app| {
            if app.path().app_data_dir()? != profile {
                return Err("QA_PROFILE_PATH".into());
            }
            tauri::window::WindowBuilder::new(app, "main")
                .title("Atsumi relay verification")
                .inner_size(1290., 720.)
                .position(-16000., -16000.)
                .focused(false)
                .skip_taskbar(true)
                .build()?;
            let handle = app.handle().clone();
            thread::spawn(move || {
                let mut report = json!({"passed":false,"isolated":true,"noCloud":true});
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    exercise(&handle, &worker_root, channel, &mut report)
                }))
                .unwrap_or_else(|_| Err("qa_worker_panic".into()));
                report["passed"] = json!(outcome.is_ok());
                report["error"] = json!(outcome.err());
                *lock(&output) = report;
                if let Some(state) = handle.try_state::<AppState>() {
                    if let Ok(snapshot) = state.active_work_snapshot() {
                        let request = AppQuitRequest {
                            expected_work_set_fingerprint: snapshot.work_set_fingerprint,
                            confirm_active_work: true,
                            force_when_status_unknown: false,
                        };
                        match state.request_graceful_quit(handle.clone(), request) {
                            Ok(answer) if answer.accepted => return,
                            _ => {
                                lock(&output)["passed"] = json!(false);
                                lock(&output)["quitError"] = json!("normal_quit_rejected");
                            }
                        }
                    }
                    // Do not force exit around an unknown or retained worker.
                    let _ = fs::write(
                        worker_root.join("pending-result.json"),
                        serde_json::to_vec_pretty(&*lock(&output)).unwrap(),
                    );
                } else {
                    handle.exit(2);
                }
            });
            Ok(())
        })
        .build(context)
        .expect("QA_BUILD");
    app.run_return(|_, _| {});
    let mut report = lock(&result).clone();
    report["normalAppExitReturned"] = json!(true);
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("QA_RELAY_RESULT={}", root.join("result.json").display());
    assert_eq!(
        report["passed"], true,
        "Actual same-receiver relay remains unverified; inspect result"
    );
}
