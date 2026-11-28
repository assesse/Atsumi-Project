use super::*;
#[test]
fn final_pending_clock_is_unknown_without_interrupting_media_or_chat() {
    let (_root, _host, mut owner) = setup();
    let shared = Mutex::new(Admission::new());
    {
        let mut state = lock(&shared);
        state.chat = Some(owner.input());
        state.probe = Some((Uuid::new_v4(), Instant::now()));
    }
    settle_pending_probe(&shared);
    assert!(lock(&shared).probe.is_none());
    assert!(!lock(&shared).interrupted);
    owner.finish(false);
    let facts = owner.join();
    assert!(facts.joined && !facts.failure && !facts.interrupted);
    assert!(facts.timing_incomplete);
    assert_eq!(facts.timing_records, 1);
    let rows = fs::read_to_string(owner.root.join("relay-timing.jsonl")).unwrap();
    let value: Value = serde_json::from_str(rows.trim()).unwrap();
    assert_eq!(value["quality"], "unknown");
    assert!(value["sample"].is_null());
}
#[test]
fn source_observation_sidecar_keeps_actual_playback_state_without_remote_mapping() {
    let (_root, _host, mut owner) = setup();
    let sample: Clock = serde_json::from_value(json!({"clock":"mse_presentation_v1",
        "sourceId":Uuid::new_v4(),"sourceTimeSeconds":20.,"currentTime":22.,
        "timestampOffset":2.,"trackIndex":0,"performanceNowMs":100.,
        "paused":true,"seeking":false,"readyState":4,"ended":false,"playbackRate":1.25})).unwrap();
    let before = Instant::now();
    owner.input.clock(Uuid::new_v4(),before,Instant::now(),Some(&sample));
    owner.finish(false); let facts=owner.join();
    assert!(facts.joined && !facts.failure); assert_eq!(facts.timing_records,1);
    let rows=fs::read_to_string(owner.root.join("relay-timing.jsonl")).unwrap();
    let value:Value=serde_json::from_str(rows.trim()).unwrap();
    assert_eq!(value["sample"]["paused"],true); assert_eq!(value["sample"]["readyState"],4);
    assert_eq!(value["sample"]["playbackRate"],1.25);
    let summary:Value=serde_json::from_slice(&fs::read(owner.root.join("relay-summary.json")).unwrap()).unwrap();
    assert_eq!(summary["remoteArchiveMapping"],"unknown");
}
#[test]
fn end_receipt_accepted_during_chat_wait_is_written_before_join() {
    let (_root, _host, mut owner) = setup();
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    *lock(&owner.input.state.pause_after_timing_drain) = Some((entered_tx, release_rx));
    let observed = entered_rx.recv_timeout(Duration::from_secs(1)).is_ok();
    owner
        .input
        .record(json!({"version":1,"kind":"controlled_end_receipt"}));
    owner.finish(false);
    let _ = release_tx.send(());
    let facts = owner.join();
    assert!(observed);
    assert_eq!(facts.event_rows, 0);
    assert_eq!(facts.timing_records, 1);
    assert!(!facts.timing_incomplete);
    let rows = fs::read_to_string(owner.root.join("relay-timing.jsonl")).unwrap();
    assert_eq!(rows.lines().count(), 1);
    assert!(rows.contains("controlled_end_receipt"));
}
#[test]
fn asset_deadline_does_not_release_owner_before_real_fence() {
    let settled = Arc::new(AtomicBool::new(false));
    let waiting = Arc::new(AtomicBool::new(false));
    let source = settled.clone();
    let entered = waiting.clone();
    let worker = thread::spawn(move || {
        let mut incomplete = false;
        let mut first = true;
        let fence = wait_fence(
            Instant::now(),
            || {
                if first {
                    first = false;
                    entered.store(true, Ordering::Release);
                    None
                } else {
                    source.load(Ordering::Acquire).then_some(())
                }
            },
            &mut incomplete,
        );
        (fence, incomplete)
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while !waiting.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::yield_now();
    }
    let observed = waiting.load(Ordering::Acquire);
    let premature = worker.is_finished();
    settled.store(true, Ordering::Release);
    let (_, incomplete) = worker.join().unwrap();
    assert!(observed);
    assert!(!premature);
    assert!(incomplete);
}
fn setup() -> (tempfile::TempDir, OfficialBrowser, Owner) {
    let root = tempfile::tempdir().unwrap();
    let host = OfficialBrowser::new(root.path().to_owned()).unwrap();
    {
        let mut v = lock(&host.inner.view);
        v.open = true;
        v.ready = true;
        v.channel = Some("a".repeat(32));
        v.page_generation = 1;
    }
    let source = host
        .select_relay_source_by_channel(&"a".repeat(32))
        .unwrap();
    let owner = Owner::start(&host.inner, Uuid::new_v4(), &source, Uuid::new_v4()).unwrap();
    (root, host, owner)
}
fn batch(owner: &Owner, id: u64, events: Vec<Value>) {
    let bytes = serde_json::to_vec(&events).unwrap();
    owner
        .input
        .sender
        .try_send(Batch {
            id,
            events,
            bytes,
            received: now_ms(),
            receipt: Instant::now(),
            reply: None,
        })
        .unwrap();
}
fn public() -> Value {
    json!({"msgTypeCode":1,"msg":"controlled-public-fixture","profile":{"nickname":"fixture","userIdHash":"PRIVATE_RAW_ID","extraToken":"PRIVATE_TOKEN"},"credentials":"PRIVATE_COOKIE"})
}
#[test]
fn native_public_projection_excludes_page_private_fields_and_reopens() {
    let (_r, _h, mut o) = setup();
    batch(&o, 1, vec![public()]);
    o.finish(false);
    let result = o.join();
    assert!(result.joined && !result.failure);
    assert_eq!(result.event_rows, 1);
    let raw = fs::read_to_string(o.root.join("chat.events.jsonl")).unwrap();
    assert!(!raw.contains("PRIVATE_"));
    let rows = crate::streaming::chat_store::read_chat_page(&o.root, None).unwrap();
    assert_eq!(rows.items.len(), 1);
    assert_eq!(rows.items[0].text, "controlled-public-fixture");
    assert!(rows.items[0].sender_key.is_none());
    assert!(!o.root.join("video.mp4").exists());
    assert_eq!(
        result.clock_quality,
        "native_receipt_and_separate_source_observations"
    );
}
#[test]
fn duplicate_batch_exact_once_and_changed_duplicate_is_partial() {
    let (_r, _h, mut o) = setup();
    batch(&o, 1, vec![public()]);
    batch(&o, 1, vec![public()]);
    let mut altered = public();
    altered["msg"] = "changed".into();
    batch(&o, 1, vec![altered]);
    o.finish(false);
    let result = o.join();
    assert_eq!(result.event_rows, 1);
    assert!(result.failure);
    assert_eq!(result.text_state, "incomplete");
}
#[test]
fn ledger_overflow_qualifies_timing_without_stopping_chat() {
    let (_r, _h, mut o) = setup();
    o.input.record(json!({"oversized":"x".repeat(4097)}));
    batch(&o, 1, vec![public()]);
    o.finish(false);
    let result = o.join();
    assert!(result.timing_incomplete);
    assert!(!result.failure);
    assert_eq!(result.event_rows, 1);
    assert_eq!(result.text_state, "ended");
}
#[test]
fn explicit_gap_and_interrupt_are_durable_unknown_not_success() {
    let (_r, _h, mut o) = setup();
    o.input.partial();
    o.finish(true);
    let result = o.join();
    assert!(result.interrupted);
    assert_eq!(result.text_state, "incomplete");
    let proof: Value =
        serde_json::from_slice(&fs::read(o.root.join("relay-summary.json")).unwrap()).unwrap();
    assert_eq!(proof["summary"]["joined"], false);
    assert_eq!(proof["remoteArchiveMapping"], "unknown");
    assert_eq!(proof["binding"]["provider"], "chzzk");
}
