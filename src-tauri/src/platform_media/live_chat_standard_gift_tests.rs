use super::*;
use crate::soop::chat::gifts::{assets::{Context, Request}, SoopGiftMetadata};
use crate::platform_media::relay_chat::{Binding, RelayChatLease};
use std::io::{Read, Seek, SeekFrom, Write};

fn gift(video: bool) -> (SoopGiftMetadata, Request) {
    let context = Context::new();
    let fields = if video { vec!["room", "fixture", "private", "Public", "7", "0", "", "0", "0", "", "", "", "ignored", "0"] }
        else { vec!["fixture", "private", "Public", "211", "0", "", "", "", "1", "0"] };
    let request = context.from_fields(if video {105} else {18}, &fields, "fixture").unwrap();
    let mut meta = crate::soop::chat::gifts::from_stored(&serde_json::json!({
        "kind":if video {"videoBalloon"} else {"balloon"}, "count":if video {7} else {211},
        "relay":false, "isDefault":!video,
    })).unwrap();
    assert!(request.apply(&mut meta)); (meta, request)
}
fn raw(meta: &SoopGiftMetadata, id: &str) -> Vec<u8> {
    let video = meta.kind==crate::soop::chat::gifts::SoopGiftKind::VideoBalloon;
    serde_json::to_vec(&serde_json::json!({"kind":"donation", "receivedAtMs":if video {1700000001500u64} else {1700000001000u64},
        "messageId":id, "nickname":"Public", "text":"", "extra":{"service":if meta.kind==crate::soop::chat::gifts::SoopGiftKind::VideoBalloon {105} else {18},
        "soopGift":meta}})).unwrap()
}
fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3).write_to(&mut bytes, image::ImageFormat::Png).unwrap(); bytes.into_inner()
}
fn until(done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !done() { assert!(Instant::now() < end, "bounded standard gift bridge"); std::thread::sleep(Duration::from_millis(2)); }
}
const STEP4_SHA: &str = "4a4e14f1c92d71ea82a0c42b868987bdac19b806128d6f4aaa53920d1a53df0d";
const VIDEO_SHA: &str = "74be50787251e9750644a1499e6d81c14886c2dbe49bc409a04323cbfd19e9be";
fn held_png(name: &str, expected: &str) -> (File, Vec<u8>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("public/assets/soop/gifts").join(name);
    super::super::storage::regular(&path).unwrap();
    let mut options = OpenOptions::new(); options.read(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // deny write/delete for the held original fixture
    }
    let mut file = options.open(path).unwrap();
    let bytes = verify_held(&mut file, expected); (file, bytes)
}
fn verify_held(file: &mut File, expected: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    assert!(file.metadata().unwrap().len() <= 64 * 1024);
    file.seek(SeekFrom::Start(0)).unwrap(); let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 64 * 1024);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected, "held image SHA"); bytes
}
fn page(directory: &Path, id: &str, at: u64) -> super::super::replay_chat::ReplayChatPage {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let result = super::super::replay_chat::read_page_for_job(directory, "soop", id, at, None, 120).unwrap();
        if result.state != "indexing" { return result; }
        assert!(Instant::now() < end, "bounded native replay index"); std::thread::sleep(Duration::from_millis(2));
    }
}
struct ReleaseIndex(PathBuf);
impl Drop for ReleaseIndex {
    fn drop(&mut self) { super::super::replay_chat::release_directory(&self.0); }
}

#[test]
fn standard_and_video_ids_cross_raw_normalized_and_recording_relay_fanout() {
    // These held PNG responses are controlled fixture bytes. The observed
    // count211 determines step4 only; its historical missing response is unknown.
    let (mut step_pin, step) = held_png("ba_step4.png", STEP4_SHA);
    let (mut video_pin, video) = held_png("m_video_balloon.png", VIDEO_SHA);
    let (step_response, video_response) = (step.clone(), video.clone());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0)); let count = calls.clone();
    let chat = SharedChat::with_test_assets(SessionAssets::with_test_fetcher(move |url| {
        count.fetch_add(1, Ordering::AcqRel);
        match url {
            "https://res.sooplive.com/new_player/items/ba_step4.png" => Ok(step_response.clone()),
            "https://res.sooplive.com/new_player/items/m_video_balloon.png" => Ok(video_response.clone()),
            _ => panic!("unexpected controlled image request"),
        }
    }));
    let root = tempfile::tempdir().unwrap(); let directory = root.path().join("recording"); fs::create_dir(&directory).unwrap();
    let id = uuid::Uuid::new_v4().to_string(); chat.attach_recording_archive(&directory, "soop", &id).unwrap();
    let binding = Binding { receiver_id:uuid::Uuid::new_v4().to_string(), run_id:uuid::Uuid::new_v4().to_string(),
        source_id:uuid::Uuid::new_v4().to_string(), document_epoch:1, channel_id:"fixture".into(), broadcast_id:"123".into() };
    let mut relay = RelayChatLease::attach(chat.clone(), None, binding, root.path()).unwrap();
    let mut ids = Vec::new();
    for is_video in [false, true] {
        let (gift, request) = gift(is_video); let bytes = raw(&gift, if is_video {"video"} else {"standard"});
        let normalized = normalize_public_line("soop", &bytes).unwrap();
        assert_eq!(normalized.len(), 1); assert!(normalized[0].gift_asset.is_none());
        assert_eq!(normalized[0].soop_gift.as_ref(), Some(&gift));
        chat.ingest_with_gift("soop", &bytes, Some(request.clone())).unwrap();
        ids.push(request.id().to_owned());
    }
    // Explicit controlled clock, never copied into any old capture.
    fs::write(directory.join("chat-timeline.json"), br#"{"timelineQuality":"receive-time-approximate","firstSegmentCommittedAtMs":1700000004000,"firstSegmentDurationMs":4000}"#).unwrap();
    let report = chat.detach_archive().unwrap().unwrap().persist(&directory, "soop", &id).unwrap();
    assert!(!report.assets_incomplete);
    relay.begin_finish(false);
    let end = Instant::now()+Duration::from_secs(3);
    let relay_report = loop { if let Some(result)=relay.poll_join().unwrap(){break result;} assert!(Instant::now()<end); std::thread::sleep(Duration::from_millis(2)); };
    assert!(relay_report.joined && !relay_report.assets_incomplete); assert_eq!(relay_report.event_rows, 2);
    assert_eq!(calls.load(Ordering::Acquire), 2);
    for file in ["chat.source.jsonl", "chat.events.jsonl"] {
        let recording = fs::read(directory.join(file)).unwrap();
        assert_eq!(fs::read(relay_report.directory.join(file)).unwrap(), recording);
        let text = std::str::from_utf8(&recording).unwrap();
        assert!(!text.contains("https://") && !text.contains("giftAsset") && !text.contains("private"));
        assert_eq!(text.lines().count(), 2);
        for id in &ids { assert!(text.contains(id)); }
    }
    chat.stop(); until(|| chat.assets.is_done());
    let mut assets = serde_json::Map::new();
    for (id, expected) in ids.iter().zip([&step, &video]) {
        let saved = super::super::chat_assets::replay_asset(&directory, &id).unwrap().unwrap();
        use base64::Engine;
        assert_eq!(&base64::engine::general_purpose::STANDARD.decode(saved.split_once(',').unwrap().1).unwrap(), expected);
        assets.insert(id.clone(), serde_json::to_value(ChatAssetRead::local(Some(saved))).unwrap());
    }
    let _index = ReleaseIndex(directory.clone());
    assert!(page(&directory, &id, 999).items.is_empty());
    let first = page(&directory, &id, 1000);
    assert_eq!(first.items.len(), 1); assert_eq!(first.items[0].soop_gift.as_ref().unwrap().count, 211);
    let all = page(&directory, &id, 1500);
    assert_eq!(all.state, "ready"); assert_eq!(all.total, 2); assert_eq!(all.items.len(), 2);
    assert_eq!(all.unsupported_items, 0);
    assert!(page(&directory, &id, 999).items.is_empty());
    super::super::replay_chat::release_directory(&directory);
    let reopened = page(&directory, &id, 1500);
    assert_eq!(serde_json::to_value(&all).unwrap(), serde_json::to_value(&reopened).unwrap());
    assert_eq!(verify_held(&mut step_pin, STEP4_SHA), step);
    assert_eq!(verify_held(&mut video_pin, VIDEO_SHA), video);
    if std::env::var("ATSUMI_STANDARD_GIFT_EXPORT").ok().as_deref() == Some("1") {
        let audit = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".runtime/requirements-audit/20261117-cloud-goal-resume");
        super::super::storage::safe_path(&audit).unwrap(); assert!(audit.is_dir());
        let destination = audit.join(format!("standard-gift-controlled-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&destination).unwrap();
        let result = serde_json::json!({"version":1,"classification":"controlled native saved-replay; no new public gift or recovered old response",
            "standardSelection":{"observedPriorEventCount":211,"step":4,"historicalResponseRestored":false},
            "videoSelection":"controlled service105 fixture","timeline":"controlled receipt-time fixture",
            "page":reopened,"assets":assets,"sourcePngHashes":[STEP4_SHA,VIDEO_SHA],"originalPngHashesUnchanged":true,
            "seekChecks":{"beforeFirstRows":0,"atFirstRows":1,"atVideoRows":2,"backwardRows":0,"reopenedRows":2},
            "actions":{"publicRequests":0,"uploads":0,"oldArchiveWrites":0}});
        let path = destination.join("native-replay.json");
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes()).unwrap(); file.sync_all().unwrap();
        println!("controlled standard-gift export: {}", path.display());
    }
}

#[test]
fn receiver_end_keeps_already_admitted_standard_image_until_proof_persistence() {
    let entered = Arc::new(AtomicBool::new(false)); let release = Arc::new(AtomicBool::new(false));
    let (e, r) = (entered.clone(), release.clone());
    let chat = SharedChat::with_test_assets(SessionAssets::with_test_fetcher(move |_| {
        e.store(true, Ordering::Release); until(|| r.load(Ordering::Acquire)); Ok(png())
    }));
    let root = tempfile::tempdir().unwrap(); let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(root.path(), "soop", &id).unwrap();
    let (gift, request) = gift(false);
    chat.ingest_with_gift("soop", &raw(&gift, "one"), Some(request.clone())).unwrap();
    until(|| entered.load(Ordering::Acquire));
    drop(ReceiverDone(chat.clone()));
    let outcome = chat.detach_archive().unwrap().unwrap();
    assert!(chat.pending_soop_archive_assets()); assert!(!chat.assets.is_done());
    release.store(true, Ordering::Release);
    assert!(!outcome.persist(root.path(), "soop", &id).unwrap().assets_incomplete);
    assert!(!chat.pending_soop_archive_assets()); until(|| chat.assets.is_done());
    assert!(root.path().join("chat-assets").join(request.id()).exists());
}

#[test]
fn serialized_capability_or_wrong_art_never_fetches_and_preserves_gift_rows() {
    let chat = SharedChat::with_test_assets(SessionAssets::with_test_fetcher(|_|panic!("no matching capture request")));
    let root = tempfile::tempdir().unwrap(); let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(root.path(), "soop", &id).unwrap();
    let (standard, request) = gift(false);
    let mut payload: serde_json::Value = serde_json::from_slice(&raw(&standard, "untrusted")).unwrap();
    payload["giftAsset"] = serde_json::json!({"url":request.url(),"id":request.id()});
    chat.ingest("soop", &serde_json::to_vec(&payload).unwrap()).unwrap();
    let (video, _) = gift(true);
    chat.ingest_with_gift("soop", &raw(&video, "wrong-art"), Some(request)).unwrap();
    let summary = chat.detach_archive().unwrap().unwrap().persist(root.path(), "soop", &id).unwrap();
    assert!(summary.assets_incomplete);
    assert!(fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap().is_empty());
    let lines = fs::read_to_string(root.path().join("chat.events.jsonl")).unwrap();
    assert_eq!(lines.lines().count(), 2);
    for line in lines.lines() {
        let event: NormalizedChatEvent = serde_json::from_str(line).unwrap();
        assert!(event.gift_asset.is_none()); assert_eq!(event.kind, "donation");
        assert!(event.soop_gift.unwrap().count > 0);
    }
    chat.stop();
}
