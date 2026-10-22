//! Stop/retry orchestration for the YouTube-owned raw archive only.
use super::{lifecycle, storage, youtube_spool, *};
use std::time::Instant;

const ERROR: &str = "YouTube 녹화 상태가 변경됐습니다. 받은 파일은 보존했습니다.";

fn same_job(left: &MediaJob, right: &MediaJob) -> bool {
    left.job_id == right.job_id
        && left.request_id == right.request_id
        && left.reference == right.reference
        && left.broadcast_id == right.broadcast_id
        && left.download_root == right.download_root
        && left.created_at == right.created_at
}
fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    if Instant::now() >= deadline {
        return Err("녹화 처리 시간이 초과됐습니다. 받은 파일은 보존했습니다.".into());
    }
    Ok(())
}
fn finalizing(
    service: &PlatformMediaService,
    job: &MediaJob,
    cancel: &Arc<AtomicBool>,
    identity: &youtube_spool::SpoolIdentity,
    deadline: Instant,
) -> Result<MediaJob, String> {
    checkpoint(cancel, deadline)?;
    if job.reference.provider != "youtube"
        || job.reference.kind != "live"
        || job.job_id != identity.job_id
        || job.request_id != identity.request_id
        || job.reference.external_id != identity.video_id
        || job.broadcast_id.as_deref() != Some(identity.video_id.as_str())
    {
        return Err(ERROR.into());
    }
    let mut state = service.inner.state.lock().map_err(|_| ERROR)?;
    let current = state.jobs.get(&job.job_id).ok_or(ERROR)?;
    if state.shutting_down
        || !same_job(current, job)
        || current.revision != job.revision
        || current.library.is_some()
        || !matches!(
            current.state.as_str(),
            "queued" | "recording" | "finalizing"
        )
        || !state
            .cancel
            .get(&job.job_id)
            .is_some_and(|known| Arc::ptr_eq(known, cancel))
    {
        return Err(ERROR.into());
    }
    let mut next = current.clone();
    next.state = "finalizing".into();
    next.revision = next.revision.checked_add(1).ok_or(ERROR)?;
    next.error = None;
    checkpoint(cancel, deadline)?;
    if let Err(error) = storage::save_job(&service.inner.journal, &next) {
        lifecycle::remember_persistence_error(
            &mut state,
            &next.job_id,
            lifecycle::persistence_message(&error),
        );
        return Err(error);
    }
    state.persistence_errors.remove(&next.job_id);
    state.jobs.insert(next.job_id.clone(), next.clone());
    Ok(next)
}

impl PlatformMediaService {
    /// Caller keeps the receiver lease and JobOwnership until this returns.
    /// Chat is already detached from the exact raw archive before worker spawn.
    pub(super) fn finalize_raw_youtube(
        &self,
        job: MediaJob,
        cancel: Arc<AtomicBool>,
        identity: youtube_spool::SpoolIdentity,
        detached: Option<live_chat::RawChatDetachOutcome>,
    ) -> Result<(), String> {
        self.finalize_raw_youtube_inner(job, cancel, identity, detached, false)
    }

    /// Only live.rs's explicit raw retry may request existing-final adoption.
    pub(super) fn finalize_raw_youtube_recovery(
        &self,
        job: MediaJob,
        cancel: Arc<AtomicBool>,
        identity: youtube_spool::SpoolIdentity,
    ) -> Result<(), String> {
        self.finalize_raw_youtube_inner(job, cancel, identity, None, true)
    }

    fn finalize_raw_youtube_inner(
        &self,
        job: MediaJob,
        cancel: Arc<AtomicBool>,
        identity: youtube_spool::SpoolIdentity,
        mut detached: Option<live_chat::RawChatDetachOutcome>,
        recovery: bool,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(24 * 3600);
        let directory = storage::job_dir(&job.download_root, "youtube", &job.job_id)?;
        let result = (|| {
            let job = finalizing(self, &job, &cancel, &identity, deadline)?;
            // A current or retried writer uses the same no-network preparation. Do
            // not make a late worker detach a newly attached successor chat sink.
            let _merge = loop {
                checkpoint(&cancel, deadline)?;
                match self.inner.merge.try_lock() {
                    Ok(permit) => break permit,
                    Err(std::sync::TryLockError::Poisoned(_)) => return Err(ERROR.into()),
                    Err(std::sync::TryLockError::WouldBlock) => {}
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let prepared = youtube_spool::prepare_recording(
                &job.download_root,
                &identity,
                &self.inner.tools,
                &cancel,
                deadline,
            );
            let candidate = match prepared {
                Ok(candidate) => candidate,
                Err(error) => {
                    tracing::warn!(diag_stage = "youtube_raw_prepare", ?error, "YouTube raw recording verification did not complete");
                    return Err(match error {
                        youtube_spool::PublicationFault::Cancelled => "cancelled".into(),
                        _ => "녹화 영상 검증을 마치지 못했습니다. 받은 영상과 채팅은 보존했습니다."
                            .into(),
                    });
                }
            };
            let timeline = candidate.raw_timeline(&cancel);
            if let Some(chat) = detached.take() {
                chat.persist_cancellable(
                    &directory,
                    "youtube",
                    &job.job_id,
                    timeline.as_ref(),
                    &cancel,
                )?;
            } else {
                // Explicit retry must not upgrade an already partial/unknown chat
                // archive merely because its independently verified video is usable.
                super::chat_status::recovery_with_cancel(
                    &directory,
                    "youtube",
                    &job.job_id,
                    &cancel,
                )?;
            }
            checkpoint(&cancel, deadline)?;
            let handoff = candidate
                .into_publication(&job.download_root, &identity, &cancel, deadline)
                .map_err(|_| ERROR)?;
            if recovery {
                self.publish_recovered_youtube(&job, &cancel, &identity, handoff, deadline)
            } else {
                self.publish_verified_youtube(&job, &cancel, &identity, handoff, deadline)
            }
        })();
        // Even an early stale-attempt/shutdown/merge error must settle the exact
        // already-closed sink conservatively. Never detach the current successor
        // or clear the caller's cancellation flag to manufacture a success.
        if result.is_err() {
            if let Some(chat) = detached.take() {
                chat.persist_cancellable(&directory, "youtube", &job.job_id, None, &cancel)?;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        tempfile::TempDir,
        PlatformMediaService,
        MediaJob,
        youtube_spool::SpoolIdentity,
        Arc<AtomicBool>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let tools = PlatformMediaTools {
            yt_dlp: root.path().join("unused"),
            deno: root.path().join("unused"),
            ffmpeg: root.path().join("unused"),
            ffprobe: root.path().join("unused"),
        };
        let service = PlatformMediaService::new(root.path().join("data"), tools, None).unwrap();
        let identity = youtube_spool::SpoolIdentity {
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            receiver_id: uuid::Uuid::new_v4().to_string(),
            source_id: uuid::Uuid::new_v4().to_string(),
            document_id: uuid::Uuid::new_v4().to_string(),
            document_epoch: 1,
            source_generation: 1,
            video_id: "abcdefghijk".into(),
            channel_id: format!("UC{}", "a".repeat(22)),
            owner: youtube_spool::Owner::Manual,
        };
        let job = MediaJob {
            job_id: identity.job_id.clone(),
            request_id: identity.request_id.clone(),
            reference: MediaRef {
                provider: "youtube".into(),
                kind: "live".into(),
                external_id: identity.video_id.clone(),
            },
            broadcast_id: Some(identity.video_id.clone()),
            revision: 3,
            state: "recording".into(),
            title: "Fixture".into(),
            channel_name: "Fixture".into(),
            created_at: now(),
            received_bytes: 1,
            total_bytes: None,
            error: None,
            library: None,
            download_root: root.path().join("media"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = service.inner.state.lock().unwrap();
            storage::save_job(&service.inner.journal, &job).unwrap();
            state.jobs.insert(job.job_id.clone(), job.clone());
            state.cancel.insert(job.job_id.clone(), cancel.clone());
        }
        (root, service, job, identity, cancel)
    }
    #[test]
    fn exact_attempt_can_only_enter_finalizing_once() {
        let (_root, service, job, identity, cancel) = fixture();
        let deadline = Instant::now() + Duration::from_secs(1);
        let next = finalizing(&service, &job, &cancel, &identity, deadline).unwrap();
        assert_eq!(next.state, "finalizing");
        assert_eq!(next.revision, 4);
        assert!(finalizing(&service, &job, &cancel, &identity, deadline).is_err());
        assert!(next.library.is_none());
    }
    #[test]
    fn stale_token_revision_identity_or_shutdown_never_changes_job() {
        let (_root, service, job, mut identity, cancel) = fixture();
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(finalizing(
            &service,
            &job,
            &Arc::new(AtomicBool::new(false)),
            &identity,
            deadline
        )
        .is_err());
        let mut stale = job.clone();
        stale.revision -= 1;
        assert!(finalizing(&service, &stale, &cancel, &identity, deadline).is_err());
        identity.video_id = "ABCDEFGHIJK".into();
        assert!(finalizing(&service, &job, &cancel, &identity, deadline).is_err());
        identity.video_id = job.reference.external_id.clone();
        service.inner.state.lock().unwrap().shutting_down = true;
        assert!(finalizing(&service, &job, &cancel, &identity, deadline).is_err());
        assert_eq!(
            service.inner.state.lock().unwrap().jobs[&job.job_id].revision,
            job.revision
        );
    }
}
