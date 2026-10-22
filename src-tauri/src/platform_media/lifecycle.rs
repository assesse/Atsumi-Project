//! Actual job ownership is separate from persisted presentation state.
use super::*;
pub(super) fn persistence_message(error: &str) -> String {
    format!(
        "작업은 중단됐지만 상태를 저장하지 못했습니다. 원본과 마지막 작업 기록은 보존했습니다. {}",
        error
            .chars()
            .filter(|c| !c.is_control())
            .take(240)
            .collect::<String>()
    )
}
pub(super) fn remember_persistence_error(state: &mut State, id: &str, error: String) {
    // The latch remains nonempty even if more failures exceed its detail cap.
    if state.persistence_errors.len() < 256 || state.persistence_errors.contains_key(id) {
        state.persistence_errors.insert(id.into(), error);
    } else {
        state.persistence_overflow = true;
    }
}
pub(super) struct JobOwnership {
    service: PlatformMediaService,
    id: String,
    cancel: Arc<AtomicBool>,
}
impl JobOwnership {
    pub(super) fn new(service: PlatformMediaService, id: String, cancel: Arc<AtomicBool>) -> Self {
        Self {
            service,
            id,
            cancel,
        }
    }
}
impl Drop for JobOwnership {
    fn drop(&mut self) {
        let unfinished = {
            let state = self
                .service
                .inner
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state
                .cancel
                .get(&self.id)
                .is_some_and(|token| Arc::ptr_eq(token, &self.cancel))
                && state.jobs.get(&self.id).is_some_and(MediaJob::active)
        };
        if unfinished {
            self.service.terminal_error(
                &self.id,
                &self.cancel,
                "작업이 최종 확인 전에 중단됐습니다. 원본은 보존했습니다.",
            );
        }
        let mut state = self
            .service
            .inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state
            .cancel
            .get(&self.id)
            .is_some_and(|token| Arc::ptr_eq(token, &self.cancel))
        {
            state.cancel.remove(&self.id);
        }
        self.service.inner.ready.notify_all();
    }
}
impl PlatformMediaService {
    pub fn shutdown_error(&self) -> Option<String> {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.persistence_overflow || !state.persistence_errors.is_empty()).then(||"일부 영상 작업의 마지막 상태를 저장하지 못했습니다. 원본과 이전 기록은 보존했습니다.".into())
    }
    pub(super) fn terminal_error(&self, id: &str, cancel: &Arc<AtomicBool>, error: &str) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state
            .cancel
            .get(id)
            .is_some_and(|token| Arc::ptr_eq(token, cancel))
        {
            return;
        }
        let Some(mut job) = state.jobs.get(id).cloned() else {
            return;
        };
        if !job.active() {
            return;
        }
        job.state = if state.shutting_down {
            "interrupted"
        } else if cancel.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "failed"
        }
        .into();
        job.revision = job.revision.saturating_add(1);
        if !encoded::raw_youtube_integrity_failed(&job) {
            job.error = Some(if state.shutting_down {
                "앱 종료로 중단됐습니다. 원본은 보존했습니다.".into()
            } else if error == "cancelled" {
                "취소됐습니다. 원본은 보존했습니다.".into()
            } else {
                error
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(400)
                    .collect()
            });
        }
        match storage::save_job(&self.inner.journal, &job) {
            Ok(()) => {
                state.persistence_errors.remove(id);
            }
            Err(error) => {
                let error = persistence_message(&error);
                if !encoded::raw_youtube_integrity_failed(&job) {
                    job.error = Some(error.clone());
                }
                remember_persistence_error(&mut state, id, error);
            }
        }
        // Failed persistence must never claim completed or strand a dead worker.
        state.jobs.insert(id.into(), job);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &std::path::Path) -> (PlatformMediaService, MediaJob, Arc<AtomicBool>) {
        let tools = PlatformMediaTools {
            yt_dlp: root.join("unavailable-network.exe"),
            deno: root.join("unavailable-deno.exe"),
            ffmpeg: root.join("unused.exe"),
            ffprobe: root.join("unused-probe.exe"),
        };
        let service = PlatformMediaService::new(root.join("data"), tools, None).unwrap();
        let job = MediaJob {
            broadcast_id: None,
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            reference: MediaRef {
                provider: "youtube".into(),
                kind: "video".into(),
                external_id: "abcdefghijk".into(),
            },
            revision: 1,
            state: "downloading".into(),
            title: "synthetic".into(),
            channel_name: "fixture".into(),
            created_at: 0,
            received_bytes: 3,
            total_bytes: None,
            error: None,
            library: None,
            download_root: root.join("media"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        storage::save_job(&service.inner.journal, &job).unwrap();
        {
            let mut state = service.inner.state.lock().unwrap();
            state.jobs.insert(job.job_id.clone(), job.clone());
            state.cancel.insert(job.job_id.clone(), cancel.clone());
        }
        (service, job, cancel)
    }
    #[test]
    fn failed_terminal_journal_does_not_strand_dead_work_or_destroy_last_record() {
        let root = tempfile::tempdir().unwrap();
        let (service, job, cancel) = fixture(root.path());
        let last = fs::read(service.inner.journal.join(format!("{}.json", job.job_id))).unwrap();
        let backup = service.inner.journal.with_extension("preserved");
        fs::rename(&service.inner.journal, &backup).unwrap();
        fs::write(&service.inner.journal, b"synthetic blocker").unwrap();
        let owner = JobOwnership::new(service.clone(), job.job_id.clone(), cancel.clone());
        service.terminal_error(&job.job_id, &cancel, "synthetic failure");
        assert_eq!(
            service.active_ids().len(),
            1,
            "owner still running despite terminal DTO"
        );
        drop(owner);
        assert!(service.active_ids().is_empty());
        assert!(service.shutdown_error().is_some());
        assert_eq!(service.jobs(None)[0].state, "failed");
        assert_eq!(
            fs::read(backup.join(format!("{}.json", job.job_id))).unwrap(),
            last
        );
        service.shutdown();
    }
    #[test]
    fn actual_thread_panic_releases_its_owner_without_claiming_completion() {
        let root = tempfile::tempdir().unwrap();
        let (service, job, cancel) = fixture(root.path());
        let worker = service.clone();
        let id = job.job_id.clone();
        assert!(std::thread::spawn(move || {
            let _owner = JobOwnership::new(worker, id, cancel);
            panic!("synthetic owned worker panic");
        })
        .join()
        .is_err());
        assert!(service.active_ids().is_empty());
        assert_eq!(service.jobs(None)[0].state, "failed");
        assert!(service.shutdown_error().is_none());
        service.shutdown();
    }
    #[test]
    fn old_owner_cannot_remove_a_retry_and_shutdown_drains_only_queued_work() {
        let root = tempfile::tempdir().unwrap();
        let (service, job, cancel) = fixture(root.path());
        let owner = JobOwnership::new(service.clone(), job.job_id.clone(), cancel);
        let replacement = Arc::new(AtomicBool::new(false));
        service
            .inner
            .state
            .lock()
            .unwrap()
            .cancel
            .insert(job.job_id.clone(), replacement.clone());
        drop(owner);
        assert_eq!(service.active_ids().len(), 1);
        {
            let mut state = service.inner.state.lock().unwrap();
            // This fixture covers queued-only settlement. The old guard wakes
            // real idle workers; close their admission before staging the item.
            // A separately claimed worker is covered by the following test.
            state.shutting_down = true;
            state.jobs.get_mut(&job.job_id).unwrap().state = "queued".into();
            state.queue.push_back(job.job_id.clone());
        }
        service.shutdown();
        assert!(service.active_ids().is_empty());
        assert_eq!(service.jobs(None)[0].state, "interrupted");
    }
    #[test]
    fn cancellation_after_queue_claim_cannot_release_the_running_owner() {
        let root = tempfile::tempdir().unwrap();
        let (service, job, cancel) = fixture(root.path());
        service
            .inner
            .state
            .lock()
            .unwrap()
            .jobs
            .get_mut(&job.job_id)
            .unwrap()
            .state = "queued".into();
        // Worker has popped the queue but not yet published its first DTO update.
        let owner = JobOwnership::new(service.clone(), job.job_id.clone(), cancel.clone());
        service.cancel(&job.job_id, job.revision).unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert_eq!(service.active_ids().len(), 1);
        service.shutdown();
        assert_eq!(service.active_ids().len(), 1);
        drop(owner);
        assert!(service.active_ids().is_empty());
        assert_eq!(service.jobs(None)[0].state, "interrupted");
    }

    #[test]
    fn persistence_overflow_remains_latched_after_tracked_failures_are_resolved() {
        let root = tempfile::tempdir().unwrap();
        let (service, job, cancel) = fixture(root.path());
        let owner = JobOwnership::new(service.clone(), job.job_id, cancel);
        {
            let mut state = service.inner.state.lock().unwrap();
            for index in 0..257 {
                remember_persistence_error(
                    &mut state,
                    &format!("failed-{index}"),
                    "synthetic journal failure".into(),
                );
            }
            assert_eq!(state.persistence_errors.len(), 256);
            assert!(state.persistence_overflow);
            state.persistence_errors.clear();
        }
        assert!(service.shutdown_error().is_some());
        drop(owner);
        assert!(service.active_ids().is_empty());
        assert!(service.shutdown_error().is_some());
        service.shutdown();
    }
}
