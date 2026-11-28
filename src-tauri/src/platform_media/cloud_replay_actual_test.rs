//! Opt-in read-only regression over the fixed, already verified isolated SOOP copy.
//! This reconstructs a trusted native descriptor from pinned catalogue metadata;
//! it does not exercise encrypted cloud-receipt admission or a remote video player.
use super::*;
use crate::cloud_recording::{replay::CloudReplayInput, CloudSourceRef};
use serde_json::json;
use sha2::{Digest, Sha256};

const ID: &str = "241bc41d-d3e6-4bf9-a205-899f2c2d21d5";
const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const CATALOG_SHA: &str = "23b5c9030f10bb1e15e38c8bcd83666bd1880976373e2bbf9c2d5febec718be5";
const MEDIA_SHA: &str = "5c5fcddd3294ae9ffe3b7b3063e8c8a1a46cecc7b6a7a004629f9e715b825f75";
const EVENTS_SHA: &str = "d3ff932249227617e995f11bca369eb674a4f1fc6ed4a533f78d10f456cd63b5";
const RAW_SHA: &str = "f92f7f0a4d3b2ef82dc0f2c184099d74fe7a1f5f2b7b687f643d9f52bade2388";
const SIDECARS: &[&str] = &[
    "chat-parts.jsonl", "chat-status.final.json", "chat-status.json", "chat-timeline.json",
    "chat.events.jsonl", "chat.source.jsonl", "encoded-owner.json", "recording-owner.json",
    "thumbnail.jpg", "channel-profile.json", "chat-assets/manifest.jsonl", "chat-assets/status.json",
];
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn bounded(path: &Path) -> Vec<u8> {
    assert!(fs::metadata(path).unwrap().len() <= 1024 * 1024, "QA_SIDECAR_LIMIT");
    fs::read(path).expect("QA_SIDECAR_READ")
}
fn snapshot(directory: &Path) -> Vec<Option<(usize, String)>> {
    SIDECARS.iter().map(|name| {
        let path = directory.join(name);
        if !path.try_exists().expect("QA_SIDECAR_STAT") { return None; }
        let bytes = bounded(&path);
        Some((bytes.len(), digest(&bytes)))
    }).collect()
}
struct Run { service: PlatformMediaService, _scratch: tempfile::TempDir }
impl Drop for Run { fn drop(&mut self) { self.service.shutdown(); } }
fn page(service: &PlatformMediaService, token: &str, at: u64) -> replay_chat::ReplayChatPage {
    for _ in 0..32 {
        let result = service.cloud_chat_page(token, at, None, 120).expect("QA_CHAT_READ");
        if result.state != "indexing" { return result; }
    }
    panic!("QA_CHAT_PREPARATION_LIMIT");
}

#[test]
#[ignore = "Read-only fixed isolated SOOP42 sidecars; ATSUMI_SOOP_SIDECAR_QA=read-only-fixed-copy required"]
fn actual_fixed_soop42_cloud_sidecars_preserve_rows_and_seek() {
    assert!(std::env::var("ATSUMI_SOOP_SIDECAR_QA").as_deref() == Ok("read-only-fixed-copy"), "QA_OPT_IN_REQUIRED");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let catalog = PathBuf::from(std::env::var_os("APPDATA").expect("QA_APPDATA"))
        .join(PROFILE).join("platform-media/jobs").join(format!("{ID}.json"));
    let catalog_bytes = bounded(&catalog);
    assert!(digest(&catalog_bytes) == CATALOG_SHA, "QA_CATALOG_CHANGED");
    let job: MediaJob = serde_json::from_slice(&catalog_bytes).expect("QA_CATALOG_PARSE");
    let item = job.library.as_ref().expect("QA_LIBRARY_MISSING");
    assert!(job.job_id == ID && job.state == "completed" && job.reference.provider == "soop", "QA_CATALOG_IDENTITY");
    assert!(item.sha256 == MEDIA_SHA && item.bytes == 6_912_943 && (item.duration - 42.102521).abs() < 0.000_001, "QA_MEDIA_METADATA_CHANGED");
    let expected_root = workspace.join(".runtime/platform-app-qa").join(PROFILE).join("media");
    assert!(fs::canonicalize(&job.download_root).unwrap() == fs::canonicalize(expected_root).unwrap(), "QA_NOT_ISOLATED_COPY");
    let directory = storage::job_dir(&job.download_root, "soop", ID).expect("QA_OWNED_DIRECTORY");
    let before = snapshot(&directory);
    assert!(digest(&bounded(&directory.join("chat.events.jsonl"))) == EVENTS_SHA, "QA_EVENTS_CHANGED");
    assert!(digest(&bounded(&directory.join("chat.source.jsonl"))) == RAW_SHA, "QA_RAW_CHANGED");
    // Metadata only: this test never opens or hashes the video.
    let media_before = fs::metadata(directory.join("media.mp4")).expect("QA_MEDIA_STAT");
    let profile_before = profile::read(&directory).expect("QA_PROFILE_READ");
    assert!(profile_before.is_none(), "QA_PROFILE_FIXTURE_CHANGED");
    assert!(bounded(&directory.join("chat-assets/manifest.jsonl")).is_empty(), "QA_ASSET_FIXTURE_CHANGED");

    let source = CloudSourceRef { provider: "soop".into(), recording_id: ID.into(), part_index: None, plan_id: None, unit_id: None };
    let authorization = CloudReplayAuthorization {
        job_id: uuid::Uuid::new_v4().to_string(), source: source.clone(), owner_channel_id: "UCreadOnlyFixture".into(),
        cohort_id: "a".repeat(64), source_sha256: item.sha256.clone(), bytes: item.bytes,
        duration_seconds: item.duration, media_offset_seconds: 0.0,
        original_inputs: vec![CloudReplayInput { index: 0, part_index: None, sha256: item.sha256.clone(), bytes: item.bytes,
            duration_seconds: item.duration, media_offset_seconds: 0.0, modified_nanos: Some(item.modified_millis as u128 * 1_000_000) }],
        plan_fingerprint: None, input_parts: false,
        description: serde_json::from_value(json!({"source":source,"title":"fixed QA source","coverage":{"state":"complete"},
            "preparationRequired":false,"durationSeconds":item.duration,"bytes":item.bytes,"parts":[]})).unwrap(),
    };
    let scratch = tempfile::tempdir().unwrap();
    let fake = scratch.path().join("never-executed");
    let service = PlatformMediaService::new(scratch.path().join("data"), PlatformMediaTools {
        yt_dlp: fake.clone(), deno: fake.clone(), ffmpeg: fake.clone(), ffprobe: fake,
    }, None).expect("QA_TEMP_SERVICE");
    let run = Run { service, _scratch: scratch };
    run.service.inner.state.lock().unwrap().jobs.insert(ID.into(), job);
    run.service.validate_cloud_chat_source(&authorization).expect("QA_ADMISSION");
    let token = run.service.open_cloud_chat(&authorization).expect("QA_CAPABILITY_OPEN");
    let before_first = page(&run.service, &token, 0);
    let all = page(&run.service, &token, 42_103);
    assert!(all.state == "ready" && all.items.len() == 14 && all.total == 14, "QA_EXPECTED_14_ROWS");
    assert!(all.items.iter().all(|row| row.soop.is_some() && row.soop_gift.is_none()), "QA_EXPECTED_ORDINARY_METADATA");
    assert!(!all.display_metadata_unavailable, "QA_METADATA_UNAVAILABLE");
    let first_at = all.items.iter().filter_map(|row| row.time_ms).min().unwrap();
    assert!(first_at > 0 && before_first.items.is_empty(), "QA_INITIAL_CLOCK_CHANGED");
    assert!(page(&run.service, &token, first_at - 1).items.is_empty(), "QA_BACKSEEK");
    assert!(!page(&run.service, &token, first_at).items.is_empty(), "QA_FIRST_BOUNDARY");
    let again = page(&run.service, &token, 42_103);
    assert!(serde_json::to_vec(&all.items).unwrap() == serde_json::to_vec(&again.items).unwrap(), "QA_FORWARD_SEEK_ROWS_CHANGED");
    assert!(run.service.cloud_chat_profile(&token).expect("QA_PROFILE_CAPABILITY") == profile_before, "QA_PROFILE_DIFFERENT");
    assert!(run.service.cloud_chat_asset(&token, &"b".repeat(64)).expect("QA_ASSET_CAPABILITY").data_url.is_none(), "QA_UNRECORDED_ASSET_INVENTED");
    run.service.close_cloud_chat(&token);
    assert!(run.service.cloud_chat_page(&token, 42_103, None, 120).is_err(), "QA_CLOSED_TOKEN_READABLE");
    assert!(snapshot(&directory) == before && digest(&bounded(&catalog)) == CATALOG_SHA, "QA_ORIGINAL_CHANGED");
    let media_after = fs::metadata(directory.join("media.mp4")).expect("QA_MEDIA_STAT_AFTER");
    assert!(media_after.len() == media_before.len() && media_after.modified().unwrap() == media_before.modified().unwrap(), "QA_MEDIA_STAT_CHANGED");
    let result = json!({"version":1,"scope":"fixed isolated actual SOOP42 sidecars; reconstructed native authorization",
        "recordingId":ID,"passed":true,"rows":all.items.len(),"normalizedRowsBeforeClockMapping":15,
        "ordinaryChatMetadataRows":14,"donationRows":0,"timelineQuality":all.timeline_quality,
        "backwardSeek":true,"forwardSeekExactRows":true,"closeRevokesToken":true,
        "profileCapabilityVerified":"absent profile remains absent","assetCapabilityVerified":"empty manifest remains empty",
        "originalSidecarsAndCatalogueHashesUnchanged":true,"videoOpenedOrHashed":false,"videoMetadataUnchanged":true,
        "encryptedReceiptAdmissionExercised":false,"actualRemoteVideoOrNativeUiVerified":false,
        "note":"The previously verified media SHA is compared with catalogue metadata only; no new video checksum or audiovisual sync claim."});
    let output = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume/soop-sidecar-actual-result.json");
    fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).expect("QA_RESULT_WRITE");
}
