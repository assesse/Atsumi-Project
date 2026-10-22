use super::*;
use super::super::super::{MediaRef, PlatformMediaTools};
use std::sync::atomic::AtomicUsize;

struct Fixture { root: tempfile::TempDir, service: PlatformMediaService }
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("no-tools.exe");
        let service = PlatformMediaService::new(root.path().join("data"), PlatformMediaTools {
            yt_dlp: absent.clone(), deno: absent.clone(), ffmpeg: absent.clone(), ffprobe: absent,
        }, None).unwrap();
        Self { root, service }
    }
    fn task(&self, lookup: Lookup) -> Task {
        let job = MediaJob {
            broadcast_id: None, job_id: uuid::Uuid::new_v4().to_string(), request_id: uuid::Uuid::new_v4().to_string(),
            reference: MediaRef { provider: "youtube".into(), kind: "video".into(), external_id: "jNQXAC9IVRw".into() },
            revision: 1, state: "resolving".into(), title: String::new(), channel_name: String::new(), created_at: 1,
            received_bytes: 0, total_bytes: None, error: None, library: None, download_root: self.root.path().join("media"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut state = self.service.inner.state.lock().unwrap();
        state.jobs.insert(job.job_id.clone(), job.clone());
        state.cancel.insert(job.job_id.clone(), cancel.clone());
        Task { service: self.service.clone(), job, cancel, receiver: None, queued: Instant::now(), lookup }
    }
}
impl Drop for Fixture { fn drop(&mut self) { self.service.shutdown(); } }
fn image(_: &MediaJob, _: &AtomicBool) -> Option<String> {
    use base64::Engine;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())))
}
fn absent(_: &MediaJob, _: &AtomicBool) -> Option<String> { None }
fn path(task: &Task) -> std::path::PathBuf {
    storage::job_dir(&task.job.download_root, &task.job.reference.provider, &task.job.job_id).unwrap()
}
fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "optional workers did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn new_job_snapshot_is_immutable_and_does_not_mutate_media_job() {
    let fixture = Fixture::new();
    let task = fixture.task(image);
    task.run();
    let directory = path(&task);
    let original = std::fs::read(directory.join(super::super::FILE)).unwrap();
    assert_eq!(super::super::read(&directory).unwrap(), image(&task.job, &task.cancel));
    task.run();
    assert_eq!(std::fs::read(directory.join(super::super::FILE)).unwrap(), original);
    let state = fixture.service.inner.state.lock().unwrap();
    assert_eq!(state.jobs[&task.job.job_id].state, "resolving");
    assert_eq!(state.jobs[&task.job.job_id].revision, 1);
    assert!(Arc::ptr_eq(&state.cancel[&task.job.job_id], &task.cancel));
}

#[test]
fn exact_owner_scope_cancel_completion_and_shutdown_reject_optional_work() {
    for case in 0..11 {
        let fixture = Fixture::new();
        let task = fixture.task(image);
        {
            let mut state = fixture.service.inner.state.lock().unwrap();
            match case {
                0 => state.jobs.get_mut(&task.job.job_id).unwrap().request_id.push('x'),
                1 => state.jobs.get_mut(&task.job.job_id).unwrap().reference.external_id = "abcdefghijk".into(),
                2 => state.jobs.get_mut(&task.job.job_id).unwrap().download_root = fixture.root.path().join("other"),
                3 => state.jobs.get_mut(&task.job.job_id).unwrap().created_at += 1,
                4 => state.jobs.get_mut(&task.job.job_id).unwrap().broadcast_id = Some("abcdefghijk".into()),
                5 => { state.cancel.insert(task.job.job_id.clone(), Arc::new(AtomicBool::new(false))); },
                6 => task.cancel.store(true, Ordering::Release),
                7 => state.jobs.get_mut(&task.job.job_id).unwrap().state = "completed".into(),
                8 => { state.jobs.remove(&task.job.job_id); },
                9 => state.shutting_down = true,
                _ => { state.cancel.remove(&task.job.job_id); },
            }
        }
        task.run();
        assert!(!path(&task).exists(), "case {case}");
    }
}

#[test]
fn late_cancel_and_expired_queue_do_not_save() {
    fn cancel_after_lookup(job: &MediaJob, cancel: &AtomicBool) -> Option<String> {
        let image = image(job, cancel);
        cancel.store(true, Ordering::Release);
        image
    }
    let fixture = Fixture::new();
    let task = fixture.task(cancel_after_lookup);
    task.run(); assert!(!path(&task).exists());
    let mut task = fixture.task(|_, _| panic!("expired task must not lookup"));
    task.queued -= QUEUE_AGE + Duration::from_secs(1);
    task.run(); assert!(!path(&task).exists());
}

#[test]
fn bounded_two_workers_return_without_waiting_and_active_ids_deduplicate() {
    static STARTED: AtomicUsize = AtomicUsize::new(0);
    fn blocked(_: &MediaJob, cancel: &AtomicBool) -> Option<String> {
        STARTED.fetch_add(1, Ordering::AcqRel);
        while !cancel.load(Ordering::Acquire) { std::thread::sleep(Duration::from_millis(2)); }
        None
    }
    let fixture = Fixture::new();
    let mut cancels = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..CAPACITY {
        let task = fixture.task(blocked);
        cancels.push(task.cancel.clone()); ids.push(task.job.job_id.clone());
        fixture.service.inner.profiles.enqueue(task);
    }
    wait_until(|| STARTED.load(Ordering::Acquire) == WORKERS);
    {
        let queue = fixture.service.inner.profiles.state.lock().unwrap();
        assert_eq!(queue.workers, WORKERS); assert_eq!(queue.active.len(), CAPACITY);
    }
    let overflow = fixture.task(absent);
    let overflow_id = overflow.job.job_id.clone();
    fixture.service.inner.profiles.enqueue(overflow);
    assert!(!fixture.service.inner.profiles.state.lock().unwrap().active.contains(&overflow_id));
    let active = fixture.service.active_ids();
    for id in ids { assert_eq!(active.iter().filter(|item| **item == format!("platform-media:{id}")).count(), 1); }
    for cancel in cancels { cancel.store(true, Ordering::Release); }
    wait_until(|| fixture.service.inner.profiles.active_ids().is_empty());
    assert_eq!(STARTED.load(Ordering::Acquire), WORKERS);
}

#[test]
fn lookup_panic_and_missing_picture_release_registration_without_media_failure() {
    let fixture = Fixture::new();
    let task = fixture.task(|_, _| panic!("synthetic optional failure"));
    let id = task.job.job_id.clone();
    fixture.service.inner.profiles.enqueue(task);
    wait_until(|| fixture.service.inner.profiles.active_ids().is_empty());
    let state = fixture.service.inner.state.lock().unwrap();
    assert_eq!(state.jobs[&id].state, "resolving");
    assert!(state.jobs[&id].error.is_none());
}
