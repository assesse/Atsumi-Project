//! Optional new-job portraits. No media worker waits for public metadata.
use super::Profile;
#[cfg(not(test))]
use super::current;
use super::super::{encoded::ProfileReceiver, storage, MediaJob, PlatformMediaService, State};
use std::{
    collections::{HashSet, VecDeque},
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    time::{Duration, Instant},
};

const CAPACITY: usize = 128;
const WORKERS: usize = 2;
const QUEUE_AGE: Duration = Duration::from_secs(15);
type Lookup = fn(&MediaJob, &AtomicBool) -> Option<String>;

#[derive(Default)]
pub(in crate::platform_media) struct Snapshots { state: Mutex<Queue> }
#[derive(Default)]
struct Queue { tasks: VecDeque<Task>, active: HashSet<String>, workers: usize }
struct Task {
    service: PlatformMediaService,
    job: MediaJob,
    cancel: Arc<AtomicBool>,
    receiver: Option<ProfileReceiver>,
    queued: Instant,
    lookup: Lookup,
}

fn same_job(current: &MediaJob, expected: &MediaJob) -> bool {
    current.active() && current.library.is_none()
        && current.job_id == expected.job_id && current.request_id == expected.request_id
        && current.reference == expected.reference && current.download_root == expected.download_root
        && current.created_at == expected.created_at && current.broadcast_id == expected.broadcast_id
}
fn authorized(state: &State, task: &Task) -> bool {
    !state.shutting_down && !task.cancel.load(Ordering::Acquire)
        && state.cancel.get(&task.job.job_id).is_some_and(|cancel| Arc::ptr_eq(cancel, &task.cancel))
        && state.jobs.get(&task.job.job_id).is_some_and(|job| same_job(job, &task.job))
}
impl Task {
    // Registry mutex is not held here. Capture order: commit -> recording -> state.
    fn guarded<R>(&self, operation: impl FnOnce() -> Option<R>) -> Option<R> {
        let check = || {
            let state = self.service.inner.state.lock().ok()?;
            if !authorized(&state, self) { return None; }
            operation()
        };
        match &self.receiver {
            Some(receiver) => receiver.with_current(&self.job, check),
            None => check(),
        }
    }
    fn run(&self) {
        if self.queued.elapsed() > QUEUE_AGE || self.guarded(|| Some(())).is_none() { return; }
        let Some(image) = (self.lookup)(&self.job, &self.cancel) else { return; };
        let Some(profile) = Profile::from_data_url(&image) else { return; };
        let _ = self.guarded(|| {
            let directory = storage::job_dir(&self.job.download_root, &self.job.reference.provider, &self.job.job_id).ok()?;
            // New download jobs have no media directory yet. Capture directories
            // already exist and are checked by the receiver proof before here.
            if self.receiver.is_none() { std::fs::create_dir_all(&directory).ok()?; }
            storage::safe_path(&directory).ok()?;
            super::save(&directory, &profile).ok()
        });
    }
}
impl Snapshots {
    pub(in crate::platform_media) fn active_ids(&self) -> Vec<String> {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).active.iter()
            .map(|id| format!("platform-media:{id}")).collect()
    }
    fn enqueue(&self, task: Task) {
        let service = task.service.clone();
        let mut queue = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if queue.active.len() >= CAPACITY || !queue.active.insert(task.job.job_id.clone()) { return; }
        queue.tasks.push_back(task);
        if queue.workers >= WORKERS { return; }
        queue.workers += 1;
        // Spawn does not run a lookup under this lock. The worker first releases
        // this queue before acquiring any receiver/service locks.
        if std::thread::Builder::new().name("platform-profile".into()).spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                let task = {
                    let mut queue = service.inner.profiles.state.lock().unwrap_or_else(|error| error.into_inner());
                    queue.tasks.pop_front()
                };
                let Some(task) = task else { break; };
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task.run()));
                service.inner.profiles.state.lock().unwrap_or_else(|error| error.into_inner()).active.remove(&task.job.job_id);
            }));
            let mut queue = service.inner.profiles.state.lock().unwrap_or_else(|error| error.into_inner());
            queue.workers -= 1;
            if queue.workers == 0 {
                // Optional work queued as the final worker retired (or after a
                // panic) must not retain service/task cycles indefinitely.
                queue.tasks.clear();
                queue.active.clear();
            }
        }).is_err() {
            queue.workers -= 1;
            if queue.workers == 0 { queue.tasks.clear(); queue.active.clear(); }
        }
    }
}
impl PlatformMediaService {
    pub(in crate::platform_media) fn schedule_profile(&self, job: &MediaJob, cancel: &Arc<AtomicBool>, receiver: Option<ProfileReceiver>) {
        #[cfg(not(test))]
        let lookup: Lookup = current::lookup;
        // Ordinary native tests must never create anonymous network traffic.
        #[cfg(test)]
        let lookup: Lookup = |_, _| None;
        self.inner.profiles.enqueue(Task { service: self.clone(), job: job.clone(), cancel: cancel.clone(), receiver, queued: Instant::now(), lookup });
    }
}

#[cfg(test)]
mod tests;
