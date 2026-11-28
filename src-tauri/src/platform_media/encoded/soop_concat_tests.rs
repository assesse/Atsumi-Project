//! Local native finalizer coverage. Real bytes are read-only; all writes go to
//! a new isolated test service. No provider worker, profile lookup or upload.
use super::*;
use std::{collections::BTreeSet, time::Instant};

struct ServiceStop(PlatformMediaService);
impl Drop for ServiceStop {
    fn drop(&mut self) {
        self.0.shutdown();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            if self
                .0
                .inner
                .state
                .lock()
                .map(|s| s.cancel.is_empty())
                .unwrap_or(false)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn archive(
    service: &PlatformMediaService,
    root: &Path,
    tracks: Vec<EncodedTrack>,
) -> Result<(OfficialReceiverLease, EncodedArchive), String> {
    service
        .inner
        .synthetic_live_receiver
        .store(true, Ordering::Release);
    let lease = service
        .reserve_official_receiver(
            MediaRef {
                provider: "soop".into(),
                kind: "live".into(),
                external_id: "fixture".into(),
            },
            "123".into(),
            uuid::Uuid::new_v4().to_string(),
            root.join("media"),
        )
        .map_err(|error| format!("QA_RESERVE_RECEIVER: {error}"))?;
    let archive = lease
        .begin_encoded(
            uuid::Uuid::new_v4().to_string(),
            1,
            uuid::Uuid::new_v4().to_string(),
            tracks,
            "Source-clock finalizer fixture".into(),
            "Fixture".into(),
            live::RecordingOwner::Manual,
        )
        .map_err(|error| format!("QA_BEGIN_ENCODED: {error}"))?;
    Ok((lease, archive))
}

#[test]
fn prepare_rejects_changed_second_init_with_valid_piece_hashes() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("no-network-or-media-tool.exe");
    let service = PlatformMediaService::new(
        root.path().join("data"),
        PlatformMediaTools {
            yt_dlp: missing.clone(),
            deno: missing.clone(),
            ffmpeg: missing.clone(),
            ffprobe: missing,
        },
        None,
    )
    .unwrap();
    let _stop = ServiceStop(service.clone());
    let (_lease, mut recording) = archive(&service, root.path(), fmp4::fixtures::inputs()).unwrap();
    let writer = recording.writer.as_mut().unwrap();
    for track in [0, 1] {
        let segments = writer
            .muxer
            .push(track, &fmp4::fixtures::fragment(track, 0, 24, true))
            .unwrap();
        writer.commit(segments).unwrap();
    }
    let tail = writer.muxer.finish().unwrap();
    writer.commit(tail).unwrap();
    assert_eq!(writer.segments.len(), 1);
    let first_path = writer.directory.join("e00000000.mp4");
    let first = fs::read(&first_path).unwrap();
    let first_sha = storage::hash(&first_path, &writer.cancel).unwrap();
    let mut changed = first.clone();
    let offset = changed
        .windows(4)
        .position(|value| value == b"avcC")
        .unwrap()
        + 5;
    changed[offset] ^= 1;
    writer
        .commit(vec![fmp4::EncodedSegment {
            bytes: changed,
            duration_seconds: 12.0,
            source_start_seconds: 12.0,
            source_end_seconds: 24.0,
        }])
        .unwrap();
    mark(writer).unwrap();
    // The part and STOP proofs are valid. Only the independently pinned init
    // comparison rejects this changed codec epoch, before any merge can run.
    preflight(
        &service,
        &writer.job,
        &writer.directory,
        &writer.cancel,
        &writer.segments,
        0,
    )
    .unwrap();
    assert!(prepare(
        &service,
        &writer.job,
        &writer.directory,
        &writer.cancel,
        &writer.segments,
        0
    )
    .is_err());
    assert_eq!(
        storage::hash(&first_path, &writer.cancel).unwrap(),
        first_sha
    );
    assert!(writer.directory.join("e00000001.mp4").is_file());
    assert!(!writer.directory.join("media.mp4").exists());
}

#[cfg(windows)]
fn source_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut files = BTreeSet::new();
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    while let Some((directory, depth)) = pending.pop() {
        storage::safe_path(&directory)?;
        if depth > 8 {
            return Err("QA_SOURCE_DEPTH".into());
        }
        for entry in fs::read_dir(directory).map_err(|_| "QA_SOURCE_LIST")? {
            visited += 1;
            if visited > 80 {
                return Err("QA_SOURCE_COUNT".into());
            }
            let path = entry.map_err(|_| "QA_SOURCE_LIST")?.path();
            storage::safe_path(&path)?;
            let metadata = fs::symlink_metadata(&path).map_err(|_| "QA_SOURCE_STAT")?;
            if metadata.is_dir() {
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                storage::regular(&path)?;
                files.insert(
                    path.strip_prefix(root)
                        .map_err(|_| "QA_SOURCE_PATH")?
                        .to_path_buf(),
                );
            } else {
                return Err("QA_SOURCE_KIND".into());
            }
        }
    }
    Ok(files)
}

#[cfg(windows)]
fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut held = pin(path)?;
    let length = held.metadata().map_err(|_| "QA_SOURCE_STAT")?.len();
    if length == 0 || length > limit as u64 {
        return Err("QA_SOURCE_SIZE".into());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    Read::by_ref(&mut held)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "QA_SOURCE_READ")?;
    if bytes.len() as u64 != length {
        return Err("QA_SOURCE_SIZE".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
#[test]
#[ignore = "Exact37 read-only source files; one isolated native SOOP finalizer derivative, no network or user profile"]
fn actual_source_clock_native_finalizer_reopens_without_original_changes() -> Result<(), String> {
    const ID: &str = "3bae70dd-ca33-401b-aa95-a508bab9ffc4";
    const EVIDENCE_SHA: &str = "5d0f38a6815a7a1d825f719c6b9bbbc039cf943131e4098aff96f7a5898c3561";
    const MANIFEST_SHA: &str = "fd251f4d0d8cea862fd68b81a4b832fa979cd66a240bea4bb7c780e6de906f0e";
    if std::env::var("ATSUMI_SOOP_SOURCE_CLOCK_QA").as_deref() != Ok(ID) {
        return Err("QA_OPT_IN".into());
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("QA_WORKSPACE")?;
    let audit = workspace.join(".runtime/requirements-audit/20261117-cloud-goal-resume");
    let cancel = Arc::new(AtomicBool::new(false));
    let evidence_path = audit.join("soop-complete-overlap-fresh-verification.json");
    let _evidence_pin = pin(&evidence_path)?;
    let bytes = bounded_read(&evidence_path, 128 * 1024)?;
    if format!("{:x}", Sha256::digest(&bytes)) != EVIDENCE_SHA {
        return Err("QA_EVIDENCE_CHANGED".into());
    }
    let evidence: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "QA_EVIDENCE_SCHEMA")?;
    let source_root = workspace.join(".runtime/soop-original-qa/0d19e1f866b84ef8a2edef3980bb923b");
    storage::safe_path(&source_root)?;
    let canonical = fs::canonicalize(&source_root).map_err(|_| "QA_SOURCE_PATH")?;
    let _directory_pins = crate::streaming::browser_store::cleanup::pin_directories(&canonical)
        .map_err(|_| "QA_SOURCE_PIN")?;
    let entries = evidence["originals"]["files"]
        .as_array()
        .filter(|rows| rows.len() == 37)
        .ok_or("QA_SOURCE_COUNT")?;
    let mut originals = Vec::new();
    let mut expected_names = BTreeSet::new();
    for entry in entries {
        let relative = Path::new(entry["file"].as_str().ok_or("QA_SOURCE_PATH")?);
        if relative.is_absolute()
            || relative
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
            || !expected_names.insert(relative.to_path_buf())
        {
            return Err("QA_SOURCE_PATH".into());
        }
        let path = source_root.join(relative);
        let held = pin(&path)?;
        let meta = held.metadata().map_err(|_| "QA_SOURCE_STAT")?;
        let modified = meta.modified().map_err(|_| "QA_SOURCE_STAT")?;
        let nanos = modified
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "QA_SOURCE_STAT")?
            .as_nanos()
            .to_string();
        let sha = entry["sha256"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("QA_SOURCE_DIGEST")?;
        if Some(meta.len()) != entry["bytes"].as_u64()
            || meta.len() > 32 * 1024 * 1024
            || entry["mtimeNs"].as_str() != Some(&nanos)
            || storage::hash(&path, &cancel)? != sha
        {
            return Err("QA_ORIGINAL_CHANGED".into());
        }
        originals.push((path, held, meta.len(), modified, sha.to_string()));
    }
    if source_files(&source_root)? != expected_names {
        return Err("QA_SOURCE_LIST_CHANGED".into());
    }
    let source = source_root.join(format!("media/SOOP/{ID}"));
    let manifest = bounded_read(&source.join(MANIFEST), 8192)?;
    if format!("{:x}", Sha256::digest(&manifest)) != MANIFEST_SHA {
        return Err("QA_MANIFEST_CHANGED".into());
    }
    let text = std::str::from_utf8(&manifest).map_err(|_| "QA_MANIFEST_SCHEMA")?;
    let parts: Vec<SegmentProof> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|_| "QA_MANIFEST_SCHEMA")?;
    if parts.len() != 10 {
        return Err("QA_PIECE_COUNT".into());
    }
    for (index, part) in parts.iter().enumerate() {
        if segment_sequence(&part.name) != Some(index as u64) {
            return Err("QA_PIECE_SEQUENCE".into());
        }
    }
    let first = bounded_read(&source.join(&parts[0].name), 32 * 1024 * 1024)?;
    let initialization = init(&mut &first[..])?;
    let output = audit.join(format!(
        "soop-source-clock-native-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&output).map_err(|_| "QA_OUTPUT_CREATE")?;
    // The audit prefix makes save_job's temporary name exceed MAX_PATH. Rust
    // file IO accepts it, while the direct Win32 rename needs an extended path.
    // Canonicalize only this new test data directory; original paths stay pinned.
    let data_directory = output.join("data");
    fs::create_dir(&data_directory).map_err(|_| "QA_DATA_CREATE")?;
    let data_directory = fs::canonicalize(&data_directory).map_err(|_| "QA_DATA_CANONICALIZE")?;
    let bin = workspace
        .join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
    let tools = PlatformMediaTools {
        yt_dlp: output.join("no-network.exe"),
        deno: output.join("no-network.exe"),
        ffmpeg: bin.join("ffmpeg.exe"),
        ffprobe: bin.join("ffprobe.exe"),
    };
    let service = PlatformMediaService::new(data_directory.clone(), tools.clone(), None)
        .map_err(|error| format!("QA_SERVICE_CREATE: {error}"))?;
    let _stop = ServiceStop(service.clone());
    let (lease, mut recording) = archive(
        &service,
        &output,
        vec![EncodedTrack {
            track_index: 0,
            mime_type: "video/mp4; codecs=\"avc1.640020,mp4a.40.2\"".into(),
            init: initialization,
        }],
    )?;
    let mut writer = recording.writer.take().ok_or("QA_WRITER")?;
    for part in &parts {
        let bytes = bounded_read(&source.join(&part.name), 32 * 1024 * 1024)?;
        if bytes.len() as u64 != part.bytes
            || format!("{:x}", Sha256::digest(&bytes)) != part.sha256
        {
            return Err("QA_PIECE_CHANGED".into());
        }
        writer
            .commit(vec![fmp4::EncodedSegment {
                bytes,
                duration_seconds: part.duration,
                source_start_seconds: part.source_start,
                source_end_seconds: part.source_end,
            }])
            .map_err(|error| format!("QA_PIECE_COMMIT: {error}"))?;
    }
    mark(&writer).map_err(|error| format!("QA_STOP_MARK: {error}"))?;
    let job_id = writer.job.job_id.clone();
    let directory = writer.directory.clone();
    dispatch(writer, None).map_err(|error| format!("QA_DISPATCH: {error}"))?;
    let started = Instant::now();
    let completed = loop {
        let state = service.inner.state.lock().map_err(|_| "QA_STATE")?;
        let current = state.jobs.get(&job_id).ok_or("QA_JOB")?.clone();
        if !current.active() && !state.cancel.contains_key(&job_id) {
            break current;
        }
        drop(state);
        if started.elapsed() > Duration::from_secs(90) {
            return Err("QA_FINALIZER_TIMEOUT".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if completed.state != "completed" {
        return Err("QA_FINALIZER_FAILED".into());
    }
    let item = completed.library.ok_or("QA_LIBRARY")?;
    let outcome = serde_json::to_value(&item.capture_outcome).map_err(|_| "QA_OUTCOME")?;
    if outcome["state"] != "interrupted" || (item.duration - 118.011334).abs() > 0.000002 {
        return Err("QA_EXPECTED_DURATION_OR_OUTCOME".into());
    }
    for part in &parts {
        if storage::hash(&directory.join(&part.name), &cancel)? != part.sha256 {
            return Err("QA_DERIVED_PIECE_CHANGED".into());
        }
    }
    drop(recording);
    drop(lease);
    service.shutdown();
    let reopened = PlatformMediaService::new(data_directory, tools, None)
        .map_err(|error| format!("QA_SERVICE_REOPEN: {error}"))?;
    let _reopened_stop = ServiceStop(reopened.clone());
    let playback = reopened.open(&job_id)?;
    if playback.item.sha256 != item.sha256 || playback.item.capture_outcome != item.capture_outcome
    {
        return Err("QA_REOPEN_CHANGED".into());
    }
    reopened.close(&playback.token);
    reopened.shutdown();
    for (path, held, bytes, modified, sha) in &originals {
        let meta = held.metadata().map_err(|_| "QA_SOURCE_STAT")?;
        if meta.len() != *bytes
            || meta.modified().map_err(|_| "QA_SOURCE_STAT")? != *modified
            || storage::hash(path, &cancel)? != *sha
        {
            return Err("QA_ORIGINAL_CHANGED".into());
        }
    }
    if source_files(&source_root)? != expected_names {
        return Err("QA_SOURCE_LIST_CHANGED".into());
    }
    let result = serde_json::json!({"scope":"Actual-piece derivative through production native SOOP finalizer; no live capture or cloud verification",
        "sourceJob":ID,"sourceFiles":37,"sourcePieces":10,"originalsUnchanged":true,
        "sourceManifestSha256":MANIFEST_SHA,"derivedJob":job_id,"derivedDirectory":directory,
        "outputDurationSeconds":item.duration,"outputSha256":item.sha256,"nativeReopen":true,"captureOutcome":outcome,
        "networkRequests":0,"uploads":0,"userProfileWrites":0,
        "qualification":"Packet/configuration identity and strict decode are independently checked by the parent; this fixture does not copy original chat or retained excluded samples into its catalog."});
    let mut report = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join("result.json"))
        .map_err(|_| "QA_REPORT")?;
    report
        .write_all(&serde_json::to_vec_pretty(&result).map_err(|_| "QA_REPORT")?)
        .and_then(|_| report.sync_all())
        .map_err(|_| "QA_REPORT")?;
    println!(
        "QA_SOOP_SOURCE_CLOCK_RESULT={}",
        output.join("result.json").display()
    );
    Ok(())
}
