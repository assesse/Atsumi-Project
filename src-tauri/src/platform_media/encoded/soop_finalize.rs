//! SOOP-only bounded salvage/retry. A stop marker is reporting evidence, not
//! permission to skip media verification or delete original data.
use super::*;

const STOP: &str = "encoded-stop.json";
const MERGE_ERROR: &str = "라이브 병합에 실패했습니다. 원본 조각을 보존했습니다.";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StopProof {
    version: u32,
    job_id: String,
    owner_sha256: String,
    manifest_sha256: String,
}

fn eligible(job: &MediaJob) -> bool {
    job.reference.provider == "soop" && job.reference.kind == "live"
}

pub(in crate::platform_media) fn owner(
    service: &PlatformMediaService, job: &MediaJob, cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let state = service.inner.state.lock().map_err(|_| ERROR)?;
    owned(&state, job, cancel)
}

fn owned(state: &State, job: &MediaJob, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    if !eligible(job) || state.shutting_down || cancel.load(Ordering::Acquire)
        || !state.cancel.get(&job.job_id).is_some_and(|current| Arc::ptr_eq(current, cancel))
        || !state.jobs.get(&job.job_id).is_some_and(|current| {
            current.active() && current.library.is_none()
                && current.reference == job.reference && current.request_id == job.request_id
                && current.broadcast_id == job.broadcast_id && current.download_root == job.download_root
        })
    { return Err("녹화 마무리 소유권이 변경됐습니다. 원본은 보존했습니다.".into()); }
    Ok(())
}

pub(in crate::platform_media) fn start(
    service: &PlatformMediaService, job: &MediaJob, cancel: &Arc<AtomicBool>,
) -> Result<MediaJob, String> {
    let mut state = service.inner.state.lock().map_err(|_| ERROR)?;
    owned(&state, job, cancel)?;
    let mut current = state.jobs.get(&job.job_id).ok_or(ERROR)?.clone();
    current.state = "finalizing".into();
    current.revision = current.revision.checked_add(1).ok_or(ERROR)?;
    storage::save_job(&service.inner.journal, &current)?;
    state.persistence_errors.remove(&job.job_id);
    state.jobs.insert(job.job_id.clone(), current.clone());
    Ok(current)
}

fn binding(job: &MediaJob, directory: &Path, cancel: &AtomicBool) -> Result<StopProof, String> {
    if !eligible(job) || directory != storage::job_dir(&job.download_root, "soop", &job.job_id)? {
        return Err(ERROR.into());
    }
    validate_proof(job, directory)?;
    let manifest = directory.join(MANIFEST);
    if storage::regular(&manifest)?.len() > 32 * 1024 * 1024 { return Err(ERROR.into()); }
    Ok(StopProof {
        version: 1, job_id: job.job_id.clone(),
        owner_sha256: storage::hash(&directory.join(PROOF), cancel)?,
        manifest_sha256: storage::hash(&manifest, cancel)?,
    })
}

pub(in crate::platform_media) fn outcome(
    job: &MediaJob, directory: &Path, cancel: &AtomicBool,
) -> Result<Option<super::super::capture_outcome::CaptureOutcome>, String> {
    if !eligible(job) { return Ok(None); }
    let path = directory.join(STOP);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ERROR.into()),
        Ok(_) => (),
    }
    if storage::regular(&path)?.len() > 2048 { return Err(ERROR.into()); }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(fs::File::open(&path).map_err(|_| ERROR)?, 2049), &mut bytes,
    ).map_err(|_| ERROR)?;
    if bytes.len() > 2048 { return Err(ERROR.into()); }
    let proof: StopProof = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    let expected = binding(job, directory, cancel)?;
    if proof.version != 1 || proof.job_id != expected.job_id
        || proof.owner_sha256 != expected.owner_sha256
        || proof.manifest_sha256 != expected.manifest_sha256
    { return Err(ERROR.into()); }
    Ok(Some(super::super::capture_outcome::CaptureOutcome::soop_interrupted()))
}

fn mark(writer: &Writer) -> Result<(), String> {
    owner(&writer.lease.service, &writer.job, &writer.cancel)?;
    if writer.segments.is_empty()
        || recovery_segments_cancellable(&writer.job, &writer.directory, &writer.cancel)? != writer.segments
    { return Err(ERROR.into()); }
    let proof = binding(&writer.job, &writer.directory, &writer.cancel)?;
    write_new_json(&writer.directory.join(STOP), &proof)?;
    owner(&writer.lease.service, &writer.job, &writer.cancel)
}

fn incomplete_box(code: &str, message: &str) -> bool {
    code == "ENCODED_UNSUPPORTED" && message == "마지막 MP4 fragment가 불완전합니다."
}
fn incomplete_audio(code: &str, message: &str) -> bool {
    // This is a native muxer-generated diagnostic, never an incoming page error.
    code == "ENCODED_UNSUPPORTED"
        && message.starts_with("마지막 영상 구간의 음성이 완전하지 않습니다. (음성 끝 차이 ")
        && message.ends_with("초)")
}

pub(super) fn finish(writer: &mut Writer, interrupted: bool) -> Result<(), String> {
    owner(&writer.lease.service, &writer.job, &writer.cancel)?;
    if writer.poisoned { return Err(ERROR.into()); }
    let mut interrupted = interrupted || writer.partial.is_some();
    let mut tail = if interrupted { writer.muxer.finish_discontinuity() } else { writer.muxer.finish() };
    if !interrupted && tail.as_ref().is_err_and(|error| incomplete_box(&error.code, &error.message)) {
        interrupted = true;
        tail = writer.muxer.finish_discontinuity();
    }
    match tail {
        Ok(parts) => writer.commit(parts)?, // A write/hash/commit failure is never salvageable here.
        Err(error) if incomplete_audio(&error.code, &error.message) && !writer.segments.is_empty() => {
            // The unfinished A/V tail was not committed. Only preceding,
            // independently valid durable segments are eligible for verification.
            interrupted = true;
        }
        Err(error) => return Err(error.message),
    }
    if interrupted { mark(writer)?; }
    owner(&writer.lease.service, &writer.job, &writer.cancel)
}

pub(in crate::platform_media) fn preflight(
    service: &PlatformMediaService, job: &MediaJob, directory: &Path,
    cancel: &Arc<AtomicBool>, segments: &[(String, f64)], reserve: u64,
) -> Result<(), String> {
    owner(service, job, cancel)?;
    if directory != storage::job_dir(&job.download_root, "soop", &job.job_id)?
        || recovery_segments_cancellable(job, directory, cancel)? != segments
    { return Err(ERROR.into()); }
    outcome(job, directory, cancel)?;
    if fs2::available_space(directory).map_err(|_| "남은 저장 공간을 확인하지 못했습니다.")? < reserve {
        return Err("저장 공간이 부족합니다. 원본 조각은 보존했습니다.".into());
    }
    owner(service, job, cancel)
}

pub(in crate::platform_media) fn backoff(
    service: &PlatformMediaService, job: &MediaJob, cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    for _ in 0..10 {
        owner(service, job, cancel)?;
        std::thread::sleep(Duration::from_millis(25));
    }
    owner(service, job, cancel)
}

/// Only `Ok(None)` denotes a process that actually exited unsuccessfully.
/// Errors (spawn, timeout, disk, proof, cancellation) never become retryable.
pub(in crate::platform_media) fn merge<T>(
    retry: bool, mut attempt: impl FnMut() -> Result<Option<T>, String>,
    mut backoff: impl FnMut() -> Result<(), String>,
) -> Result<T, String> {
    if let Some(value) = attempt()? { return Ok(value); }
    if retry {
        backoff()?;
        if let Some(value) = attempt()? { return Ok(value); }
    }
    Err(MERGE_ERROR.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn only_nonzero_exit_can_retry_and_only_once() {
        let calls = Cell::new(0);
        let waits = Cell::new(0);
        let result = merge(true, || {
            calls.set(calls.get() + 1);
            Ok((calls.get() == 2).then_some(7))
        }, || { waits.set(waits.get() + 1); Ok(()) });
        assert_eq!(result.unwrap(), 7);
        assert_eq!((calls.get(), waits.get()), (2, 1));
        for retry in [false, true] {
            calls.set(0);
            let result: Result<(), _> = merge(retry, || { calls.set(calls.get() + 1); Ok(None) }, || Ok(()));
            assert!(result.is_err());
            assert_eq!(calls.get(), if retry { 2 } else { 1 });
        }
    }

    #[test]
    fn failure_or_revocation_never_starts_second_attempt() {
        let calls = Cell::new(0);
        let result: Result<(), _> = merge(true, || { calls.set(calls.get() + 1); Err("spawn".into()) }, || panic!("no wait"));
        assert_eq!(result.unwrap_err(), "spawn");
        assert_eq!(calls.get(), 1);
        calls.set(0);
        let result: Result<(), _> = merge(true, || { calls.set(calls.get() + 1); Ok(None) }, || Err("revoked".into()));
        assert_eq!(result.unwrap_err(), "revoked");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn tail_errors_are_native_and_narrow() {
        assert!(incomplete_box("ENCODED_UNSUPPORTED", "마지막 MP4 fragment가 불완전합니다."));
        assert!(!incomplete_box("OTHER", "마지막 MP4 fragment가 불완전합니다."));
        assert!(!incomplete_box("ENCODED_UNSUPPORTED", "완전한 영상·음성 시작 구간을 받지 못했습니다."));
        assert!(incomplete_audio("ENCODED_UNSUPPORTED", "마지막 영상 구간의 음성이 완전하지 않습니다. (음성 끝 차이 Some(1.0)초)"));
        assert!(!incomplete_audio("OTHER", "마지막 영상 구간의 음성이 완전하지 않습니다. (음성 끝 차이 Some(1.0)초)"));
    }

    #[test]
    fn stopped_outcome_is_interrupted_not_complete_and_schema_is_closed() {
        let outcome = super::super::super::capture_outcome::CaptureOutcome::soop_interrupted();
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["state"], "interrupted");
        assert_eq!(value["reasons"], serde_json::json!(["source_interrupted"]));
        assert_eq!(serde_json::from_value::<super::super::super::capture_outcome::CaptureOutcome>(value).unwrap(), outcome);
        let proof = serde_json::json!({"version":1,"job_id":"fixture","owner_sha256":"a","manifest_sha256":"b","extra":true});
        assert!(serde_json::from_value::<StopProof>(proof).is_err());
    }

    #[test]
    #[ignore = "root-only isolated synthetic local finalizer; no provider/account/user recordings"]
    fn actual_soop_interrupted_prefix_is_verified_reopened_and_retained() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let root = repo.join(".runtime/soop-finalization-retry-tests").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let bin = repo.join(".runtime/media-tools/ffmpeg-n9.0.1-29-gad500d59cb-win64-lgpl-shared-9.0/bin");
        let tools = PlatformMediaTools {
            yt_dlp: root.join("no-network.exe"), deno: root.join("no-network.exe"),
            ffmpeg: bin.join("ffmpeg.exe"), ffprobe: bin.join("ffprobe.exe"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let generated = process::run(&tools.ffmpeg, &[
            "-hide_banner", "-nostdin", "-v", "error", "-n",
            "-f", "lavfi", "-i", "color=c=black:s=160x90:r=10",
            "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-t", "3",
            "-c:v", "libopenh264", "-c:a", "aac", "-threads", "1",
            "-movflags", "+faststart", "fixture.mp4",
        ].map(String::from), &root, &cancel, Duration::from_secs(20), 1024).unwrap();
        assert!(generated.success);
        let service = PlatformMediaService::new(root.join("data"), tools.clone(), None).unwrap();
        service.inner.synthetic_live_receiver.store(true, Ordering::Release);
        let lease = service.reserve_official_receiver(
            MediaRef { provider: "soop".into(), kind: "live".into(), external_id: "fixture".into() },
            "123".into(), uuid::Uuid::new_v4().to_string(), root.join("media"),
        ).unwrap();
        let mut archive = lease.begin_encoded(
            uuid::Uuid::new_v4().to_string(), 1, uuid::Uuid::new_v4().to_string(),
            fmp4::fixtures::inputs(), "Synthetic stopped recording".into(), "Fixture".into(),
            live::RecordingOwner::Manual,
        ).unwrap();
        let mut writer = archive.writer.take().unwrap();
        // This fixture proves local finalization/reporting/reopen, not browser
        // capture or parser admission. The pure archive tests cover that seam.
        writer.commit(vec![fmp4::EncodedSegment {
            bytes: fs::read(root.join("fixture.mp4")).unwrap(), duration_seconds: 3.0,
            source_start_seconds: 0.0, source_end_seconds: 3.0,
        }]).unwrap();
        mark(&writer).unwrap();
        let job = writer.job.clone();
        let directory = writer.directory.clone();
        let source_hash = storage::hash(&directory.join("e00000000.mp4"), &cancel).unwrap();
        dispatch(writer, None).unwrap();
        let started = std::time::Instant::now();
        let completed = loop {
            let state = service.inner.state.lock().unwrap();
            let current = state.jobs[&job.job_id].clone();
            if !current.active() && !state.cancel.contains_key(&job.job_id) { break current; }
            drop(state);
            assert!(started.elapsed() < Duration::from_secs(45));
            std::thread::sleep(Duration::from_millis(25));
        };
        assert_eq!(completed.state, "completed", "{:?}", completed.error);
        let item = completed.library.unwrap();
        assert_eq!(serde_json::to_value(item.capture_outcome.as_ref().unwrap()).unwrap()["state"], "interrupted");
        assert!((item.duration - 3.0).abs() < 0.2);
        assert_eq!(storage::hash(&directory.join("e00000000.mp4"), &cancel).unwrap(), source_hash);
        assert!(directory.join("encoded-stop.json").is_file());
        assert!(directory.join("chat-status.final.json").is_file());
        let chat_hash = storage::hash(&directory.join("chat-status.final.json"), &cancel).unwrap();
        assert!(fs::read_dir(&directory).unwrap().filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with("merge-")));
        drop(archive);
        drop(lease);
        service.shutdown();
        let reopened = PlatformMediaService::new(root.join("data"), tools, None).unwrap();
        let playback = reopened.open(&job.job_id).unwrap();
        assert_eq!(playback.item.sha256, item.sha256);
        assert_eq!(playback.item.capture_outcome, item.capture_outcome);
        assert_eq!(storage::hash(&directory.join("chat-status.final.json"), &cancel).unwrap(), chat_hash);
        reopened.close(&playback.token);
        reopened.shutdown();
    }
}
