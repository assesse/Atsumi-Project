//! Controlled lifecycle tests. No fixture event is sent to a public service.
use super::*;
use crate::platform_media::{chat_assets::SessionAssets, live_chat::SharedChat};
use serde_json::{json, Value};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::atomic::AtomicUsize,
    thread,
    time::Instant,
};

struct Gate {
    entered: AtomicUsize,
    released: AtomicUsize,
    returned: AtomicUsize,
}
impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            returned: AtomicUsize::new(0),
        })
    }
    fn release(&self, n: usize) {
        self.released.fetch_max(n, Ordering::AcqRel);
    }
}
struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release(usize::MAX);
    }
}
fn until(done: impl Fn() -> bool, seconds: u64) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !done() {
        if Instant::now() >= deadline {
            return Err("qa_bounded_wait".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn delayed_chat(bytes: Vec<u8>) -> (Arc<SharedChat>, Arc<Gate>) {
    let gate = Gate::new();
    let worker = gate.clone();
    let assets = SessionAssets::with_test_fetcher(move |_| {
        let n = worker.entered.fetch_add(1, Ordering::AcqRel) + 1;
        until(|| worker.released.load(Ordering::Acquire) >= n, 10)?;
        worker.returned.fetch_add(1, Ordering::AcqRel);
        Ok(bytes.clone())
    });
    (SharedChat::with_test_assets(assets), gate)
}
fn lease_with_chat(
    service: &PlatformMediaService,
    root: &Path,
    chat: Arc<SharedChat>,
) -> Result<OfficialReceiverLease, String> {
    service
        .inner
        .synthetic_live_receiver
        .store(true, Ordering::Release);
    let id = uuid::Uuid::new_v4().to_string();
    let mut lease = service.reserve_official_receiver(
        MediaRef {
            provider: "soop".into(),
            kind: "live".into(),
            external_id: "controlledfixture".into(),
        },
        "123".into(),
        id.clone(),
        root.into(),
    )?;
    // This fresh synthetic receiver has no callback/worker. Replace its chat
    // before any archive begins; no mutable production receiver API is added.
    let mut registry = service.inner.live.lock().map_err(|_| "qa_registry")?;
    let registered = registry.official.remove(&id).ok_or("qa_receiver_missing")?;
    if !Arc::ptr_eq(&registered, &lease.receiver) {
        return Err("qa_receiver_changed".into());
    }
    drop(registered);
    Arc::get_mut(&mut lease.receiver)
        .ok_or("qa_receiver_shared")?
        .chat = chat;
    registry.official.insert(id, lease.receiver.clone());
    drop(registry);
    Ok(lease)
}
fn image_event(chat: &SharedChat) -> Result<String, String> {
    // Local controlled input, never an actual newly observed official event.
    let context = crate::soop::chat::personalcon::Context::from_channel(
        &json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/373867c54c3ee5ae5.png?_=1741052109"}]}}),
    );
    let request = context
        .select("32|262144", "17", "0")
        .0
        .ok_or("qa_typed_request")?;
    let id = request.id().to_owned();
    let received = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "qa_clock")?
        .as_millis();
    let line = serde_json::to_vec(&json!({"kind":"chat","receivedAtMs":received,"messageId":uuid::Uuid::new_v4().to_string(),"nickname":"Controlled fixture","text":"Local lifecycle fixture","flags":"32|262144","extra":{"service":5,"personalcon":request.metadata()}})).map_err(|_| "qa_fixture_json")?;
    chat.ingest_test_personalcon(&line, request)?;
    Ok(id)
}
fn owned(service: &PlatformMediaService, id: &str) -> bool {
    service
        .active_ids()
        .iter()
        .any(|v| v == &format!("platform-media:{id}"))
}
fn settled(service: &PlatformMediaService, id: &str) -> Result<MediaJob, String> {
    until(|| !owned(service, id), 45)?;
    service
        .inner
        .state
        .lock()
        .map_err(|_| "qa_state")?
        .jobs
        .get(id)
        .cloned()
        .ok_or("qa_job".into())
}
fn asset_facts(directory: &Path, id: &str, expected: &[u8]) -> Result<Value, String> {
    let bytes = fs::read(directory.join("chat-assets").join(id)).map_err(|_| "qa_asset_missing")?;
    if bytes != expected {
        return Err("qa_asset_bytes".into());
    }
    let status: Value = serde_json::from_slice(
        &fs::read(directory.join("chat-assets/status.json")).map_err(|_| "qa_asset_status")?,
    )
    .map_err(|_| "qa_asset_status")?;
    if status["expected"] != 1 || status["stored"] != 1 || status["incomplete"] != false {
        return Err("qa_asset_incomplete".into());
    }
    let manifest = fs::read_to_string(directory.join("chat-assets/manifest.jsonl"))
        .map_err(|_| "qa_asset_manifest")?;
    let rows = manifest
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "qa_asset_manifest")?;
    let sha = format!("{:x}", Sha256::digest(expected));
    if rows.len() != 1
        || rows[0]["id"] != id
        || rows[0]["sha256"] != sha
        || rows[0]["mime"] != "image/png"
        || rows[0]["bytes"] != expected.len()
    {
        return Err("qa_asset_manifest_binding".into());
    }
    Ok(
        json!({"expected":1,"stored":1,"incomplete":false,"bytes":bytes.len(),"sha256":sha,"mime":"image/png"}),
    )
}

#[test]
fn encoded_finalizer_retains_pending_image_owner_until_target_settles() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("no-media-or-network-tool.exe");
    let service = PlatformMediaService::new(
        root.path().join("data"),
        PlatformMediaTools {
            yt_dlp: missing.clone(),
            deno: missing.clone(),
            ffmpeg: missing.clone(),
            ffprobe: missing,
        },
        None,
    )
    .unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let expected = png.into_inner();
    let (chat, gate) = delayed_chat(expected.clone());
    let release = ReleaseOnDrop(gate.clone());
    let lease = lease_with_chat(&service, &root.path().join("media"), chat.clone()).unwrap();
    let source = uuid::Uuid::new_v4().to_string();
    let mut archive = lease
        .begin_encoded(
            source.clone(),
            1,
            uuid::Uuid::new_v4().to_string(),
            fmp4::fixtures::inputs(),
            "Controlled".into(),
            "Fixture".into(),
            live::RecordingOwner::Manual,
        )
        .unwrap();
    for track in [0, 1] {
        archive
            .append(EncodedAppend {
                source_id: source.clone(),
                generation: 1,
                track_index: track,
                index: 0,
                chunk: 0,
                final_chunk: true,
                bytes: fmp4::fixtures::fragment(track, 0, 4, true),
            })
            .unwrap();
    }
    let id = image_event(&chat).unwrap();
    until(|| gate.entered.load(Ordering::Acquire) == 1, 3).unwrap();
    let job = archive.snapshot().unwrap();
    let directory = archive.writer.as_ref().unwrap().directory.clone();
    archive.finish(None).unwrap();
    assert!(owned(&service, &job.job_id));
    assert!(!directory.join("chat-assets/status.json").exists());
    assert!(!directory.join("chat-status.final.json").exists());
    gate.release(1);
    // Structural fixture deliberately lacks tools: video fails honestly, but
    // the real dispatch/finalizer must first durably preserve the pending image.
    assert_eq!(settled(&service, &job.job_id).unwrap().state, "failed");
    asset_facts(&directory, &id, &expected).unwrap();
    assert!(directory.join("chat-status.final.json").is_file());
    drop(release);
    drop(lease);
    service.shutdown();
}

#[cfg(windows)]
mod actual {
    use super::*;
    use crate::interface::{AppQuitRequest, AppState};
    use tauri::{AppHandle, Manager};
    const PNG: &str = ".runtime/soop-original-qa/f06ad51f9c5c4b55b896eb0b3af4a007/media/SOOP/67e02309-df4c-4360-9a24-3835664f23ed/chat-assets/f466d8a10d48d0525e2279afd45700cd7542bc0fd27c6482d0da14fd8d85076e";
    const PNG_SHA: &str = "e7a308b177936812c81b2ed8f339b791943c7aea1a278484ababf252147f3ad7";
    const MP4: &str = ".runtime/soop-original-qa/baa5338478ee4c8cb1cdda57d278d918/media/SOOP/8fd00700-81d4-401f-a083-0d13fcf286ea/e00000008.mp4";
    const MP4_SHA: &str = "861f08cf68605ff4e7393e652a149d137552aaef4093b981637dc6e43911d088";
    fn no_links(path: &Path) -> Result<(), String> {
        use std::os::windows::fs::MetadataExt;
        for part in path.ancestors() {
            match fs::symlink_metadata(part) {
                Ok(m) if m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 => {
                    return Err("qa_link".into())
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("qa_path".into()),
            }
        }
        Ok(())
    }
    struct Pinned {
        file: fs::File,
        _parents: Vec<fs::File>,
        bytes: Vec<u8>,
        sha: &'static str,
    }
    impl Pinned {
        fn open(root: &Path, path: &str, size: u64, sha: &'static str) -> Result<Self, String> {
            let path = root.join(path);
            no_links(&path)?;
            let path = fs::canonicalize(path).map_err(|_| "qa_source_path")?;
            let parents = crate::streaming::browser_store::cleanup::pin_directories(
                path.parent().ok_or("qa_source_parent")?,
            )
            .map_err(|_| "qa_source_pin")?;
            let mut file = crate::streaming::browser_store::cleanup::verification_guard(&path)
                .map_err(|_| "qa_source_pin")?;
            if file.metadata().map_err(|_| "qa_source_stat")?.len() != size {
                return Err("qa_source_size".into());
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(size + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "qa_source_read")?;
            if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(&bytes)) != sha {
                return Err("qa_source_sha".into());
            }
            Ok(Self {
                file,
                _parents: parents,
                bytes,
                sha,
            })
        }
        fn unchanged(&mut self) -> Result<(), String> {
            self.file
                .seek(SeekFrom::Start(0))
                .map_err(|_| "qa_source_seek")?;
            let mut after = Vec::new();
            (&mut self.file)
                .take(self.bytes.len() as u64 + 1)
                .read_to_end(&mut after)
                .map_err(|_| "qa_source_read")?;
            if after != self.bytes || format!("{:x}", Sha256::digest(&after)) != self.sha {
                return Err("qa_source_changed".into());
            }
            Ok(())
        }
    }
    fn prefix(bytes: &[u8]) -> Result<usize, String> {
        let mut at = 0usize;
        for kind in [b"ftyp", b"moov"] {
            let header = bytes.get(at..at + 8).ok_or("qa_source_atoms")?;
            let size =
                u32::from_be_bytes(header[..4].try_into().map_err(|_| "qa_source_atoms")?) as usize;
            if size < 8
                || header[4..8] != kind[..]
                || at.checked_add(size).is_none_or(|end| end > bytes.len())
            {
                return Err("qa_source_atoms".into());
            }
            at += size;
        }
        Ok(at)
    }
    fn begin(lease: &OfficialReceiverLease, bytes: &[u8]) -> Result<EncodedArchive, String> {
        let split = prefix(bytes)?;
        let source = uuid::Uuid::new_v4().to_string();
        let mut archive = lease.begin_encoded(
            source.clone(),
            1,
            uuid::Uuid::new_v4().to_string(),
            vec![EncodedTrack {
                track_index: 0,
                mime_type: "video/mp4;codecs=avc1.640028,mp4a.40.2".into(),
                init: bytes[..split].to_vec(),
            }],
            "Controlled asset drain".into(),
            "Local fixture".into(),
            live::RecordingOwner::Manual,
        )?;
        let body = &bytes[split..];
        let count = body.len().div_ceil(128 * 1024);
        for (chunk, data) in body.chunks(128 * 1024).enumerate() {
            archive.append(EncodedAppend {
                source_id: source.clone(),
                generation: 1,
                track_index: 0,
                index: 0,
                chunk: chunk as u32,
                final_chunk: chunk + 1 == count,
                bytes: data.to_vec(),
            })?;
        }
        Ok(archive)
    }
    fn write(path: &Path, value: &Value) -> Result<(), String> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "qa_output")?;
        serde_json::to_writer_pretty(&mut f, value).map_err(|_| "qa_output")?;
        f.sync_all().map_err(|_| "qa_output".into())
    }
    fn quit(app: &AppHandle) -> Result<(), String> {
        let state = app.try_state::<AppState>().ok_or("qa_state_missing")?;
        let snapshot = state
            .active_work_snapshot()
            .map_err(|_| "qa_work_snapshot")?;
        let response = state
            .request_graceful_quit(
                app.clone(),
                AppQuitRequest {
                    expected_work_set_fingerprint: snapshot.work_set_fingerprint,
                    confirm_active_work: true,
                    force_when_status_unknown: false,
                },
            )
            .map_err(|_| "qa_quit_request")?;
        if !response.accepted {
            return Err("qa_quit_rejected".into());
        }
        Ok(())
    }
    fn exercise(
        app: &AppHandle,
        root: &Path,
        mp4: &[u8],
        png: &[u8],
        returned: &AtomicBool,
        service_out: &Mutex<Option<PlatformMediaService>>,
        report: &mut Value,
    ) -> Result<(), String> {
        let startup = app.state::<Arc<crate::startup::Startup>>();
        crate::initialize_backend(app, &startup).map_err(|_| "qa_backend")?;
        let state = app.state::<AppState>();
        let media = state.platform_media(app)?;
        *service_out.lock().map_err(|_| "qa_service_slot")? = Some(media.clone());
        if !media.jobs(None).is_empty() {
            return Err("qa_profile_not_empty".into());
        }
        let (chat, gate) = delayed_chat(png.to_vec());
        let _release = ReleaseOnDrop(gate.clone());
        let lease = lease_with_chat(&media, &root.join("media"), chat.clone())?;
        for phase in 1..=2 {
            let archive = begin(&lease, mp4)?;
            let job = archive.snapshot()?;
            let directory = archive
                .writer
                .as_ref()
                .ok_or("qa_writer")?
                .directory
                .clone();
            let cancel = archive.writer.as_ref().ok_or("qa_writer")?.cancel.clone();
            let asset = image_event(&chat)?;
            until(|| gate.entered.load(Ordering::Acquire) == phase, 3)?;
            archive.finish(None)?;
            if !owned(&media, &job.job_id) || directory.join("chat-assets/status.json").exists() {
                return Err("qa_pending_owner_missing".into());
            }
            let entry = json!({"jobId":job.job_id,"directory":directory,"assetId":asset,"pendingBeforeRelease":true,"ownerBeforeRelease":true});
            report[if phase == 1 {
                "normalStop"
            } else {
                "normalQuit"
            }] = entry;
            if phase == 1 {
                gate.release(phase);
                let completed = settled(&media, &job.job_id)?;
                if completed.state != "completed" {
                    return Err("qa_media_not_completed".into());
                }
                let facts = asset_facts(&directory, &asset, png)?;
                let playback = media.open(&job.job_id)?;
                media.close(&playback.token);
                let read = media.replay_chat_asset(&job.job_id, &asset)?;
                use base64::{engine::general_purpose::STANDARD, Engine};
                if read.state != "ready"
                    || read.data_url.as_deref()
                        != Some(format!("data:image/png;base64,{}", STANDARD.encode(png)).as_str())
                {
                    return Err("qa_local_replay_asset".into());
                }
                report["normalStop"]["asset"] = facts;
                report["normalStop"]["mediaCompleted"] = json!(true);
                report["normalStop"]["localPlaybackOpened"] = json!(true);
                report["normalStop"]["localReplayBytesEqual"] = json!(true);
            } else {
                if chat.test_stop_requested() || lease.is_closed() {
                    return Err("qa_source_stopped_before_quit".into());
                }
                quit(app)?;
                until(|| state.is_quitting(), 2)?;
                until(|| chat.test_stop_requested(), 3)?;
                thread::sleep(Duration::from_millis(200));
                if returned.load(Ordering::Acquire)
                    || !owned(&media, &job.job_id)
                    || cancel.load(Ordering::Acquire)
                    || lease.is_closed()
                    || directory.join("chat-assets/status.json").exists()
                {
                    return Err("qa_quit_did_not_hold_pending_target".into());
                }
                report["normalQuit"]["pendingDuringAcceptedQuit"] = json!(true);
                report["normalQuit"]["preShutdownDrainObserved"] = json!(true);
                report["normalQuit"]["cancelBeforeRelease"] = json!(false);
                gate.release(phase);
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "Root-only controlled image delay through actual local finalizer and normal AppState exit; no public-site activity"]
    fn controlled_pending_image_survives_recording_finish_and_actual_app_quit() {
        assert_eq!(
            std::env::var("ATSUMI_ARCHIVE_ASSET_DRAIN_QA").as_deref(),
            Ok("owned-fixtures-normal-exit"),
            "QA_OPT_IN"
        );
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut png = Pinned::open(workspace, PNG, 875, PNG_SHA).unwrap();
        let mut mp4 = Pinned::open(workspace, MP4, 1_789_078, MP4_SHA).unwrap();
        let run = uuid::Uuid::new_v4().simple().to_string();
        let root = workspace
            .join(".runtime/archive-asset-drain-native-qa")
            .join(&run);
        no_links(&root).unwrap();
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("media")).unwrap();
        let identity = format!("local.atsumi.assetdrainqa.{run}");
        let profile = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(&identity);
        no_links(&profile).unwrap();
        assert!(!profile.exists(), "QA_PROFILE_EXISTS");
        let result = Arc::new(Mutex::new(
            json!({"passed":false,"classification":"controlled-native-lifecycle-fixture","publicSourceCalls":0,"newOfficialEvents":0,"existingProfilesUsed":false}),
        ));
        let returned = Arc::new(AtomicBool::new(false));
        let output = result.clone();
        let done = returned.clone();
        let work = root.clone();
        let input = mp4.bytes.clone();
        let image = png.bytes.clone();
        let worker = Arc::new(Mutex::new(None));
        let worker_slot = worker.clone();
        let service_out = Arc::new(Mutex::new(None));
        let worker_service = service_out.clone();
        let mut context = tauri::generate_context!();
        context.config_mut().identifier = identity;
        context.config_mut().app.windows.clear();
        context.config_mut().app.tray_icon = None;
        let app = tauri::Builder::default()
            .any_thread()
            .manage(crate::soop::original::SoopOriginalHost::default())
            .manage(crate::youtube::original::YouTubeOriginalHost::default())
            .manage(Arc::new(crate::startup::Startup::default()))
            .manage(Arc::new(
                crate::ui_download_events::DownloadEvents::default(),
            ))
            .setup(move |app| {
                if app.path().app_data_dir()? != profile {
                    return Err("QA_PROFILE_PATH".into());
                }
                no_links(&profile)?;
                tauri::window::WindowBuilder::new(app, "main")
                    .title("Atsumi controlled asset drain verification")
                    .position(-16000., -16000.)
                    .focused(false)
                    .skip_taskbar(true)
                    .build()?;
                let handle = app.handle().clone();
                let thread = thread::spawn(move || {
                    let mut report = output.lock().unwrap().clone();
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        exercise(
                            &handle,
                            &work,
                            &input,
                            &image,
                            &done,
                            &worker_service,
                            &mut report,
                        )
                    }))
                    .unwrap_or_else(|_| Err("qa_worker_panic".into()));
                    report["passed"] = json!(outcome.is_ok());
                    report["error"] = json!(outcome.err());
                    *output.lock().unwrap() = report;
                    if !handle
                        .try_state::<AppState>()
                        .is_some_and(|s| s.is_quitting())
                        && quit(&handle).is_err()
                    {
                        let _ = write(&work.join("pending-result.json"), &output.lock().unwrap());
                        handle.exit(2);
                    }
                });
                *worker_slot.lock().unwrap() = Some(thread);
                Ok(())
            })
            .build(context)
            .expect("QA_BUILD");
        app.run_return(|_, _| {});
        returned.store(true, Ordering::Release);
        worker.lock().unwrap().take().unwrap().join().unwrap();
        let mut report = result.lock().unwrap().clone();
        let checked = (|| -> Result<(), String> {
            png.unchanged()?;
            mp4.unchanged()?;
            if report["passed"] != true || report["normalQuit"]["pendingDuringAcceptedQuit"] != true
            {
                return Err("qa_exercise_failed".into());
            }
            let directory = PathBuf::from(
                report["normalQuit"]["directory"]
                    .as_str()
                    .ok_or("qa_directory")?,
            );
            let asset = report["normalQuit"]["assetId"].as_str().ok_or("qa_asset")?;
            let facts = asset_facts(&directory, asset, &png.bytes)?;
            let id = report["normalQuit"]["jobId"].as_str().ok_or("qa_job_id")?;
            let service = service_out
                .lock()
                .map_err(|_| "qa_service_slot")?
                .clone()
                .ok_or("qa_service_missing")?;
            let job = service
                .jobs(Some("soop"))
                .into_iter()
                .find(|job| job.job_id == id)
                .ok_or("qa_final_job")?;
            if job.active() || owned(&service, id) {
                return Err("qa_final_owner_pending".into());
            }
            validate_proof(&job, &directory)?;
            let summary = crate::platform_media::chat_status::inspect(&directory, "soop", id)?
                .ok_or("qa_final_chat_proof")?;
            if summary.assets_incomplete {
                return Err("qa_final_chat_assets_incomplete".into());
            }
            report["normalQuit"]["asset"] = facts;
            report["normalQuit"]["chatProof"] =
                serde_json::to_value(summary).map_err(|_| "qa_final_chat_proof")?;
            report["normalQuit"]["mediaFinalState"] = json!(job.state);
            report["normalQuit"]["normalAppExitReturned"] = json!(true);
            report["originalFixtureHashesUnchanged"] = json!(true);
            Ok(())
        })();
        if let Err(error) = checked {
            report["passed"] = json!(false);
            report["finalError"] = json!(error)
        }
        report["qualifications"]=json!(["Controlled fixture is not a newly observed official image event or visual equivalence proof.","Original inputs retained; only new QA output records were written.","Normal quit does not imply the interrupted media job was published as completed.","Local playback open proves admission only; no native frames, scene appearance or audible playback is verified by this batch."]);
        write(&root.join("result.json"), &report).unwrap();
        println!(
            "QA_ARCHIVE_ASSET_DRAIN_RESULT={}",
            root.join("result.json").display()
        );
        assert_eq!(
            report["passed"], true,
            "Controlled lifecycle verification failed; retained result describes its boundary"
        );
    }
}
