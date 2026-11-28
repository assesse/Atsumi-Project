use super::*;
use crate::soop::chat::gifts::assets::Request;

struct Fixture {
    chat: Arc<SharedChat>,
    root: tempfile::TempDir,
    id: String,
    request: Request,
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.release.store(true, Ordering::Release);
        self.chat.stop();
        let end = Instant::now() + Duration::from_secs(3);
        while !self.chat.assets.is_done() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
fn until(check: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < end, "bounded recording fixture");
        std::thread::sleep(Duration::from_millis(2));
    }
}
impl Fixture {
    fn new() -> Self {
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let (e, r) = (entered.clone(), release.clone());
        let assets = SessionAssets::with_test_fetcher(move |_| {
            e.store(true, Ordering::Release);
            until(|| r.load(Ordering::Acquire));
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2).write_to(&mut png, image::ImageFormat::Png).unwrap();
            Ok(png.into_inner())
        });
        let chat = SharedChat::with_test_assets(assets);
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let request = Request::from_fields(18, &["fixture", "private", "Public", "50", "0", "", "", "50", "0", "0"], "fixture").unwrap();
        chat.attach_recording_archive(root.path(), "soop", &id).unwrap();
        let result = Self { chat, root, id, request, entered, release };
        result.ingest("first");
        until(|| result.entered.load(Ordering::Acquire));
        result
    }
    fn ingest(&self, id: &str) {
        let raw = serde_json::to_vec(&serde_json::json!({
            "kind":"donation", "receivedAtMs":1700000001000u64, "messageId":id,
            "nickname":"Public", "text":"", "extra":{"service":18, "fileName":"50",
            "soopGift":{"kind":"balloon", "count":50, "relay":false, "isDefault":false}}
        })).unwrap();
        self.chat.ingest_with_gift("soop", &raw, Some(self.request.clone())).unwrap();
    }
}

#[test]
fn receiver_end_before_detach_preserves_admitted_asset_through_proof() {
    let fixture = Fixture::new();
    // Actual receiver RAII path, not only the explicit SharedChat.stop method.
    drop(ReceiverDone(fixture.chat.clone()));
    let outcome = fixture.chat.detach_archive().unwrap().unwrap();
    assert!(fixture.chat.pending_soop_archive_assets());
    assert!(!fixture.chat.assets.is_done());
    fixture.release.store(true, Ordering::Release);
    let summary = outcome.persist(fixture.root.path(), "soop", &fixture.id).unwrap();
    assert!(!summary.assets_incomplete);
    assert!(!fixture.chat.pending_soop_archive_assets());
    until(|| fixture.chat.assets.is_done());
    assert!(fixture.root.path().join("chat-assets").join(fixture.request.id()).exists());
}

#[test]
fn attached_owner_is_aborted_without_waiting_for_missing_finalizer() {
    let fixture = Fixture::new();
    fixture.chat.prepare_soop_archive_asset_shutdown();
    assert!(!fixture.chat.pending_soop_archive_assets());
    assert!(!fixture.chat.assets.is_done());
    let outcome = fixture.chat.detach_archive().unwrap().unwrap();
    assert!(outcome.persist(fixture.root.path(), "soop", &fixture.id).unwrap().assets_incomplete);
    fixture.release.store(true, Ordering::Release);
    fixture.chat.stop();
    until(|| fixture.chat.assets.is_done());
    assert!(!fixture.root.path().join("chat-assets").join(fixture.request.id()).exists());
}

#[test]
fn invalid_detach_identity_drops_guard_and_seals_partial_target() {
    let fixture = Fixture::new();
    let outcome = fixture.chat.detach_archive().unwrap().unwrap();
    assert!(outcome.persist(fixture.root.path(), "soop", &uuid::Uuid::new_v4().to_string()).is_err());
    assert!(!fixture.chat.pending_soop_archive_assets());
    let status: serde_json::Value = serde_json::from_slice(&fs::read(fixture.root.path().join("chat-assets/status.json")).unwrap()).unwrap();
    assert_eq!(status["incomplete"], true);
    assert_eq!(status["stored"], 0);
}

#[test]
fn detached_owner_and_new_archive_remain_separate() {
    let fixture = Fixture::new();
    let old = fixture.chat.detach_archive().unwrap().unwrap();
    let next = tempfile::tempdir().unwrap();
    let next_id = uuid::Uuid::new_v4().to_string();
    fixture.chat.attach_recording_archive(next.path(), "soop", &next_id).unwrap();
    fixture.ingest("second");
    drop(old); // failed/spawn-abandoned first finalizer cannot abort next target
    assert!(fixture.chat.pending_soop_archive_assets());
    fixture.release.store(true, Ordering::Release);
    let summary = fixture.chat.detach_archive().unwrap().unwrap().persist(next.path(), "soop", &next_id).unwrap();
    assert!(!summary.assets_incomplete);
    assert!(!fixture.chat.pending_soop_archive_assets());
    assert!(!fixture.root.path().join("chat-assets").join(fixture.request.id()).exists());
    assert!(next.path().join("chat-assets").join(fixture.request.id()).exists());
}
