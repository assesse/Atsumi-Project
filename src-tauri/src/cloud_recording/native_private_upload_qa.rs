//! Explicit, real-network QA of the same native service/planner used by AppState.
//! Never part of production or ordinary test runs; this is not app UI evidence.
use super::super::{source, CloudError, CloudService, CloudSnapshot, CloudSourceRef};
use crate::{
    platform_media::{storage, MediaJob, PlatformMediaService, PlatformMediaTools},
    streaming::startup::DeferredBrowser,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const IDENTITY: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const SOURCES: [(&str, &str); 2] = [
    ("youtube", "38965dca-b875-4758-8b84-642507653bd1"),
    ("soop", "9011ac2c-8063-4e1a-a85a-3bc449879c8f"),
];
const AUTO_RECORDING: &str = "241bc41d-d3e6-4bf9-a205-899f2c2d21d5";
type Checked<T> = Result<T, String>;

fn code(error: CloudError) -> String {
    if error
        .code
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && error.code.len() <= 64
    {
        error.code
    } else {
        "QA_CLOUD_ERROR".into()
    }
}
fn hash(path: &Path) -> Checked<String> {
    storage::regular(path).map_err(|_| "QA_COPY_PATH")?;
    let mut input = fs::File::open(path).map_err(|_| "QA_COPY_READ")?;
    let mut digest = Sha256::new();
    let mut block = [0; 65536];
    loop {
        let read = input.read(&mut block).map_err(|_| "QA_COPY_READ")?;
        if read == 0 {
            break;
        }
        digest.update(&block[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn allowed(reference: &CloudSourceRef) -> bool {
    SOURCES
        .iter()
        .any(|(provider, id)| reference.provider == *provider && reference.recording_id == *id)
}
fn json_file(path: &Path) -> Checked<Value> {
    if storage::regular(path).map_err(|_| "QA_FILE_PATH")?.len() > 1024 * 1024 {
        return Err("QA_FILE_SIZE".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| "QA_FILE_READ")?)
        .map_err(|_| "QA_FILE_JSON".into())
}

// Check only the already copied files. The separate original-preservation Verify
// script is run by the supervising session before/after this explicit test.
fn verify_copies(qa: &Path, data: &Path) -> Checked<()> {
    verify_copy_set(qa, data, false)
}
fn verify_copy_set(qa: &Path, data: &Path, include_automatic: bool) -> Checked<()> {
    storage::safe_path(qa).map_err(|_| "QA_ROOT_PATH")?;
    storage::safe_path(data).map_err(|_| "QA_DATA_PATH")?;
    let receipt = json_file(&qa.join("cloud-real-copy-receipt.json"))?;
    if receipt["version"] != 2
        || receipt["mode"] != "Seed"
        || receipt["identity"] != IDENTITY
        || receipt["originalFilesUnchanged"] != true
        || receipt["mediaAndSidecarsByteIdentical"] != true
        || receipt["catalogMetadataMatchesExpected"] != true
        || receipt["sourceManifestSha256"]
            != "8DE52405AEB53566A5316244711CEE81ABB67FDA64A8E93112B745612769E8A8"
    {
        return Err("QA_COPY_RECEIPT".into());
    }
    let mut sources = receipt["sources"]
        .as_array()
        .filter(|s| s.len() == SOURCES.len())
        .ok_or("QA_COPY_COUNT")?
        .clone();
    let mut allowed_sources = SOURCES.to_vec();
    if include_automatic {
        let receipt = json_file(&qa.join("cloud-auto-copy-receipt.json"))?;
        if receipt["version"] != 2
            || receipt["mode"] != "Seed"
            || receipt["identity"] != IDENTITY
            || receipt["originalFilesUnchanged"] != true
            || receipt["mediaAndSidecarsByteIdentical"] != true
            || receipt["catalogMetadataMatchesExpected"] != true
        {
            return Err("QA_AUTO_COPY_RECEIPT".into());
        }
        let extra = receipt["sources"]
            .as_array()
            .filter(|s| s.len() == 1)
            .ok_or("QA_AUTO_COPY_COUNT")?;
        if extra[0]["provider"] != "soop" || extra[0]["recordingId"] != AUTO_RECORDING {
            return Err("QA_AUTO_COPY_SOURCE".into());
        }
        sources.extend(extra.iter().cloned());
        allowed_sources.push(("soop", AUTO_RECORDING));
    }
    let media = qa.join("media");
    let journal = data.join("platform-media/jobs");
    let entries = fs::read_dir(&journal)
        .map_err(|_| "QA_JOURNAL")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "QA_JOURNAL")?;
    if entries.len() != allowed_sources.len() {
        return Err("QA_UNEXPECTED_JOBS".into());
    }
    for (provider, id) in allowed_sources {
        let saved = sources
            .iter()
            .find(|s| s["provider"] == provider && s["recordingId"] == id)
            .ok_or("QA_COPY_SOURCE")?;
        let catalog = journal.join(format!("{id}.json"));
        if !hash(&catalog)?.eq_ignore_ascii_case(
            saved["copiedCatalogSha256"]
                .as_str()
                .ok_or("QA_COPY_CATALOG_HASH")?,
        ) {
            return Err("QA_COPY_CATALOG_CHANGED".into());
        }
        let job: MediaJob =
            serde_json::from_value(json_file(&catalog)?).map_err(|_| "QA_CATALOG_SCHEMA")?;
        if job.job_id != id
            || job.reference.provider != provider
            || job.state != "completed"
            || job.download_root != media
            || job.library.as_ref().is_none_or(|item| {
                item.library_id != id || item.duration <= 0.0 || item.duration > 45.0
            })
        {
            return Err("QA_CATALOG_BOUNDARY".into());
        }
        let directory = media
            .join(if provider == "youtube" {
                "YouTube"
            } else {
                "SOOP"
            })
            .join(id);
        let files = saved["files"]
            .as_array()
            .filter(|f| !f.is_empty())
            .ok_or("QA_COPY_FILES")?;
        for item in files {
            let relative = Path::new(item["relative"].as_str().ok_or("QA_COPY_RELATIVE")?);
            if relative.as_os_str().is_empty()
                || !relative
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
            {
                return Err("QA_COPY_RELATIVE".into());
            }
            let path = directory.join(relative);
            let info = storage::regular(&path).map_err(|_| "QA_COPY_PATH")?;
            if Some(info.len()) != item["bytes"].as_u64()
                || !hash(&path)?
                    .eq_ignore_ascii_case(item["copySha256"].as_str().ok_or("QA_COPY_HASH")?)
            {
                return Err("QA_COPY_CHANGED".into());
            }
        }
    }
    Ok(())
}

struct Session {
    cloud: Arc<CloudService>,
    platform: PlatformMediaService,
}
impl Drop for Session {
    fn drop(&mut self) {
        let drained = self.cloud.shutdown_and_wait().is_ok();
        self.platform.shutdown();
        println!(
            "{}",
            json!({"stage":"native-private-upload-drain","drained":drained,"appUiTested":false,"replayTested":false})
        );
    }
}
fn report(snapshot: &CloudSnapshot) -> Value {
    json!({"stage":"native-private-upload", "connected":snapshot.account.connected,
        "jobs":snapshot.jobs.iter().map(|job|json!({"provider":job.source.provider,"recordingId":job.source.recording_id,
            "jobId":job.id,"state":job.state,"recordingPlan":job.recording_plan,"uploadedBytes":job.uploaded_bytes,
            "totalBytes":job.total_bytes,"durationSeconds":job.duration_seconds,"videoId":job.video_id,"visibility":job.visibility,
            "errorCode":job.error.clone().map(code)})).collect::<Vec<_>>(),"appUiTested":false,"replayTested":false})
}

#[test]
#[ignore = "Real private Google uploads: fixed isolated copies, explicit ATSUMI_CLOUD_PRIVATE_UPLOAD_QA identity required"]
fn actual_private_upload_fixed_copies() -> Checked<()> {
    if std::env::var("ATSUMI_CLOUD_PRIVATE_UPLOAD_QA")
        .ok()
        .as_deref()
        != Some(IDENTITY)
    {
        return Err("QA_EXPLICIT_OPT_IN_REQUIRED".into());
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("QA_WORKSPACE")?;
    let qa = workspace.join(".runtime/platform-app-qa").join(IDENTITY);
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(IDENTITY);
    verify_copies(&qa, &data)?;
    let bin = workspace
        .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let tools = PlatformMediaTools {
        yt_dlp: workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"),
        deno: workspace.join(".runtime/platform-tools/bin/deno.exe"),
        ffmpeg: bin.join("ffmpeg.exe"),
        ffprobe: bin.join("ffprobe.exe"),
    };
    for path in [&tools.yt_dlp, &tools.deno, &tools.ffmpeg, &tools.ffprobe] {
        storage::regular(path).map_err(|_| "QA_TOOL_PATH")?;
    }
    let platform =
        PlatformMediaService::new(data.clone(), tools, None).map_err(|_| "QA_PLATFORM_START")?;
    let browser = Arc::new(DeferredBrowser::default());
    let resolve_platform = platform.clone();
    let resolve_browser = browser.clone();
    let resolve_root = data.clone();
    let plan_platform = platform.clone();
    let plan_browser = browser.clone();
    let plan_root = data.clone();
    let cloud = match CloudService::new_with_planner(
        data,
        Arc::new(move |reference, context| {
            if !allowed(reference) {
                return Err("QA_SOURCE_REFUSED".into());
            }
            source::resolve(
                &resolve_browser,
                &resolve_platform,
                &resolve_root,
                reference,
                context,
            )
        }),
        Arc::new(|_| Err("QA_REAUTH_NOT_ALLOWED".into())),
        Arc::new(move |reference, context| {
            if !allowed(reference) {
                return Err("QA_SOURCE_REFUSED".into());
            }
            source::prepare_plan(
                &plan_browser,
                &plan_platform,
                &plan_root,
                reference,
                context,
            )
        }),
    ) {
        Ok(cloud) => cloud,
        Err(error) => {
            platform.shutdown();
            return Err(code(error));
        }
    };
    let session = Session { cloud, platform };
    let outcome = (|| -> Checked<()> {
        let initial = session.cloud.snapshot().map_err(code)?;
        if !initial.account.connected
            || initial.account.authorizing
            || initial.jobs.iter().any(|job| !allowed(&job.source))
        {
            return Err("QA_ACCOUNT_OR_JOB_BOUNDARY".into());
        }
        for (provider, id) in SOURCES {
            // Never retry/resume/replace an existing job, even after a failed run.
            if !initial
                .jobs
                .iter()
                .any(|job| job.source.provider == provider && job.source.recording_id == id)
            {
                session
                    .cloud
                    .enqueue_recording(CloudSourceRef {
                        provider: provider.into(),
                        recording_id: id.into(),
                        part_index: None,
                        plan_id: None,
                        unit_id: None,
                    })
                    .map_err(code)?;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(600);
        let mut previous = String::new();
        loop {
            let snapshot = session.cloud.snapshot().map_err(code)?;
            let output = report(&snapshot).to_string();
            if output != previous {
                println!("{output}");
                let _ = std::io::stdout().flush();
                previous = output;
            }
            if snapshot.jobs.iter().any(|job| !allowed(&job.source)) {
                return Err("QA_UNEXPECTED_CLOUD_JOB".into());
            }
            if snapshot.jobs.iter().any(|job| {
                matches!(
                    job.state.as_str(),
                    "failed" | "paused" | "waiting_auth" | "retry_wait"
                )
            }) {
                return Err("QA_JOB_STOPPED_SEE_SANITIZED_STATE".into());
            }
            if snapshot.jobs.len() == SOURCES.len()
                && snapshot
                    .jobs
                    .iter()
                    .all(|job| job.state == "ready" && job.video_id.is_some())
            {
                let receipt = qa.join(format!(
                    "cloud-native-upload-result-{}.json",
                    uuid::Uuid::new_v4().simple()
                ));
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(receipt)
                    .map_err(|_| "QA_RESULT_WRITE")?;
                file.write_all(previous.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "QA_RESULT_WRITE")?;
                return Ok(());
            }
            if Instant::now() >= deadline || qa.join("cloud-native-upload.cancel").exists() {
                return Err("QA_DEADLINE_OR_CANCEL".into());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    })();
    // Explicit normal drain precedes copy revalidation; Drop also handles unwinds.
    let drained = session.cloud.shutdown_and_wait().map_err(code);
    session.platform.shutdown();
    drop(session);
    let copies = verify_copies(
        &qa,
        &PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(IDENTITY),
    );
    drained?;
    copies?;
    outcome
}

#[test]
#[ignore = "Real automatic private SOOP upload: fixed third QA copy and ATSUMI_CLOUD_AUTOMATIC_UPLOAD_QA identity required"]
fn actual_automatic_private_upload_fixed_soop_copy() -> Checked<()> {
    use super::super::CloudAutomaticTarget;
    if std::env::var("ATSUMI_CLOUD_AUTOMATIC_UPLOAD_QA")
        .ok()
        .as_deref()
        != Some(IDENTITY)
    {
        return Err("QA_EXPLICIT_OPT_IN_REQUIRED".into());
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("QA_WORKSPACE")?;
    let qa = workspace.join(".runtime/platform-app-qa").join(IDENTITY);
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(IDENTITY);
    verify_copy_set(&qa, &data, true)?;
    let bin = workspace
        .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let tools = PlatformMediaTools {
        yt_dlp: workspace.join(".runtime/platform-tools/bin/yt-dlp.exe"),
        deno: workspace.join(".runtime/platform-tools/bin/deno.exe"),
        ffmpeg: bin.join("ffmpeg.exe"),
        ffprobe: bin.join("ffprobe.exe"),
    };
    for path in [&tools.yt_dlp, &tools.deno, &tools.ffmpeg, &tools.ffprobe] {
        storage::regular(path).map_err(|_| "QA_TOOL_PATH")?;
    }
    let platform =
        PlatformMediaService::new(data.clone(), tools, None).map_err(|_| "QA_PLATFORM_START")?;
    let browser = Arc::new(DeferredBrowser::default());
    let source_platform = platform.clone();
    let source_browser = browser.clone();
    let source_root = data.clone();
    let plan_platform = platform.clone();
    let plan_browser = browser.clone();
    let plan_root = data.clone();
    let cloud = match CloudService::new_with_planner(
        data.clone(),
        Arc::new(move |reference, context| {
            if reference.provider != "soop" || reference.recording_id != AUTO_RECORDING {
                return Err("QA_SOURCE_REFUSED".into());
            }
            source::resolve(
                &source_browser,
                &source_platform,
                &source_root,
                reference,
                context,
            )
        }),
        Arc::new(|_| Err("QA_REAUTH_NOT_ALLOWED".into())),
        Arc::new(move |reference, context| {
            if reference.provider != "soop" || reference.recording_id != AUTO_RECORDING {
                return Err("QA_SOURCE_REFUSED".into());
            }
            source::prepare_plan(
                &plan_browser,
                &plan_platform,
                &plan_root,
                reference,
                context,
            )
        }),
    ) {
        Ok(cloud) => cloud,
        Err(error) => {
            platform.shutdown();
            return Err(code(error));
        }
    };
    let session = Session { cloud, platform };
    let outcome = (|| -> Checked<()> {
        let initial = session.cloud.snapshot().map_err(code)?;
        let target = CloudAutomaticTarget::Recording {
            provider: "soop".into(),
            recording_id: AUTO_RECORDING.into(),
        };
        if !initial.account.connected
            || initial.account.authorizing
            || initial.jobs.iter().any(|job| {
                if job.source.provider == "soop" && job.source.recording_id == AUTO_RECORDING {
                    false
                } else {
                    !allowed(&job.source) || job.state != "ready"
                }
            })
            || initial
                .automatic
                .rules
                .iter()
                .any(|rule| rule.target != target)
        {
            return Err("QA_AUTOMATIC_BOUNDARY".into());
        }
        let new_selection = initial.automatic.rules.is_empty();
        let new_admission = !initial
            .jobs
            .iter()
            .any(|job| job.source.provider == "soop" && job.source.recording_id == AUTO_RECORDING);
        let selected = session
            .cloud
            .automatic_set(target, true, initial.automatic.revision)
            .map_err(code)?;
        let rule_id = selected
            .automatic
            .rules
            .first()
            .ok_or("QA_AUTOMATIC_RULE")?
            .id
            .clone();
        let candidates = session.platform.clone();
        session
            .cloud
            .start_automatic(Arc::new(move |rules| {
                candidates.automatic_upload_candidates(rules)
            }))
            .map_err(code)?;
        let deadline = Instant::now() + Duration::from_secs(600);
        let mut previous = String::new();
        loop {
            let snapshot = session.cloud.snapshot().map_err(code)?;
            let auto_jobs = snapshot
                .jobs
                .iter()
                .filter(|job| {
                    job.source.provider == "soop" && job.source.recording_id == AUTO_RECORDING
                })
                .collect::<Vec<_>>();
            let origin = session
                .cloud
                .lock()
                .map_err(code)?
                .catalog
                .jobs
                .iter()
                .filter(|job| {
                    job.view.source.provider == "soop"
                        && job.view.source.recording_id == AUTO_RECORDING
                })
                .all(|job| {
                    job.automatic.as_ref().is_some_and(|origin| {
                        origin.rule_id == rule_id && origin.resume_intent && !origin.paused_by_user
                    })
                });
            let evidence = json!({"stage":"native-selected-soop-automatic-upload","newSelection":new_selection,"newAdmission":new_admission,
                "ruleStatus":snapshot.automatic.rules.first().map(|rule|rule.status.as_str()),"automaticOrigin":!auto_jobs.is_empty() && origin,
                "jobs":report(&snapshot)["jobs"],"automaticErrorCode":snapshot.automatic.error.clone().map(code),"appUiTested":false,"replayTested":false});
            let output = evidence.to_string();
            if output != previous {
                println!("{output}");
                let _ = std::io::stdout().flush();
                previous = output;
            }
            if auto_jobs.iter().any(|job| {
                matches!(
                    job.state.as_str(),
                    "failed" | "paused" | "waiting_auth" | "retry_wait"
                )
            }) {
                return Err("QA_AUTOMATIC_JOB_STOPPED".into());
            }
            if auto_jobs.len() == 1
                && auto_jobs[0].state == "ready"
                && auto_jobs[0].video_id.is_some()
                && origin
            {
                let path = qa.join(format!(
                    "cloud-native-automatic-result-{}.json",
                    uuid::Uuid::new_v4().simple()
                ));
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|_| "QA_RESULT_WRITE")?;
                file.write_all(previous.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "QA_RESULT_WRITE")?;
                return Ok(());
            }
            if Instant::now() >= deadline || qa.join("cloud-native-upload.cancel").exists() {
                return Err("QA_DEADLINE_OR_CANCEL".into());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    })();
    let drained = session.cloud.shutdown_and_wait().map_err(code);
    session.platform.shutdown();
    drop(session);
    let copies = verify_copy_set(&qa, &data, true);
    drained?;
    copies?;
    outcome
}

/// The supervising session changes only these three QA videos in Studio first.
/// This test performs metadata GETs, then updates local labels atomically. It
/// cannot change a remote video, upload media or open an authorization browser.
#[test]
#[ignore = "Read-only Google verification after explicit Studio changes; exact ATSUMI_CLOUD_UNLISTED_VERIFY_QA identity required"]
fn actual_verify_unlisted_fixed_three_receipts() -> Checked<()> {
    use super::super::{upload, CloudVisibility};
    const EXPECTED: [(&str, &str, &str, &str); 3] = [
        ("ecc3cba2-2897-42da-8c7d-1bbffc869a3f", "8Zfc8TY0gvc", "youtube", "38965dca-b875-4758-8b84-642507653bd1"),
        ("dbb2c18c-e721-4556-9ecd-1608e997c2db", "jV0AtXnc1X8", "soop", "9011ac2c-8063-4e1a-a85a-3bc449879c8f"),
        ("e0d2937d-74bb-48c7-97fe-c558b66bfb32", "LqlldjSGLgc", "soop", AUTO_RECORDING),
    ];
    if std::env::var("ATSUMI_CLOUD_UNLISTED_VERIFY_QA").ok().as_deref() != Some(IDENTITY) {
        return Err("QA_EXPLICIT_OPT_IN_REQUIRED".into());
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let qa = workspace.join(".runtime/platform-app-qa").join(IDENTITY);
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(IDENTITY);
    verify_copy_set(&qa, &data, true)?;
    let cloud = CloudService::new(
        data.clone(),
        Arc::new(|_, _| Err("QA_SOURCE_ACCESS_NOT_ALLOWED".into())),
        Arc::new(|_| Err("QA_REAUTH_NOT_ALLOWED".into())),
    ).map_err(code)?;
    let outcome = (|| -> Checked<()> {
        let jobs = cloud.lock().map_err(code)?.catalog.jobs.clone();
        if jobs.len() != EXPECTED.len() || !cloud.snapshot().map_err(code)?.account.connected {
            return Err("QA_RECEIPT_COUNT_OR_ACCOUNT".into());
        }
        for (job_id, video, provider, recording) in EXPECTED {
            if !jobs.iter().any(|job| job.view.id == job_id
                && job.view.video_id.as_deref() == Some(video)
                && job.view.source.provider == provider
                && job.view.source.recording_id == recording
                && job.view.state == "ready"
                && job.has_upload_receipt()) {
                return Err("QA_FIXED_RECEIPT_MISMATCH".into());
            }
        }
        let deadline = Instant::now() + Duration::from_secs(180);
        for job in &jobs {
            if Instant::now() >= deadline {
                return Err("QA_METADATA_DEADLINE".into());
            }
            let ready = cloud.authorized(&job.channel_id, |token| upload::verify_remote(
                cloud.transport.as_ref(), token,
                job.view.video_id.as_deref().ok_or_else(CloudError::protocol)?,
                &job.channel_id, job.view.duration_seconds, CloudVisibility::Unlisted,
            )).map_err(code)?;
            if !ready {
                return Err("QA_METADATA_NOT_READY".into());
            }
        }
        let mut inner = cloud.lock().map_err(code)?;
        let mut catalog = inner.catalog.clone();
        for job in &mut catalog.jobs {
            job.view.visibility = CloudVisibility::Unlisted;
        }
        cloud.publish(&mut inner, catalog).map_err(code)?;
        drop(inner);
        let result = json!({"stage":"native-unlisted-metadata-verify", "identity":IDENTITY,
            "allThreeOwnerDurationProcessingAndUnlistedVerified":true,
            "remoteMutationPerformed":false,"mediaUploadPerformed":false,
            "jobs":cloud.snapshot().map_err(code)?.jobs.iter().map(|job| json!({
                "jobId":job.id,"videoId":job.video_id,"visibility":job.visibility,"state":job.state
            })).collect::<Vec<_>>(),"appUiTested":false,"replayTested":false});
        let encoded = serde_json::to_vec_pretty(&result).map_err(|_| "QA_RESULT_JSON")?;
        let mut file = fs::OpenOptions::new().write(true).create_new(true)
            .open(qa.join(format!("cloud-unlisted-verified-{}.json",uuid::Uuid::new_v4().simple())))
            .map_err(|_| "QA_RESULT_WRITE")?;
        file.write_all(&encoded).and_then(|_| file.sync_all()).map_err(|_| "QA_RESULT_WRITE")?;
        println!("{}", result);
        Ok(())
    })();
    let drained = cloud.shutdown_and_wait().map_err(code);
    drop(cloud);
    println!("{}", json!({"stage":"native-unlisted-metadata-drain","drained":drained.is_ok()}));
    drained?;
    verify_copy_set(&qa, &data, true)?;
    outcome
}
