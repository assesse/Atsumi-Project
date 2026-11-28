use super::*;
fn gift(version: &str) -> (SoopGiftMetadata, SoopGiftRequest) {
    let fields = [
        "fixture",
        "sender-never-saved",
        "Public",
        "50",
        "0",
        "",
        "",
        "50",
        "0",
        "0",
        "",
        "",
        "ko_KR",
        version,
    ];
    let request = SoopGiftRequest::from_fields(18, &fields, "fixture").unwrap();
    let mut gift = crate::soop::chat::gifts::from_stored(
        &serde_json::json!({"kind":"balloon","count":50,"relay":false,"isDefault":false}),
    )
    .unwrap();
    gift.image_asset_id = Some(request.id().into());
    (gift, request)
}
fn png() -> Blob {
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    blob(output.into_inner(), Some("image/png")).unwrap()
}
fn until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "bounded custom image test timeout"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn typed_custom_images_deduplicate_store_once_and_replay_only_local_bytes() {
    assert_eq!(asset_id("https://yt3.ggpht.com/fixture-1"), "25520d0c218237d5420615d75eb93b5f427a2b5b6ac021b9e564a17c11c91210");
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |source| {
        assert!(matches!(source, AssetSource::SoopGift(_)));
        assert!(source.valid());
        count.fetch_add(1, Ordering::AcqRel);
        Ok(png())
    });
    let root = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(root.path()).unwrap();
    let (first, request) = gift("123");
    let (second, request2) = gift("124");
    assert_ne!(request.id(), request2.id());
    assert_ne!(request.id(), asset_id(request.url()));
    assert!(sanitize_url(request.url()).is_none());
    assert!(crate::soop::chat::gifts::assets::canonical_url("https://yt3.ggpht.com/a").is_none());
    for _ in 0..20 {
        session.observe_gift(&first, Some(&request), Some(&archive));
    }
    session.observe_gift(&second, Some(&request2), Some(&archive));
    until(|| archive.state.lock().unwrap().stored.len() == 2);
    assert_eq!(calls.load(Ordering::Acquire), 2);
    assert!(!archive.close());
    session.stop();
    until(|| session.is_done());
    drop(archive);
    let expected = data_url(&png());
    assert_eq!(
        replay_asset(root.path(), request.id()).unwrap(),
        Some(expected.clone())
    );
    assert_eq!(
        replay_asset(root.path(), request2.id()).unwrap(),
        Some(expected)
    );
    let manifest = fs::read_to_string(root.path().join("chat-assets/manifest.jsonl")).unwrap();
    assert!(!manifest.contains("https://") && !manifest.contains("fixture"));
}
#[test]
fn delayed_custom_image_cannot_write_closed_archive_and_can_join_new_attachment() {
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let entered_worker = entered.clone();
    let release_worker = release.clone();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |_| {
        entered_worker.store(true, Ordering::Release);
        until(|| release_worker.load(Ordering::Acquire));
        Ok(png())
    });
    let old = tempfile::tempdir().unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let first = ArchiveAssets::new(old.path()).unwrap();
    let next = ArchiveAssets::new(fresh.path()).unwrap();
    let (gift, request) = gift("");
    session.observe_gift(&gift, Some(&request), Some(&first));
    until(|| entered.load(Ordering::Acquire));
    assert!(first.close());
    let closed_manifest = fs::read(old.path().join("chat-assets/manifest.jsonl")).unwrap();
    // A recent normalized event carries only the opaque ID; attach can reuse
    // this session's already validated source, without accepting a caller URL.
    session.observe_gift(&gift, None, Some(&next));
    release.store(true, Ordering::Release);
    until(|| next.state.lock().unwrap().stored.len() == 1);
    assert!(!old.path().join("chat-assets").join(request.id()).exists());
    assert_eq!(
        fs::read(old.path().join("chat-assets/manifest.jsonl")).unwrap(),
        closed_manifest
    );
    assert!(!next.close());
    session.stop();
    until(|| session.is_done());
}
#[test]
fn caller_urls_and_unknown_ids_do_not_schedule_fetches_and_missing_image_is_honest() {
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(|_| panic!("untrusted image must never be fetched"));
    let root = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(root.path()).unwrap();
    let (gift, request) = gift("");
    session.observe(
        &[ChatPart::Emoji {
            asset_id: asset_id(request.url()),
            alt: String::new(),
            source_url: Some(request.url().into()),
        }],
        Some(&archive),
    );
    assert!(session.core.state.lock().unwrap().queue.is_empty());
    session.observe_gift(&gift, None, Some(&archive));
    assert!(archive.close());
    assert!(session.incomplete());
    assert!(session.core.state.lock().unwrap().entries.is_empty());
    session.stop();
}
#[test]
fn failed_or_oversized_custom_image_preserves_expected_asset_failure() {
    for oversized in [false, true] {
        let session = SessionAssets::with_test_fetcher(move |_| {
            if oversized {
                Ok(vec![0; BODY + 1])
            } else {
                Err(ERROR.into())
            }
        });
        let root = tempfile::tempdir().unwrap();
        let archive = ArchiveAssets::new(root.path()).unwrap();
        let (gift, request) = gift("");
        session.observe_gift(&gift, Some(&request), Some(&archive));
        until(|| session.incomplete());
        assert_eq!(archive.state.lock().unwrap().expected.len(), 1);
        assert!(archive.close());
        assert!(session.read(request.id()).unwrap().is_none());
        session.stop();
        until(|| session.is_done());
    }
}
