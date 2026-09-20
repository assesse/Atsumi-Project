//! Bounded page work and independent, durable-checkpoint-backed finalization.
use super::*;
use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration,
};

const POLL: Duration = Duration::from_millis(25);

pub(super) struct WorkQueue<T> {
    state: Mutex<(VecDeque<T>, bool)>,
    changed: Condvar,
    capacity: usize,
}

impl<T> WorkQueue<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new((VecDeque::new(), false)),
            changed: Condvar::new(),
            capacity,
        }
    }

    pub(super) fn push(&self, value: T, cancellation: &CancellationToken) -> Result<(), RunError> {
        let mut state = unpoison(self.state.lock());
        loop {
            check_cancelled(cancellation)?;
            if state.1 {
                return Err(DownloadPipelineError::cancelled().into());
            }
            if state.0.len() < self.capacity {
                state.0.push_back(value);
                self.changed.notify_all();
                return Ok(());
            }
            state = self
                .changed
                .wait_timeout(state, POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }

    pub(super) fn pop(&self) -> Option<T> {
        let mut state = unpoison(self.state.lock());
        loop {
            if let Some(value) = state.0.pop_front() {
                self.changed.notify_all();
                return Some(value);
            }
            if state.1 {
                return None;
            }
            state = unpoison(self.changed.wait(state));
        }
    }

    pub(super) fn close(&self) {
        unpoison(self.state.lock()).1 = true;
        self.changed.notify_all();
    }
}

pub(super) struct PageTask {
    descriptor: DownloadJobDescriptor,
    layout: ArtifactLayout,
    page: super::super::DownloadSourcePage,
    checkpoint: Option<super::super::DownloadCheckpoint>,
    cancellation: CancellationToken,
    result: mpsc::Sender<Result<Option<(StoredPage, usize)>, RunError>>,
}

pub(super) struct FinalizationTask {
    pub descriptor: DownloadJobDescriptor,
    pub layout: ArtifactLayout,
    pub cancellation: CancellationToken,
    pub enqueued_at: std::time::Instant,
}

pub(super) fn claim_job(
    inner: &SupervisorInner,
    descriptor: &DownloadJobDescriptor,
    cancellation: &CancellationToken,
) -> bool {
    loop {
        if inner.shutting_down.load(Ordering::Acquire) {
            return false;
        }
        let mut active = unpoison(inner.cancellations.lock());
        // A cancelled attempt may still be draining a write or finalization.
        // Do not let a rapid cancel/retry open the same artifact concurrently.
        if !active
            .values()
            .any(|owner| owner.entry_id == descriptor.entry_id)
        {
            active.insert(
                descriptor_key(descriptor),
                ActiveCancellation {
                    entry_id: descriptor.entry_id.clone(),
                    token: cancellation.clone(),
                },
            );
            return true;
        }
        drop(active);
        thread::sleep(POLL);
    }
}

pub(super) fn page_worker(inner: Arc<SupervisorInner>) {
    while let Some(task) = inner.pages.pop() {
        // Always answer accepted work, including an unexpected decoder panic:
        // otherwise the coordinator could wait forever for a dead worker.
        let result = guarded_work(|| {
            download_one_page(
                &inner,
                &task.descriptor,
                &task.layout,
                &task.page,
                task.checkpoint.as_ref(),
                &task.cancellation,
            )
        });
        // Errors cancel siblings, not the parent: the original cause must still
        // be persisted as a failure instead of being mistaken for user cancel.
        let _ = task.result.send(result);
    }
}

pub(super) fn download_pages(
    inner: &SupervisorInner,
    descriptor: &DownloadJobDescriptor,
    layout: &ArtifactLayout,
    pages: &[super::super::DownloadSourcePage],
    checkpoints: &BTreeMap<crate::domain::SourcePageNumber, super::super::DownloadCheckpoint>,
    cancellation: &CancellationToken,
) -> Result<(), RunError> {
    let child = cancellation.child();
    let (send, receive) = mpsc::channel();
    let mut next = pages.iter();
    let mut in_flight = 0;
    let mut failure = None;
    loop {
        if cancellation.is_cancelled() && failure.is_none() {
            failure = Some(DownloadPipelineError::cancelled().into());
            child.cancel();
        }
        while failure.is_none() && in_flight < inner.page_workers {
            let Some(page) = next.next() else {
                break;
            };
            let task = PageTask {
                descriptor: descriptor.clone(),
                layout: layout.clone(),
                page: page.clone(),
                checkpoint: checkpoints.get(&page.source_page_number).cloned(),
                cancellation: child.clone(),
                result: send.clone(),
            };
            match inner.pages.push(task, &child) {
                Ok(()) => in_flight += 1,
                Err(error) => {
                    failure = Some(error);
                    child.cancel();
                }
            }
        }
        if in_flight == 0 {
            return failure.map_or(Ok(()), Err);
        }
        match receive.recv_timeout(POLL) {
            Ok(result) => {
                in_flight -= 1;
                // Only the album coordinator commits/emits progress. Page
                // completion may be out of order, but revisions never are.
                let result = result.and_then(|page| {
                    if let Some((stored, source_bytes)) = page {
                        let projection = inner
                            .repository
                            .pipeline_page_verified(descriptor, &stored)?;
                        if source_bytes > 0 {
                            super::super::image_work_budget::record_stored_page(source_bytes);
                        }
                        emit(inner, projection);
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    if failure.is_none() {
                        failure = Some(error);
                        child.cancel();
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::WorkerUnavailable,
                    "The page worker stopped",
                    true,
                )
                .into())
            }
        }
        // Drain every in-flight page before failing/retrying this job. No old
        // page may checkpoint against a replacement worker attempt.
    }
}

pub(super) fn finalization_worker(inner: Arc<SupervisorInner>) {
    while let Some(task) = inner.finalizing.pop() {
        let started = std::time::Instant::now();
        let result = guarded_work(|| {
            finalize_download(&inner, &task.descriptor, &task.layout, &task.cancellation)
        });
        tracing::info!(
            gallery_id = task.descriptor.gallery_id.get(),
            worker_attempt = task.descriptor.worker_attempt,
            queue_wait_ms = started.duration_since(task.enqueued_at).as_millis() as u64,
            elapsed_ms = started.elapsed().as_millis() as u64,
            succeeded = result.is_ok(),
            cancelled = task.cancellation.is_cancelled(),
            "download final verification ended"
        );
        if let Err(error) = result {
            handle_download_error(&inner, &task.descriptor, &task.cancellation, error);
        }
        finish_job(&inner, &task.descriptor);
    }
}

fn guarded_work<T>(work: impl FnOnce() -> Result<T, RunError>) -> Result<T, RunError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::WorkerUnavailable,
            "A download processing worker could not finish safely",
            true,
        )
        .into())
    })
}

pub(super) fn finish_job(inner: &SupervisorInner, descriptor: &DownloadJobDescriptor) {
    let key = descriptor_key(descriptor);
    unpoison(inner.cancellations.lock()).remove(&key);
    unpoison(inner.queue.lock()).known.remove(&key);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panicking_work_returns_a_stable_failure_instead_of_losing_a_worker() {
        let result = guarded_work::<()>(|| panic!("synthetic worker failure"));
        assert!(matches!(result, Err(RunError::Pipeline(error))
            if error.code == DownloadPipelineErrorCode::WorkerUnavailable));
        assert!(guarded_work(|| Ok(())).is_ok());
    }

    #[test]
    fn child_cancellation_never_cancels_parent_but_observes_shutdown() {
        let parent = CancellationToken::new();
        let child = parent.child();
        child.cancel();
        assert!(!parent.is_cancelled());
        let other = parent.child();
        parent.cancel();
        assert!(other.is_cancelled());
    }

    #[test]
    fn full_queue_applies_backpressure_until_a_consumer_releases_space() {
        let queue = Arc::new(WorkQueue::new(1));
        assert!(queue.push(1, &CancellationToken::new()).is_ok());
        let waiting = queue.clone();
        let (done, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = done.send(waiting.push(2, &CancellationToken::new()).is_ok());
        });
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(30)),
            Err(RecvTimeoutError::Timeout)
        ));
        assert_eq!(queue.pop(), Some(1));
        let accepted = receiver.recv_timeout(Duration::from_secs(2));
        queue.close();
        worker.join().unwrap();
        assert!(accepted.unwrap());
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn bounded_queue_close_drains_accepted_work_and_rejects_new_work() {
        let queue = Arc::new(WorkQueue::new(1));
        let cancellation = CancellationToken::new();
        assert!(queue.push(1, &cancellation).is_ok());
        let waiting = queue.clone();
        let worker = thread::spawn(move || waiting.push(2, &CancellationToken::new()).is_err());
        queue.close();
        assert!(worker.join().unwrap());
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), None);
    }
}
