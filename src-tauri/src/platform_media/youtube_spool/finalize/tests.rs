use super::super::{vp9_admission, Chunk, Mode, Owner, SpoolWriter, Window};
use super::*;

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn packet(track: Track, pts: i64, duration: i64) -> PacketFact {
    let mut bytes = [0u8; 80];
    bytes[0] = track.index() as u8;
    bytes[1] = 1;
    bytes[4..12].copy_from_slice(&pts.to_be_bytes());
    bytes[12..20].copy_from_slice(&pts.to_be_bytes());
    bytes[20..28].copy_from_slice(&duration.to_be_bytes());
    bytes[28..36].copy_from_slice(&4u64.to_be_bytes());
    bytes[36..68].fill(1);
    PacketFact::decode_record(&bytes).unwrap()
}
fn clock(track: Track, first: i128, last: i128, end: i128) -> Clock {
    Clock {
        first,
        last,
        end,
        ticks_per_second: if track == Track::Video {
            1_000_000_000
        } else {
            44100
        },
    }
}
fn unit(track: Track, items: u64, first: i128, last: i128, end: i128) -> index::UnitDraft {
    index::UnitDraft {
        start: 0,
        init_end: 10,
        end: 100,
        boundary: index::Boundary::CommittedEof,
        init_sha256: [0; 32],
        configuration: Configuration {
            format: if track == Track::Video {
                Format::Vp9Webm
            } else {
                Format::AacMp4
            },
            track_id: 1,
            timebase: if track == Track::Video {
                1_000_000
            } else {
                44100
            },
            width: if track == Track::Video { 854 } else { 0 },
            height: if track == Track::Video { 480 } else { 0 },
            default_duration_ns: if track == Track::Video {
                Some(33_333_333)
            } else {
                None
            },
            sample_rate: if track == Track::Audio { 44100 } else { 0 },
            channels: if track == Track::Audio { 2 } else { 0 },
            semantic_sha256: [0; 32],
        },
        clock: clock(track, first, last, end),
        items,
        fragments: 1,
        rap_candidate: true,
        references: 0,
        reconstruction: index::ReconstructionFacts::Webm(index::WebmReconstruction::default()),
    }
}
fn report() -> PairReport {
    PairReport {
        track: Track::Video,
        count: 1,
        bytes: 4,
        source_tick: seconds(1, 1000).unwrap(),
        output_tick: seconds(1, 16000).unwrap(),
        max_pts_error: seconds(0, 1).unwrap(),
        max_dts_error: seconds(0, 1).unwrap(),
        max_duration_error: seconds(0, 1).unwrap(),
    }
}

#[test]
fn native_coverage_is_exact_and_never_invents_audio_edges() {
    let video = clock(Track::Video, 1_000_000_000, 1_966_000_000, 2_000_000_000);
    assert!(coverage(video, clock(Track::Audio, 44000, 88000, 89000)).is_ok());
    assert_eq!(
        coverage(video, clock(Track::Audio, 44101, 88000, 89000)),
        Err(Fault::Coverage)
    );
    assert_eq!(
        coverage(video, clock(Track::Audio, 44000, 88000, 88199)),
        Err(Fault::Coverage)
    );
    assert_eq!(
        coverage(video, clock(Track::Audio, 44100, 44100, 44100)),
        Err(Fault::Coverage)
    );
}

#[test]
fn exact_video_quantization_is_derived_from_default_duration_only() {
    let native = unit(Track::Video, 3, 0, 67_000_000, 100_333_333);
    let mut check = PacketClock::new(Track::Video, &native).unwrap();
    for pts in [0, 33, 67] {
        check.push(packet(Track::Video, pts, 33)).unwrap();
    }
    check.finish().unwrap();
    let mut gap = PacketClock::new(Track::Video, &native).unwrap();
    gap.push(packet(Track::Video, 0, 33)).unwrap();
    assert_eq!(gap.push(packet(Track::Video, 35, 33)), Err(Fault::Coverage));
    let mut wrong = PacketClock::new(Track::Video, &native).unwrap();
    assert_eq!(
        wrong.push(packet(Track::Video, 0, 34)),
        Err(Fault::Coverage)
    );
    let mut shifted = PacketClock::new(Track::Video, &native).unwrap();
    for pts in [1, 34, 68] {
        shifted.push(packet(Track::Video, pts, 33)).unwrap();
    }
    assert_eq!(shifted.finish(), Err(Fault::Coverage));
}

#[test]
fn audio_requires_contiguous_known_durations_and_no_unproved_skip() {
    let native = unit(Track::Audio, 2, 0, 1024, 2048);
    let mut check = PacketClock::new(Track::Audio, &native).unwrap();
    check.push(packet(Track::Audio, 0, 1024)).unwrap();
    check.push(packet(Track::Audio, 1024, 1024)).unwrap();
    check.finish().unwrap();
    let mut gap = PacketClock::new(Track::Audio, &native).unwrap();
    gap.push(packet(Track::Audio, 0, 1024)).unwrap();
    assert_eq!(
        gap.push(packet(Track::Audio, 1025, 1024)),
        Err(Fault::Coverage)
    );
    let mut skip = packet(Track::Audio, 0, 1024).encode_record().unwrap();
    skip[2] = 1;
    skip[68..72].copy_from_slice(&1u32.to_be_bytes());
    let mut check = PacketClock::new(Track::Audio, &native).unwrap();
    assert_eq!(
        check.push(PacketFact::decode_record(&skip).unwrap()),
        Err(Fault::Coverage)
    );
    let mut zero = packet(Track::Audio, 0, 1024).encode_record().unwrap();
    zero[20..28].fill(0);
    assert!(PacketFact::decode_record(&zero).is_err());
}

#[test]
fn selected_audio_first_duration_is_native_proven_not_inferred_from_last() {
    let mut native = unit(Track::Audio, 3, 0, 4100, 4300);
    native.reconstruction = index::ReconstructionFacts::Aac(index::AacReconstruction {
        first_sample_duration:100, all_fragments_default_base_is_moof:true,
        first_sequence:1,last_sequence:1,movie_duration:index::HeaderDuration::Zero,
        track_duration:index::HeaderDuration::Zero,media_duration:index::HeaderDuration::Zero,
    });
    let mut exact = PacketClock::new(Track::Audio,&native).unwrap();
    exact.require_selected_audio_first(&native).unwrap();
    for (pts,duration) in [(0,100),(100,4000),(4100,200)] {
        exact.push(packet(Track::Audio,pts,duration)).unwrap();
    }
    exact.finish().unwrap();
    let mut wrong = PacketClock::new(Track::Audio,&native).unwrap();
    wrong.require_selected_audio_first(&native).unwrap();
    assert_eq!(wrong.push(packet(Track::Audio,0,200)),Err(Fault::Coverage));
    if let index::ReconstructionFacts::Aac(ref mut facts) = native.reconstruction { facts.first_sample_duration = 0; }
    assert!(PacketClock::new(Track::Audio,&native).unwrap().require_selected_audio_first(&native).is_err());
    // Whole's existing checker still has no new phase/admission policy.
    let mut whole = PacketClock::new(Track::Audio,&native).unwrap();
    whole.push(packet(Track::Audio,0,200)).unwrap();
}

#[test]
fn final_block_duration_uses_native_tail_not_default_frame_duration() {
    let native = unit(Track::Video, 3, 0, 67_000_000, 101_500_000);
    let mut exact = PacketClock::new(Track::Video, &native).unwrap();
    exact.push(packet(Track::Video, 0, 33)).unwrap();
    exact.push(packet(Track::Video, 33, 33)).unwrap();
    exact.push(packet(Track::Video, 67, 34)).unwrap();
    exact.finish().unwrap();
    for wrong in [33, 35] {
        let mut check = PacketClock::new(Track::Video, &native).unwrap();
        check.push(packet(Track::Video, 0, 33)).unwrap();
        check.push(packet(Track::Video, 33, 33)).unwrap();
        assert_eq!(
            check.push(packet(Track::Video, 67, wrong)),
            Err(Fault::Coverage)
        );
    }
}

#[test]
fn tolerance_is_enforced_instead_of_merely_reported() {
    let mut value = report();
    value.max_pts_error = seconds(17, 16000).unwrap();
    check_report(&value, false).unwrap();
    assert_eq!(check_report(&value, true), Err(Fault::Coverage));
    value.max_pts_error = seconds(18, 16000).unwrap();
    assert_eq!(check_report(&value, false), Err(Fault::Coverage));
    value.max_pts_error = Rational {
        numerator: i128::MAX,
        denominator: 1,
    };
    assert!(check_report(&value, false).is_err());
}

#[test]
fn caps_are_checked_with_no_wrapping_or_unbounded_packet_tape() {
    assert_eq!(tape_size(MAX_PACKETS).unwrap(), 1_600_000_008);
    assert_eq!(tape_size(MAX_PACKETS + 1), Err(Fault::Bounds));
    assert_eq!(tape_size(0), Err(Fault::Bounds));
    assert!(add_rational(
        Rational {
            numerator: i128::MAX,
            denominator: 1
        },
        seconds(1, 1).unwrap()
    )
    .is_err());
}

#[test]
fn owned_write_failure_and_cancel_are_latched_without_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Directories::hold(temp.path()).unwrap();
    let path = temp.path().join("video.packets");
    let file = GuardedFile::open(&path, true).unwrap();
    let cancel = AtomicBool::new(false);
    let mut writer =
        BudgetWriter::new(&file, &dirs, temp.path(), &cancel, deadline(), 8, 0).unwrap();
    writer.write_all(b"abcd").unwrap();
    assert!(writer.write_all(b"12345").is_err());
    assert_eq!(writer.error(), Some(Fault::Bounds));
    assert!(writer.write_all(b"e").is_err());
    assert_eq!(writer.complete(), Err(Fault::Bounds));
    assert_eq!(file.file.metadata().unwrap().len(), 4);
    drop(writer);
    drop(file);
    assert_eq!(fs::read(&path).unwrap(), b"abcd");

    let path = temp.path().join("audio.packets");
    let file = GuardedFile::open(&path, true).unwrap();
    let mut writer =
        BudgetWriter::new(&file, &dirs, temp.path(), &cancel, deadline(), 8, 0).unwrap();
    writer.write_all(b"abcd").unwrap();
    cancel.store(true, Ordering::Release);
    assert!(writer.flush().is_err());
    cancel.store(false, Ordering::Release);
    assert_eq!(writer.complete(), Err(Fault::Cancelled));
    assert_eq!(file.file.metadata().unwrap().len(), 4);
}

#[test]
fn read_only_writer_failure_is_never_an_empty_success() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Directories::hold(temp.path()).unwrap();
    let path = temp.path().join("video.packets");
    drop(GuardedFile::open(&path, true).unwrap());
    let file = GuardedFile::open(&path, false).unwrap();
    let cancel = AtomicBool::new(false);
    let mut writer =
        BudgetWriter::new(&file, &dirs, temp.path(), &cancel, deadline(), 8, 0).unwrap();
    assert!(writer.write_all(b"abcd").is_err());
    assert_eq!(writer.complete(), Err(Fault::Io));
    assert_eq!(file.file.metadata().unwrap().len(), 0);
}

#[test]
fn held_files_reject_wrong_hash_and_preserve_cursor_and_identity() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Arc::new(Directories::hold(temp.path()).unwrap());
    let path = temp.path().join("video.webm");
    let file = GuardedFile::open(&path, true).unwrap();
    (&file.file).write_all(b"test-owned-evidence").unwrap();
    file.file.sync_all().unwrap();
    drop(file);
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        HeldInput::open_owned(
            path.clone(),
            OwnedKind::DerivedVideo,
            dirs.clone(),
            Some((19, [0; 32])),
            &cancel,
            deadline()
        ),
        Err(Fault::Changed)
    ));
    let held = HeldInput::open_owned(
        path,
        OwnedKind::DerivedVideo,
        dirs,
        None,
        &cancel,
        deadline(),
    )
    .unwrap();
    let mut reader = &held.file.file;
    reader.seek(SeekFrom::Start(4)).unwrap();
    held.rehash(&cancel, deadline()).unwrap();
    assert_eq!(reader.stream_position().unwrap(), 4);
    assert_eq!(held.check(&cancel, Instant::now()), Err(Fault::Deadline));
}

#[test]
fn output_slot_is_one_shot_and_never_overwrites_a_collision() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Arc::new(Directories::hold(temp.path()).unwrap());
    let cancel = AtomicBool::new(false);
    let output = OutputSlot::new(temp.path(), dirs.clone(), 64).unwrap();
    let collision = GuardedFile::open(output.path(), true).unwrap();
    (&collision.file).write_all(b"keep").unwrap();
    collision.file.sync_all().unwrap();
    assert_eq!(output.begin(&cancel, deadline()), Err(Fault::Path));
    assert_eq!(output.begin(&cancel, deadline()), Err(Fault::Terminal));
    assert_eq!(collision.check().unwrap(), 4);
    let output = OutputSlot::new(temp.path(), dirs, 64).unwrap();
    assert_eq!(output.check(&cancel, deadline()), Err(Fault::Terminal));
    output.begin(&cancel, deadline()).unwrap();
    output.check(&cancel, deadline()).unwrap();
    assert!(matches!(
        output.finish(&cancel, deadline()),
        Err(Fault::Changed)
    ));
}

#[test]
fn observed_output_becomes_read_only_only_after_sync_and_exact_hash() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Arc::new(Directories::hold(temp.path()).unwrap());
    let cancel = AtomicBool::new(false);
    let output = OutputSlot::new(temp.path(), dirs, 64).unwrap();
    output.begin(&cancel, deadline()).unwrap();
    let file = GuardedFile::open(output.path(), true).unwrap();
    (&file.file).write_all(b"fixed-result").unwrap();
    output.check(&cancel, deadline()).unwrap();
    drop(file);
    let held = output.finish(&cancel, deadline()).unwrap();
    assert_eq!(held.len(), 12);
    assert_eq!(held.kind(), OwnedKind::Candidate);
    assert_eq!(
        held.sha256(),
        <[u8; 32]>::from(Sha256::digest(b"fixed-result"))
    );
    held.rehash(&cancel, deadline()).unwrap();
}

#[test]
fn disk_tape_longer_sequence_requires_full_physical_eof() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Directories::hold(temp.path()).unwrap();
    let file = GuardedFile::open(&temp.path().join("audio.packets"), true).unwrap();
    let cancel = AtomicBool::new(false);
    let rows = 4096;
    let mut output = BudgetWriter::new(
        &file,
        &dirs,
        temp.path(),
        &cancel,
        deadline(),
        tape_size(rows).unwrap(),
        0,
    )
    .unwrap();
    let mut tape = packet_tape::Writer::new(&mut output, Track::Audio).unwrap();
    let mut fold = TrackFold::new(Track::Audio);
    for n in 0..rows {
        let p = packet(Track::Audio, (n * 1024) as i64, 1024);
        fold.push(p).unwrap();
        tape.push(p).unwrap();
    }
    let facts = ProbeFacts {
        packets: fold.finish().unwrap(),
        stream: StreamFacts {
            track: Track::Audio,
            time_base: packet_facts::TimeBase::new(1, 44100).unwrap(),
            width: 0,
            height: 0,
            sample_rate: 44100,
            channels: 2,
        },
    };
    let summary = tape.finish(&facts).unwrap();
    drop(tape);
    output.complete().unwrap();
    drop(output);
    let mut input = &file.file;
    input.seek(SeekFrom::Start(0)).unwrap();
    let mut reader = packet_tape::Reader::new(&mut input, &summary).unwrap();
    for n in 0..rows {
        assert_eq!(
            reader.next_fact().unwrap(),
            Some(packet(Track::Audio, (n * 1024) as i64, 1024))
        );
    }
    assert!(reader.next_fact().unwrap().is_none());
    drop(reader);
    input.seek(SeekFrom::End(0)).unwrap();
    input.write_all(b"x").unwrap();
    input.seek(SeekFrom::Start(0)).unwrap();
    let mut reader = packet_tape::Reader::new(&mut input, &summary).unwrap();
    for _ in 0..rows {
        assert!(reader.next_fact().unwrap().is_some());
    }
    assert!(reader.next_fact().is_err());
    assert!(reader.next_fact().is_err());
}

#[test]
fn linked_owned_file_is_rejected_without_removing_either_link() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = Arc::new(Directories::hold(temp.path()).unwrap());
    let path = temp.path().join("video.webm");
    let file = GuardedFile::open(&path, true).unwrap();
    (&file.file).write_all(b"keep").unwrap();
    drop(file);
    let link = temp.path().join("other.webm");
    fs::hard_link(&path, &link).unwrap();
    assert!(HeldInput::open_owned(
        path.clone(),
        OwnedKind::DerivedVideo,
        dirs,
        None,
        &AtomicBool::new(false),
        deadline()
    )
    .is_err());
    assert_eq!(fs::read(path).unwrap(), b"keep");
    assert_eq!(fs::read(link).unwrap(), b"keep");
}

fn identity() -> SpoolIdentity {
    SpoolIdentity {
        job_id: uuid::Uuid::new_v4().to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        receiver_id: uuid::Uuid::new_v4().to_string(),
        source_id: uuid::Uuid::new_v4().to_string(),
        document_id: uuid::Uuid::new_v4().to_string(),
        document_epoch: 1,
        source_generation: 1,
        video_id: "T4XZmMPQ9Kw".into(),
        channel_id: format!("UC{}", "a".repeat(22)),
        owner: Owner::Manual,
    }
}

// Native structural binding plus typed packet algebra only, not playable media
// or an admission bypass. No subprocess is used by these focused regressions.
fn selection_fixture() -> (tempfile::TempDir, SpoolIdentity, CompletedSelection) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("YouTube")).unwrap();
    let id = identity();
    let mut bytes = index::authored_pair();
    let at = bytes[0].windows(4).position(|x| x == [0xe7, 0x82, 3, 232]).unwrap();
    bytes[0][at + 2..at + 4].copy_from_slice(&1100u16.to_be_bytes());
    let at = bytes[1].windows(4).position(|x| x == b"tfdt").unwrap();
    bytes[1][at + 8..at + 16].copy_from_slice(&44100u64.to_be_bytes());
    let at = bytes[1].windows(4).position(|x| x == b"trex").unwrap();
    bytes[1][at + 16..at + 20].copy_from_slice(&4410u32.to_be_bytes());
    let descriptor = ClockDescriptor { video_timestamp_scale_ns: 1_000_000, audio_timescale: 44100,
        mode: Mode::Segments, append_window: Window::Default, video_timestamp_offset: 0, audio_timestamp_offset: 0 };
    let mut writer = SpoolWriter::create_new(root.path(), id.clone(), descriptor, Arc::new(AtomicBool::new(false))).unwrap();
    for (track, bytes) in [(Track::Video, bytes[0].clone()), (Track::Audio, bytes[1].clone())] {
        writer.append(&id, Chunk { track, append_index: 0, chunk_index: 0, final_chunk: true, bytes }).unwrap();
    }
    writer.seal(&id, SealCause::Stopped).unwrap();
    drop(writer);
    let source = CompletedSelection::prepare(root.path(), &id, &AtomicBool::new(false)).unwrap();
    (root, id, source)
}
fn partition_fixture(binding: &SelectionBinding, cancel: &AtomicBool) -> (packet_tape::Summary, AacRetention) {
    let mut bytes = Vec::new();
    let mut writer = packet_tape::Writer::new(&mut bytes, Track::Audio).unwrap();
    let mut fold = TrackFold::new(Track::Audio);
    let timebase = packet_facts::TimeBase::new(1, 44100).unwrap();
    let mut partition = Partition::new(binding, timebase, cancel, deadline()).unwrap();
    for n in 0..3 {
        let fact = packet(Track::Audio, 44100 + n * 4410, 4410);
        writer.push(fact).unwrap(); fold.push(fact).unwrap(); partition.push(fact).unwrap();
    }
    let summary = writer.finish(&ProbeFacts { packets: fold.finish().unwrap(), stream: StreamFacts {
        track: Track::Audio, time_base: timebase, width: 0, height: 0, sample_rate: 44100, channels: 2,
    }}).unwrap();
    let plan = partition.finish(&summary).unwrap();
    (summary, plan)
}
fn held_test_audio(root: &Path, name: &str, cancel: &AtomicBool) -> HeldInput {
    let dirs = Arc::new(Directories::hold(root).unwrap());
    let path = root.join(name);
    let file = GuardedFile::open(&path, true).unwrap();
    (&file.file).write_all(b"same-owned-audio-test-bytes").unwrap();
    file.file.sync_all().unwrap(); drop(file);
    HeldInput::open_owned(path, OwnedKind::DerivedAudio, dirs, None, cancel, deadline()).unwrap()
}

#[test]
fn selected_remux_binds_exact_held_audio_and_native_tape_before_output() {
    let (root, _, source) = selection_fixture();
    let cancel = AtomicBool::new(false);
    let binding = source.binding(&cancel).unwrap();
    let (summary, plan) = partition_fixture(&binding, &cancel);
    assert_eq!(plan.ordinals(), (1, 2));
    let audio = held_test_audio(root.path(), "one.mp4", &cancel);
    let mut other = held_test_audio(root.path(), "two.mp4", &cancel);
    assert_eq!((audio.len(), audio.sha256()), (other.len(), other.sha256()));
    let evidence = SelectedRemux::new(&audio, &binding, &summary, &plan, seconds(11, 10).unwrap(), &cancel, deadline()).unwrap();
    assert_eq!(evidence.ordinals(), (1, 2));
    assert_eq!(evidence.origin(), seconds(11, 10).unwrap());
    evidence.check_audio(&audio, &cancel, deadline()).unwrap();
    assert_eq!(evidence.check_audio(&other, &cancel, deadline()), Err(Fault::Changed));
    other.kind = OwnedKind::DerivedVideo;
    assert_eq!(evidence.check_audio(&other, &cancel, deadline()), Err(Fault::Changed));
    cancel.store(true, Ordering::Release);
    assert_eq!(evidence.check_audio(&audio, &cancel, deadline()), Err(Fault::Cancelled));
    cancel.store(false, Ordering::Release);
    assert_eq!(evidence.check_audio(&audio, &cancel, Instant::now()), Err(Fault::Deadline));
}

#[test]
fn selected_remux_rejects_foreign_binding_summary_and_changed_held_facts() {
    let (root, _, source) = selection_fixture();
    let (_foreign_root, _, foreign) = selection_fixture();
    let cancel = AtomicBool::new(false);
    let binding = source.binding(&cancel).unwrap();
    let foreign = foreign.binding(&cancel).unwrap();
    let (summary, plan) = partition_fixture(&binding, &cancel);
    let audio = held_test_audio(root.path(), "one.mp4", &cancel);
    assert!(matches!(SelectedRemux::new(&audio, &foreign, &summary, &plan, seconds(0, 1).unwrap(), &cancel, deadline()), Err(Fault::Identity)));
    let mut changed_summary = Vec::new();
    let mut tape = packet_tape::Writer::new(&mut changed_summary, Track::Audio).unwrap();
    let mut fold = TrackFold::new(Track::Audio);
    for n in 0..3 { let fact = packet(Track::Audio, 44101 + n * 4410, 4410); tape.push(fact).unwrap(); fold.push(fact).unwrap(); }
    let summary2 = tape.finish(&ProbeFacts { packets: fold.finish().unwrap(), stream: summary.facts().stream.clone() }).unwrap();
    assert!(matches!(SelectedRemux::new(&audio, &binding, &summary2, &plan, seconds(0, 1).unwrap(), &cancel, deadline()), Err(Fault::Changed)));
    let mut evidence = SelectedRemux::new(&audio, &binding, &summary, &plan, seconds(0, 1).unwrap(), &cancel, deadline()).unwrap();
    evidence.audio_hash[0] ^= 1;
    assert_eq!(evidence.check_audio(&audio, &cancel, deadline()), Err(Fault::Changed));
    evidence.audio_hash = audio.sha256(); evidence.audio_length += 1;
    assert_eq!(evidence.check_audio(&audio, &cancel, deadline()), Err(Fault::Changed));
}

#[test]
fn selected_origin_ignores_excluded_leading_audio_and_source_poison_is_sticky() {
    let (_root, _, selected) = selection_fixture();
    let cancel = AtomicBool::new(false);
    let binding = selected.binding(&cancel).unwrap();
    let (_, plan) = partition_fixture(&binding, &cancel);
    let plans = [TrackPlan { configuration: unit(Track::Video, 1, 0, 1, 2).configuration,
        clock: clock(Track::Video, 0, 1, 2), units: 1, items: 1 },
        TrackPlan { configuration: unit(Track::Audio, 1, 0, 1, 2).configuration,
        clock: clock(Track::Audio, 0, 1, 2), units: 1, items: 1 }];
    let video = Times::from(packet(Track::Video, 1100, 33));
    assert_eq!(origin_times(video, plan.first_retained(), &plans).unwrap(), seconds(11, 10).unwrap());
    assert_eq!(origin_times(video, Times::from(packet(Track::Audio, 44100, 4410)), &plans).unwrap(), seconds(1, 1).unwrap());
    let mut source = FinalizerSource::Selected(selected);
    assert_eq!(visit(&mut source, Track::Video, &cancel, |_| Err(Fault::Changed)), Err(Fault::Changed));
    assert_eq!(source.check(&cancel), Err(Fault::Terminal));
    assert!(source.binding(&cancel).is_err());
}

#[test]
fn selected_source_cannot_be_redirected_to_copied_same_identity_root() {
    let (root, id, selected) = selection_fixture();
    let other = tempfile::tempdir().unwrap();
    fs::create_dir(other.path().join("YouTube")).unwrap();
    let copied = other.path().join("YouTube").join(&id.job_id);
    fs::create_dir(&copied).unwrap();
    let original = root.path().join("YouTube").join(&id.job_id);
    for name in [super::super::schema::PROOF, super::super::schema::JOURNAL,
        super::super::schema::RAW[0], super::super::schema::RAW[1]] {
        fs::copy(original.join(name), copied.join(name)).unwrap();
    }
    let cancel = AtomicBool::new(false);
    // Byte/proof identity is intentionally identical, but file identity is not.
    let copy = CompletedSelection::prepare(other.path(), &id, &cancel).unwrap();
    assert_eq!(copy.binding(&cancel).unwrap(), selected.binding(&cancel).unwrap());
    let dirs = Directories::hold(&copied).unwrap();
    let file = attributes(&copied.join(super::super::schema::RAW[0]), false).unwrap();
    let mut source = FinalizerSource::Selected(selected);
    assert_eq!(visit(&mut source, Track::Video, &cancel, |mut span| check_span_root(&mut span, &file, &dirs)), Err(Fault::Identity));
    assert_eq!(source.check(&cancel), Err(Fault::Terminal));
    assert!(fs::read_dir(&copied).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with("youtube-finalize-")));
}

#[derive(Clone, Copy)]
enum ActualCase { Whole, Selected, PartialVideo }

#[test]
#[ignore = "Root-only actual immutable public capture, pinned tools, no production publication"]
fn actual_captured_pair_runs_admission_and_production_finalizer_api() {
    actual_finalizer(ActualCase::Whole, false);
}

#[test]
#[ignore = "Root-only actual selected packet remux; immutable sources and pinned tools"]
fn actual_captured_pair_runs_selected_finalizer_api() {
    actual_finalizer(ActualCase::Selected, false);
}

#[test]
#[ignore = "Root-only partial test-owned tail; never changes immutable original capture"]
fn actual_captured_partial_video_runs_selected_finalizer_api() {
    actual_finalizer(ActualCase::PartialVideo, false);
}

#[test]
#[ignore = "Root-only real finalizer to isolated library publication/reopen, no user data"]
fn actual_captured_pair_publishes_and_reopens_library() {
    actual_finalizer(ActualCase::Whole, true);
}

#[test]
#[ignore = "Root-only selected real finalizer to isolated library publication/reopen"]
fn actual_captured_selected_pair_publishes_and_reopens_library() {
    actual_finalizer(ActualCase::Selected, true);
}

#[test]
#[ignore = "Root-only partial real finalizer remains playable and reports partial after restart"]
fn actual_captured_partial_video_publishes_and_reopens_library() {
    actual_finalizer(ActualCase::PartialVideo, true);
}

fn actual_finalizer(case: ActualCase, publish: bool) {
    use crate::platform_media::PlatformMediaTools;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let qa_parent = workspace.join(".runtime/youtube-webm-raw-pair-qa");
    let original = qa_parent.join("c26c9147-735f-4ed3-821b-31e92cef9afd");
    let source_dirs = Arc::new(Directories::hold(&original).unwrap());
    let cancel = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut held = Vec::new(); // Exactly five immutable fixture files, not packets.
    for (name, len, sha) in [
        (
            "manifest.json",
            3840,
            "c4a92fbff4e9c27870b817e1d2015cb66990eaf9ec47568ead953026c15508ad",
        ),
        (
            "video-1.webm",
            95635,
            "9be82861467e27ed66fa8220db1a7c4f1e2872996755257cbeafe149035070a4",
        ),
        (
            "video-2.webm",
            76753,
            "7bb5f6e6f3fcd435042dc5b7227ca023799014f7f3e7886e4a99bc732451a4b6",
        ),
        (
            "audio-1.mp4",
            81810,
            "c349dc8672a8ade3d961d19c7d0b30f09473b08b96c7eb58641cbaceb385779a",
        ),
        (
            "audio-2.mp4",
            82185,
            "b9fa4f91f7dc48bee2a9f7128188f27b3b041b6faf16f91e19b7a3e17daf7457",
        ),
    ] {
        // This test-private source holding is never supplied to an owned runner.
        let file = GuardedFile::open(&original.join(name), false).unwrap();
        assert_eq!(file.check().unwrap(), len);
        let hash = hash_file(&file.file, len, &cancel, deadline).unwrap();
        assert_eq!(
            hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            sha
        );
        held.push((file, len, hash));
    }
    let id = identity();
    let root = qa_parent.join(format!("production-finalizer-{}", id.job_id));
    let mut dirs = Directories::hold(&qa_parent).unwrap();
    dirs.create_child(&root).unwrap();
    dirs.create_child(&root.join("YouTube")).unwrap();
    println!("preserved_fixture={}", root.display());
    let clock = ClockDescriptor {
        video_timestamp_scale_ns: 1_000_000,
        audio_timescale: 44100,
        mode: Mode::Segments,
        append_window: Window::Default,
        video_timestamp_offset: 0,
        audio_timestamp_offset: 0,
    };
    let mut writer = SpoolWriter::create_new(&root, id.clone(), clock, cancel.clone()).unwrap();
    for (n, (file, len, _)) in held[1..].iter().enumerate() {
        assert!(*len <= CHUNK as u64);
        let mut bytes = vec![0; *len as usize];
        let mut reader = &file.file;
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_exact(&mut bytes).unwrap();
        let init_length = [452, 451, 632, 632][n];
        let initialization = if n < 2 {
            super::super::inspect_vp9_initialization(&bytes[..init_length], &cancel)
        } else {
            super::super::inspect_aac_initialization(&bytes[..init_length], &cancel)
        }.expect("native init-only acceptance of actual captured prefix");
        assert_eq!(initialization.len(), init_length);
        assert_eq!(initialization.timebase(), if n < 2 { 1_000_000 } else { 44100 });
        assert_eq!(initialization.track(), if n < 2 { Track::Video } else { Track::Audio });
        if n == 1 && matches!(case, ActualCase::PartialVideo) {
            // Only the new owned raw append loses a byte; the original stays held.
            bytes.pop();
        }
        writer
            .append(
                &id,
                Chunk {
                    track: if n < 2 { Track::Video } else { Track::Audio },
                    append_index: (n % 2) as u64,
                    chunk_index: 0,
                    final_chunk: true,
                    bytes,
                },
            )
            .unwrap();
    }
    writer.seal(&id, SealCause::Stopped).unwrap();
    drop(writer);
    let bin = workspace
        .join(".runtime/media-tools")
        .join(super::super::managed_probe::PACKAGE)
        .join("bin");
    let tool = ManagedProbe::open(
        &PlatformMediaTools {
            yt_dlp: PathBuf::new(),
            deno: PathBuf::new(),
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        },
        &cancel,
        deadline,
    )
    .unwrap();
    let mut expected_audio = None;
    let mut expected_selection = None;
    let candidate = if matches!(case, ActualCase::Whole) {
        let mut source = SequentialSource::prepare(&root, &id, &cancel).unwrap();
        let admission = vp9_admission::admit(&mut source, &cancel).expect("native first-RAP/in-band admission");
        drop(source);
        prepare(&root, &id, &admission, &tool, &cancel, deadline).expect("real Whole finalizer API")
    } else {
        let mut source = CompletedSelection::prepare(&root, &id, &cancel).unwrap();
        let admission = vp9_admission::admit_selected(&mut source, &cancel).expect("native selected first-RAP/in-band admission");
        let range = source.facts().tracks[0].selected;
        let mut ordinal = 0u64;
        let mut first = None;
        let mut end = 0;
        let mut retained = 0;
        // Independent original-packet interval calculation, not Partition or
        // retained candidate output. No packet-sized collection is allocated.
        source.visit(Track::Audio, &cancel, |span| {
            let facts = packet_probe::probe_span(span, &tool, &cancel, deadline, |packet| {
                let time = Times::from(packet);
                let start = i128::from(time.pts).checked_mul(i128::from(range.ticks_per_second)).unwrap();
                let finish = i128::from(time.pts.checked_add(time.duration).unwrap()).checked_mul(i128::from(range.ticks_per_second)).unwrap();
                if finish > range.first_tick.checked_mul(44100).unwrap()
                    && start < range.end_tick.checked_mul(44100).unwrap() {
                    first.get_or_insert(ordinal);
                    end = ordinal + 1;
                    retained += 1;
                }
                ordinal += 1;
                Ok(())
            }).unwrap();
            assert_eq!(facts.stream.time_base, packet_facts::TimeBase::new(1, 44100).unwrap());
            Ok(())
        }).unwrap();
        expected_audio = Some((first.unwrap(), end, retained, ordinal));
        expected_selection = Some(source.facts().clone());
        if publish {
            drop(source);
            super::super::prepare_recording(&root, &id, &PlatformMediaTools {
                yt_dlp: PathBuf::new(), deno: PathBuf::new(),
                ffmpeg: bin.join("ffmpeg.exe"), ffprobe: bin.join("ffprobe.exe"),
            }, &cancel, deadline).expect("production recording preparation entry")
        } else {
            prepare_selected(&root, source, &admission, &tool, &cancel, deadline).expect("real selected finalizer API")
        }
    };
    if let Some(evidence) = candidate.selected.as_ref() {
        let job = root.join("YouTube").join(&id.job_id);
        let attempts = fs::read_dir(&job).unwrap().filter_map(|entry| {
            let path = entry.unwrap().path();
            path.file_name().unwrap().to_string_lossy().starts_with("youtube-finalize-").then_some(path)
        }).collect::<Vec<_>>();
        assert_eq!(attempts.len(), 1);
        let video = HeldInput::open_owned(attempts[0].join("video.webm"), OwnedKind::DerivedVideo, candidate._attempt.clone(), None, &cancel, deadline).unwrap();
        let audio = HeldInput::open_owned(attempts[0].join("audio.mp4"), OwnedKind::DerivedAudio, candidate._attempt.clone(), None, &cancel, deadline).unwrap();
        let copy_path = attempts[0].join("audio-wrong-identity.mp4");
        assert!(!copy_path.exists());
        fs::copy(audio.path(), &copy_path).unwrap();
        let other = HeldInput::open_owned(copy_path, OwnedKind::DerivedAudio, candidate._attempt.clone(), Some((audio.len(), audio.sha256())), &cancel, deadline).unwrap();
        let proof = SelectedRemux::new(&audio, &evidence.binding, evidence.audio.input(), &evidence.audio, candidate.origin, &cancel, deadline).unwrap();
        let output = OutputSlot::new(&job, candidate._attempt.clone(), 1024).unwrap();
        assert_eq!(packet_probe::remux_selected_owned(&video, &other, &output, &proof, &tool, &cancel, deadline), Err(packet_probe::Fault::SourceChanged));
        assert!(!output.begun.load(Ordering::Acquire)); assert!(!output.path().exists());
        let cancelled = AtomicBool::new(true);
        assert_eq!(packet_probe::remux_selected_owned(&video, &audio, &output, &proof, &tool, &cancelled, deadline), Err(packet_probe::Fault::Cancelled));
        assert!(!output.begun.load(Ordering::Acquire)); assert!(!output.path().exists());
    }
    candidate.with_verified_path(&cancel,deadline,|path,facts| {
        assert!(path.starts_with(root.join("YouTube").join(&id.job_id)));
        assert_eq!(facts.identity,&id);assert!(facts.bytes>0);
        match expected_audio {
            None => {
                assert_eq!([facts.comparisons[0].count,facts.comparisons[1].count],[300,431]);
                assert_eq!(facts.origin,seconds(2063438879,44100).unwrap());
                assert!(facts.selected.is_none());
            }
            Some((first,end,retained,input)) => {
                let evidence = facts.selected.expect("selected provenance");
                assert_eq!(evidence.facts(), expected_selection.as_ref().unwrap());
                assert_eq!(evidence.audio().ordinals(), (first,end));
                assert_eq!(evidence.audio().input().facts().packets.count, input);
                assert_eq!(evidence.audio().retained().count, retained);
                assert_eq!(facts.comparisons[1].count, retained);
                assert!(evidence.facts().packet_selection_pending && evidence.facts().codec_validation_pending);
                if matches!(case, ActualCase::PartialVideo) {
                    assert_eq!(facts.comparisons[0].count, 150);
                    assert!(retained < input, "excluded unit audio must not leak to candidate");
                    assert!(matches!(evidence.facts().tracks[0].original_outcome.terminal, index::Terminal::PartialTail { .. }));
                    assert!(evidence.facts().tracks[0].trailing.is_some());
                } else { assert_eq!(facts.comparisons[0].count, 300); }
            }
        }
        assert_eq!(facts.seal_cause,Some(SealCause::Stopped));
        assert!(!facts.incomplete_append&&!facts.torn_journal);assert_eq!(facts.uncommitted_bytes,[0,0]);
        println!("candidate={} bytes={} sha256={} packet_counts={},{} selected={} productionRecordingVerified=false",path.display(),facts.bytes,facts.sha256.iter().map(|b|format!("{b:02x}")).collect::<String>(),facts.comparisons[0].count,facts.comparisons[1].count,facts.selected.is_some());
        Ok(())
    }).unwrap();
    if publish {
        crate::platform_media::youtube_publication::publish_actual_fixture(
            &root, &id, cancel.clone(), candidate, deadline,
            matches!(case, ActualCase::PartialVideo),
        );
    }
    source_dirs.check().unwrap();
    for (file, len, hash) in &held {
        assert_eq!(file.check().unwrap(), *len);
        assert_eq!(
            hash_file(&file.file, *len, &cancel, deadline).unwrap(),
            *hash
        );
    }
    // Sources, raw job, tapes, derived files and candidate remain recoverable.
}
