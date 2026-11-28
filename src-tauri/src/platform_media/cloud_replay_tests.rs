//! Synthetic capability tests. No credentials, network, real media deletion or UI.
use super::*;
use crate::cloud_recording::{replay::CloudReplayInput, CloudSourceRef};
use serde_json::json;
use sha2::{Digest, Sha256};

struct Fixture {
    service: PlatformMediaService,
    authorization: CloudReplayAuthorization,
    directory: PathBuf,
    asset_id: String,
    image: String,
    _root: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.shutdown();
    }
}
fn authorization(job: &MediaJob) -> CloudReplayAuthorization {
    let item = job.library.as_ref().unwrap();
    let source = CloudSourceRef {
        provider: job.reference.provider.clone(),
        recording_id: job.job_id.clone(),
        part_index: None,
        plan_id: None,
        unit_id: None,
    };
    CloudReplayAuthorization {
        job_id: uuid::Uuid::new_v4().to_string(), source: source.clone(), owner_channel_id: "UCfixture".into(), cohort_id: "a".repeat(64),
        source_sha256: item.sha256.clone(), bytes: item.bytes, duration_seconds: item.duration, media_offset_seconds: 0.0,
        original_inputs: vec![CloudReplayInput { index: 0, part_index: None, sha256: item.sha256.clone(), bytes: item.bytes,
            duration_seconds: item.duration, media_offset_seconds: 0.0, modified_nanos: None }], plan_fingerprint: None, input_parts: false,
        description: serde_json::from_value(json!({"source":source,"title":"fixture","coverage":{"state":"complete"},
            "preparationRequired":false,"durationSeconds":item.duration,"bytes":item.bytes,"parts":[]})).unwrap(),
    }
}
fn service(root: &Path) -> PlatformMediaService {
    let fake = root.join("never-executed");
    PlatformMediaService::new(
        root.join("data"),
        PlatformMediaTools {
            yt_dlp: fake.clone(),
            deno: fake.clone(),
            ffmpeg: fake.clone(),
            ffprobe: fake,
        },
        None,
    )
    .unwrap()
}
fn fixture(provider: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let service = service(root.path());
    let id = uuid::Uuid::new_v4().to_string();
    let reference = MediaRef {
        provider: provider.into(),
        kind: "live".into(),
        external_id: if provider == "soop" {
            "fixture"
        } else {
            "abcdefghijk"
        }
        .into(),
    };
    let job: MediaJob = serde_json::from_value(json!({"jobId":id,"requestId":uuid::Uuid::new_v4().to_string(),"reference":reference,
        "revision":1,"state":"completed","title":"fixture","channelName":"fixture","createdAt":1,"receivedBytes":9,
        "downloadRoot":root.path().join("media"),"library":{"libraryId":id,"revision":1,"reference":reference,"title":"fixture",
        "channelName":"fixture","bytes":9,"duration":10.0,"sha256":"b".repeat(64),"completedAt":1}})).unwrap();
    let directory = storage::job_dir(&job.download_root, provider, &id).unwrap();
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("chat-timeline.json"),
        json!({"version":1,"provider":provider,"timelineQuality":"receive-time-approximate",
        "firstSegmentCommittedAtMs":1700000001000u64,"firstSegmentDurationMs":1000})
        .to_string(),
    )
    .unwrap();
    let rows = [(1000,"chat"),(2500,"donation"),(8000,"chat")].map(|(offset,kind)| {
        let mut row = json!({"format":"atsumi-chat-v1","receivedAtMs":1700000000000u64+offset,"sourceTimestampMs":1700000000000u64+offset,
            "messageId":format!("fixture-{offset}"),"kind":kind,"author":"fixture","text":if kind=="donation" {""} else {"public fixture"},"badges":[],"color":null});
        if kind=="donation" && provider=="soop" { row["soopGift"]=json!({"kind":"balloon","count":50,"relay":false,"isDefault":true}); }
        row.to_string()+"\n"
    }).concat();
    fs::write(directory.join("chat.events.jsonl"), rows).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let image = format!("data:image/png;base64,{}", STANDARD.encode(&png));
    profile::save(
        &directory,
        &profile::Profile::from_data_url(&image).unwrap(),
    )
    .unwrap();
    let asset_id = "c".repeat(64);
    fs::create_dir(directory.join("chat-assets")).unwrap();
    fs::write(directory.join("chat-assets").join(&asset_id), &png).unwrap();
    fs::write(directory.join("chat-assets/manifest.jsonl"),json!({"version":1,"id":asset_id,"sha256":format!("{:x}",Sha256::digest(&png)),"bytes":png.len(),"mime":"image/png"}).to_string()+"\n").unwrap();
    let authorization = authorization(&job);
    service.inner.state.lock().unwrap().jobs.insert(id, job);
    Fixture {
        service,
        authorization,
        directory,
        asset_id,
        image,
        _root: root,
    }
}
fn page(f: &Fixture, token: &str, position: u64) -> replay_chat::ReplayChatPage {
    for _ in 0..32 {
        let page = f
            .service
            .cloud_chat_page(token, position, None, 120)
            .unwrap();
        if page.state != "indexing" {
            return page;
        }
    }
    panic!("bounded fixture preparation did not finish");
}

#[test]
fn capture_warning_preserves_owned_sidecar_replay_without_opening_video() {
    for provider in ["soop", "youtube"] {
        let f = fixture(provider);
        let before = fs::read(f.directory.join("chat.events.jsonl")).unwrap();
        let outcome = json!({"state":"interrupted","reasons":["source_interrupted"],"excludedUnits":[0,0],"excludedBytes":[0,0]});
        f.service.inner.state.lock().unwrap().jobs.get_mut(&f.authorization.source.recording_id)
            .unwrap().library.as_mut().unwrap().capture_outcome = Some(serde_json::from_value(outcome).unwrap());
        assert!(!f.directory.join("media.mp4").exists());
        assert_eq!(f.service.cloud_capture_warning(&f.authorization).unwrap().as_deref(),
            Some("녹화가 정상적으로 마무리되지 않았습니다. 저장된 구간만 보관되어 있습니다."));
        f.service.validate_cloud_chat_source(&f.authorization).unwrap();
        let token = f.service.open_cloud_chat(&f.authorization).unwrap();
        assert_eq!(page(&f, &token, 10000).items.len(), 3);
        assert_eq!(page(&f, &token, 1500).items.len(), 1);
        assert_eq!(page(&f, &token, 10000).items.len(), 3);
        f.service.close_cloud_chat(&token);
        assert!(!f.directory.join("media.mp4").exists());
        assert_eq!(fs::read(f.directory.join("chat.events.jsonl")).unwrap(), before);
    }
}

#[test]
fn capture_warning_keeps_missing_complete_partial_and_identity_failures_distinct() {
    let f = fixture("soop");
    assert_eq!(f.service.cloud_capture_warning(&f.authorization).unwrap(), None);
    for (state, reasons, expected) in [
        ("complete", vec![], None),
        ("partial", vec!["audio_coverage_tail"], Some("일부 수신 구간이 녹화에 포함되지 않았습니다. 저장된 구간만 보관되어 있습니다.")),
        ("interrupted", vec!["source_interrupted"], Some("녹화가 정상적으로 마무리되지 않았습니다. 저장된 구간만 보관되어 있습니다.")),
    ] {
        let value = json!({"state":state,"reasons":reasons,"excludedUnits":[0,0],"excludedBytes":[0,0]});
        f.service.inner.state.lock().unwrap().jobs.get_mut(&f.authorization.source.recording_id)
            .unwrap().library.as_mut().unwrap().capture_outcome = Some(serde_json::from_value(value).unwrap());
        assert_eq!(f.service.cloud_capture_warning(&f.authorization).unwrap().as_deref(), expected);
    }
    for case in 0..4 {
        let mut foreign = f.authorization.clone();
        match case {
            0 => foreign.source.provider = "youtube".into(),
            1 => foreign.source.recording_id = uuid::Uuid::new_v4().to_string(),
            2 => foreign.original_inputs[0].sha256 = "c".repeat(64),
            _ => foreign.original_inputs[0].bytes += 1,
        }
        assert!(f.service.cloud_capture_warning(&foreign).is_err());
        assert!(f.service.open_cloud_chat(&foreign).is_err());
    }
    f.service.inner.state.lock().unwrap().jobs.get_mut(&f.authorization.source.recording_id).unwrap().state = "recording".into();
    assert!(f.service.cloud_capture_warning(&f.authorization).is_err());
    assert!(f.service.open_cloud_chat(&f.authorization).is_err());
}

#[test]
fn interrupted_capture_warning_survives_direct_upload_plan_without_changing_file_completeness() {
    use crate::{cloud_recording::{source, SourceResolveContext}, streaming::startup::DeferredBrowser};
    use std::io::Read;
    let f = fixture("soop");
    let media = b"synthetic";
    fs::write(f.directory.join("media.mp4"), media).unwrap();
    let before = {
        let mut state = f.service.inner.state.lock().unwrap();
        let job = state.jobs.get_mut(&f.authorization.source.recording_id).unwrap();
        let item = job.library.as_mut().unwrap();
        item.sha256 = format!("{:x}", Sha256::digest(media));
        item.capture_outcome = Some(serde_json::from_value(json!({"state":"interrupted","reasons":["source_interrupted"],"excludedUnits":[0,0],"excludedBytes":[0,0]})).unwrap());
        serde_json::to_vec(job).unwrap()
    };
    let data = f._root.path().join("cloud-plan-test");
    let browser = DeferredBrowser::default();
    let context = SourceResolveContext { cancel:Arc::new(AtomicBool::new(false)), progress:Arc::new(|_| {}) };
    let description = source::describe(&browser, &f.service, &data, &f.authorization.source).unwrap();
    assert_eq!(description.coverage.state, "complete");
    assert!(description.coverage.capture_warning.as_deref().is_some_and(|v| v.contains("정상적으로 마무리되지")));
    let planned = source::prepare_plan(&browser, &f.service, &data, &f.authorization.source, &context).unwrap();
    assert_eq!(planned.len(), 1);
    let saved = source::get_plan(&data, planned[0].plan_id.as_deref().unwrap()).unwrap();
    assert_eq!(saved.coverage.state, "complete");
    assert_eq!(saved.coverage.capture_warning, description.coverage.capture_warning);
    assert_eq!(saved.parts[0].kind, "direct");
    let mut upload = source::resolve(&browser, &f.service, &data, &planned[0], &context).unwrap();
    let mut bytes = Vec::new(); upload.file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, media);
    drop(upload);
    assert_eq!(fs::read(f.directory.join("media.mp4")).unwrap(), media);
    let state = f.service.inner.state.lock().unwrap();
    assert_eq!(serde_json::to_vec(state.jobs.get(&f.authorization.source.recording_id).unwrap()).unwrap(), before);
    assert!(!f.directory.join(".atsumi-cloud-upload").exists());
}
#[test]
fn cloud_sidecars_preserve_chat_gift_seek_profile_and_assets_without_video() {
    for provider in ["soop", "youtube"] {
        let f = fixture(provider);
        let before = fs::read(f.directory.join("chat.events.jsonl")).unwrap();
        assert!(!f.directory.join("media.mp4").exists());
        f.service
            .validate_cloud_chat_source(&f.authorization)
            .unwrap();
        assert!(f
            .service
            .open(&f.authorization.source.recording_id)
            .is_err());
        let token = f.service.open_cloud_chat(&f.authorization).unwrap();
        let all = page(&f, &token, 10000);
        assert_eq!(all.items.len(), 3);
        if provider == "soop" {
            assert!(all.items[1]
                .soop_gift
                .as_ref()
                .is_some_and(|gift| gift.count == 50));
        }
        assert_eq!(page(&f, &token, 1500).items.len(), 1);
        let again = page(&f, &token, 10000);
        assert!(
            serde_json::to_vec(&all.items).unwrap() == serde_json::to_vec(&again.items).unwrap(),
            "seek changed saved rows"
        );
        assert!(f.service.cloud_chat_profile(&token).unwrap().as_deref() == Some(f.image.as_str()));
        assert!(
            f.service
                .cloud_chat_asset(&token, &f.asset_id)
                .unwrap()
                .data_url
                .as_deref()
                == Some(f.image.as_str())
        );
        assert!(f.service.cloud_chat_asset(&token, "../secret").is_err());
        for method in ["GET", "HEAD"] {
            let request = tauri::http::Request::builder()
                .method(method)
                .uri(format!("http://platform-media.localhost/{token}/media.mp4"))
                .header("Origin", "http://tauri.localhost")
                .header("Range", "bytes=0-1")
                .body(Vec::new())
                .unwrap();
            assert!(!protocol::respond(&f.service, "main", request)
                .status()
                .is_success());
        }
        assert!(f
            .service
            .delete(&f.authorization.source.recording_id, 1)
            .is_err());
        f.service.close_cloud_chat(&token);
        assert!(f.service.cloud_chat_page(&token, 0, None, 1).is_err());
        assert!(f.service.cloud_chat_profile(&token).is_err());
        assert!(f.service.cloud_chat_asset(&token, &f.asset_id).is_err());
        assert!(before == fs::read(f.directory.join("chat.events.jsonl")).unwrap());
    }
}
#[test]
fn cloud_admission_rejects_wrong_original_identity_and_active_catalog() {
    let f = fixture("soop");
    for case in 0..9 {
        let mut bad = f.authorization.clone();
        match case {
            0 => bad.source.provider = "youtube".into(),
            1 => bad.original_inputs[0].sha256 = "d".repeat(64),
            2 => bad.original_inputs[0].bytes += 1,
            3 => bad.original_inputs[0].duration_seconds += 1.0,
            4 => bad.original_inputs[0].media_offset_seconds = 1.0,
            5 => bad.original_inputs[0].modified_nanos = Some(1_000_000),
            6 => bad.input_parts = true,
            7 => bad.original_inputs.push(bad.original_inputs[0].clone()),
            _ => bad.plan_fingerprint = Some("e".repeat(64)),
        }
        assert!(
            f.service.open_cloud_chat(&bad).is_err(),
            "invalid proof admitted"
        );
    }
    f.service
        .inner
        .state
        .lock()
        .unwrap()
        .jobs
        .get_mut(&f.authorization.source.recording_id)
        .unwrap()
        .state = "recording".into();
    assert!(f.service.open_cloud_chat(&f.authorization).is_err());
    assert!(f.service.inner.upload_leases.lock().unwrap().is_empty());
}
#[test]
fn cloud_admission_requires_owned_chat_and_live_clock() {
    for name in ["chat-timeline.json", "chat.events.jsonl"] {
        let f = fixture("soop");
        fs::remove_file(f.directory.join(name)).unwrap();
        assert!(f.service.open_cloud_chat(&f.authorization).is_err());
        assert!(f.service.inner.upload_leases.lock().unwrap().is_empty());
    }
    let f = fixture("soop");
    let chat = f.directory.join("chat.events.jsonl");
    fs::hard_link(&chat, f._root.path().join("alias")).unwrap();
    assert!(f.service.open_cloud_chat(&f.authorization).is_err());
}
#[test]
fn cloud_sidecar_file_and_catalog_replacement_are_fenced() {
    let f = fixture("soop");
    let token = f.service.open_cloud_chat(&f.authorization).unwrap();
    let result = fs::write(f.directory.join("chat.events.jsonl"), b"{}\n");
    #[cfg(windows)]
    assert!(result.is_err());
    #[cfg(not(windows))]
    {
        result.unwrap();
        assert!(f.service.cloud_chat_page(&token, 10000, None, 120).is_err());
    }
    f.service
        .inner
        .state
        .lock()
        .unwrap()
        .jobs
        .get_mut(&f.authorization.source.recording_id)
        .unwrap()
        .library
        .as_mut()
        .unwrap()
        .revision += 1;
    assert!(f.service.cloud_chat_page(&token, 10000, None, 120).is_err());
}
#[test]
fn cloud_siblings_keep_shared_index_alive_until_last_close() {
    let f = fixture("soop");
    let first = f.service.open_cloud_chat(&f.authorization).unwrap();
    let second = f.service.open_cloud_chat(&f.authorization).unwrap();
    assert_eq!(page(&f, &first, 10000).items.len(), 3);
    f.service.close_cloud_chat(&first);
    assert_eq!(page(&f, &second, 10000).items.len(), 3);
    assert!(f
        .service
        .delete(&f.authorization.source.recording_id, 1)
        .is_err());
    f.service.close_cloud_chat(&second);
    assert!(f.service.inner.upload_leases.lock().unwrap().is_empty());
    f.service
        .delete(&f.authorization.source.recording_id, 1)
        .unwrap();
}
#[test]
fn local_close_does_not_cancel_cloud_sibling() {
    let f = fixture("soop");
    fs::write(f.directory.join("media.mp4"), b"synthetic").unwrap();
    let local = f
        .service
        .open(&f.authorization.source.recording_id)
        .unwrap();
    let remote = f.service.open_cloud_chat(&f.authorization).unwrap();
    assert_eq!(page(&f, &remote, 10000).items.len(), 3);
    f.service.close(&local.token);
    assert_eq!(page(&f, &remote, 10000).items.len(), 3);
}
#[test]
fn cloud_close_fences_waiting_reads_and_shutdown_revokes_capabilities() {
    let f = fixture("soop");
    let token = f.service.open_cloud_chat(&f.authorization).unwrap();
    let session = f.service.cloud_session(&token).unwrap();
    let held = session.read.lock().unwrap();
    let closing = f.service.clone();
    let closing_token = token.clone();
    let worker = std::thread::spawn(move || closing.close_cloud_chat(&closing_token));
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while !session.closed.load(Ordering::Acquire) && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(session.closed.load(Ordering::Acquire));
    drop(held);
    worker.join().unwrap();
    drop(session);
    assert!(f.service.cloud_chat_page(&token, 1000, None, 120).is_err());
    let second = f.service.open_cloud_chat(&f.authorization).unwrap();
    f.service.shutdown();
    assert!(f.service.cloud_chat_page(&second, 1000, None, 120).is_err());
    assert!(f.service.open_cloud_chat(&f.authorization).is_err());
    assert!(f.service.inner.upload_leases.lock().unwrap().is_empty());
}
#[test]
fn unsafe_optional_raw_cannot_later_become_trusted_metadata() {
    let f = fixture("soop");
    let source = f.directory.join("chat.source.jsonl");
    fs::write(&source, b"{}\n").unwrap();
    let alias = f._root.path().join("raw-alias");
    fs::hard_link(&source, &alias).unwrap();
    let token = f.service.open_cloud_chat(&f.authorization).unwrap();
    let initial = page(&f, &token, 10000);
    assert!(initial.display_metadata_unavailable);
    assert_eq!(initial.items.len(), 3);
    fs::remove_file(alias).unwrap();
    let after = page(&f, &token, 10000);
    assert!(after.display_metadata_unavailable);
    assert_eq!(after.items.len(), 3);
    assert!(after.items.iter().all(|row| row.soop.is_none()));
}

#[test]
fn stored_normalized_badges_survive_unpinnable_optional_raw() {
    for unsafe_raw in [false, true] {
        let f = fixture("soop");
        let events = f.directory.join("chat.events.jsonl");
        let rows: Vec<_> = fs::read_to_string(&events)
            .unwrap()
            .lines()
            .enumerate()
            .map(|(index, line)| {
                let mut row: serde_json::Value = serde_json::from_str(line).unwrap();
                if index == 0 {
                    row["soop"] = json!({"flag1":32,"flag2":0,"role":"fan","badge":"fan"});
                }
                row.to_string() + "\n"
            })
            .collect();
        fs::write(events, rows.concat()).unwrap();
        if unsafe_raw {
            let source = f.directory.join("chat.source.jsonl");
            fs::write(&source, b"{}\n").unwrap();
            fs::hard_link(source, f._root.path().join("raw-alias")).unwrap();
        }
        let token = f.service.open_cloud_chat(&f.authorization).unwrap();
        let read = page(&f, &token, 10000);
        assert_eq!(read.items.len(), 3);
        assert!(read.items[0]
            .soop
            .as_ref()
            .is_some_and(|meta| meta.badge == "fan"));
        assert_eq!(read.display_metadata_unavailable, unsafe_raw);
    }
}
