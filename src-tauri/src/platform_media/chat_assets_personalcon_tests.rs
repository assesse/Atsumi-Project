use super::*;
fn fixture() -> crate::soop::chat::personalcon::Request {
    crate::soop::chat::personalcon::Context::from_channel(&serde_json::json!({"PCON_OBJECT":{"tier1":[{"MONTH":0,"FILENAME":"https://static.file.sooplive.com/spcon/373867c54c3ee5ae5.png?_=1741052109"}]}})).select("0|262144","17","0").0.unwrap()
}
fn png() -> Blob {
    let mut b = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(3, 3)
        .write_to(&mut b, image::ImageFormat::Png)
        .unwrap();
    blob(b.into_inner(), None).unwrap()
}
fn until(done: impl Fn() -> bool) {
    let limit = Instant::now() + Duration::from_secs(3);
    while !done() {
        assert!(Instant::now() < limit);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn personalcon_capture_dedups_connection_and_replays_only_immutable_local_bytes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |source| {
        assert!(matches!(source, AssetSource::SoopPersonalcon(_)));
        assert!(source.valid());
        c.fetch_add(1, Ordering::AcqRel);
        Ok(png())
    });
    let request = fixture();
    let dir = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(dir.path()).unwrap();
    for _ in 0..4 {
        session.observe_personalcon(request.metadata(), Some(&request), Some(&archive));
    }
    until(|| archive.state.lock().unwrap().stored.len() == 1);
    assert!(!archive.close());
    session.stop();
    until(|| session.is_done());
    assert_eq!(calls.load(Ordering::Acquire), 1);
    drop(archive);
    let before = fs::read(dir.path().join("chat-assets/manifest.jsonl")).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains("https://"));
    assert!(replay_asset(dir.path(), request.id()).unwrap().is_some());
    assert_eq!(
        before,
        fs::read(dir.path().join("chat-assets/manifest.jsonl")).unwrap()
    );
}
#[test]
fn personalcon_unknown_capability_or_stopped_capture_marks_missing_without_fetch() {
    for stopped in [false, true] {
        let mut session = SessionAssets::new();
        session.fetcher = Arc::new(|_| panic!("no fetch"));
        let request = fixture();
        let dir = tempfile::tempdir().unwrap();
        let archive = ArchiveAssets::new(dir.path()).unwrap();
        if stopped {
            session.stop();
        }
        session.observe_personalcon(
            request.metadata(),
            if stopped { Some(&request) } else { None },
            Some(&archive),
        );
        assert!(archive.close());
        session.stop();
        until(|| session.is_done());
    }
}
#[test]
fn broadcaster_avatar_jpeg_is_archived_and_read_without_current_profile_lookup() {
    let context = crate::soop::chat::personalcon::Context::from_channel(&serde_json::json!({}));
    let request = context
        .portrait("fixture", "fixture(1)", "4|0", "0")
        .unwrap();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(|source| {
        assert!(matches!(source, AssetSource::SoopAvatar(_)));
        assert!(source.valid());
        let mut b = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(3, 3)
            .write_to(&mut b, image::ImageFormat::Jpeg)
            .unwrap();
        blob(b.into_inner(), Some("image/jpeg"))
    });
    let dir = tempfile::tempdir().unwrap();
    let archive = ArchiveAssets::new(dir.path()).unwrap();
    session.observe_avatar(request.metadata(), Some(&request), Some(&archive));
    until(|| archive.state.lock().unwrap().stored.len() == 1);
    assert!(!archive.close());
    session.stop();
    until(|| session.is_done());
    drop(archive);
    assert!(replay_asset(dir.path(), request.id()).unwrap().is_some());
    assert!(
        !fs::read_to_string(dir.path().join("chat-assets/manifest.jsonl"))
            .unwrap()
            .contains("https://")
    );
}

fn png_response_roundtrip(response: Vec<u8>) -> serde_json::Value {
    let sha = format!("{:x}", Sha256::digest(&response));
    let bytes = response.len();
    let calls = Arc::new(AtomicUsize::new(0));
    let fetch_calls = calls.clone();
    let fetched = response.clone();
    let mut session = SessionAssets::new();
    session.fetcher = Arc::new(move |source| {
        assert!(matches!(source, AssetSource::SoopPersonalcon(_)));
        assert!(source.valid());
        fetch_calls.fetch_add(1, Ordering::AcqRel);
        blob(fetched.clone(), Some("image/png"))
    });
    // Two independent targets exercise the shared recording/relay asset fanout.
    // Their event identity is controlled; no historical event is reconstructed.
    let directories = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let targets: Vec<_> = directories
        .iter()
        .map(|directory| ArchiveAssets::new(directory.path()).unwrap())
        .collect();
    let request = fixture();
    for _ in 0..2 {
        for target in &targets {
            session.observe_personalcon(request.metadata(), Some(&request), Some(target));
        }
    }
    until(|| targets.iter().all(|target| !target.pending()));
    session.stop();
    until(|| session.is_done());
    assert!(!session.incomplete());
    assert_eq!(calls.load(Ordering::Acquire), 1);
    for target in &targets {
        assert!(!target.close());
    }
    drop(targets); // Release manifest handles before immutable replay reads on Windows.
    for directory in &directories {
        let assets = directory.path().join("chat-assets");
        assert_eq!(fs::read(assets.join(request.id())).unwrap(), response);
        let manifest = fs::read(assets.join("manifest.jsonl")).unwrap();
        let entry: ManifestEntry = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(entry.id, request.id());
        assert_eq!(entry.sha256, sha);
        assert_eq!(entry.bytes, bytes);
        assert_eq!(entry.mime, "image/png");
        assert!(!String::from_utf8_lossy(&manifest).contains("https://"));
        let replay = replay_asset(directory.path(), request.id()).unwrap().unwrap();
        let encoded = replay.strip_prefix("data:image/png;base64,").unwrap();
        assert_eq!(STANDARD.decode(encoded).unwrap(), response);
        assert_eq!(fs::read(assets.join("manifest.jsonl")).unwrap(), manifest);
    }
    serde_json::json!({"bytes":bytes,"sha256":sha,"fetches":1,"targets":2,
        "storedAndReplayedExactResponse":true,"incomplete":false})
}

#[test]
fn personalcon_png_trailer_keeps_exact_bytes_in_both_local_asset_targets() {
    let mut response = png().bytes;
    response.extend_from_slice(&[0xff; 176]);
    png_response_roundtrip(response);
}

#[test]
#[ignore = "Opt-in offline public-response fixture; no network or old sidecar writes"]
fn observed_public_personalcon_pngs_roundtrip_without_reencoding() {
    assert_eq!(std::env::var("ATSUMI_PERSONALCON_PNG_QA").as_deref(), Ok("1"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let audit = root.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    let source = audit.join("personalcon-fc76-diagnostic-3d535249590848799a08fac8bca6d1a5");
    let expected = [
        ("asset-0.bin", "c458e1157fa39cab51db001469a48701f97caf7ee1cc377f0602561f08742833", 683),
        ("asset-1.bin", "c61bf357abb85b2e97b6f6a3a95f670816b7e3ccabf6898c0ff1bb7af7eb22d9", 714),
        ("asset-2.bin", "10832637ea36b3789f5710b47060c43b994651002463e92d579e5d12e0c50fc7", 866),
    ];
    let mut facts = Vec::new();
    for (file_name, expected_sha, png_end) in expected {
        let path = source.join(file_name);
        let _parents = pin_parents(&path).unwrap();
        storage::regular(&path).unwrap();
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let held = options.open(&path).unwrap();
        let mut response = Vec::new();
        held.try_clone().unwrap().take((BODY + 1) as u64).read_to_end(&mut response).unwrap();
        assert!(response.len() <= BODY && response.len() > png_end);
        assert_eq!(format!("{:x}", Sha256::digest(&response)), expected_sha);
        let whole = image::load_from_memory(&response).unwrap().to_rgba8();
        let stream = image::load_from_memory(&response[..png_end]).unwrap().to_rgba8();
        assert_eq!(whole.dimensions(), (36, 36));
        assert_eq!(whole, stream);
        let mut fact = png_response_roundtrip(response.clone());
        fact["file"] = file_name.into();
        fact["width"] = 36.into();
        fact["height"] = 36.into();
        fact["pngDatastreamBytes"] = png_end.into();
        fact["trailingBytes"] = (response.len() - png_end).into();
        fact["decodedPixelsSha256"] = format!("{:x}", Sha256::digest(whole.as_raw())).into();
        assert_eq!(read_owned(&path, BODY).unwrap(), response);
        drop(held);
        facts.push(fact);
    }
    let output = audit.join(format!("personalcon-png-offline-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&output).unwrap();
    let report = serde_json::json!({"version":1,"passed":true,
        "classification":"controlled-native-persistence-of-current-public-responses",
        "assets":facts,"originalResponsesUnchanged":true,"networkRequests":0,
        "historicalFc76ResponsesRestored":false,"newOfficialEventCapture":false});
    let result = output.join("result.json");
    create(&result).unwrap().write_all(&serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{}", result.display());
}
