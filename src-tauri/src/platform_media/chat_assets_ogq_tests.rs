use super::*;
fn fixture() -> (SoopOgqMetadata, Vec<SoopOgqRequest>) {
    let f = [
        "room",
        "",
        "gem_fixture",
        "0",
        "unused",
        "private",
        "Public",
        "32|0",
        "0",
        "0",
        "0",
        "jpg",
        "0",
        "",
        "",
        "0",
        "0",
        "1",
    ];
    let parsed = crate::soop::chat::ogq::parse(&f).unwrap().unwrap();
    let mut meta = parsed.metadata.unwrap();
    for request in &parsed.requests {
        assert!(request.apply(&mut meta));
    }
    (meta, parsed.requests)
}
fn png() -> Blob {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    blob(bytes.into_inner(), None).unwrap()
}
fn until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !done() {
        assert!(Instant::now() < deadline, "bounded asset test timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn four_variants_dedup_are_local_only_and_survive_attachment_change() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |source| {
        assert!(matches!(source, AssetSource::SoopOgq(_)));
        assert!(source.valid());
        count.fetch_add(1, Ordering::AcqRel);
        Ok(png())
    });
    let (meta, requests) = fixture();
    let root = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(root.path()).unwrap();
    for _ in 0..8 {
        session.observe_ogq(&meta, &requests, Some(&archive));
    }
    until(|| archive.state.lock().unwrap().stored.len() == 4);
    assert_eq!(calls.load(Ordering::Acquire), 4);
    assert!(!archive.close());
    let next = tempfile::tempdir().unwrap();
    let next_archive = ArchiveAssets::new(next.path()).unwrap();
    session.observe_ogq(&meta, &[], Some(&next_archive));
    until(|| next_archive.state.lock().unwrap().stored.len() == 4);
    assert!(!next_archive.close());
    assert_eq!(calls.load(Ordering::Acquire), 4);
    session.stop();
    until(|| session.is_done());
    drop(next_archive);
    for id in meta.ids() {
        assert!(replay_asset(next.path(), id).unwrap().is_some());
    }
    let manifest = fs::read_to_string(next.path().join("chat-assets/manifest.jsonl")).unwrap();
    assert!(!manifest.contains("https://") && !manifest.contains("fixture"));
}
#[test]
fn stopped_unknown_and_queue_saturated_variants_never_claim_complete() {
    let (meta, requests) = fixture();
    for mode in 0..3 {
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(|_| panic!("no fetch admitted in this test"));
        let root = tempfile::tempdir().unwrap();
        let archive = ArchiveAssets::new(root.path()).unwrap();
        if mode == 0 {
            session.stop();
        } else if mode == 2 {
            session.started.store(true, Ordering::Release);
            let mut state = session.core.state.lock().unwrap();
            for n in 0..64 {
                state.queue.push_back(format!("blocked-{n}"));
            }
        }
        session.observe_ogq(
            &meta,
            if mode == 1 { &[] } else { &requests },
            Some(&archive),
        );
        assert_eq!(archive.state.lock().unwrap().expected.len(), 4);
        assert!(archive.close());
        session.stop();
    }
}
#[test]
fn in_flight_variants_cannot_write_a_detached_archive() {
    let mut session = SessionAssets::new();
    let started = Arc::new(AtomicUsize::new(0));
    let observed = started.clone();
    let barrier = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_barrier = barrier.clone();
    session.fetcher = Arc::new(move |_| {
        observed.fetch_add(1, Ordering::AcqRel);
        let (lock, wake) = &*worker_barrier;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
        Ok(png())
    });
    let (meta, requests) = fixture();
    let root = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(root.path()).unwrap();
    session.observe_ogq(&meta, &requests, Some(&archive));
    until(|| started.load(Ordering::Acquire) == 1);
    assert!(archive.close());
    let before = fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap();
    let (lock, wake) = &*barrier;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    until(|| {
        let state = session.core.state.lock().unwrap();
        state.entries.len() == 4 && state.entries.values().all(|entry| entry.attempted)
    });
    assert_eq!(started.load(Ordering::Acquire), 4);
    session.stop();
    until(|| session.is_done());
    assert_eq!(
        fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap(),
        before
    );
    assert_eq!(archive.state.lock().unwrap().expected.len(), 4);
    assert!(archive.state.lock().unwrap().stored.is_empty());
}
