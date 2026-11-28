//! SOOP-only bounded salvage/retry. A stop marker is reporting evidence, not
//! permission to skip media verification or delete original data.
use super::*;
#[path = "soop_tail.rs"]
mod tail;
#[path = "soop_concat.rs"]
pub(in crate::platform_media) mod concat;

const STOP: &str = "encoded-stop.json";
const DIAGNOSTIC: &str = "encoded-stop-diagnostic.json";
const MAX_DIAGNOSTIC_BYTES: usize = 8192;
const MERGE_ERROR: &str = "라이브 병합에 실패했습니다. 원본 조각을 보존했습니다.";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StopProof {
    version: u32,
    job_id: String,
    owner_sha256: String,
    manifest_sha256: String,
}

// This separate, write-once record is never read by outcome/preflight. It
// explains a finish attempt but cannot authorize salvage or change its result.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StopReason {
    CallerInterrupted,
    PartialAppend,
    IncompleteFragment,
    IncompleteAudioTail,
    OtherMuxerError,
    CommitFailure,
    StopProofFailure,
    OwnershipChanged,
}

fn track_stats(stats: &fmp4::PendingTrackStats) -> serde_json::Value {
    serde_json::json!({
        "timescale":stats.timescale, "samples":stats.samples, "bytes":stats.bytes,
        "firstDts":stats.first_dts, "lastEnd":stats.last_end,
        "lastDuration":stats.last_duration,
    })
}

fn pending_stats(stats: &fmp4::PendingStats) -> serde_json::Value {
    serde_json::json!({
        "video":track_stats(&stats.video), "audio":track_stats(&stats.audio),
        "nextVideoCutDts":stats.next_video_cut_dts,
        "inputPendingBytes":stats.input_pending_bytes, "inputMoofBytes":stats.input_moof_bytes,
        "inputMoofCount":stats.input_moof_count, "started":stats.started, "failed":stats.failed,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StopDiagnostic {
    version: u32,
    binding: StopProof,
    reasons: Vec<StopReason>,
    finish_succeeded: bool,
    committed_segments_before: u64,
    committed_segments_after: u64,
    partial_append_bytes: u64,
    before: serde_json::Value,
    after: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tail_retention: Option<tail::Report>,
}

fn write_diagnostic(path: &Path, diagnostic: &StopDiagnostic) -> Result<(), String> {
    let bytes = serde_json::to_vec(diagnostic).map_err(|_| ERROR)?;
    if bytes.len() > MAX_DIAGNOSTIC_BYTES { return Err(ERROR.into()); }
    storage::safe_path(path)?;
    let mut file = fs::OpenOptions::new().create_new(true).write(true).open(path).map_err(|_| ERROR)?;
    std::io::Write::write_all(&mut file, &bytes).map_err(|_| ERROR)?;
    file.sync_all().map_err(|_| ERROR.into())
}

fn diagnostic(
    writer: &Writer, before: &fmp4::PendingStats, reasons: Vec<StopReason>,
    committed_before: u64, partial_append_bytes: u64, finish_succeeded: bool,
    tail_retention: Option<tail::Report>,
) -> Result<(), String> {
    owner(&writer.lease.service, &writer.job, &writer.cancel)?;
    let proof = binding(&writer.job, &writer.directory, &writer.cancel)?;
    write_diagnostic(&writer.directory.join(DIAGNOSTIC), &StopDiagnostic {
        version: 1, binding: proof, reasons, finish_succeeded,
        committed_segments_before: committed_before,
        committed_segments_after: writer.segments.len() as u64,
        partial_append_bytes, before: pending_stats(before), after: pending_stats(&writer.muxer.pending_stats()),
        tail_retention,
    })
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
    let retained = tail::validate_overlap(job, directory, cancel)?;
    let path = directory.join(STOP);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Interrupted admission may have stopped after the segment append
            // or proof reservation. A restart must never call it complete.
            return if retained.is_some() { Err(ERROR.into()) } else { Ok(None) };
        }
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
    if let Some((units, bytes)) = retained.filter(|(units, _)| *units != [0;2]) {
        return serde_json::from_value(serde_json::json!({"state":"interrupted",
            "reasons":["source_interrupted","audio_coverage_tail"],"excludedUnits":units,"excludedBytes":bytes}))
            .map(Some).map_err(|_| ERROR.into());
    }
    Ok(Some(super::super::capture_outcome::CaptureOutcome::soop_interrupted()))
}

fn mark(writer: &Writer) -> Result<(), String> {
    owner(&writer.lease.service, &writer.job, &writer.cancel)?;
    if writer.segments.is_empty()
        || !same_segments(
            &recovery_segments_cancellable(&writer.job, &writer.directory, &writer.cancel)?,
            &writer.segments,
        )
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
    let before = writer.muxer.pending_stats();
    // A later flush error can follow an already consumed earlier cut. Preserve
    // the complete pre-finish samples; never reset the muxer's failed flag.
    let pending = writer.muxer.pending_snapshot();
    let mut tail_retention = None;
    let committed_before = writer.segments.len() as u64;
    let partial_append_bytes = writer.partial.as_ref().map_or(0, |partial| partial.bytes.len() as u64);
    let mut reasons = Vec::new();
    if interrupted { reasons.push(StopReason::CallerInterrupted); }
    if writer.partial.is_some() { reasons.push(StopReason::PartialAppend); }
    let result = (|| {
        let mut interrupted = interrupted || writer.partial.is_some();
        let mut tail = if interrupted { writer.muxer.finish_discontinuity() } else { writer.muxer.finish() };
        if !interrupted && tail.as_ref().is_err_and(|error| incomplete_box(&error.code, &error.message)) {
            reasons.push(StopReason::IncompleteFragment);
            interrupted = true;
            tail = writer.muxer.finish_discontinuity();
        }
        match tail {
            Ok(parts) => {
                if let Err(error) = writer.commit(parts) {
                    reasons.push(StopReason::CommitFailure);
                    return Err(error); // Never salvage a write/hash/commit failure.
                }
            }
            Err(error) if incomplete_audio(&error.code, &error.message) => {
                reasons.push(StopReason::IncompleteAudioTail);
                if let Ok(snapshot) = pending.as_ref() {
                    match tail::preserve(writer, snapshot) {
                        Ok(report) => {
                            tail_retention = Some(report);
                            // The original lossless bundle is durable FIRST.
                            // Any later admission error stays an error, including
                            // after a partial append or an empty reserved proof.
                            tail::commit_overlap(writer, snapshot)?;
                        }
                        Err(_) => tail_retention = Some(tail::Report::unavailable()),
                    }
                } else { tail_retention = Some(tail::Report::unavailable()); }
                if writer.segments.is_empty() { return Err(error.message); }
                // The common prefix is playable, but excluded samples remain
                // retained and the capture is still explicitly interrupted.
                interrupted = true;
            }
            Err(error) => {
                reasons.push(StopReason::OtherMuxerError);
                return Err(error.message);
            }
        }
        if interrupted {
            if let Err(error) = mark(writer) {
                reasons.push(StopReason::StopProofFailure);
                return Err(error);
            }
        }
        owner(&writer.lease.service, &writer.job, &writer.cancel).map_err(|error| {
            reasons.push(StopReason::OwnershipChanged);
            error
        })
    })();
    if diagnostic(writer, &before, reasons, committed_before, partial_append_bytes, result.is_ok(), tail_retention).is_err() {
        // Diagnostics cannot turn a valid finish into a failure or hide a core
        // failure. Do not log caller errors, source identifiers, or media bytes.
        eprintln!("SOOP_STOP_DIAGNOSTIC_UNAVAILABLE");
    }
    result
}

fn same_segments(saved: &[(String, f64)], current: &[(String, f64)]) -> bool {
    saved.len() == current.len() && saved.iter().zip(current).all(|((name, duration), (other, expected))| {
        // serde_json's default parser can move a serialized f64 by one ULP.
        // Keep exact segment identity and reject any larger duration change;
        // file hashes, owner proofs and decoded-media checks remain separate.
        name == other
            && duration.is_finite() && *duration > 0.0 && *duration <= 120.0
            && expected.is_finite() && *expected > 0.0 && *expected <= 120.0
            && duration.to_bits().abs_diff(expected.to_bits()) <= 1
    })
}

pub(in crate::platform_media) fn preflight(
    service: &PlatformMediaService, job: &MediaJob, directory: &Path,
    cancel: &Arc<AtomicBool>, segments: &[(String, f64)], reserve: u64,
) -> Result<(), String> {
    owner(service, job, cancel)?;
    if directory != storage::job_dir(&job.download_root, "soop", &job.job_id)?
        || !same_segments(&recovery_segments_cancellable(job, directory, cancel)?, segments)
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

    fn with_diagnostic_writer(test: impl FnOnce(&mut Writer)) {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("no-network-or-media-tool.exe");
        let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
            yt_dlp: missing.clone(), deno: missing.clone(), ffmpeg: missing.clone(), ffprobe: missing,
        }, None).unwrap();
        service.inner.synthetic_live_receiver.store(true, Ordering::Release);
        let lease = service.reserve_official_receiver(
            MediaRef { provider: "soop".into(), kind: "live".into(), external_id: "fixture".into() },
            "123".into(), uuid::Uuid::new_v4().to_string(), root.path().join("media"),
        ).unwrap();
        let mut archive = lease.begin_encoded(
            uuid::Uuid::new_v4().to_string(), 1, uuid::Uuid::new_v4().to_string(),
            fmp4::fixtures::inputs(), "fixture".into(), "fixture".into(), live::RecordingOwner::Manual,
        ).unwrap();
        let job_id = archive.writer.as_ref().unwrap().job.job_id.clone();
        test(archive.writer.as_mut().unwrap());
        // Normal archive destruction drains the synthetic owner; no real media
        // tools, provider session, user's profile or recording is involved.
        drop(archive);
        let deadline = std::time::Instant::now();
        loop {
            let state = service.inner.state.lock().unwrap();
            if !state.cancel.contains_key(&job_id) { break; }
            drop(state);
            assert!(deadline.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(lease);
        service.shutdown();
    }

    fn feed(writer: &mut Writer, track: u32, start: u64, count: u32) {
        let parts = writer.muxer.push(track, &fmp4::fixtures::fragment(track, start, count, true)).unwrap();
        writer.commit(parts).unwrap();
    }

    fn read_diagnostic(writer: &Writer) -> serde_json::Value {
        let bytes = fs::read(writer.directory.join(DIAGNOSTIC)).unwrap();
        assert!(bytes.len() <= MAX_DIAGNOSTIC_BYTES);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let expected = serde_json::to_value(binding(&writer.job, &writer.directory, &writer.cancel).unwrap()).unwrap();
        assert_eq!(value["binding"], expected);
        assert_eq!(value["version"], 1);
        assert!(value.get("message").is_none());
        value
    }

    #[test]
    fn stop_diagnostic_records_actual_pending_audio_gap_without_changing_prefix() {
        with_diagnostic_writer(|writer| {
            feed(writer, 0, 0, 24);
            feed(writer, 1, 0, 24);
            feed(writer, 0, 12 * u64::from(fmp4::fixtures::VIDEO_SCALE), 4);
            let previous = writer.segments.clone();
            assert_eq!(previous.len(), 1);
            finish(writer, false).unwrap();
            assert_eq!(writer.segments, previous);
            let diagnostic = read_diagnostic(writer);
            assert_eq!(diagnostic["reasons"], serde_json::json!(["incomplete_audio_tail"]));
            assert_eq!(diagnostic["finishSucceeded"], true);
            assert_eq!(diagnostic["committedSegmentsBefore"], 1);
            assert_eq!(diagnostic["committedSegmentsAfter"], 1);
            assert_eq!(diagnostic["before"]["video"]["samples"], 4);
            assert_eq!(diagnostic["before"]["video"]["firstDts"], 1_080_000);
            assert_eq!(diagnostic["before"]["video"]["lastEnd"], 1_260_000);
            assert_eq!(diagnostic["before"]["audio"]["samples"], 0);
            assert_eq!(diagnostic["before"]["video"], diagnostic["after"]["video"]);
            assert_eq!(diagnostic["tailRetention"]["state"], "retained");
            assert_eq!(diagnostic["tailRetention"]["sampleCount"], 4);
            assert_eq!(diagnostic["tailRetention"]["partialMp4"], "unavailable");
            assert!(writer.directory.join(tail::BUNDLE).is_file());
            // The old proof remains the sole interrupted-outcome authority.
            assert!(outcome(&writer.job, &writer.directory, &writer.cancel).unwrap().is_some());
            let old = fs::read(writer.directory.join(STOP)).unwrap();
            assert_eq!(serde_json::from_slice::<StopProof>(&old).unwrap().version, 1);
            fs::write(writer.directory.join(DIAGNOSTIC), b"not a proof").unwrap();
            assert!(outcome(&writer.job, &writer.directory, &writer.cancel).unwrap().is_some());
        });
    }

    #[test]
    fn stop_diagnostic_distinguishes_normal_partial_fragment_and_partial_append() {
        for mode in 0..4 {
            with_diagnostic_writer(|writer| {
                feed(writer, 0, 0, 4);
                feed(writer, 1, 0, 4);
                if mode == 1 { writer.muxer.push(0, &[0, 0, 0]).unwrap(); }
                if mode == 2 {
                    writer.partial = Some(PartialAppend { track: 0, index: 1, next_chunk: 1, bytes: vec![0; 3] });
                }
                finish(writer, mode == 3).unwrap();
                let diagnostic = read_diagnostic(writer);
                let expected = match mode {
                    0 => serde_json::json!([]),
                    1 => serde_json::json!(["incomplete_fragment"]),
                    2 => serde_json::json!(["partial_append"]),
                    _ => serde_json::json!(["caller_interrupted"]),
                };
                assert_eq!(diagnostic["reasons"], expected);
                assert_eq!(diagnostic["finishSucceeded"], true);
                assert_eq!(diagnostic["committedSegmentsAfter"], 1);
                assert_eq!(diagnostic["before"]["inputPendingBytes"], if mode == 1 { 3 } else { 0 });
                assert_eq!(diagnostic["after"]["inputPendingBytes"], 0);
                assert_eq!(diagnostic["partialAppendBytes"], if mode == 2 { 3 } else { 0 });
                assert!(diagnostic.get("tailRetention").is_none());
                assert!(!writer.directory.join(tail::BUNDLE).exists());
                assert_eq!(outcome(&writer.job, &writer.directory, &writer.cancel).unwrap().is_some(), mode != 0);
            });
        }
    }

    #[test]
    fn stop_diagnostic_failure_never_overwrites_or_changes_success_or_core_error() {
        for fail_core in [false, true] {
            with_diagnostic_writer(|writer| {
                feed(writer, 0, 0, 24);
                feed(writer, 1, 0, 24);
                feed(writer, 0, 12 * u64::from(fmp4::fixtures::VIDEO_SCALE), 4);
                let path = writer.directory.join(DIAGNOSTIC);
                fs::write(&path, b"existing diagnostic stays").unwrap();
                if fail_core { fs::write(writer.directory.join(STOP), b"existing proof stays").unwrap(); }
                let result = finish(writer, false);
                assert_eq!(result.is_ok(), !fail_core);
                assert_eq!(fs::read(&path).unwrap(), b"existing diagnostic stays");
                assert_eq!(writer.segments.len(), 1);
                if fail_core { assert_eq!(fs::read(writer.directory.join(STOP)).unwrap(), b"existing proof stays"); }
                else { assert!(outcome(&writer.job, &writer.directory, &writer.cancel).unwrap().is_some()); }
            });
        }
    }

    #[test]
    fn audio_tail_retention_commits_only_complete_overlap_and_keeps_historical_binding() {
        with_diagnostic_writer(|writer| {
            feed(writer,0,0,24);feed(writer,1,0,24);
            feed(writer,0,12*u64::from(fmp4::fixtures::VIDEO_SCALE),4);
            feed(writer,1,12*u64::from(fmp4::fixtures::AUDIO_SCALE),2);
            let previous=writer.segments.clone();let manifest=fs::read(writer.directory.join(MANIFEST)).unwrap();
            finish(writer,false).unwrap();
            assert_eq!(&writer.segments[..previous.len()],&previous);
            assert_eq!(writer.segments.len(),previous.len()+1);
            assert!(fs::read(writer.directory.join(MANIFEST)).unwrap().starts_with(&manifest));
            let retained:serde_json::Value=serde_json::from_slice(&fs::read(writer.directory.join(tail::MANIFEST)).unwrap()).unwrap();
            assert_eq!(retained["state"],"retained");assert_eq!(retained["video"]["samples"],4);assert_eq!(retained["audio"]["samples"],2);
            assert_eq!(retained["partialMp4"]["state"],"unverified");assert_eq!(retained["publicationAuthorized"],false);
            assert_eq!(retained["bundle"]["sha256"],storage::hash(&writer.directory.join(tail::BUNDLE),&writer.cancel).unwrap());
            assert_eq!(retained["binding"]["manifest_sha256"],format!("{:x}",Sha256::digest(&manifest)));
            let result=serde_json::to_value(outcome(&writer.job,&writer.directory,&writer.cancel).unwrap().unwrap()).unwrap();
            assert_eq!(result["state"],"interrupted");assert_eq!(result["excludedUnits"],serde_json::json!([2,0]));
            let proof:serde_json::Value=serde_json::from_slice(&fs::read(writer.directory.join("encoded-tail-overlap.json")).unwrap()).unwrap();
            assert_eq!(proof["publicationAuthorized"],false);
            assert_eq!(proof["video"]["samples"],2);assert_eq!(proof["audio"]["samples"],2);
            assert_eq!(proof["video"]["excludedSamples"],2);
        });
    }
    #[test]
    fn first_rejected_tail_is_retained_without_inventing_prefix_or_stop_proof() {
        with_diagnostic_writer(|writer| {
            feed(writer,0,0,4);feed(writer,1,0,2);
            // Unsupported presentation reordering is retained, never guessed.
            let mut snapshot=writer.muxer.pending_snapshot().unwrap();snapshot.video.samples[0].cts=1;
            tail::preserve(writer,&snapshot).unwrap();
            assert!(!tail::commit_overlap(writer,&snapshot).unwrap());
            assert!(writer.segments.is_empty());assert!(!writer.directory.join(STOP).exists());
            assert!(!writer.directory.join(MANIFEST).exists());
            assert!(outcome(&writer.job,&writer.directory,&writer.cancel).is_err());
            let retained:serde_json::Value=serde_json::from_slice(&fs::read(writer.directory.join(tail::MANIFEST)).unwrap()).unwrap();
            assert_eq!(retained["state"],"retained");assert!(retained["binding"]["manifest_sha256"].is_null());
        });
    }
    #[test]
    fn retention_collision_does_not_overwrite_or_change_prefix_result() {
        with_diagnostic_writer(|writer| {
            feed(writer,0,0,24);feed(writer,1,0,24);
            feed(writer,0,12*u64::from(fmp4::fixtures::VIDEO_SCALE),4);
            fs::write(writer.directory.join(tail::BUNDLE),b"existing").unwrap();
            finish(writer,false).unwrap();
            assert_eq!(fs::read(writer.directory.join(tail::BUNDLE)).unwrap(),b"existing");
            assert_eq!(read_diagnostic(writer)["tailRetention"]["state"],"unavailable");
            // An orphan retained artifact cannot be treated as a complete
            // capture on restart even when the old prefix is valid.
            assert!(outcome(&writer.job,&writer.directory,&writer.cancel).is_err());
        });
    }
    #[test]
    fn first_complete_overlap_uses_normal_anchor_and_remains_interrupted() {
        with_diagnostic_writer(|writer| {
            feed(writer,0,0,4);feed(writer,1,0,2);
            finish(writer,false).unwrap();assert_eq!(writer.segments.len(),1);
            assert!(writer.directory.join("chat-timeline.json").is_file());
            let retained:serde_json::Value=serde_json::from_slice(&fs::read(writer.directory.join(tail::MANIFEST)).unwrap()).unwrap();
            assert!(retained["binding"]["manifest_sha256"].is_null());
            let result=serde_json::to_value(outcome(&writer.job,&writer.directory,&writer.cancel).unwrap()).unwrap();
            assert_eq!(result["state"],"interrupted");assert_eq!(result["excludedUnits"],serde_json::json!([2,0]));
            preflight(&writer.lease.service,&writer.job,&writer.directory,&writer.cancel,&writer.segments,0).unwrap();
        });
    }
    #[test]
    fn overlap_crash_or_missing_proof_never_becomes_complete_on_restart() {
        for mode in 0..7 {
            with_diagnostic_writer(|writer| {
                feed(writer,0,0,24);feed(writer,1,0,24);
                feed(writer,0,12*u64::from(fmp4::fixtures::VIDEO_SCALE),4);
                feed(writer,1,12*u64::from(fmp4::fixtures::AUDIO_SCALE),2);
                finish(writer,false).unwrap();
                let path=writer.directory.join("encoded-tail-overlap.json");
                let mut proof:serde_json::Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                match mode {
                    0=>fs::write(&path,b"").unwrap(),
                    1=>fs::write(&path,b"{\"version\":1").unwrap(),
                    2=>fs::remove_file(&path).unwrap(),
                    3=>fs::remove_file(writer.directory.join(STOP)).unwrap(),
                    4=>{proof["parentSha256"]=serde_json::json!("0".repeat(64));fs::write(&path,serde_json::to_vec(&proof).unwrap()).unwrap();},
                    5=>{proof["resultSha256"]=serde_json::json!("0".repeat(64));fs::write(&path,serde_json::to_vec(&proof).unwrap()).unwrap();},
                    _=>{proof["publicationAuthorized"]=serde_json::json!(true);fs::write(&path,serde_json::to_vec(&proof).unwrap()).unwrap();},
                }
                assert!(outcome(&writer.job,&writer.directory,&writer.cancel).is_err());
                assert!(preflight(&writer.lease.service,&writer.job,&writer.directory,&writer.cancel,&writer.segments,0).is_err());
                assert_eq!(writer.segments.len(),2);assert!(writer.directory.join(tail::BUNDLE).is_file());
            });
        }
    }
    #[test]
    fn overlap_reservation_commit_failure_and_cancellation_preserve_originals() {
        for mode in 0..3 {
            with_diagnostic_writer(|writer| {
                feed(writer,0,0,24);feed(writer,1,0,24);
                feed(writer,0,12*u64::from(fmp4::fixtures::VIDEO_SCALE),4);
                feed(writer,1,12*u64::from(fmp4::fixtures::AUDIO_SCALE),2);
                let manifest=fs::read(writer.directory.join(MANIFEST)).unwrap();
                let snapshot=writer.muxer.pending_snapshot().unwrap();tail::preserve(writer,&snapshot).unwrap();
                let bundle=storage::hash(&writer.directory.join(tail::BUNDLE),&writer.cancel).unwrap();
                if mode==0 {fs::write(writer.directory.join("encoded-tail-overlap.json"),b"existing").unwrap();}
                if mode==1 {fs::write(writer.directory.join("e00000001.mp4"),b"existing").unwrap();}
                if mode==2 {writer.cancel.store(true,Ordering::Release);}
                assert!(tail::commit_overlap(writer,&snapshot).is_err());
                writer.cancel.store(false,Ordering::Release);
                assert_eq!(writer.segments.len(),1);assert_eq!(fs::read(writer.directory.join(MANIFEST)).unwrap(),manifest);
                assert_eq!(storage::hash(&writer.directory.join(tail::BUNDLE),&writer.cancel).unwrap(),bundle);
                assert!(outcome(&writer.job,&writer.directory,&writer.cancel).is_err());
                if mode==0 {assert_eq!(fs::read(writer.directory.join("encoded-tail-overlap.json")).unwrap(),b"existing");}
                if mode==1 {assert_eq!(fs::read(writer.directory.join("e00000001.mp4")).unwrap(),b"existing");}
            });
        }
    }
    #[test]
    fn orphan_retention_artifacts_without_manifest_cannot_bypass_restart_guard() {
        for file in [tail::BUNDLE,"encoded-pending-tail.mp4"] { for stop in [false,true] {
            with_diagnostic_writer(|writer| {
                feed(writer,0,0,24);feed(writer,1,0,24);
                // Exactly12s has no following IDR to trigger push-time commit.
                let prefix=writer.muxer.finish().unwrap();writer.commit(prefix).unwrap();
                assert_eq!(writer.segments.len(),1);
                if stop {mark(writer).unwrap();}
                fs::write(writer.directory.join(file),b"orphan").unwrap();
                assert!(outcome(&writer.job,&writer.directory,&writer.cancel).is_err());
                assert!(preflight(&writer.lease.service,&writer.job,&writer.directory,&writer.cancel,&writer.segments,0).is_err());
            });
        }}
    }
    #[test]
    fn actual_recording_durations_survive_json_roundtrip_identity() {
        let durations: [f64; 10] = [
            12.018333333333333, 12.023000000000003, 12.006333333333338,
            12.011000000000003, 12.015666666666668, 12.020333333333326,
            13.731666666666655, 12.727333333333334, 12.004666666666665, 10.0,
        ];
        let current: Vec<_> = durations.into_iter().enumerate()
            .map(|(index, duration)| (format!("e{index:08}.mp4"), duration)).collect();
        let saved: Vec<(String, f64)> = serde_json::from_str(
            &serde_json::to_string(&current).unwrap(),
        ).unwrap();
        assert!(same_segments(&saved, &current));
        assert!(same_segments(&current, &saved));
        // Lock the observed boundary independently of future parser upgrades.
        let original = 12.011000000000003_f64;
        let one_ulp = f64::from_bits(original.to_bits() + 1);
        assert!(same_segments(&[("e00000000.mp4".into(), original)],
            &[("e00000000.mp4".into(), one_ulp)]));
    }

    #[test]
    fn duration_identity_rejects_nonroundtrip_or_invalid_changes() {
        let duration = 12.011000000000003_f64;
        let original = vec![("e00000000.mp4".to_owned(), duration)];
        for invalid in [
            f64::from_bits(duration.to_bits() + 2), f64::from_bits(duration.to_bits() - 2),
            duration + 0.001, f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
            0.0, -0.0, -duration, f64::from_bits(120_f64.to_bits() + 1),
        ] {
            let changed = vec![("e00000000.mp4".to_owned(), invalid)];
            assert!(!same_segments(&original, &changed), "{invalid:?}");
            assert!(!same_segments(&changed, &original), "{invalid:?}");
            if !invalid.is_finite() || invalid <= 0.0 || invalid > 120.0 {
                assert!(!same_segments(&changed, &changed), "{invalid:?}");
            }
        }
        assert!(!same_segments(&original, &[]));
        assert!(!same_segments(&original, &[("e00000001.mp4".into(), duration)]));
        let mut two = original.clone();
        two.push(("e00000001.mp4".into(), duration));
        let mut reordered = two.clone();
        reordered.reverse();
        assert!(!same_segments(&two, &reordered));
    }

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
