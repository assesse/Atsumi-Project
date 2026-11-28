//! Opt-in actual DPAPI receipt -> production authorization -> real SOOP sidecars.
//! No native UI, original-video access, account mutation, worker or network.
#[path = "cloud_replay_native_actual_test.rs"]
mod native_actual_tests;
#[path = "cloud_replay_native_gift_test.rs"]
mod native_gift_tests;
#[path = "cloud_replay_native_private_test.rs"]
mod native_private_tests;
use super::*;
use crate::cloud_recording::replay_receipt_actual::{self as actual, ActualReceiptHost};
use serde_json::json;

const CATALOG_SHA: &str = "23b5c9030f10bb1e15e38c8bcd83666bd1880976373e2bbf9c2d5febec718be5";
const EVENTS_SHA: &str = "d3ff932249227617e995f11bca369eb674a4f1fc6ed4a533f78d10f456cd63b5";
const RAW_SHA: &str = "f92f7f0a4d3b2ef82dc0f2c184099d74fe7a1f5f2b7b687f643d9f52bade2388";
const SIDECARS: &[&str] = &[
    "chat-parts.jsonl", "chat-status.final.json", "chat-status.json", "chat-timeline.json",
    "chat.events.jsonl", "chat.source.jsonl", "encoded-owner.json", "recording-owner.json",
    "thumbnail.jpg", "channel-profile.json", "chat-assets/manifest.jsonl", "chat-assets/status.json",
];
fn snapshot(host: &ActualReceiptHost, directory: &Path) -> Vec<Option<(usize, String)>> {
    SIDECARS.iter().map(|name| {
        let path = directory.join(name);
        if !path.try_exists().unwrap_or_else(|_| panic!("QA_SIDECAR_STAT")) { return None; }
        let bytes = host.read(&path, 1024 * 1024);
        Some((bytes.len(), actual::digest(&bytes)))
    }).collect()
}
fn no_workers(root: &Path, job: MediaJob) -> PlatformMediaService {
    let fake = root.join("never-executed");
    let mut jobs = HashMap::new();
    jobs.insert(job.job_id.clone(), job);
    // Test-only construction prevents startup recovery and its two queue workers.
    PlatformMediaService {
        inner: Arc::new(Inner {
            state: Mutex::new(State {
                idle_merge: None,
                jobs, queue: VecDeque::new(), cancel: HashMap::new(), shutting_down: false,
                persistence_errors: HashMap::new(), persistence_overflow: false,
            }),
            ready: Condvar::new(), verify: Mutex::new(()), merge: Mutex::new(()),
            journal: root.join("platform-media/jobs"),
            tools: PlatformMediaTools { yt_dlp: fake.clone(), deno: fake.clone(), ffmpeg: fake.clone(), ffprobe: fake },
            playback: Mutex::new(HashMap::new()), cloud_chat: Mutex::new(HashMap::new()),
            upload_leases: Arc::new(Mutex::new(HashMap::new())), development_url: None,
            live: Mutex::new(live::LiveRegistry::default()), network: Mutex::new(0),
            screenshots: screenshots::Store::new(root.join("platform-media/screenshots")),
            profiles: profile::Snapshots::default(), synthetic_live_receiver: AtomicBool::new(false),
        }),
    }
}
fn page(service: &PlatformMediaService, token: &str, position: u64) -> replay_chat::ReplayChatPage {
    for _ in 0..32 {
        let value = service.cloud_chat_page(token, position, None, 120)
            .unwrap_or_else(|_| panic!("QA_ACTUAL_CHAT_PAGE"));
        if value.state != "indexing" { return value; }
    }
    panic!("QA_CHAT_PREPARATION_BOUND");
}
#[test]
#[ignore = "Actual encrypted isolated SOOP42 receipt, read-only; ATSUMI_SOOP_RECEIPT_QA=read-only-encrypted-copy required"]
fn actual_persisted_soop42_receipt_to_sidecar_readonly() {
    let started = std::time::Instant::now();
    let host = ActualReceiptHost::open();
    verify_actual_receipt(host, started, false);
}
#[test]
#[ignore = "Current stable encrypted SOOP42 snapshot semantic validation; ATSUMI_SOOP_CURRENT_RECEIPT_QA=read-only-current-semantic required"]
fn current_persisted_soop42_receipt_to_sidecar_readonly() {
    let started=std::time::Instant::now();
    let host=ActualReceiptHost::open_current();
    verify_actual_receipt(host, started, true);
}
fn verify_actual_receipt(host:ActualReceiptHost, started:std::time::Instant, current_semantic:bool) {
    let ciphertext_sha=host.ciphertext_sha256().to_owned();
    let ready_counts=host.ready_provider_counts();
    let authorization = host.authorization();
    let without_grant = host.without_current_grant();
    assert!(authorization.job_id == without_grant.job_id
        && authorization.source == without_grant.source
        && authorization.owner_channel_id == without_grant.owner_channel_id
        && authorization.cohort_id == without_grant.cohort_id
        && authorization.source_sha256 == without_grant.source_sha256
        && authorization.plan_fingerprint == without_grant.plan_fingerprint,
        "QA_DISCONNECTED_RECEIPT_CHANGED");

    let catalog = host.profile().join("platform-media/jobs").join(format!("{}.json", actual::RECORDING));
    let bytes = host.read(&catalog, 16 * 1024);
    assert!(actual::digest(&bytes) == CATALOG_SHA, "QA_ACTUAL_LIBRARY_CHANGED");
    let job: MediaJob = serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("QA_ACTUAL_LIBRARY_PARSE"));
    let item = job.library.as_ref().unwrap_or_else(|| panic!("QA_ACTUAL_LIBRARY_ABSENT"));
    assert!(job.job_id == actual::RECORDING && job.state == "completed" && job.reference.provider == "soop"
        && item.sha256 == actual::MEDIA_SHA && item.bytes == 6_912_943
        && (item.duration - 42.102521).abs() < 0.000_001, "QA_ACTUAL_SOURCE_BINDING");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let expected = workspace.join(".runtime/platform-app-qa").join(actual::PROFILE).join("media");
    assert!(fs::canonicalize(&job.download_root).unwrap_or_else(|_| panic!("QA_SOURCE_ROOT"))
        == fs::canonicalize(expected).unwrap_or_else(|_| panic!("QA_EXPECTED_ROOT")), "QA_NOT_ISOLATED_SOURCE");
    let directory = storage::job_dir(&job.download_root, "soop", actual::RECORDING)
        .unwrap_or_else(|_| panic!("QA_OWNED_SOURCE_DIRECTORY"));
    let before = snapshot(&host, &directory);
    assert!(actual::digest(&host.read(&directory.join("chat.events.jsonl"), 1024 * 1024)) == EVENTS_SHA, "QA_EVENTS_CHANGED");
    assert!(actual::digest(&host.read(&directory.join("chat.source.jsonl"), 1024 * 1024)) == RAW_SHA, "QA_RAW_CHANGED");
    assert!(!directory.join("channel-profile.json").exists()
        && host.read(&directory.join("chat-assets/manifest.jsonl"), 1024 * 1024).is_empty(), "QA_OPTIONAL_FIXTURE_CHANGED");

    let service = no_workers(host.scratch(), job);
    service.validate_cloud_chat_source(&without_grant).unwrap_or_else(|_| panic!("QA_RECEIPT_SOURCE_ADMISSION"));
    let token = service.open_cloud_chat(&without_grant).unwrap_or_else(|_| panic!("QA_RECEIPT_SIDECAR_OPEN"));
    assert!(service.inner.upload_leases.lock().unwrap().get(actual::RECORDING) == Some(&1), "QA_CAPABILITY_LEASE_MISSING");
    assert!(service.inner.playback.lock().unwrap().is_empty(), "QA_VIDEO_SESSION_CREATED");
    let first = page(&service, &token, 0);
    let all = page(&service, &token, 42_103);
    assert!(first.items.is_empty() && all.state == "ready" && all.total == 14 && all.items.len() == 14, "QA_EXPECTED_14_ROWS");
    assert!(all.items.iter().all(|row| row.soop.is_some() && row.soop_gift.is_none())
        && !all.display_metadata_unavailable, "QA_ACTUAL_ROW_METADATA");
    let first_at = all.items.iter().filter_map(|row| row.time_ms).min().unwrap();
    assert!(first_at > 0 && page(&service, &token, first_at - 1).items.is_empty(), "QA_BACKWARD_SEEK");
    assert!(!page(&service, &token, first_at).items.is_empty(), "QA_EXACT_FIRST_BOUNDARY");
    let again = page(&service, &token, 42_103);
    assert!(serde_json::to_vec(&all.items).unwrap() == serde_json::to_vec(&again.items).unwrap(), "QA_FORWARD_SEEK_CHANGED");
    assert!(service.cloud_chat_profile(&token).unwrap_or_else(|_| panic!("QA_PROFILE_CAPABILITY")).is_none(), "QA_PROFILE_INVENTED");
    assert!(service.cloud_chat_asset(&token, &"b".repeat(64)).unwrap_or_else(|_| panic!("QA_ASSET_CAPABILITY")).data_url.is_none(), "QA_ASSET_INVENTED");
    for method in ["GET", "HEAD"] {
        let request = tauri::http::Request::builder().method(method)
            .uri(format!("http://platform-media.localhost/{token}/media.mp4"))
            .header("Origin", "http://tauri.localhost").header("Range", "bytes=0-1")
            .body(Vec::new()).unwrap();
        assert!(!protocol::respond(&service, "main", request).status().is_success(), "QA_VIDEO_ROUTE_ALLOWED");
    }
    service.close_cloud_chat(&token);
    assert!(service.cloud_chat_page(&token, 0, None, 1).is_err()
        && service.cloud_chat_profile(&token).is_err()
        && service.cloud_chat_asset(&token, &"b".repeat(64)).is_err(), "QA_CLOSED_TOKEN_READABLE");
    assert!(service.inner.cloud_chat.lock().unwrap().is_empty()
        && service.inner.upload_leases.lock().unwrap().is_empty()
        && *service.inner.network.lock().unwrap() == 0, "QA_WORK_OR_LEASE_REMAINED");
    {
        let state = service.inner.state.lock().unwrap();
        assert!(state.jobs.len() == 1 && state.queue.is_empty() && state.cancel.is_empty()
            && !state.shutting_down && state.persistence_errors.is_empty(), "QA_PLATFORM_WORK_STARTED");
    }
    assert!(snapshot(&host, &directory) == before, "QA_ORIGINAL_SIDECARS_CHANGED");
    assert!(actual::digest(&host.read(&catalog, 16 * 1024)) == CATALOG_SHA, "QA_ORIGINAL_CATALOG_CHANGED");
    host.assert_unchanged();
    if current_semantic {
        // Explicitly approved ignored local preview only; never stdout, release,
        // network, or a claim of actual native audiovisual verification.
        let preview=workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume/soop42-actual-chat-preview.json");
        fs::write(preview,serde_json::to_vec_pretty(&all).unwrap()).unwrap_or_else(|_|panic!("QA_LOCAL_PREVIEW_WRITE"));
    }
    let grant_initially_present = host.grant_initially_present;
    drop(service);
    let bounded_source_and_scratch_read_bytes = host.finish();

    let result = json!({
        "version":1,"scope":"Actual isolated encrypted ready/unlisted SOOP42 receipt -> native replay_authorization -> production sidecar capability",
        "currentSnapshotSemanticRevalidation":current_semantic,"historicalCiphertextPinned":!current_semantic,
        "historicalOwnerEqualityVerified":false,"ciphertextSha256":ciphertext_sha,
        "readySoopReceipts":ready_counts.0,"readyChzzkReceipts":ready_counts.1,"readyYoutubeReceipts":ready_counts.2,
        "passed":true,"actualEncryptedReceiptDecryptedWithSameUserDpapi":true,"actualJobSourceOwnerPlanUnitBindingVerified":true,
        "authorizationReconstructedOrOverridden":false,"grantInitiallyPresent":grant_initially_present,
        "currentGrantRemovedInMemoryOnly":true,"withoutCurrentGrantSameOwnerCohort":true,
        "rows":all.items.len(),"normalizedRowsBeforeClockMapping":15,"ordinaryChatMetadataRows":14,"donationRows":0,
        "timelineQuality":all.timeline_quality,"backwardSeek":true,"forwardSeekExactRows":true,"closeRevokesToken":true,
        "videoRoutesDenied":true,"profileCapabilityVerified":"absent profile stays absent","assetCapabilityVerified":"empty manifest stays empty",
        "originalSidecarsAndCatalogHashesUnchanged":true,"originalEncryptedCatalogAndPlanHashesUnchanged":true,
        "scratchCiphertextAndPlanBytesUnchanged":true,"osTemporaryCopyCleaned":true,
        "transportResolverPlannerBrowserCalls":0,"backgroundWorkersStarted":0,"videoBytesRead":0,
        "actualRemoteVideoOrNativeUiVerified":false,"freshGoogleResponseVerified":false,"donationAvSyncVerified":false,
        "sourceAndScratchExplicitReadBytes":bounded_source_and_scratch_read_bytes,"elapsedMilliseconds":started.elapsed().as_millis(),
        "note":"The real persisted receipt is used, but current remote player state, visual acceptance and donation synchronization are separate checks. No source-media operation or user account mutation."
    });
    let output = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume").join(if current_semantic {
        "soop-current-receipt-readonly-result.json"
    } else { "soop-actual-receipt-readonly-result.json" });
    fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap_or_else(|_| panic!("QA_RESULT_WRITE"));
}

