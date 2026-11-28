use super::*;

struct FetchGate {
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}
impl Drop for FetchGate {
    fn drop(&mut self) { self.release.store(true, Ordering::Release); }
}
fn until(check: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < end, "bounded asset fixture");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn source(index: usize) -> AssetSource {
    AssetSource::YoutubeEmoji(format!("https://yt3.ggpht.com/drain-fixture-{index}"))
}
fn session() -> (SessionAssets, FetchGate) {
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let (e, r, c) = (entered.clone(), release.clone(), calls.clone());
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |_| {
        c.fetch_add(1, Ordering::AcqRel);
        e.store(true, Ordering::Release);
        until(|| r.load(Ordering::Acquire));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2).write_to(&mut out, image::ImageFormat::Png).unwrap();
        blob(out.into_inner(), None)
    });
    (session, FetchGate { entered, release, calls })
}
fn stopped(session: &SessionAssets, gate: &FetchGate) {
    gate.release.store(true, Ordering::Release);
    session.stop();
    until(|| session.is_done());
}

#[test]
fn protected_pending_duplicate_after_stop_does_not_settle_or_refetch() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    until(|| gate.entered.load(Ordering::Acquire));
    session.stop();
    session.observe_sources([source(0)], Some(&target));
    assert!(target.pending());
    assert!(!session.is_done());
    gate.release.store(true, Ordering::Release);
    until(|| !target.pending());
    assert!(!owner.settle_cancellable(&AtomicBool::new(false)));
    assert!(session.protected_archive_drains_pending(), "owner survives image close for chat proof");
    assert_eq!(gate.calls.load(Ordering::Acquire), 1);
    drop(owner);
    assert!(!session.protected_archive_drains_pending());
    stopped(&session, &gate);
    assert_eq!(target.state.lock().unwrap().stored.len(), 1);
}

#[test]
fn stored_duplicate_after_stop_remains_complete() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    gate.release.store(true, Ordering::Release);
    until(|| !target.pending());
    session.stop();
    session.observe_sources([source(0)], Some(&target));
    assert!(!owner.settle_cancellable(&AtomicBool::new(false)));
    assert!(!session.incomplete());
    assert_eq!(gate.calls.load(Ordering::Acquire), 1);
    drop(owner);
    stopped(&session, &gate);
}

#[test]
fn unprotected_stored_duplicate_keeps_existing_session_stop_warning() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    session.observe_sources([source(0)], Some(&target));
    gate.release.store(true, Ordering::Release);
    until(|| !target.pending());
    session.stop();
    session.observe_sources([source(0)], Some(&target));
    assert!(session.incomplete(), "unprotected stop retains its prior session warning");
    assert!(!target.close(), "the already stored image itself remains complete");
    assert_eq!(gate.calls.load(Ordering::Acquire), 1);
    stopped(&session, &gate);
}

#[test]
fn stopped_new_id_is_missing_but_accepted_id_still_stores() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    until(|| gate.entered.load(Ordering::Acquire));
    session.stop();
    session.observe_sources([source(1)], Some(&target));
    assert!(target.pending());
    gate.release.store(true, Ordering::Release);
    until(|| !target.pending());
    assert!(owner.settle_cancellable(&AtomicBool::new(false)));
    assert_eq!(target.state.lock().unwrap().stored.len(), 1);
    assert_eq!(target.state.lock().unwrap().expected.len(), 2);
    assert_eq!(gate.calls.load(Ordering::Acquire), 1);
    drop(owner);
    stopped(&session, &gate);
}

#[test]
fn cancelled_target_does_not_cancel_parallel_unprotected_target() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let sibling_root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let sibling = ArchiveAssets::new(sibling_root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    session.observe_sources([source(0)], Some(&sibling));
    until(|| gate.entered.load(Ordering::Acquire));
    assert!(owner.settle_cancellable(&AtomicBool::new(true)));
    drop(owner);
    assert!(sibling.pending());
    assert!(!session.core.state.lock().unwrap().stopped);
    gate.release.store(true, Ordering::Release);
    until(|| !sibling.pending());
    assert!(!sibling.close());
    assert_eq!(target.state.lock().unwrap().stored.len(), 0);
    assert_eq!(sibling.state.lock().unwrap().stored.len(), 1);
    assert_eq!(gate.calls.load(Ordering::Acquire), 1);
    stopped(&session, &gate);
}

#[test]
fn deadline_closes_target_but_does_not_claim_active_fetch_joined() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    until(|| gate.entered.load(Ordering::Acquire));
    assert!(owner.settle_with_budget(&AtomicBool::new(false), Duration::from_millis(15)));
    assert!(!session.is_done());
    let manifest = fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap();
    drop(owner);
    stopped(&session, &gate);
    assert_eq!(fs::read(root.path().join("chat-assets/manifest.jsonl")).unwrap(), manifest);
    assert_eq!(target.state.lock().unwrap().stored.len(), 0);
}

#[test]
fn repeated_stop_does_not_extend_an_existing_drain_deadline() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    until(|| gate.entered.load(Ordering::Acquire));
    target.begin_drain(Instant::now() - Duration::from_secs(1), Duration::from_millis(1));
    session.stop();
    session.stop();
    assert!(owner.settle_with_budget(&AtomicBool::new(false), Duration::from_secs(2)));
    drop(owner);
    stopped(&session, &gate);
    assert_eq!(target.state.lock().unwrap().stored.len(), 0);
}

#[test]
fn dropped_guard_seals_only_its_target_and_releases_owner() {
    let (session, gate) = session();
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    until(|| gate.entered.load(Ordering::Acquire));
    drop(owner);
    assert!(!session.protected_archive_drains_pending());
    assert!(target.close());
    assert!(!session.is_done());
    stopped(&session, &gate);
    assert_eq!(target.state.lock().unwrap().stored.len(), 0);
}

#[test]
fn fetch_panic_settles_target_without_leaking_guard_or_worker() {
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(|_| panic!("controlled asset failure"));
    let root = tempfile::tempdir().unwrap();
    let target = ArchiveAssets::new(root.path()).unwrap();
    let mut owner = session.protect_archive(target.clone());
    session.observe_sources([source(0)], Some(&target));
    until(|| session.is_done());
    assert!(owner.settle_cancellable(&AtomicBool::new(false)));
    drop(owner);
    assert!(!session.protected_archive_drains_pending());
    assert!(session.incomplete());
}
