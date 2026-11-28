use super::*;
fn fixture(code: &str, version: &str) -> (SoopSubscriptionMetadata, Vec<SoopSubscriptionRequest>) {
    let fields = [
        "room", "receiver", "sender", "Public", code, "", "", "1", "", "ko_KR", version,
    ];
    let mut parsed = crate::soop::chat::subscriptions::parse(91, &fields, None).unwrap();
    for request in &parsed.requests {
        assert!(request.apply(&mut parsed.metadata));
    }
    (parsed.metadata, parsed.requests)
}
fn png() -> Blob {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    blob(bytes.into_inner(), None).unwrap()
}
fn until(done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !done() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn subscriptions_use_distinct_native_source_dedup_and_local_archive_reuse() {
    for (code, version, expected) in [("103", "123", 2), ("100", "", 1)] {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(move |source| {
            assert!(matches!(source, AssetSource::SoopSubscription(_)));
            assert!(source.valid());
            count.fetch_add(1, Ordering::AcqRel);
            Ok(png())
        });
        let (meta, requests) = fixture(code, version);
        let first = tempfile::tempdir().unwrap();
        let archive = ArchiveAssets::new(first.path()).unwrap();
        for _ in 0..5 {
            session.observe_subscription(&meta, &requests, Some(&archive));
        }
        until(|| archive.state.lock().unwrap().stored.len() == expected);
        assert!(!archive.close());
        assert_eq!(calls.load(Ordering::Acquire), expected);
        let second = tempfile::tempdir().unwrap();
        let next = ArchiveAssets::new(second.path()).unwrap();
        session.observe_subscription(&meta, &[], Some(&next));
        until(|| next.state.lock().unwrap().stored.len() == expected);
        assert!(!next.close());
        session.stop();
        until(|| session.is_done());
        assert_eq!(calls.load(Ordering::Acquire), expected);
        drop(next);
        for id in meta.ids() {
            assert!(replay_asset(second.path(), id).unwrap().is_some());
        }
        assert!(
            !fs::read_to_string(second.path().join("chat-assets/manifest.jsonl"))
                .unwrap()
                .contains("https://")
        );
    }
}
#[test]
fn subscription_stop_unknown_or_missing_primary_always_marks_incomplete() {
    for mode in 0..3 {
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(|_| panic!("no request should run"));
        let (mut meta, requests) = fixture("103", "123");
        let dir = tempfile::tempdir().unwrap();
        let archive = ArchiveAssets::new(dir.path()).unwrap();
        if mode == 0 {
            session.stop();
        } else if mode == 2 {
            meta.image_asset_id = None;
        }
        session.observe_subscription(
            &meta,
            if mode == 0 { &requests } else { &[] },
            Some(&archive),
        );
        assert!(archive.close());
        session.stop();
    }
}
#[test]
fn subscription_in_flight_requests_cannot_write_after_archive_detach() {
    let mut session = SessionAssets::new();
    let started = Arc::new(AtomicUsize::new(0));
    let observed = started.clone();
    let barrier = Arc::new((Mutex::new(false), Condvar::new()));
    let worker = barrier.clone();
    session.fetcher = Arc::new(move |_| {
        observed.fetch_add(1, Ordering::AcqRel);
        let (lock, wake) = &*worker;
        let mut ready = lock.lock().unwrap();
        while !*ready {
            ready = wake.wait(ready).unwrap();
        }
        Ok(png())
    });
    let (meta, requests) = fixture("103", "123");
    let root = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(root.path()).unwrap();
    session.observe_subscription(&meta, &requests, Some(&archive));
    until(|| started.load(Ordering::Acquire) == 1);
    assert!(archive.close());
    let before = fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap();
    let (lock, wake) = &*barrier;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    until(|| {
        session
            .core
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .all(|entry| entry.attempted)
    });
    session.stop();
    until(|| session.is_done());
    assert_eq!(started.load(Ordering::Acquire), 2);
    assert_eq!(
        fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap(),
        before
    );
    assert!(archive.state.lock().unwrap().stored.is_empty());
}
