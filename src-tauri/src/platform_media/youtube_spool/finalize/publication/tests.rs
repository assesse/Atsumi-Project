use super::*;

fn deadline() -> Instant { Instant::now() + Duration::from_secs(30) }
fn duration(n: i128, d: i128) -> Rational { Rational::new(n, d).unwrap() }
fn pair_report(track: Track) -> PairReport {
    PairReport { track, count: 1, bytes: 4, source_tick: duration(1, 1000), output_tick: duration(1, 16000),
        max_pts_error: duration(0, 1), max_dts_error: duration(0, 1), max_duration_error: duration(0, 1) }
}

#[test]
fn verified_output_duration_uses_exact_packet_end_and_both_tracks() {
    let comparison = Comparison::new(pair_report(Track::Video), Times { pts: 80_000, dts: 80_000, duration: 533 }).unwrap();
    assert_eq!(comparison.end, duration(80_533, 16_000));
    let audio = duration(221_184, 44_100);
    assert_eq!(candidate_duration([comparison.end, audio]).unwrap(), comparison.end);
    assert_eq!(candidate_duration([duration(5, 1), audio]).unwrap(), audio);
    for invalid in [duration(0, 1), duration(-1, 1), duration(86_400_001, 1000)] {
        assert_eq!(candidate_duration([invalid, duration(1, 1)]), Err(Fault::Coverage));
    }
    let mut report = pair_report(Track::Video);
    report.output_tick = Rational { numerator: i128::MAX, denominator: 1 };
    assert!(matches!(Comparison::new(report, Times { pts: 2, dts: 2, duration: 1 }), Err(Fault::Bounds)));
}

#[test]
fn fixed_merge_name_and_exclusion_projection_are_closed() {
    assert!(valid_merge_name(&format!("merge-{}.mp4", uuid::Uuid::new_v4())));
    for name in ["media.mp4", "merge-../test.mp4", "merge-00000000000000000000000000000000.mp4", "merge-no.mp4", "download.mp4"] {
        assert!(!valid_merge_name(name));
    }
    let reasons = selection::ExclusionReasons { startup_coverage: true, audio_coverage_tail: true,
        outside_selected_video: true, incomplete_structure: true, incomplete_append: true };
    let result = PublicationExclusion::project(Some(selection::Excluded { start: 10, end: 25, complete_units: 2, reasons })).unwrap();
    assert_eq!((result.bytes, result.units), (15, 2));
    assert!(result.startup_coverage && result.audio_coverage_tail && result.outside_selected_video
        && result.incomplete_structure && result.incomplete_append);
    assert_eq!(PublicationExclusion::project(None).unwrap(), PublicationExclusion::default());
    assert!(PublicationExclusion::project(Some(selection::Excluded { start: 30, end: 25, complete_units: 2, reasons })).is_err());
}

// This module's private fixture exercises file lifetime/phase/provenance only.
// It deliberately does not claim encoded media validity or create public test
// admission constructors. Root's actual tests run the full real finalizer API.
#[cfg(windows)]
fn candidate_fixture() -> (tempfile::TempDir, SpoolIdentity, VerifiedCandidate) {
    use super::super::super::{Chunk, Mode, Owner, SpoolWriter, Window};
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("YouTube")).unwrap();
    let id = SpoolIdentity { job_id: uuid::Uuid::new_v4().to_string(), request_id: uuid::Uuid::new_v4().to_string(),
        receiver_id: uuid::Uuid::new_v4().to_string(), source_id: uuid::Uuid::new_v4().to_string(), document_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1, source_generation: 1, video_id: "T4XZmMPQ9Kw".into(), channel_id: format!("UC{}", "a".repeat(22)), owner: Owner::Manual };
    let clock = ClockDescriptor { video_timestamp_scale_ns: 1_000_000, audio_timescale: 44100, mode: Mode::Segments,
        append_window: Window::Default, video_timestamp_offset: 0, audio_timestamp_offset: 0 };
    let mut writer = SpoolWriter::create_new(temp.path(), id.clone(), clock, Arc::new(AtomicBool::new(false))).unwrap();
    for (at, bytes) in index::authored_pair().into_iter().enumerate() {
        writer.append(&id, Chunk { track: if at == 0 { Track::Video } else { Track::Audio }, append_index: 0,
            chunk_index: 0, final_chunk: true, bytes }).unwrap();
    }
    writer.seal(&id, SealCause::Stopped).unwrap(); drop(writer);
    let cancel = AtomicBool::new(false);
    let mut source = SequentialSource::prepare(temp.path(), &id, &cancel).unwrap();
    let mut scope = None;
    source.visit(Track::Video, &cancel, |span| { scope = Some(SourceScope::from_span(&span)); Ok(()) }).unwrap();
    let job = crate::platform_media::storage::job_dir(temp.path(), "youtube", &id.job_id).unwrap();
    let dirs = Arc::new(Directories::hold(&job).unwrap());
    let path = job.join(format!("merge-{}.mp4", uuid::Uuid::new_v4()));
    let created = GuardedFile::open(&path, true).unwrap();
    (&created.file).write_all(b"retained file-lifetime fixture, not playable media").unwrap();
    created.file.sync_all().unwrap(); drop(created);
    let candidate = HeldInput::open_owned(path, OwnedKind::Candidate, dirs.clone(), None, &cancel, deadline()).unwrap();
    (temp, id, VerifiedCandidate { source: FinalizerSource::Whole(source), scope: scope.unwrap(), candidate,
        origin: duration(0, 1), comparisons: [pair_report(Track::Video), pair_report(Track::Audio)],
        duration: duration(5, 1), selected: None, _attempt: dirs })
}

#[cfg(windows)]
fn held_fixture(root: &Path) -> HeldInput {
    let path = root.join(format!("merge-{}.mp4", uuid::Uuid::new_v4()));
    let created = GuardedFile::open(&path, true).unwrap();
    (&created.file).write_all(b"preserve exactly").unwrap(); created.file.sync_all().unwrap(); drop(created);
    HeldInput::open_owned(path, OwnedKind::Candidate, Arc::new(Directories::hold(root).unwrap()), None,
        &AtomicBool::new(false), deadline()).unwrap()
}
#[cfg(windows)]
fn assert_writes_denied(path: &Path) {
    use std::os::windows::fs::OpenOptionsExt;
    assert!(OpenOptions::new().write(true).share_mode(7).open(path).is_err());
}

#[cfg(windows)]
#[test]
fn windows_witness_and_owner_hold_exact_file_and_deny_writes() {
    let temp = tempfile::tempdir().unwrap();
    let held = held_fixture(temp.path());
    let path = held.path.clone();
    let before = file_key(&held.file.file).unwrap();
    let cancel = AtomicBool::new(false);
    let mut owner = RenameFile::acquire_after_witness(held, &cancel, deadline(), || {
        assert_writes_denied(&path); Ok(())
    }).unwrap();
    assert_writes_denied(&path);
    assert!(fs::rename(&path, temp.path().join("foreign.mp4")).is_err());
    let target = temp.path().join("media.mp4");
    owner.install(&target, &cancel, deadline()).unwrap();
    assert_eq!(file_key(&owner.file).unwrap(), before);
    assert_writes_denied(&target);
    assert!(fs::remove_file(&target).is_err());
    assert!(open_publication(&target, Access::Inspect).is_ok());
    owner.verify(&cancel, deadline()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"preserve exactly");
}

#[cfg(windows)]
#[test]
fn windows_transition_replacement_is_rejected_without_mutating_either_file() {
    let temp = tempfile::tempdir().unwrap(); let held = held_fixture(temp.path());
    let path = held.path.clone(); let preserved = temp.path().join("preserved.mp4");
    let result = RenameFile::acquire_after_witness(held, &AtomicBool::new(false), deadline(), || {
        fs::rename(&path, &preserved).unwrap(); fs::write(&path, b"replacement").unwrap(); Ok(())
    });
    assert!(matches!(result, Err(Fault::Changed)));
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    assert_eq!(fs::read(&preserved).unwrap(), b"preserve exactly");
    assert!(!temp.path().join("media.mp4").exists());
}

#[cfg(windows)]
#[test]
fn windows_collision_and_transition_failures_preserve_sources() {
    let temp = tempfile::tempdir().unwrap(); let held = held_fixture(temp.path()); let path = held.path.clone();
    let target = temp.path().join("media.mp4"); fs::write(&target, b"existing").unwrap();
    let mut owner = RenameFile::acquire(held, &AtomicBool::new(false), deadline()).unwrap();
    assert!(owner.install(&target, &AtomicBool::new(false), deadline()).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"existing"); assert_eq!(fs::read(&path).unwrap(), b"preserve exactly");
    drop(owner);
    for fault in [Fault::Io, Fault::Cancelled, Fault::Deadline] {
        let held = held_fixture(temp.path()); let path = held.path.clone();
        assert!(RenameFile::acquire_after_witness(held, &AtomicBool::new(false), deadline(), || Err(fault)).is_err());
        assert_eq!(fs::read(path).unwrap(), b"preserve exactly");
    }
}

#[cfg(windows)]
#[test]
fn consuming_handoff_installs_once_and_retains_full_scope() {
    let (root, id, candidate) = candidate_fixture(); let cancel = AtomicBool::new(false);
    let original = candidate.candidate.path.clone(); let hash = candidate.candidate.hash;
    let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
    assert!(handoff.matches_scope(root.path(), &id));
    let foreign = tempfile::tempdir().unwrap(); assert!(!handoff.matches_scope(foreign.path(), &id));
    let mut other = id.clone(); other.source_generation += 1; assert!(!handoff.matches_scope(root.path(), &other));
    assert_eq!(handoff.facts().sha256, hash);
    handoff.install_no_replace(&cancel, deadline()).unwrap();
    assert!(!original.exists());
    handoff.verify_installed(&cancel, deadline()).unwrap();
    handoff.check_for_commit(&cancel, deadline()).unwrap();
    assert!(handoff.modified_millis(&cancel, deadline()).unwrap() > 0);
    handoff.with_final_path(&cancel, deadline(), |path, facts| {
        assert_eq!(path.file_name().unwrap(), "media.mp4");
        assert_eq!(facts.source.state, PublicationState::Sealed);
        assert_eq!(facts.duration, 5.0);
        assert_writes_denied(path);
    }).unwrap();
    assert_eq!(handoff.install_no_replace(&cancel, deadline()), Err(Fault::Terminal));
    assert_eq!(handoff.check_for_commit(&cancel, deadline()), Err(Fault::Terminal));
    assert!(handoff.job.join("media.mp4").exists());
    assert!(handoff.job.join("video.raw").exists() && handoff.job.join("audio.raw").exists());
}

#[cfg(windows)]
#[test]
fn handoff_rejects_wrong_identity_root_and_preverification_commit() {
    let (root, id, candidate) = candidate_fixture(); let original = candidate.candidate.path.clone();
    let mut wrong = id.clone(); wrong.request_id = uuid::Uuid::new_v4().to_string();
    assert!(matches!(candidate.into_publication(root.path(), &wrong, &AtomicBool::new(false), deadline()), Err(Fault::Identity)));
    assert!(original.exists());
    let (_root, id, candidate) = candidate_fixture(); let foreign = tempfile::tempdir().unwrap();
    assert!(candidate.into_publication(foreign.path(), &id, &AtomicBool::new(false), deadline()).is_err());
    let (root, id, candidate) = candidate_fixture(); let cancel = AtomicBool::new(false);
    let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
    assert_eq!(handoff.check_for_commit(&cancel, deadline()), Err(Fault::Terminal));
    assert_eq!(handoff.install_no_replace(&cancel, deadline()), Err(Fault::Terminal));
    assert!(!handoff.job.join("media.mp4").exists());
}

#[cfg(windows)]
#[test]
fn installed_cancel_hash_failure_and_expiry_never_commit_or_remove_files() {
    for mode in 0..3 {
        let (root, id, candidate) = candidate_fixture(); let cancel = AtomicBool::new(false);
        let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
        handoff.install_no_replace(&cancel, deadline()).unwrap();
        let end = if mode == 2 { Instant::now() } else { deadline() };
        if mode == 0 { cancel.store(true, Ordering::Release); }
        if mode == 1 { handoff.file.hash[0] ^= 1; }
        assert!(handoff.verify_installed(&cancel, end).is_err());
        cancel.store(false, Ordering::Release);
        assert_eq!(handoff.check_for_commit(&cancel, deadline()), Err(Fault::Terminal));
        assert!(handoff.job.join("media.mp4").exists());
        assert!(handoff.job.join("video.raw").exists());
    }
}

#[cfg(windows)]
#[test]
fn interrupted_provenance_is_preserved_but_cancelled_corrupt_are_rejected() {
    let (_root, _id, candidate) = candidate_fixture(); let mut scope = candidate.scope.clone();
    scope.state = PrefixState::Interrupted; scope.seal_cause = Some(SealCause::Limit);
    scope.torn_journal = true; scope.incomplete_append = true; scope.uncommitted_bytes = [3, 5];
    let facts = PublicationSourceFacts::project(&scope, &None).unwrap();
    assert_eq!(facts.state, PublicationState::Interrupted); assert_eq!(facts.seal_cause, Some(SealCause::Limit));
    assert!(facts.torn_journal && facts.incomplete_append); assert_eq!(facts.uncommitted_bytes, [3, 5]);
    for state in [PrefixState::Cancelled, PrefixState::Corrupt] {
        scope.state = state; assert!(PublicationSourceFacts::project(&scope, &None).is_err());
    }
    scope.state = PrefixState::Sealed; scope.seal_cause = Some(SealCause::Cancelled);
    assert!(PublicationSourceFacts::project(&scope, &None).is_err());
}

#[cfg(windows)]
#[test]
fn recovery_adopts_exact_final_without_rename_or_deleting_new_candidate() {
    let (root, id, candidate) = candidate_fixture();
    let original = candidate.candidate.path.clone();
    let bytes = fs::read(&original).unwrap();
    let job = original.parent().unwrap();
    let target = job.join("media.mp4");
    fs::write(&target, &bytes).unwrap();
    let before = file_key(&open_publication(&target, Access::Existing).unwrap()).unwrap();
    let modified = fs::metadata(&target).unwrap().modified().unwrap();
    let cancel = AtomicBool::new(false);
    let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
    assert!(handoff.adopt_existing_if_present(&cancel, deadline()).unwrap());
    assert_eq!(handoff.phase.get(), Phase::Installed);
    assert_writes_denied(&target);
    assert!(fs::rename(&target, job.join("must-not-move.mp4")).is_err());
    handoff.verify_installed(&cancel, deadline()).unwrap();
    handoff.check_for_commit(&cancel, deadline()).unwrap();
    assert_eq!(file_key(&handoff.file.file).unwrap(), before);
    assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read(&original).unwrap(), bytes);
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert!(job.join("video.raw").is_file() && job.join("audio.raw").is_file());
    assert_eq!(handoff.adopt_existing_if_present(&cancel, deadline()), Err(Fault::Terminal));
    assert_eq!(handoff.check_for_commit(&cancel, deadline()), Err(Fault::Terminal));
}

#[cfg(windows)]
#[test]
fn recovery_absence_allows_install_but_late_collision_never_adopts() {
    for collision in [false, true] {
        let (root, id, candidate) = candidate_fixture();
        let original = candidate.candidate.path.clone();
        let target = original.parent().unwrap().join("media.mp4");
        let cancel = AtomicBool::new(false);
        let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
        assert!(!handoff.adopt_existing_if_present(&cancel, deadline()).unwrap());
        assert_eq!(handoff.phase.get(), Phase::Prepared);
        if collision { fs::write(&target, b"late external collision").unwrap(); }
        let installed = handoff.install_no_replace(&cancel, deadline());
        if collision {
            assert!(installed.is_err());
            assert_eq!(handoff.adopt_existing_if_present(&cancel, deadline()), Err(Fault::Terminal));
            assert_eq!(fs::read(&target).unwrap(), b"late external collision");
            assert!(original.is_file());
        } else {
            installed.unwrap();
            handoff.verify_installed(&cancel, deadline()).unwrap();
            assert!(target.is_file());
        }
    }
}

#[cfg(windows)]
#[test]
fn recovery_rejects_different_bytes_lengths_directory_and_hardlink() {
    for mode in 0..4 {
        let (root, id, candidate) = candidate_fixture();
        let original = candidate.candidate.path.clone();
        let bytes = fs::read(&original).unwrap();
        let target = original.parent().unwrap().join("media.mp4");
        match mode {
            0 => { let mut changed = bytes.clone(); changed[0] ^= 1; fs::write(&target, changed).unwrap(); }
            1 => fs::write(&target, b"short").unwrap(),
            2 => fs::create_dir(&target).unwrap(),
            _ => { fs::write(&target, &bytes).unwrap(); fs::hard_link(&target, target.with_extension("link")).unwrap(); }
        }
        let cancel = AtomicBool::new(false);
        let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
        if mode == 0 {
            assert!(handoff.adopt_existing_if_present(&cancel, deadline()).unwrap());
            assert_eq!(handoff.verify_installed(&cancel, deadline()), Err(Fault::Changed));
        } else {
            assert!(handoff.adopt_existing_if_present(&cancel, deadline()).is_err());
        }
        assert_eq!(handoff.check_for_commit(&cancel, deadline()), Err(Fault::Terminal));
        assert_eq!(fs::read(&original).unwrap(), bytes);
        assert!(target.exists());
    }
}

#[cfg(windows)]
#[test]
fn new_publication_rejects_even_identical_existing_final() {
    let (root, id, candidate) = candidate_fixture();
    let original = candidate.candidate.path.clone();
    let bytes = fs::read(&original).unwrap();
    let target = original.parent().unwrap().join("media.mp4");
    fs::write(&target, &bytes).unwrap();
    let cancel = AtomicBool::new(false);
    let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
    assert!(handoff.install_no_replace(&cancel, deadline()).is_err());
    assert_eq!(handoff.adopt_existing_if_present(&cancel, deadline()), Err(Fault::Terminal));
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert_eq!(fs::read(&original).unwrap(), bytes);
}

#[cfg(windows)]
#[test]
fn adopted_final_cancel_or_expiry_is_terminal_and_preserved() {
    for mode in 0..3 {
        let (root, id, candidate) = candidate_fixture();
        let original = candidate.candidate.path.clone();
        let target = original.parent().unwrap().join("media.mp4");
        let bytes = fs::read(&original).unwrap();
        fs::write(&target, &bytes).unwrap();
        let cancel = AtomicBool::new(false);
        let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
        if mode == 0 {
            cancel.store(true, Ordering::Release);
            assert_eq!(handoff.adopt_existing_if_present(&cancel, deadline()), Err(Fault::Cancelled));
        } else {
            assert!(handoff.adopt_existing_if_present(&cancel, deadline()).unwrap());
            if mode == 1 { cancel.store(true, Ordering::Release); }
            assert!(handoff.verify_installed(&cancel, if mode == 2 { Instant::now() } else { deadline() }).is_err());
        }
        cancel.store(false, Ordering::Release);
        assert_eq!(handoff.check_for_commit(&cancel, deadline()), Err(Fault::Terminal));
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }
}

#[cfg(windows)]
#[test]
fn recovery_rejects_a_preexisting_writer_without_weakening_its_share_guard() {
    use std::os::windows::fs::OpenOptionsExt;
    let (root, id, candidate) = candidate_fixture();
    let original = candidate.candidate.path.clone();
    let bytes = fs::read(&original).unwrap();
    let target = original.parent().unwrap().join("media.mp4");
    fs::write(&target, &bytes).unwrap();
    let writer = OpenOptions::new().write(true).share_mode(7).open(&target).unwrap();
    let cancel = AtomicBool::new(false);
    let mut handoff = candidate.into_publication(root.path(), &id, &cancel, deadline()).unwrap();
    assert!(handoff.adopt_existing_if_present(&cancel, deadline()).is_err());
    drop(writer);
    assert_eq!(handoff.adopt_existing_if_present(&cancel, deadline()), Err(Fault::Terminal));
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert_eq!(fs::read(&original).unwrap(), bytes);
}

#[cfg(windows)]
#[test]
#[ignore = "Root-only pinned actual media: creates isolated copies, real local finalizer and crash-journal recovery; no UI or original mutation"]
fn actual_raw_existing_final_recovers_after_completed_journal_failure() {
    use crate::platform_media::{PlatformMediaTools, youtube_spool::{self, Chunk, Mode, Owner, SpoolWriter, Window}};
    use std::io::{Read, Seek, SeekFrom};
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let original = workspace.join(".runtime/youtube-original-qa/89c5a86b-833f-43d4-966a-6b891fa16215/media/YouTube/47ac1001-18c9-4e8d-962c-931de53e87a5");
    let pins = [
        ("proof.json",805u64,"4461f8bbdf376320d5ad0c7f40f2e535488c6c60ec2793e797ddb4d6842409bb"),
        ("journal.jsonl",11969,"143a747aeb3a57270ddff2f49e600f7fc5995aeb3d16479371e8171b11fe36b8"),
        ("video.raw",351859,"5a5d034282dc82dbc6146edcf241cf5461946f3b98307bbb0c87940dd4e3daa8"),
        ("audio.raw",327619,"0cb1e1a7fdbc59526c2ecb43e6842451770600cfd0bab12cd9818488f94cf951"),
    ];
    let cancel = AtomicBool::new(false);
    let source_dirs = Directories::hold(&original).unwrap();
    let held: Vec<_> = pins.iter().map(|(name,length,expected)| {
        let file = GuardedFile::open(&original.join(name),false).unwrap();
        assert_eq!(file.check().unwrap(),*length);
        let hash = hash_file(&file.file,*length,&cancel,Instant::now()+Duration::from_secs(30)).unwrap();
        assert_eq!(hash.iter().map(|b|format!("{b:02x}")).collect::<String>(),*expected);
        file
    }).collect();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let bin = workspace.join(".runtime/media-tools").join(youtube_spool::managed_probe::PACKAGE).join("bin");
        let tools = PlatformMediaTools { yt_dlp: Default::default(), deno: Default::default(),
            ffmpeg:bin.join("ffmpeg.exe"),ffprobe:bin.join("ffprobe.exe") };
        for partial in [false,true] {
            let root = workspace.join(".runtime").join(format!("youtube-existing-recovery-{}",uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap(); fs::create_dir(root.join("YouTube")).unwrap();
            let id = SpoolIdentity { job_id:uuid::Uuid::new_v4().to_string(),request_id:uuid::Uuid::new_v4().to_string(),
                receiver_id:uuid::Uuid::new_v4().to_string(),source_id:uuid::Uuid::new_v4().to_string(),
                document_id:uuid::Uuid::new_v4().to_string(),document_epoch:1,source_generation:1,
                video_id:"T4XZmMPQ9Kw".into(),channel_id:format!("UC{}","a".repeat(22)),owner:Owner::Manual };
            let clock = ClockDescriptor { video_timestamp_scale_ns:1_000_000,audio_timescale:44100,
                mode:Mode::Segments,append_window:Window::Default,video_timestamp_offset:0,audio_timestamp_offset:0 };
            let mut writer = SpoolWriter::create_new(&root,id.clone(),clock,Arc::new(AtomicBool::new(false))).unwrap();
            for track_index in 0..2 {
                let source = &held[track_index+2];
                let full_length = pins[track_index+2].1;
                let length = full_length-u64::from(partial && track_index==0);
                let mut file = &source.file;
                file.seek(SeekFrom::Start(0)).unwrap();
                let mut offset = 0;
                let mut index = 0;
                while offset < length {
                    let count = (length-offset).min(131072) as usize;
                    let mut bytes = vec![0;count]; file.read_exact(&mut bytes).unwrap();
                    offset += count as u64;
                    writer.append(&id,Chunk { track:if track_index==0 {Track::Video}else{Track::Audio},append_index:0,
                        chunk_index:index,final_chunk:offset==length,bytes }).unwrap();
                    index += 1;
                }
                file.seek(SeekFrom::Start(0)).unwrap();
                assert_eq!(source.check().unwrap(),full_length);
            }
            writer.seal(&id,SealCause::Stopped).unwrap(); drop(writer);
            crate::platform_media::youtube_publication::recover_actual_fixture(&root,&id,tools.clone(),partial);
        }
    }));
    // Check every original even after an assertion panic in the real runner.
    let mut unchanged = source_dirs.check().is_ok();
    for (file,(_,length,expected)) in held.iter().zip(pins) {
        unchanged &= file.check().ok()==Some(length);
        let actual = hash_file(&file.file,length,&cancel,Instant::now()+Duration::from_secs(30));
        unchanged &= actual.is_ok_and(|hash|hash.iter().map(|b|format!("{b:02x}")).collect::<String>()==expected);
    }
    assert!(unchanged,"pinned source posthash/guard failed");
    if let Err(panic) = result { std::panic::resume_unwind(panic); }
}
