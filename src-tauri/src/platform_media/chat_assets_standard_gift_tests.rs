use super::*;
use crate::soop::chat::gifts::assets::Context;

fn gift(context: &Context) -> (SoopGiftMetadata, SoopGiftRequest) {
    let fields = ["fixture", "private", "Public", "211", "0", "", "", "", "1", "0"];
    let request = context.from_fields(18, &fields, "fixture").unwrap();
    let mut gift = crate::soop::chat::gifts::from_stored(&serde_json::json!({
        "kind":"balloon", "count":211, "relay":false, "isDefault":true,
    })).unwrap();
    assert!(request.apply(&mut gift));
    (gift, request)
}
fn png(value: u8) -> Blob {
    let image = image::RgbImage::from_pixel(3, 3, image::Rgb([value, 0, 0]));
    let mut data = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image).write_to(&mut data, image::ImageFormat::Png).unwrap();
    blob(data.into_inner(), Some("image/png")).unwrap()
}
fn until(done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !done() { assert!(Instant::now() < end, "bounded standard-art fixture"); std::thread::sleep(Duration::from_millis(2)); }
}

#[test]
fn same_url_in_two_connections_preserves_distinct_response_bytes_and_local_replay() {
    let (first, a) = gift(&Context::new()); let (second, b) = gift(&Context::new());
    assert_eq!(a.url(), b.url()); assert_ne!(a.id(), b.id());
    let calls = Arc::new(AtomicUsize::new(0)); let counter = calls.clone();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |source| {
        assert!(source.valid());
        assert_eq!(source.url(), "https://res.sooplive.com/new_player/items/ba_step4.png");
        Ok(png(if counter.fetch_add(1, Ordering::AcqRel) == 0 { 20 } else { 220 }))
    });
    let root = tempfile::tempdir().unwrap(); let target = ArchiveAssets::new(root.path()).unwrap();
    for _ in 0..4 { session.observe_gift(&first, Some(&a), Some(&target)); }
    until(|| target.state.lock().unwrap().stored.contains(a.id()));
    session.observe_gift(&second, Some(&b), Some(&target));
    until(|| target.state.lock().unwrap().stored.len() == 2);
    assert_eq!(calls.load(Ordering::Acquire), 2); assert!(!target.close());
    session.stop(); until(|| session.is_done()); drop(target);
    assert_eq!(replay_asset(root.path(), a.id()).unwrap(), Some(data_url(&png(20))));
    assert_eq!(replay_asset(root.path(), b.id()).unwrap(), Some(data_url(&png(220))));
    let manifest = fs::read_to_string(root.path().join("chat-assets/manifest.jsonl")).unwrap();
    assert_eq!(manifest.lines().count(), 2);
    assert!(!manifest.contains("https://") && !manifest.contains("fixture"));
}

#[test]
fn legacy_missing_id_and_caller_data_never_authorize_standard_fetch() {
    let mut session = SessionAssets::new(); session.fetcher = Arc::new(|_| panic!("no capture capability"));
    let root = tempfile::tempdir().unwrap(); let target = ArchiveAssets::new(root.path()).unwrap();
    let (mut legacy, request) = gift(&Context::new()); legacy.image_asset_id = None;
    session.observe_gift(&legacy, None, Some(&target));
    assert!(!session.incomplete()); assert!(target.state.lock().unwrap().expected.is_empty());
    session.observe(&[ChatPart::Emoji { asset_id:asset_id(request.url()), alt:String::new(), source_url:Some(request.url().into()) }], Some(&target));
    assert!(session.core.state.lock().unwrap().entries.is_empty());
    legacy.image_asset_id = Some(request.id().into());
    session.observe_gift(&legacy, None, Some(&target));
    assert!(target.close()); assert!(session.incomplete());
    assert!(session.core.state.lock().unwrap().entries.is_empty()); session.stop();
}

#[test]
fn delayed_standard_image_fanout_keeps_protected_target_and_never_writes_closed_target() {
    let entered = Arc::new(AtomicBool::new(false)); let release = Arc::new(AtomicBool::new(false));
    let (e, r) = (entered.clone(), release.clone());
    let mut session = SessionAssets::new(); session.fetcher = Arc::new(move |_| {
        e.store(true, Ordering::Release); until(|| r.load(Ordering::Acquire)); Ok(png(30))
    });
    let old = tempfile::tempdir().unwrap(); let next = tempfile::tempdir().unwrap();
    let closed = ArchiveAssets::new(old.path()).unwrap(); let protected = ArchiveAssets::new(next.path()).unwrap();
    let mut owner = session.protect_archive(protected.clone());
    let (gift, request) = gift(&Context::new());
    session.observe_gift(&gift, Some(&request), Some(&closed)); until(|| entered.load(Ordering::Acquire));
    session.observe_gift(&gift, None, Some(&protected)); // only an already admitted typed cache entry
    assert!(closed.close()); let before = fs::read(old.path().join("chat-assets/manifest.jsonl")).unwrap();
    session.stop();
    session.observe_gift(&gift, Some(&request), Some(&protected)); // duplicate must not settle it early
    assert!(protected.pending()); release.store(true, Ordering::Release);
    assert!(!owner.settle_cancellable(&AtomicBool::new(false)));
    drop(owner); until(|| session.is_done());
    assert!(next.path().join("chat-assets").join(request.id()).exists());
    assert!(!old.path().join("chat-assets").join(request.id()).exists());
    assert_eq!(fs::read(old.path().join("chat-assets/manifest.jsonl")).unwrap(), before);
}

#[test]
fn failed_standard_response_stays_incomplete_and_is_never_refetched() {
    for oversized in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0)); let counter = calls.clone();
        let session = SessionAssets::with_test_fetcher(move |_| {
            counter.fetch_add(1, Ordering::AcqRel);
            if oversized { Ok(vec![0; BODY + 1]) } else { Err(ERROR.into()) }
        });
        let root = tempfile::tempdir().unwrap(); let target = ArchiveAssets::new(root.path()).unwrap();
        let (gift, request) = gift(&Context::new());
        session.observe_gift(&gift, Some(&request), Some(&target));
        until(|| !target.pending());
        session.observe_gift(&gift, Some(&request), Some(&target));
        assert_eq!(calls.load(Ordering::Acquire), 1); assert!(target.close());
        assert!(session.read(request.id()).unwrap().is_none());
        session.stop(); until(|| session.is_done());
    }
}
