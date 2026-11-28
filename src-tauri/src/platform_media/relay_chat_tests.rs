use super::*;
use crate::platform_media::relay_chat::{Binding, RelayChatLease, RelayChatSummary};

fn binding() -> Binding {
    Binding {
        receiver_id: uuid::Uuid::new_v4().to_string(),
        run_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        channel_id: "fixture".into(),
        broadcast_id: "123".into(),
    }
}
fn attach(chat: &Arc<SharedChat>, root: &Path) -> RelayChatLease {
    RelayChatLease::attach(chat.clone(), None, binding(), root).unwrap()
}
fn finish(lease: &mut RelayChatLease, interrupted: bool) -> RelayChatSummary {
    lease.begin_finish(interrupted);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(r) = lease.poll_join().unwrap() {
            return r;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn row(id: &str) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"kind":"chat","nickname":"Fixture","text":"Saved",
        "messageId":id,"receivedAtMs":1700000001000u64,"elapsedMs":25,"flags":"32|0",
        "extra":{"service":5,"nicknameColorKey":"red","color":"#aabbcc"}}),
    )
    .unwrap()
}
#[test]
fn same_receiver_fanout_is_byte_identical_and_archive_stop_is_independent() {
    let chat = SharedChat::new();
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("recording");
    fs::create_dir(&archive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(&archive, "soop", &id)
        .unwrap();
    let mut relay = attach(&chat, root.path());
    chat.ingest("soop", &row("one")).unwrap();
    chat.ingest("soop", &row("one")).unwrap();
    chat.ingest("soop", &row("two")).unwrap();
    let closed = chat
        .detach_archive()
        .unwrap()
        .unwrap()
        .persist(&archive, "soop", &id)
        .unwrap();
    assert_eq!(closed.state, chat_status::TextState::Complete);
    chat.ingest("soop", &row("relay-only")).unwrap();
    let report = finish(&mut relay, false);
    assert!(report.joined);
    assert_eq!(report.text_state, "complete");
    assert_eq!(report.raw_rows, 4);
    assert_eq!(report.event_rows, 3);
    let old = fs::read(archive.join("chat.events.jsonl")).unwrap();
    let new = fs::read(report.directory.join("chat.events.jsonl")).unwrap();
    assert!(new.starts_with(&old));
    let old_raw = fs::read(archive.join("chat.source.jsonl")).unwrap();
    assert!(fs::read(report.directory.join("chat.source.jsonl"))
        .unwrap()
        .starts_with(&old_raw));
    assert!(!chat.stop.load(Ordering::Acquire));
    assert!(chat.state.lock().unwrap().relay.is_none());
    assert!(fs::read_dir(&report.directory).unwrap().all(|p| !p
        .unwrap()
        .path()
        .extension()
        .is_some_and(|e| e == "mp4" || e == "ts")));
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("directory"));
    chat.stop();
}
#[test]
fn relay_stop_does_not_stop_recording_and_rejoin_requires_real_worker_join() {
    let chat = SharedChat::new();
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("recording");
    fs::create_dir(&archive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(&archive, "soop", &id)
        .unwrap();
    let mut relay = attach(&chat, root.path());
    chat.ingest("soop", &row("one")).unwrap();
    relay.begin_finish(false);
    assert!(RelayChatLease::attach(chat.clone(), None, binding(), root.path()).is_err());
    chat.ingest("soop", &row("two")).unwrap();
    let r = finish(&mut relay, false);
    assert_eq!(r.event_rows, 1);
    assert!(!r.interrupted);
    let mut next = attach(&chat, root.path());
    chat.ingest("soop", &row("three")).unwrap();
    assert_eq!(finish(&mut next, false).event_rows, 1);
    chat.detach_archive()
        .unwrap()
        .unwrap()
        .persist(&archive, "soop", &id)
        .unwrap();
    assert_eq!(
        fs::read_to_string(archive.join("chat.events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        3
    );
    chat.stop();
}
#[test]
fn receive_instant_is_separate_and_pre_attachment_time_is_not_clamped() {
    let before = Instant::now();
    let chat = SharedChat::new();
    let root = tempfile::tempdir().unwrap();
    let mut relay = attach(&chat, root.path());
    chat.ingest_received(
        "soop",
        &row("early"),
        None,
        vec![],
        vec![],
        None,
        None,
        None,
        before,
    )
    .unwrap();
    chat.ingest_received(
        "soop",
        &row("later"),
        None,
        vec![],
        vec![],
        None,
        None,
        None,
        Instant::now(),
    )
    .unwrap();
    let r = finish(&mut relay, false);
    let ledger = fs::read_to_string(r.directory.join("relay-receive.jsonl")).unwrap();
    let rows = ledger
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 4);
    assert!(rows[0]["receiveMonotonicNs"].is_null());
    assert!(rows[2]["receiveMonotonicNs"].as_u64().is_some());
    assert_eq!(rows[0]["receipt"], rows[1]["receipt"]);
    assert_eq!(rows[2]["receipt"], rows[3]["receipt"]);
    let normal = fs::read_to_string(r.directory.join("chat.events.jsonl")).unwrap();
    assert!(!normal.contains("receiveMonotonicNs"));
    assert!(!normal.contains("videoOffset"));
    assert_eq!(r.clock_quality, "receive-monotonic-only");
    chat.stop();
}
#[test]
fn source_gap_and_optional_missing_image_remain_honest_partial() {
    let mut chat = SharedChat::new();
    Arc::get_mut(&mut chat).unwrap().assets =
        SessionAssets::with_test_fetcher(|_| panic!("untrusted metadata cannot fetch"));
    let root = tempfile::tempdir().unwrap();
    let mut relay = attach(&chat, root.path());
    chat.fail_receiver("source failed".into());
    chat.ingest("soop", &row("retained")).unwrap();
    let report = finish(&mut relay, false);
    assert_eq!(report.event_rows, 1);
    assert_eq!(report.text_state, "partial");
    assert!(report.failure.is_none());
    chat.stop();
}
#[test]
fn failed_relay_write_never_poisoned_the_existing_recording_sink() {
    let chat = SharedChat::new();
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("recording");
    fs::create_dir(&archive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(&archive, "soop", &id)
        .unwrap();
    let mut relay = attach(&chat, root.path());
    chat.state
        .lock()
        .unwrap()
        .relay
        .as_ref()
        .unwrap()
        .fail_next_write();
    chat.ingest("soop", &row("one")).unwrap();
    let report = finish(&mut relay, false);
    assert_eq!(report.failure.as_deref(), Some("write_failed"));
    assert_eq!(report.text_state, "partial");
    chat.ingest("soop", &row("two")).unwrap();
    let saved = chat
        .detach_archive()
        .unwrap()
        .unwrap()
        .persist(&archive, "soop", &id)
        .unwrap();
    assert_eq!(saved.state, chat_status::TextState::Complete);
    assert_eq!(
        fs::read_to_string(archive.join("chat.events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    chat.stop();
}
#[test]
fn missing_capture_capability_settles_image_without_refetching_current_profile() {
    let context = crate::soop::chat::personalcon::Context::from_channel(
        &serde_json::json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/373867c54c3ee5ae5.png?_=1741052109"}]}}),
    );
    let request = context.select("0|262144", "17", "0").0.unwrap();
    let mut chat = SharedChat::new();
    Arc::get_mut(&mut chat).unwrap().assets =
        SessionAssets::with_test_fetcher(|_| panic!("no current profile fetch"));
    let root = tempfile::tempdir().unwrap();
    let mut relay = attach(&chat, root.path());
    let raw=serde_json::to_vec(&serde_json::json!({"kind":"chat","receivedAtMs":1700000001000u64,"nickname":"Public","text":"Saved","flags":"32|262144","extra":{"service":5,"personalcon":request.metadata()}})).unwrap();
    chat.ingest("soop", &raw).unwrap();
    let report = finish(&mut relay, false);
    assert_eq!(report.event_rows, 1);
    assert!(report.assets_incomplete);
    assert_eq!(report.text_state, "complete");
    chat.stop();
}
#[test]
fn captured_personalcon_fetch_is_shared_and_relay_waits_only_its_target() {
    let context = crate::soop::chat::personalcon::Context::from_channel(
        &serde_json::json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/373867c54c3ee5ae5.png?_=1741052109"}]}}),
    );
    let request = context.select("0|262144", "17", "0").0.unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let expected = png.into_inner();
    let payload = expected.clone();
    let mut chat = SharedChat::new();
    Arc::get_mut(&mut chat).unwrap().assets = SessionAssets::with_test_fetcher(move |_| {
        count.fetch_add(1, Ordering::AcqRel);
        std::thread::sleep(Duration::from_millis(50));
        Ok(payload.clone())
    });
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("recording");
    fs::create_dir(&archive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    chat.attach_recording_archive(&archive, "soop", &id)
        .unwrap();
    let mut relay = attach(&chat, root.path());
    let raw=serde_json::to_vec(&serde_json::json!({"kind":"chat","receivedAtMs":1700000001000u64,"messageId":"pcon","nickname":"Public","text":"Saved","flags":"32|262144","extra":{"service":5,"personalcon":request.metadata()}})).unwrap();
    chat.ingest_with_appearance("soop", &raw, None, vec![], vec![], Some(request.clone()))
        .unwrap();
    let report = finish(&mut relay, false);
    assert!(!report.assets_incomplete);
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(
        fs::read(report.directory.join("chat-assets").join(request.id())).unwrap(),
        expected
    );
    assert_eq!(
        fs::read(archive.join("chat-assets").join(request.id())).unwrap(),
        expected
    );
    chat.detach_archive()
        .unwrap()
        .unwrap()
        .persist(&archive, "soop", &id)
        .unwrap();
    assert_eq!(
        fs::read(archive.join("chat.events.jsonl")).unwrap(),
        fs::read(report.directory.join("chat.events.jsonl")).unwrap()
    );
    chat.stop();
}

#[test]
fn cancellation_during_target_drain_is_in_final_durable_outcome() {
    let request=crate::soop::chat::personalcon::Context::from_channel(&serde_json::json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/373867c54c3ee5ae5.png?_=1741052109"}]}})).select("0|262144","17","0").0.unwrap();
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let started = entered.clone();
    let gate = release.clone();
    let mut chat = SharedChat::new();
    Arc::get_mut(&mut chat).unwrap().assets = SessionAssets::with_test_fetcher(move |_| {
        started.store(true, Ordering::Release);
        while !gate.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        Ok(bytes.into_inner())
    });
    let root = tempfile::tempdir().unwrap();
    let mut relay = attach(&chat, root.path());
    let raw=serde_json::to_vec(&serde_json::json!({"kind":"chat","receivedAtMs":1700000001000u64,"nickname":"Public","text":"Saved","flags":"32|262144","extra":{"service":5,"personalcon":request.metadata()}})).unwrap();
    chat.ingest_with_appearance("soop", &raw, None, vec![], vec![], Some(request))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !entered.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    relay.begin_finish(false);
    std::thread::sleep(Duration::from_millis(30));
    assert!(relay.poll_join().unwrap().is_none());
    relay.begin_finish(true);
    release.store(true, Ordering::Release);
    let report = finish(&mut relay, true);
    assert!(report.interrupted);
    assert_eq!(report.text_state, "partial");
    assert!(!report.assets_incomplete);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(report.directory.join("relay-summary.json")).unwrap())
            .unwrap();
    assert_eq!(saved["interrupted"], true);
    assert_eq!(saved["textState"], "partial");
    chat.stop();
}
