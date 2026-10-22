//! Local publication of an already verified YouTube recording. This path never
//! receives a browser filename or relaxes the legacy providers' verification.
use super::{capture_outcome, lifecycle, storage, youtube_spool, *};
use std::time::Instant;

// Consumed only by the calling test thread. No release hook or cross-worker
// fault state; the isolated test introduces a real storage::save_job IO error.
#[cfg(test)]
thread_local! {
    static BEFORE_COMPLETED_SAVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

const ERROR: &str = "녹화 보관 상태가 변경됐습니다. 받은 파일은 보존했습니다.";

fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    if Instant::now() >= deadline {
        return Err("녹화 보관 시간이 초과됐습니다. 받은 파일은 보존했습니다.".into());
    }
    Ok(())
}

fn same_job(left: &MediaJob, right: &MediaJob) -> bool {
    left.job_id == right.job_id
        && left.request_id == right.request_id
        && left.reference == right.reference
        && left.broadcast_id == right.broadcast_id
        && left.download_root == right.download_root
        && left.created_at == right.created_at
}

fn check_attempt(
    state: &State,
    job: &MediaJob,
    cancel: &Arc<AtomicBool>,
    revision: u64,
    deadline: Instant,
) -> Result<MediaJob, String> {
    checkpoint(cancel, deadline)?;
    let current = state.jobs.get(&job.job_id).ok_or(ERROR)?;
    if state.shutting_down
        || current.revision != revision
        || !same_job(current, job)
        || current.library.is_some()
        || !matches!(current.state.as_str(), "finalizing" | "verifying")
        || !state.cancel.get(&job.job_id).is_some_and(|known| Arc::ptr_eq(known, cancel))
    {
        return Err(ERROR.into());
    }
    Ok(current.clone())
}

fn check_identity(job: &MediaJob, expected: &youtube_spool::SpoolIdentity) -> Result<(), String> {
    if job.reference.provider != "youtube"
        || job.reference.kind != "live"
        || job.job_id != expected.job_id
        || job.request_id != expected.request_id
        || job.reference.external_id != expected.video_id
        || job.broadcast_id.as_deref() != Some(expected.video_id.as_str())
    {
        return Err(ERROR.into());
    }
    Ok(())
}

fn persist(service: &PlatformMediaService, state: &mut State, next: MediaJob) -> Result<(), String> {
    if let Err(error) = storage::save_job(&service.inner.journal, &next) {
        lifecycle::remember_persistence_error(state, &next.job_id, lifecycle::persistence_message(&error));
        return Err(error);
    }
    state.persistence_errors.remove(&next.job_id);
    state.jobs.insert(next.job_id.clone(), next);
    Ok(())
}

impl PlatformMediaService {
    /// The caller retains JobOwnership and supplies its immutable native spool
    /// identity. A successful rename alone never creates a completed job.
    pub(super) fn publish_verified_youtube(
        &self,
        job: &MediaJob,
        cancel: &Arc<AtomicBool>,
        expected: &youtube_spool::SpoolIdentity,
        handoff: youtube_spool::PublicationHandoff,
        deadline: Instant,
    ) -> Result<(), String> {
        self.publish_youtube_inner(job, cancel, expected, handoff, deadline, false)
    }

    /// Reached only by the explicit raw-recovery service entry, never ordinary
    /// capture finish or the absence of a detached chat sink.
    pub(super) fn publish_recovered_youtube(
        &self,
        job: &MediaJob,
        cancel: &Arc<AtomicBool>,
        expected: &youtube_spool::SpoolIdentity,
        handoff: youtube_spool::PublicationHandoff,
        deadline: Instant,
    ) -> Result<(), String> {
        self.publish_youtube_inner(job, cancel, expected, handoff, deadline, true)
    }

    fn publish_youtube_inner(
        &self,
        job: &MediaJob,
        cancel: &Arc<AtomicBool>,
        expected: &youtube_spool::SpoolIdentity,
        mut handoff: youtube_spool::PublicationHandoff,
        deadline: Instant,
        recovery: bool,
    ) -> Result<(), String> {
        check_identity(job, expected)?;
        if !handoff.matches_scope(&job.download_root, expected) {
            return Err(ERROR.into());
        }
        let facts = handoff.facts();
        if facts.bytes < 1024 || !facts.duration.is_finite() || facts.duration <= 0.0 || facts.duration > 86_400.0 {
            return Err(ERROR.into());
        }
        let bytes = facts.bytes;
        let duration = facts.duration;
        let sha256 = facts.sha256.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let capture_outcome = capture_outcome::from_source(&facts.source)?;
        let mut revision = job.revision;
        // Share the existing bounded verification slot. No hash/process/chat I/O
        // occurs while the state mutex is held.
        let _verification = loop {
            checkpoint(cancel, deadline)?;
            match self.inner.verify.try_lock() {
                Ok(permit) => break permit,
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(ERROR.into()),
                Err(std::sync::TryLockError::WouldBlock) => {}
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        {
            let mut state = self.inner.state.lock().map_err(|_| ERROR)?;
            let mut next = check_attempt(&state, job, cancel, revision, deadline)?;
            next.state = "verifying".into();
            next.received_bytes = bytes;
            next.total_bytes = Some(bytes);
            next.revision = revision.checked_add(1).ok_or(ERROR)?;
            revision = next.revision;
            persist(self, &mut state, next)?;
        }
        {
            let state = self.inner.state.lock().map_err(|_| ERROR)?;
            check_attempt(&state, job, cancel, revision, deadline)?;
            let adopted = recovery && handoff.adopt_existing_if_present(cancel, deadline).map_err(|_| ERROR)?;
            if !adopted { handoff.install_no_replace(cancel, deadline).map_err(|_| ERROR)?; }
        }
        handoff.verify_installed(cancel, deadline).map_err(|_| ERROR)?;
        // Existing recovery keeps partial/unknown chat sticky. It never writes a
        // new timeline using the time at which remuxing happened to finish.
        let chat_archive = handoff.with_final_path(cancel, deadline, |path, _| {
            super::chat_status::recovery_with_cancel(
                path.parent().ok_or_else(|| ERROR.to_string())?, "youtube", &job.job_id, cancel,
            )
        }).map_err(|_| ERROR)??;
        let modified_millis = handoff.modified_millis(cancel, deadline).map_err(|_| ERROR)?;
        #[cfg(test)]
        {
            let hook = BEFORE_COMPLETED_SAVE.with(|slot| slot.borrow_mut().take());
            if let Some(hook) = hook { hook(); }
        }
        {
            let mut state = self.inner.state.lock().map_err(|_| ERROR)?;
            let mut next = check_attempt(&state, job, cancel, revision, deadline)?;
            handoff.check_for_commit(cancel, deadline).map_err(|_| ERROR)?;
            next.revision = revision.checked_add(1).ok_or(ERROR)?;
            next.state = "completed".into();
            next.error = None;
            next.library = Some(LibraryItem {
                broadcast_id: next.broadcast_id.clone(),
                chat_archive: Some(chat_archive),
                capture_outcome: Some(capture_outcome),
                library_id: next.job_id.clone(),
                revision: next.revision,
                reference: next.reference.clone(),
                title: next.title.clone(),
                channel_name: next.channel_name.clone(),
                bytes, duration, sha256,
                completed_at: now(),
                modified_millis,
            });
            // Cancel/shutdown/retry serialize on this same mutex. The immutable
            // installed handle stays alive until the durable state is replaced.
            persist(self, &mut state, next)?;
        }
        self.inner.ready.notify_all();
        // Raw pieces, failed attempts and chat remain untouched here. Cleanup
        // needs the later exact-owned proof and truthful partial-source policy.
        Ok(())
    }
}

/// Only the ignored real-capture tests call this, with an actual finalizer proof.
/// It cannot construct a candidate, change the fixture, or access app data.
#[cfg(test)]
pub(super) fn publish_actual_fixture(
    root: &std::path::Path,
    identity: &youtube_spool::SpoolIdentity,
    cancel: Arc<AtomicBool>,
    candidate: youtube_spool::VerifiedCandidate,
    deadline: Instant,
    partial: bool,
) {
    use sha2::{Digest, Sha256};
    let tools = PlatformMediaTools {
        yt_dlp: root.join("unused"), deno: root.join("unused"),
        ffmpeg: root.join("unused"), ffprobe: root.join("unused"),
    };
    // The real application keeps state outside the media tree. Mirroring that
    // also avoids a test-only >MAX_PATH nested journal temporary filename.
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        .join(".runtime").join(format!("yt-publication-{}", identity.job_id));
    assert!(!data.exists());
    let service = PlatformMediaService::new(data.clone(), tools.clone(), None).unwrap();
    let job = MediaJob {
        broadcast_id: Some(identity.video_id.clone()), job_id: identity.job_id.clone(),
        request_id: identity.request_id.clone(), reference: MediaRef {
            provider: "youtube".into(), kind: "live".into(), external_id: identity.video_id.clone(),
        }, revision: 7, state: "finalizing".into(), title: "Actual isolated capture".into(),
        channel_name: "Fixture".into(), created_at: now(), received_bytes: 0,
        total_bytes: None, error: None, library: None, download_root: root.to_owned(),
    };
    {
        let mut state = service.inner.state.lock().unwrap();
        persist(&service, &mut state, job.clone()).unwrap();
        state.cancel.insert(job.job_id.clone(), cancel.clone());
    }
    let owner = lifecycle::JobOwnership::new(service.clone(), job.job_id.clone(), cancel.clone());
    let handoff = candidate.into_publication(root, identity, &cancel, deadline).unwrap();
    let expected_bytes = handoff.facts().bytes;
    let expected_duration = handoff.facts().duration;
    let expected_hash = handoff.facts().sha256.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    service.publish_verified_youtube(&job, &cancel, identity, handoff, deadline).unwrap();
    drop(owner);
    assert!(service.active_ids().is_empty());
    let listed = service.library("youtube", None, 10).unwrap();
    assert_eq!(listed.items.len(), 1);
    assert!(service.library("soop", None, 10).unwrap().items.is_empty());
    let item = &listed.items[0];
    assert_eq!(item.bytes, expected_bytes);
    assert_eq!(item.duration, expected_duration);
    assert_eq!(item.sha256, expected_hash);
    assert_eq!(item.revision, 9);
    let summary = serde_json::to_value(item.capture_outcome.as_ref().unwrap()).unwrap();
    assert_eq!(summary["state"], if partial { "partial" } else { "complete" });
    assert!(item.chat_archive.as_ref().is_some_and(|chat| {
        serde_json::to_value(chat).unwrap()["state"] != "complete"
    }), "missing fixture chat must never become complete");
    let final_path = storage::job_dir(root, "youtube", &job.job_id).unwrap().join("media.mp4");
    let final_bytes = fs::read(&final_path).unwrap();
    assert_eq!(final_bytes.len() as u64, expected_bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&final_bytes)), expected_hash);
    let saved = storage::load_jobs(&service.inner.journal).pop().unwrap();
    assert_eq!(saved.state, "completed");
    assert_eq!(saved.library.as_ref().unwrap().revision, 9);
    assert_eq!(serde_json::to_value(saved.library.unwrap().capture_outcome).unwrap(), summary);
    service.shutdown();
    drop(service);

    let restored = PlatformMediaService::new(data, tools, None).unwrap();
    let restored_item = restored.library("youtube", None, 10).unwrap().items.pop().unwrap();
    assert_eq!(serde_json::to_value(restored_item.capture_outcome).unwrap(), summary);
    let playback = restored.open(&job.job_id).expect("actual published file opens after service restart");
    for (start, end) in [(0, 31), (expected_bytes / 2, expected_bytes / 2 + 31), (expected_bytes - 32, expected_bytes - 1)] {
        let request = tauri::http::Request::builder().uri(&playback.url)
            .header("Range", format!("bytes={start}-{end}")).body(Vec::new()).unwrap();
        let response = super::protocol::respond(&restored, "main", request);
        assert_eq!(response.status(), tauri::http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body(), &final_bytes[start as usize..=end as usize]);
    }
    restored.close(&playback.token);
    let directory = final_path.parent().unwrap();
    assert!(directory.join("video.raw").is_file() && directory.join("audio.raw").is_file());
    println!("published={} bytes={} sha256={} outcome={} restartOpen=true rangeReads=true productionCaptureVerified=false",
        final_path.display(), expected_bytes, expected_hash, summary["state"]);
    restored.shutdown();
}

/// Called only by the ignored pinned-source test after real raw preparation.
/// All paths are a newly created test root, never the original capture folder.
#[cfg(all(test, windows))]
pub(super) fn recover_actual_fixture(
    root: &std::path::Path,
    identity: &youtube_spool::SpoolIdentity,
    tools: PlatformMediaTools,
    partial: bool,
) {
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION}};
    struct Shutdown(PlatformMediaService);
    impl Drop for Shutdown { fn drop(&mut self) { self.0.shutdown(); } }
    fn key(path: &std::path::Path) -> (u32, u32, u32) {
        let file = fs::File::open(path).unwrap();
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.unwrap();
        assert_eq!(info.nNumberOfLinks, 1);
        (info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow)
    }
    fn create(path: &std::path::Path, bytes: &[u8]) {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
        file.write_all(bytes).unwrap(); file.sync_all().unwrap();
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let data = workspace.join(".runtime").join(format!("youtube-recovery-state-{}", uuid::Uuid::new_v4()));
    assert!(!data.exists());
    let service = PlatformMediaService::new(data.clone(), tools.clone(), None).unwrap();
    let _shutdown = Shutdown(service.clone());
    let directory = storage::job_dir(root, "youtube", &identity.job_id).unwrap();
    assert_eq!(identity.owner, youtube_spool::Owner::Manual);
    let job = MediaJob {
        broadcast_id: Some(identity.video_id.clone()), job_id: identity.job_id.clone(), request_id: identity.request_id.clone(),
        reference: MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: identity.video_id.clone() },
        revision: 7, state: "finalizing".into(), title: "Isolated crash fixture".into(), channel_name: "Fixture".into(),
        created_at: now(), received_bytes: 0, total_bytes: None, error: None, library: None, download_root: root.to_owned(),
    };
    let owner = serde_json::to_value(super::live::RecordingOwner::Manual).unwrap();
    create(&directory.join("recording-owner.json"), &serde_json::to_vec(&owner).unwrap());
    create(&directory.join(super::encoded::PROOF), &serde_json::to_vec(&serde_json::json!({
        "version":1,"reference":job.reference,"broadcastId":identity.video_id,"receiverId":identity.receiver_id,
        "sourceId":identity.source_id,"generation":identity.document_epoch,"requestId":identity.request_id,
        "jobId":identity.job_id,"owner":owner
    })).unwrap());
    let marker = super::chat_status::begin(&directory, "youtube", &identity.job_id).unwrap();
    create(&directory.join("chat.events.jsonl"), b"");
    let chat = super::chat_status::finish(&directory, &marker, Some(super::chat_status::Reason::StatusUnknown), false).unwrap();
    // This is a controlled legacy-timeline preservation fixture, not new raw
    // timing evidence or proof of actual chat replay synchronization.
    create(&directory.join("chat-timeline.json"), br#"{"version":1,"timelineQuality":"receive-time-approximate","firstSegmentCommittedAtMs":1700000005000,"firstSegmentDurationMs":5000}"#);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut preserved: Vec<(PathBuf, String)> = fs::read_dir(&directory).unwrap().map(|entry| {
        let path = entry.unwrap().path();
        assert!(path.is_file());
        let digest = storage::hash(&path, &cancel).unwrap();
        (path, digest)
    }).collect();
    preserved.sort_by(|a,b| a.0.cmp(&b.0));
    {
        let mut state = service.inner.state.lock().unwrap();
        storage::save_job(&service.inner.journal, &job).unwrap();
        state.jobs.insert(job.job_id.clone(), job.clone());
        state.cancel.insert(job.job_id.clone(), cancel.clone());
    }
    let guard = lifecycle::JobOwnership::new(service.clone(), job.job_id.clone(), cancel.clone());
    let end = Instant::now() + Duration::from_secs(240);
    let candidate = youtube_spool::prepare_recording(root, identity, &tools, &cancel, end).unwrap();
    let handoff = candidate.into_publication(root, identity, &cancel, end).unwrap();
    let expected_hash = handoff.facts().sha256.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let expected_bytes = handoff.facts().bytes;
    let expected_duration = handoff.facts().duration;
    let final_path = directory.join("media.mp4");
    let journal = service.inner.journal.clone();
    let old_journal = journal.with_file_name("jobs-before-completion");
    let callback_journal = journal.clone(); let callback_preserved = old_journal.clone();
    let callback_final = final_path.clone();
    BEFORE_COMPLETED_SAVE.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            assert!(callback_final.is_file());
            fs::rename(&callback_journal, &callback_preserved).unwrap();
            create(&callback_journal, b"test-owned journal IO blocker");
        }));
    });
    let result = service.publish_verified_youtube(&job, &cancel, identity, handoff, end);
    // Clear even if an earlier stage failed, to avoid affecting a later test on
    // the same libtest thread. The expected path consumes the hook before save.
    let unused_hook = BEFORE_COMPLETED_SAVE.with(|slot| slot.borrow_mut().take());
    assert!(unused_hook.is_none(), "failure occurred before completed-journal injection");
    assert!(result.is_err());
    assert!(service.shutdown_error().is_some());
    assert!(service.inner.state.lock().unwrap().jobs[&job.job_id].library.is_none());
    let saved = storage::load_jobs(&old_journal).pop().unwrap();
    assert_eq!(saved.state, "verifying"); assert!(saved.library.is_none());
    assert_eq!(storage::hash(&final_path, &AtomicBool::new(false)).unwrap(), expected_hash);
    let final_key = key(&final_path);
    let modified = fs::metadata(&final_path).unwrap().modified().unwrap();
    drop(guard); service.shutdown(); drop(service);
    // Preserve the injected blocker as evidence too; no test cleanup/delete.
    fs::rename(&journal, journal.with_file_name("journal-io-blocker.bin")).unwrap();
    fs::rename(&old_journal, &journal).unwrap();
    let restored = PlatformMediaService::new(data.clone(), tools.clone(), None).unwrap();
    let _restored_shutdown = Shutdown(restored.clone());
    let interrupted = restored.inner.state.lock().unwrap().jobs[&job.job_id].clone();
    assert_eq!(interrupted.state, "interrupted"); assert!(interrupted.library.is_none());
    restored.retry(&job.job_id, interrupted.revision).unwrap();
    let wait_until = Instant::now() + Duration::from_secs(240);
    let completed = loop {
        let current = restored.inner.state.lock().unwrap().jobs[&job.job_id].clone();
        if !current.active() { break current; }
        if Instant::now() >= wait_until { restored.shutdown(); panic!("bounded recovery retry timed out"); }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(completed.state, "completed", "actual recovery failed: {:?}", completed.error);
    let item = completed.library.as_ref().unwrap();
    assert_eq!(item.sha256, expected_hash); assert_eq!(item.bytes, expected_bytes);
    assert_eq!(item.duration, expected_duration);
    assert_eq!(serde_json::to_value(&item.chat_archive).unwrap(), serde_json::to_value(Some(&chat)).unwrap());
    let outcome = serde_json::to_value(&item.capture_outcome).unwrap();
    assert_eq!(outcome["state"], if partial { "partial" } else { "complete" });
    assert_eq!(key(&final_path), final_key);
    assert_eq!(fs::metadata(&final_path).unwrap().modified().unwrap(), modified);
    for (path, hash) in &preserved { assert_eq!(&storage::hash(path, &AtomicBool::new(false)).unwrap(), hash); }
    assert!(restored.retry(&job.job_id, completed.revision).is_err());
    assert_eq!(restored.library("youtube", None, 10).unwrap().items.len(), 1);
    restored.shutdown(); drop(restored);
    let reopened = PlatformMediaService::new(data, tools, None).unwrap();
    let _reopened_shutdown = Shutdown(reopened.clone());
    assert_eq!(reopened.library("youtube", None, 10).unwrap().items.len(), 1);
    let playback = reopened.open(&job.job_id).unwrap();
    for (start, last) in [(0,31),(expected_bytes-32,expected_bytes-1)] {
        let request = tauri::http::Request::builder().uri(&playback.url)
            .header("Range",format!("bytes={start}-{last}")).body(Vec::new()).unwrap();
        let response = super::protocol::respond(&reopened,"main",request);
        assert_eq!(response.status(),tauri::http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body().len(),32);
    }
    reopened.close(&playback.token); reopened.shutdown();
    println!("recoveryRoot={} exactFinalPreserved=true bytes={} captureOutcome={} chatPartialPreserved=true restartOpen=true actualPlayerVerified=false",
        root.display(), expected_bytes, outcome["state"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &std::path::Path) -> (State, MediaJob, Arc<AtomicBool>) {
        let job = MediaJob {
            broadcast_id: Some("abcdefghijk".into()), job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(), reference: MediaRef { provider: "youtube".into(), kind: "live".into(), external_id: "abcdefghijk".into() },
            revision: 7, state: "finalizing".into(), title: "fixture".into(), channel_name: "fixture".into(),
            created_at: 1, received_bytes: 2, total_bytes: None, error: None, library: None, download_root: root.join("media"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let state = State { jobs: [(job.job_id.clone(), job.clone())].into(), cancel: [(job.job_id.clone(), cancel.clone())].into(), queue: VecDeque::new(), shutting_down: false, persistence_errors: HashMap::new(), persistence_overflow: false };
        (state, job, cancel)
    }
    fn deadline() -> Instant { Instant::now() + Duration::from_secs(30) }

    #[test]
    fn exact_youtube_live_job_identity_is_required() {
        let root = tempfile::tempdir().unwrap();
        let (_, job, _) = fixture(root.path());
        let id = youtube_spool::SpoolIdentity {
            job_id: job.job_id.clone(), request_id: job.request_id.clone(),
            receiver_id: uuid::Uuid::new_v4().to_string(), source_id: uuid::Uuid::new_v4().to_string(),
            document_id: uuid::Uuid::new_v4().to_string(), document_epoch: 1, source_generation: 1,
            video_id: job.reference.external_id.clone(), channel_id: format!("UC{}", "a".repeat(22)),
            owner: youtube_spool::Owner::Manual,
        };
        assert!(check_identity(&job, &id).is_ok());
        for field in 0..6 {
            let mut other = job.clone();
            match field {
                0 => other.job_id = uuid::Uuid::new_v4().to_string(),
                1 => other.request_id = uuid::Uuid::new_v4().to_string(),
                2 => other.reference.provider = "soop".into(),
                3 => other.reference.kind = "video".into(),
                4 => other.reference.external_id = "other-video".into(),
                _ => other.broadcast_id = None,
            }
            assert!(check_identity(&other, &id).is_err());
        }
    }

    #[test]
    fn old_library_missing_or_null_outcome_is_never_inferred_complete() {
        let value = serde_json::json!({
            "libraryId": uuid::Uuid::new_v4().to_string(), "revision": 1,
            "reference": {"provider": "youtube", "kind": "live", "externalId": "abcdefghijk"},
            "title": "old fixture", "channelName": "fixture", "bytes": 2048,
            "duration": 5.0, "sha256": "0".repeat(64), "completedAt": 1,
        });
        for explicit_null in [false, true] {
            let mut json = value.clone();
            if explicit_null { json["captureOutcome"] = serde_json::Value::Null; }
            let item: LibraryItem = serde_json::from_value(json).unwrap();
            assert!(item.capture_outcome.is_none());
            assert!(serde_json::to_value(item).unwrap().get("captureOutcome").is_none());
        }
    }

    #[test]
    fn exact_attempt_revision_and_token_are_required() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, job, cancel) = fixture(root.path());
        assert!(check_attempt(&state, &job, &cancel, 7, deadline()).is_ok());
        assert!(check_attempt(&state, &job, &cancel, 6, deadline()).is_err());
        assert!(check_attempt(&state, &job, &Arc::new(AtomicBool::new(false)), 7, deadline()).is_err());
        for field in 0..6 {
            let mut other = job.clone();
            match field {
                0 => other.request_id = uuid::Uuid::new_v4().to_string(),
                1 => other.reference.provider = "soop".into(),
                2 => other.reference.external_id = "other-video".into(),
                3 => other.broadcast_id = None,
                4 => other.download_root = root.path().join("other"),
                _ => other.created_at += 1,
            }
            state.jobs.insert(job.job_id.clone(), other);
            assert!(check_attempt(&state, &job, &cancel, 7, deadline()).is_err());
        }
    }

    #[test]
    fn terminal_cancelled_shutdown_or_expired_attempt_cannot_install() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, job, cancel) = fixture(root.path());
        for phase in ["completed", "cancelled", "recording", "queued", "interrupted"] {
            state.jobs.get_mut(&job.job_id).unwrap().state = phase.into();
            assert!(check_attempt(&state, &job, &cancel, 7, deadline()).is_err());
        }
        state.jobs.get_mut(&job.job_id).unwrap().state = "finalizing".into();
        state.shutting_down = true;
        assert!(check_attempt(&state, &job, &cancel, 7, deadline()).is_err());
        state.shutting_down = false;
        cancel.store(true, Ordering::Release);
        assert!(check_attempt(&state, &job, &cancel, 7, deadline()).is_err());
        cancel.store(false, Ordering::Release);
        assert!(check_attempt(&state, &job, &cancel, 7, Instant::now()).is_err());
    }

    #[test]
    fn failed_journal_preserves_previous_memory_and_journal() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, job, _) = fixture(root.path());
        let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
            yt_dlp: root.path().join("unused"), deno: root.path().join("unused"), ffmpeg: root.path().join("unused"), ffprobe: root.path().join("unused"),
        }, None).unwrap();
        storage::save_job(&service.inner.journal, &job).unwrap();
        let old = fs::read(service.inner.journal.join(format!("{}.json", job.job_id))).unwrap();
        let preserved = root.path().join("preserved");
        fs::rename(&service.inner.journal, &preserved).unwrap();
        fs::write(&service.inner.journal, b"test-owned blocker").unwrap();
        let mut next = job.clone(); next.revision += 1; next.state = "completed".into();
        assert!(persist(&service, &mut state, next).is_err());
        assert_eq!(state.jobs[&job.job_id].revision, 7);
        assert_eq!(state.jobs[&job.job_id].state, "finalizing");
        assert!(state.persistence_errors.contains_key(&job.job_id));
        assert_eq!(fs::read(preserved.join(format!("{}.json", job.job_id))).unwrap(), old);
        service.shutdown();
    }

    #[test]
    fn recovery_final_cas_does_not_accept_a_newer_attempt_or_library() {
        let root = tempfile::tempdir().unwrap();
        let (mut state, job, cancel) = fixture(root.path());
        state.jobs.get_mut(&job.job_id).unwrap().state = "verifying".into();
        assert!(check_attempt(&state, &job, &cancel, job.revision, deadline()).is_ok());
        // Represents the mandatory second fence after adopted-file hashing.
        state.cancel.insert(job.job_id.clone(), Arc::new(AtomicBool::new(false)));
        assert!(check_attempt(&state, &job, &cancel, job.revision, deadline()).is_err());
        state.cancel.insert(job.job_id.clone(), cancel.clone());
        state.jobs.get_mut(&job.job_id).unwrap().revision += 1;
        assert!(check_attempt(&state, &job, &cancel, job.revision, deadline()).is_err());
        state.jobs.get_mut(&job.job_id).unwrap().revision = job.revision;
        state.jobs.get_mut(&job.job_id).unwrap().state = "completed".into();
        assert!(check_attempt(&state, &job, &cancel, job.revision, deadline()).is_err());
    }
}
