use super::*;

fn inputs() -> (Vec<EncodedTrack>, [Vec<u8>; 2]) {
    let units = spool::authored_raw_pair();
    let cancel = AtomicBool::new(false);
    let tracks = units.iter().enumerate().map(|(index, unit)| {
        // Reuse native-authored structural fixtures, not copied codec headers.
        let end = (1..=unit.len().min(65536)).find(|end| {
            if index == 0 { spool::inspect_vp9_initialization(&unit[..*end], &cancel).is_ok() }
            else { spool::inspect_aac_initialization(&unit[..*end], &cancel).is_ok() }
        }).unwrap();
        EncodedTrack { track_index: index as u32, mime_type: if index == 0 {
            "video/webm; codecs=\"vp09.00.51.08.01.01.01.01.00\""
        } else { "audio/mp4; codecs=\"mp4a.40.2\"" }.into(), init: unit[..end].to_vec() }
    }).collect();
    (tracks, units)
}
struct Fixture { root: tempfile::TempDir, service: PlatformMediaService, lease: OfficialReceiverLease }
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let unused = root.path().join("not-a-media-tool.exe");
        let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
            yt_dlp: unused.clone(), deno: unused.clone(), ffmpeg: unused.clone(), ffprobe: unused,
        }, None).unwrap();
        service.inner.synthetic_live_receiver.store(true, Ordering::Release);
        let lease = service.reserve_official_receiver(MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: "abcdefghijk".into() },
            "abcdefghijk".into(), uuid::Uuid::new_v4().to_string(), root.path().join("media")).unwrap();
        *lease.receiver.youtube_chat_binding.lock().unwrap() = Some(YouTubeChatBinding { channel_id: format!("UC{}", "a".repeat(22)), producer_started: false });
        Self { root, service, lease }
    }
    fn begin(&self) -> YouTubeRawArchive {
        self.lease.begin_youtube_raw(uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string(), 3, 7,
            uuid::Uuid::new_v4().to_string(), inputs().0, "fixture".into(), "fixture".into(), live::RecordingOwner::Manual).unwrap()
    }
    fn stop(&self, mut archive: YouTubeRawArchive) {
        // Avoid invoking any external media process from these structural tests.
        let mut writer = archive.writer.take().unwrap();
        let _ = writer.spool.take().map(|mut spool| spool.seal(&writer.identity, spool::SealCause::Interrupted));
        let _ = self.lease.receiver.chat.detach_youtube_raw_archive(&writer.job.job_id);
        *self.lease.receiver.recording.lock().unwrap() = None;
        self.service.terminal_error(&writer.job.job_id, &writer.cancel, "test-only structural archive");
        drop(writer);
    }
}
impl Drop for Fixture { fn drop(&mut self) { self.service.shutdown(); } }
fn packet(archive: &YouTubeRawArchive, track: u32, index: u64, bytes: Vec<u8>) -> EncodedAppend {
    let identity = &archive.writer.as_ref().unwrap().identity;
    EncodedAppend { source_id: identity.source_id.clone(), generation: identity.document_epoch,
        track_index: track, index, chunk: 0, final_chunk: true, bytes }
}

#[test]
fn raw_admission_native_clocks_roles_and_exact_owned_proofs() {
    let fixture = Fixture::new();
    let archive = fixture.begin();
    let writer = archive.writer.as_ref().unwrap();
    assert_eq!(writer.identity.document_epoch, 3);
    assert_eq!(writer.identity.source_generation, 7);
    assert_eq!(writer.tracks[&0].timebase, 1_000_000);
    assert_eq!(writer.tracks[&1].timebase, 44_100);
    assert!(!writer.directory.join(MANIFEST).exists());
    assert!(writer.directory.join("proof.json").exists());
    assert!(fixture.root.path().join("media").is_dir());
    let identity = writer.identity.clone();
    let directory = writer.directory.clone();
    let job = archive.snapshot().unwrap();
    // Spool write holdings deliberately prevent a simultaneous recovery read.
    fixture.stop(archive);
    assert_eq!(recover_identity(&job, &directory, &AtomicBool::new(false)).unwrap(), identity);
}

#[test]
fn raw_admission_rejects_duplicate_role_unsupported_mime_and_missing_channel() {
    let (mut tracks, _) = inputs();
    let cancel = AtomicBool::new(false);
    tracks[1] = tracks[0].clone(); tracks[1].track_index = 1;
    assert!(track_inputs(&tracks, &cancel).is_err());
    let (mut tracks, _) = inputs(); tracks[0].mime_type = "video/mp4;codecs=\"avc1.64001f\"".into();
    assert!(track_inputs(&tracks, &cancel).is_err());
    let (mut tracks, _) = inputs(); tracks[0].mime_type = "video/webm;codecs=\"vp09.bad\"".into();
    assert!(track_inputs(&tracks, &cancel).is_err());
    let fixture = Fixture::new(); *fixture.lease.receiver.youtube_chat_binding.lock().unwrap() = None;
    assert!(fixture.lease.begin_youtube_raw(uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string(), 1, 1,
        uuid::Uuid::new_v4().to_string(), inputs().0, String::new(), String::new(), live::RecordingOwner::Manual).is_err());
    assert!(fixture.service.inner.state.lock().unwrap().jobs.is_empty());
}

#[test]
fn raw_mime_hint_matches_browser_closed_subset_and_native_init_still_decides() {
    let (mut tracks, _) = inputs(); let cancel = AtomicBool::new(false);
    for mime in ["video/webm;codecs=vp9", "VIDEO/WEBM; codecs=\"vp09.00.51.08\"", "video/webm;\tcodecs=vp09.00.51.08.01.01.01.01.00"] {
        tracks[0].mime_type = mime.into(); assert!(track_inputs(&tracks, &cancel).is_ok());
    }
    for mime in ["video/webm;codecs=vp09.0.51.08", "video/webm;codecs=vp09.00.51", "video/webm;codecs=vp9,opus", "video/webm;codecs=vp09.00.51.08.01.01.01.01.00.00"] {
        tracks[0].mime_type = mime.into(); assert!(track_inputs(&tracks, &cancel).is_err());
    }
    tracks[0].mime_type = "video/webm;codecs=vp9".into(); tracks[0].init = tracks[1].init.clone();
    assert!(track_inputs(&tracks, &cancel).is_err());
}

#[test]
fn raw_admission_rejects_existing_archive_request_and_wrong_scheduled_channel() {
    let fixture = Fixture::new(); let archive = fixture.begin();
    let identity = archive.writer.as_ref().unwrap().identity.clone();
    assert!(fixture.lease.begin_youtube_raw(identity.source_id.clone(), identity.document_id.clone(), 3, 8,
        uuid::Uuid::new_v4().to_string(), inputs().0, String::new(), String::new(), live::RecordingOwner::Manual).is_err());
    fixture.stop(archive);
    assert!(fixture.lease.begin_youtube_raw(identity.source_id.clone(), identity.document_id.clone(), 3, 8,
        identity.request_id, inputs().0, String::new(), String::new(), live::RecordingOwner::Manual).is_err());
    let owner = live::RecordingOwner::Scheduled { schedule_id: format!("UC{}", "b".repeat(22)), run_id: uuid::Uuid::new_v4().to_string(), broadcast_key: "abcdefghijk".into(), generation: 1 };
    assert!(fixture.lease.begin_youtube_raw(identity.source_id, identity.document_id, 3, 8,
        uuid::Uuid::new_v4().to_string(), inputs().0, String::new(), String::new(), owner).is_err());
}

#[test]
fn raw_bytes_require_admitted_exact_init_before_durable_append() {
    let fixture = Fixture::new(); let mut archive = fixture.begin(); let (_, units) = inputs();
    let packet = packet(&archive, 0, 0, units[0].clone());
    assert!(archive.append(packet, None).is_err());
    let writer = archive.writer.as_ref().unwrap();
    assert!(writer.poisoned);
    assert_eq!(fs::metadata(writer.directory.join("video.raw")).unwrap().len(), 0);
    assert!(writer.directory.join(FATAL_FILE).is_file());
    fixture.stop(archive);
}

#[test]
fn raw_exact_init_retry_and_duplicate_ack_do_not_double_progress() {
    let fixture = Fixture::new(); let mut archive = fixture.begin(); let (tracks, units) = inputs();
    assert_eq!(archive.observe_init(0, 0, 0, &tracks[0].init).unwrap(), RawInitDecision::Accepted);
    assert_eq!(archive.observe_init(0, 0, 0, &tracks[0].init).unwrap(), RawInitDecision::Accepted);
    let first = packet(&archive, 0, 0, units[0].clone()); assert!(!archive.append(first, None).unwrap().duplicate);
    let before = archive.snapshot().unwrap().received_bytes;
    let retry = packet(&archive, 0, 0, units[0].clone()); assert!(archive.append(retry, None).unwrap().duplicate);
    assert_eq!(archive.snapshot().unwrap().received_bytes, before);
    assert!(!archive.startup_ready());
    fixture.stop(archive);
}

#[test]
fn raw_changed_duplicate_bytes_poison_and_recovery_cannot_salvage_fatal() {
    let fixture = Fixture::new(); let mut archive = fixture.begin(); let (tracks, units) = inputs();
    archive.observe_init(0, 0, 0, &tracks[0].init).unwrap();
    let first = packet(&archive, 0, 0, units[0].clone()); archive.append(first, None).unwrap();
    let mut altered = units[0].clone(); *altered.last_mut().unwrap() ^= 1;
    let retry = packet(&archive, 0, 0, altered); assert!(archive.append(retry, None).is_err());
    let directory = archive.writer.as_ref().unwrap().directory.clone(); let job = archive.snapshot().unwrap();
    fixture.stop(archive);
    assert!(recover_identity(&job, &directory, &AtomicBool::new(false)).is_err());
}

#[test]
fn raw_startup_requires_both_payloads_and_first_boundary_not_init_only() {
    let fixture = Fixture::new(); let mut archive = fixture.begin(); let (tracks, units) = inputs();
    for index in 0..2 {
        archive.observe_init(index, 0, 0, &tracks[index as usize].init).unwrap();
        let init = packet(&archive, index, 0, tracks[index as usize].init.clone()); archive.append(init, None).unwrap();
    }
    assert!(!archive.startup_ready());
    for index in 0..2 {
        let rest = packet(&archive, index, 1, units[index as usize][tracks[index as usize].init.len()..].to_vec()); archive.append(rest, None).unwrap();
    }
    assert!(!archive.startup_ready());
    let end = units[0].len() as u64;
    archive.observe_init(0, 1, end, &tracks[0].init).unwrap();
    let successor = packet(&archive, 0, 2, tracks[0].init.clone()); archive.append(successor, Some(end)).unwrap();
    assert!(archive.startup_ready());
    let directory = archive.writer.as_ref().unwrap().directory.clone();
    let anchor = fs::read(directory.join("youtube-raw-anchor.json")).unwrap();
    let retry = packet(&archive, 0, 2, tracks[0].init.clone()); assert!(archive.append(retry, Some(end)).unwrap().duplicate);
    assert_eq!(fs::read(directory.join("youtube-raw-anchor.json")).unwrap(), anchor);
    fixture.stop(archive);
}

#[test]
fn raw_init_prefix_is_checked_incrementally_and_same_config_padding_is_allowed() {
    let (tracks, _) = inputs(); let cancel = AtomicBool::new(false);
    let mut states = track_inputs(&tracks, &cancel).unwrap(); let track = states.get_mut(&0).unwrap();
    track.observe(0, 0, &tracks[0].init, &cancel).unwrap();
    let count = track.check_bytes(&tracks[0].init[..3]).unwrap(); assert_eq!(count, 3);
    track.matched += count; track.length += count as u64;
    let mut wrong = tracks[0].init[3..].to_vec(); wrong[0] ^= 1; assert!(track.check_bytes(&wrong).is_err());
    assert_eq!(track.check_bytes(&tracks[0].init[3..]).unwrap(), tracks[0].init.len() - 3);
    track.matched = tracks[0].init.len(); track.length = track.matched as u64 + 10;
    let mut padded = tracks[0].init.clone(); padded.extend([0xec, 0x81, 0]);
    assert_eq!(track.observe(1, track.length, &padded, &cancel).unwrap(), RawInitDecision::Accepted);
}

#[test]
fn raw_malformed_init_and_source_epoch_mismatch_are_fatal_before_bytes() {
    for wrong_epoch in [false, true] {
        let fixture = Fixture::new(); let mut archive = fixture.begin(); let (tracks, units) = inputs();
        if wrong_epoch {
            archive.observe_init(0, 0, 0, &tracks[0].init).unwrap();
            let mut value = packet(&archive, 0, 0, units[0].clone()); value.generation += 1;
            assert!(archive.append(value, None).is_err());
        } else { assert!(archive.observe_init(0, 0, 0, &[0, 1]).is_err()); }
        assert!(archive.writer.as_ref().unwrap().poisoned);
        assert_eq!(archive.snapshot().unwrap().received_bytes, 0);
        fixture.stop(archive);
    }
}

#[test]
fn raw_init_unit_order_and_offsets_cannot_be_rebased() {
    let (tracks, _) = inputs(); let cancel = AtomicBool::new(false);
    let mut states = track_inputs(&tracks, &cancel).unwrap(); let track = states.get_mut(&0).unwrap();
    assert!(track.observe(1, 0, &tracks[0].init, &cancel).is_err());
    assert!(track.observe(0, 1, &tracks[0].init, &cancel).is_err());
    track.observe(0, 0, &tracks[0].init, &cancel).unwrap();
    assert!(track.observe(1, 0, &tracks[0].init, &cancel).is_err());
    assert_eq!(track.next_unit, 1);
}

#[test]
fn raw_changed_configuration_denies_new_bytes_without_discarding_old_prefix() {
    let (tracks, _) = inputs(); let cancel = AtomicBool::new(false);
    let mut states = track_inputs(&tracks, &cancel).unwrap(); let track = states.get_mut(&0).unwrap();
    track.observe(0, 0, &tracks[0].init, &cancel).unwrap();
    track.matched = tracks[0].init.len(); track.length = track.matched as u64 + 10;
    let mut changed = tracks[0].init.clone();
    let at = changed.windows(4).position(|bytes| bytes == [0xb0, 0x82, 0x03, 0x56]).unwrap();
    changed[at + 3] = 0x58; // A valid 856px configuration, not malformed EBML.
    let before = track.length;
    assert_eq!(track.observe(1, before, &changed, &cancel).unwrap(), RawInitDecision::Changed);
    assert_eq!(track.length, before);
    assert_eq!(track.next_unit, 1);
    assert!(track.check_bytes(&changed).is_err());
}

#[test]
fn raw_recovery_cross_checks_fixed_owner_and_cancellation() {
    let fixture = Fixture::new(); let archive = fixture.begin();
    let job = archive.snapshot().unwrap(); let directory = archive.writer.as_ref().unwrap().directory.clone();
    fixture.stop(archive);
    assert!(recover_identity(&job, &directory, &AtomicBool::new(true)).is_err());
    let mut different = job.clone(); different.broadcast_id = Some("lmnopqrstuv".into());
    assert!(recover_identity(&different, &directory, &AtomicBool::new(false)).is_err());
    assert!(recover_identity(&job, &directory.parent().unwrap().join("other"), &AtomicBool::new(false)).is_err());
    // An unreadable/malformed fatal marker is still presence and cannot fall through.
    fs::write(directory.join(FATAL_FILE), b"{").unwrap();
    assert!(recover_identity(&job, &directory, &AtomicBool::new(false)).is_err());
}

#[test]
fn raw_fatal_job_only_survives_active_restart_and_cannot_retry_without_marker() {
    let fixture = Fixture::new(); let archive = fixture.begin();
    let job = archive.snapshot().unwrap(); let directory = archive.writer.as_ref().unwrap().directory.clone();
    // Simulate the independently successful job write after marker-write failure;
    // no marker is created or deleted, and the persisted job remains active.
    fixture.service.update(&job.job_id, |job| job.error = Some(FATAL.into())).unwrap();
    assert!(!directory.join(FATAL_FILE).exists());
    let before = storage::load_jobs(&fixture.service.inner.journal).into_iter().find(|saved| saved.job_id == job.job_id).unwrap();
    assert!(before.active()); assert!(integrity_failed(&before));
    let restored = PlatformMediaService::new(fixture.root.path().join("data"), fixture.service.inner.tools.clone(), None).unwrap();
    let recovered = restored.inner.state.lock().unwrap().jobs[&job.job_id].clone();
    assert_eq!(recovered.state, "interrupted");
    assert!(integrity_failed(&recovered));
    assert!(restored.retry(&recovered.job_id, recovered.revision).is_err());
    let state = restored.inner.state.lock().unwrap();
    assert_eq!(state.jobs[&job.job_id].revision, recovered.revision);
    assert!(state.cancel.is_empty() && state.queue.is_empty()); drop(state);
    let saved = storage::load_jobs(&restored.inner.journal).into_iter().find(|saved| saved.job_id == job.job_id).unwrap();
    assert!(integrity_failed(&saved));
    assert!(!directory.join(FATAL_FILE).exists());
    restored.shutdown();
    fixture.stop(archive);
}

#[test]
fn raw_fatal_job_error_survives_shutdown_and_cancel_terminal_classification() {
    for shutdown in [false, true] {
        let fixture = Fixture::new(); let archive = fixture.begin();
        let writer = archive.writer.as_ref().unwrap(); let id = writer.job.job_id.clone();
        fixture.service.update(&id, |job| job.error = Some(FATAL.into())).unwrap();
        if shutdown { fixture.service.inner.state.lock().unwrap().shutting_down = true; }
        else { writer.cancel.store(true, Ordering::Release); }
        fixture.service.terminal_error(&id, &writer.cancel, "cancelled");
        let result = fixture.service.inner.state.lock().unwrap().jobs[&id].clone();
        assert_eq!(result.state, if shutdown { "interrupted" } else { "cancelled" });
        assert!(integrity_failed(&result));
        let saved = storage::load_jobs(&fixture.service.inner.journal).into_iter().find(|saved| saved.job_id == id).unwrap();
        assert!(integrity_failed(&saved));
        assert!(!writer.directory.join(FATAL_FILE).exists());
        fixture.stop(archive);
    }
}

#[test]
fn raw_integrity_latch_matches_only_exact_youtube_live_fatal_error() {
    let fixture = Fixture::new(); let archive = fixture.begin();
    let mut job = archive.snapshot().unwrap();
    assert!(!integrity_failed(&job)); job.error = Some(FATAL.into()); assert!(integrity_failed(&job));
    job.reference.provider = "soop".into(); assert!(!integrity_failed(&job));
    job.reference.provider = "youtube".into(); job.reference.kind = "video".into(); assert!(!integrity_failed(&job));
    job.reference.kind = "live".into(); job.error = Some(format!("{FATAL} ")); assert!(!integrity_failed(&job));
    fixture.stop(archive);
}
