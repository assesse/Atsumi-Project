use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Condvar, Mutex,
    },
    time::Duration,
};

use crate::{domain::ProcessingMode, thumbnail::CancellationToken};

// All albums share one budget, including high-performance page fan-out. Never
// multiply the decoder memory limit by the number of active albums.
static FULL_IMAGE_WORK: ImageWorkBudget = ImageWorkBudget::new(2);
static HASH_READS: ImageWorkBudget = ImageWorkBudget::new(1);
static PARALLEL_WORKERS: AtomicUsize = AtomicUsize::new(1);
static STORED_BYTES: AtomicU64 = AtomicU64::new(0);
static STORED_PAGES: AtomicU64 = AtomicU64::new(0);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

thread_local! {
    // A verified read can call a decoder/hash that also enforces this budget.
    // Reuse its slot on this thread; keep it until the last scope is dropped.
    static CURRENT_IMAGE_WORK: RefCell<Weak<ImageWorkLease>> = const { RefCell::new(Weak::new()) };
}

/// Hardware-based CPU concurrency, separate from network and HDD pacing.
pub(crate) fn configure(mode: ProcessingMode) {
    let logical = std::thread::available_parallelism().map_or(2, usize::from);
    let limit = processing_limit(mode, logical, memory_bytes());
    FULL_IMAGE_WORK.set_policy(mode, limit);
    PARALLEL_WORKERS.store(
        if mode == ProcessingMode::Basic {
            1
        } else {
            limit
        },
        Ordering::Release,
    );
    tracing::info!(
        processing_mode = mode.as_str(),
        logical_processors = logical,
        image_slots = limit,
        "image processing budget configured; HDD and network limits unchanged"
    );
}

pub(crate) fn parallel_workers() -> usize {
    PARALLEL_WORKERS.load(Ordering::Acquire)
}

/// Keep a high-CPU mode from creating simultaneous seek-heavy HDD reads.
pub(crate) fn acquire_hdd_read(
    cancellation: &CancellationToken,
) -> Option<ImageWorkPermit<'static>> {
    HASH_READS.acquire(Some(cancellation))
}

fn processing_limit(mode: ProcessingMode, logical: usize, memory: Option<u64>) -> usize {
    // Concurrency caps do not reserve cores or enforce a CPU percentage.
    let cpu_slots = match mode {
        ProcessingMode::Basic => (logical / 2).clamp(1, 2),
        ProcessingMode::GamePriority => (logical / 4).clamp(1, 4),
        ProcessingMode::AtsumiPriority => logical.saturating_sub(2).clamp(1, 16),
    };
    // At most a quarter of physical RAM (and half of available RAM), allowing
    // 512 MiB per image including encoded/decode/resize scratch. Unknown RAM
    // falls back to two slots, including high mode.
    let memory_slots = memory.map_or(2, |bytes| (bytes / (4 * 512 * 1024 * 1024)) as usize);
    cpu_slots.min(memory_slots.max(1))
}

#[cfg(windows)]
fn memory_bytes() -> Option<u64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut memory = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe {
        GlobalMemoryStatusEx(&mut memory).ok()?;
    }
    // Also bound by currently available RAM so enabling this while other apps
    // consume memory cannot immediately turn CPU work into paging work.
    Some(
        memory
            .ullTotalPhys
            .min(memory.ullAvailPhys.saturating_mul(2)),
    )
}

#[cfg(not(windows))]
fn memory_bytes() -> Option<u64> {
    None
}

pub(crate) fn acquire(cancellation: Option<&CancellationToken>) -> Option<ImageWorkScope> {
    acquire_scoped(&FULL_IMAGE_WORK, cancellation)
}

fn acquire_scoped(
    budget: &'static ImageWorkBudget,
    cancellation: Option<&CancellationToken>,
) -> Option<ImageWorkScope> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return None;
    }
    if let Some(lease) = CURRENT_IMAGE_WORK.with(|current| current.borrow().upgrade()) {
        if std::ptr::eq(lease._permit.budget, budget) {
            return Some(ImageWorkScope { _lease: lease });
        }
    }
    let permit = budget.acquire(cancellation)?;
    // The queue mutex is already released. Restore priority before the permit
    // drops, so normal work never waits on our queue lock at background priority.
    let lease = Rc::new(ImageWorkLease {
        _priority: WorkPriority::enter(permit.mode),
        _permit: permit,
    });
    CURRENT_IMAGE_WORK.with(|current| *current.borrow_mut() = Rc::downgrade(&lease));
    Some(ImageWorkScope { _lease: lease })
}

/// Rc deliberately makes this scope !Send/!Sync: Windows priority must be
/// restored on the same thread, including errors and unwinding.
pub(crate) struct ImageWorkScope {
    _lease: Rc<ImageWorkLease>,
}

struct ImageWorkLease {
    // Field drop order is significant: restore before touching the budget lock.
    _priority: WorkPriority,
    _permit: ImageWorkPermit<'static>,
}

struct WorkPriority {
    #[cfg(windows)]
    background_entered: bool,
    #[cfg(windows)]
    previous: Option<i32>,
}

impl WorkPriority {
    fn enter(mode: ProcessingMode) -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                GetCurrentThread, GetThreadPriority, SetThreadPriority,
                THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_IDLE,
            };
            let thread = unsafe { GetCurrentThread() };
            let original = unsafe { GetThreadPriority(thread) };
            let mut priority = Self {
                background_entered: false,
                previous: None,
            };
            // Never change a reusable thread's priority if we cannot restore it.
            if original == i32::MAX {
                return priority;
            }
            if mode != ProcessingMode::AtsumiPriority {
                // Lower I/O and memory priority too; leave playback/UI alone.
                match unsafe { SetThreadPriority(thread, THREAD_MODE_BACKGROUND_BEGIN) } {
                    Ok(()) => priority.background_entered = true,
                    Err(error) => {
                        static REPORTED: std::sync::atomic::AtomicBool =
                            std::sync::atomic::AtomicBool::new(false);
                        if !REPORTED.swap(true, Ordering::Relaxed) {
                            tracing::warn!(%error, "image background priority unavailable; shared concurrency limit still applies");
                        }
                    }
                }
            }
            let target = match mode {
                ProcessingMode::Basic => None,
                ProcessingMode::GamePriority => Some(THREAD_PRIORITY_IDLE),
                ProcessingMode::AtsumiPriority => Some(THREAD_PRIORITY_ABOVE_NORMAL),
            };
            if let Some(target) = target {
                match unsafe { SetThreadPriority(thread, target) } {
                    Ok(()) => priority.previous = Some(original),
                    Err(error) => {
                        tracing::warn!(%error, processing_mode = mode.as_str(), "image CPU priority could not be applied")
                    }
                }
            }
            priority
        }
        #[cfg(not(windows))]
        {
            let _ = mode;
            Self {}
        }
    }
}

impl Drop for WorkPriority {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_END, THREAD_PRIORITY,
            };
            let thread = unsafe { GetCurrentThread() };
            if self.background_entered {
                if let Err(error) = unsafe { SetThreadPriority(thread, THREAD_MODE_BACKGROUND_END) }
                {
                    tracing::warn!(%error, "image worker background priority could not be restored");
                }
            }
            if let Some(previous) = self.previous {
                if let Err(error) = unsafe { SetThreadPriority(thread, THREAD_PRIORITY(previous)) }
                {
                    tracing::warn!(%error, "image worker CPU priority could not be restored");
                }
            }
        }
    }
}

/// Reports pressure on the shared decode, transcode, and full-image hash slots.
pub(crate) fn is_backlogged() -> bool {
    FULL_IMAGE_WORK.is_backlogged()
}

/// Retains evidence of blocked image work between adaptive throughput samples.
pub(crate) fn pressure_epoch() -> u64 {
    FULL_IMAGE_WORK.pressure_epoch.load(Ordering::Relaxed)
}

/// Count source bytes only after both the page file and its checkpoint persist.
pub(crate) fn record_stored_page(bytes: usize) {
    STORED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    STORED_PAGES.fetch_add(1, Ordering::Relaxed);
}

/// Process-local cumulative source bytes and durable page completions.
pub(crate) fn stored_progress() -> (u64, u64) {
    (
        STORED_BYTES.load(Ordering::Relaxed),
        STORED_PAGES.load(Ordering::Relaxed),
    )
}

struct ImageWorkState {
    active: usize,
    waiting: usize,
    mode: ProcessingMode,
}

struct ImageWorkBudget {
    limit: AtomicUsize,
    state: Mutex<ImageWorkState>,
    pressure_epoch: AtomicU64,
    wake: Condvar,
}

impl ImageWorkBudget {
    const fn new(limit: usize) -> Self {
        Self {
            limit: AtomicUsize::new(limit),
            state: Mutex::new(ImageWorkState {
                active: 0,
                waiting: 0,
                mode: ProcessingMode::Basic,
            }),
            pressure_epoch: AtomicU64::new(0),
            wake: Condvar::new(),
        }
    }

    fn is_backlogged(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.waiting > 0 || state.active >= self.limit.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn set_limit(&self, limit: usize) {
        let _state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.limit.store(limit.max(1), Ordering::Release);
        self.wake.notify_all();
    }

    fn set_policy(&self, mode: ProcessingMode, limit: usize) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.mode = mode;
        self.limit.store(limit.max(1), Ordering::Release);
        self.wake.notify_all();
    }

    fn acquire(&self, cancellation: Option<&CancellationToken>) -> Option<ImageWorkPermit<'_>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut waiting = false;
        loop {
            if cancellation.is_some_and(CancellationToken::is_cancelled) {
                if waiting {
                    state.waiting -= 1;
                }
                return None;
            }
            if state.active < self.limit.load(Ordering::Acquire) {
                if waiting {
                    state.waiting -= 1;
                }
                state.active += 1;
                return Some(ImageWorkPermit {
                    budget: self,
                    mode: state.mode,
                });
            }
            if !waiting {
                state.waiting += 1;
                waiting = true;
            }
            // Refresh the epoch while a wait persists, so a sampling window
            // still observes pressure when the queue drains before its end.
            self.pressure_epoch.fetch_add(1, Ordering::Relaxed);
            // CancellationToken has no wake callback. Poll even while all slots
            // are occupied so stopping a download need not wait for a decoder.
            let (guard, _) = self
                .wake
                .wait_timeout(state, CANCELLATION_POLL_INTERVAL)
                .unwrap_or_else(|error| error.into_inner());
            state = guard;
        }
    }
}

pub(crate) struct ImageWorkPermit<'a> {
    budget: &'a ImageWorkBudget,
    mode: ProcessingMode,
}

impl Drop for ImageWorkPermit<'_> {
    fn drop(&mut self) {
        let mut state = self
            .budget
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active -= 1;
        self.budget.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Instant};

    use super::*;

    #[test]
    fn third_image_waits_until_an_active_stage_releases_its_slot() {
        let budget = ImageWorkBudget::new(2);
        assert!(!budget.is_backlogged());
        let first = budget.acquire(None).unwrap();
        assert!(!budget.is_backlogged());
        let second = budget.acquire(None).unwrap();
        assert!(budget.is_backlogged());
        thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (acquired_tx, acquired_rx) = mpsc::channel();
            let budget = &budget;
            scope.spawn(move || {
                started_tx.send(()).unwrap();
                let _permit = budget.acquire(None).unwrap();
                acquired_tx.send(()).unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(acquired_rx.recv_timeout(Duration::from_millis(60)).is_err());
            assert_eq!(budget.state.lock().unwrap().waiting, 1);
            assert!(budget.pressure_epoch.load(Ordering::Relaxed) > 0);
            drop(first);
            acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            drop(second);
        });
        let state = budget.state.lock().unwrap();
        assert_eq!(state.active, 0);
        assert_eq!(state.waiting, 0);
        drop(state);
        assert!(!budget.is_backlogged());
        assert!(budget.pressure_epoch.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn cancelling_a_waiter_does_not_wait_for_active_image_work() {
        let budget = ImageWorkBudget::new(1);
        let active = budget.acquire(None).unwrap();
        let cancellation = CancellationToken::new();
        thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (cancelled_tx, cancelled_rx) = mpsc::channel();
            let budget = &budget;
            let cancellation = &cancellation;
            scope.spawn(move || {
                started_tx.send(()).unwrap();
                cancelled_tx
                    .send(budget.acquire(Some(cancellation)).is_none())
                    .unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while budget.state.lock().unwrap().waiting == 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            let waiting = budget.state.lock().unwrap().waiting;
            cancellation.cancel();
            assert!(cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap());
            assert_eq!(waiting, 1);
        });
        let state = budget.state.lock().unwrap();
        assert_eq!(state.active, 1);
        assert_eq!(state.waiting, 0);
        drop(state);
        drop(active);
        assert!(!budget.is_backlogged());
    }

    #[test]
    fn image_slot_is_released_when_processing_unwinds() {
        let budget = ImageWorkBudget::new(1);
        let result = std::panic::catch_unwind(|| {
            let _permit = budget.acquire(None).unwrap();
            panic!("synthetic image processing failure");
        });
        assert!(result.is_err());
        assert_eq!(budget.state.lock().unwrap().active, 0);
        assert_eq!(budget.state.lock().unwrap().waiting, 0);
        assert!(budget.acquire(None).is_some());
    }

    #[test]
    fn cancelled_work_does_not_enter_the_image_queue() {
        let budget = ImageWorkBudget::new(2);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(budget.acquire(Some(&cancellation)).is_none());
        let state = budget.state.lock().unwrap();
        assert_eq!(state.active, 0);
        assert_eq!(state.waiting, 0);
        assert_eq!(budget.pressure_epoch.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn modes_use_hardware_and_memory_bounds_without_assuming_a_cpu_percentage() {
        use ProcessingMode::*;
        let gib = 1024 * 1024 * 1024;
        assert_eq!(ImageWorkBudget::new(2).limit.load(Ordering::Relaxed), 2);
        assert_eq!(processing_limit(Basic, 16, Some(32 * gib)), 2);
        assert_eq!(processing_limit(GamePriority, 16, Some(32 * gib)), 4);
        assert_eq!(processing_limit(AtsumiPriority, 16, Some(32 * gib)), 14);
        assert_eq!(processing_limit(GamePriority, 64, Some(64 * gib)), 4);
        assert_eq!(processing_limit(AtsumiPriority, 64, Some(64 * gib)), 16);
        for mode in [Basic, GamePriority, AtsumiPriority] {
            assert!(processing_limit(mode, 16, Some(8 * gib)) <= 4);
            assert!(processing_limit(mode, 16, None) <= 2);
            assert_eq!(processing_limit(mode, 1, Some(0)), 1);
            assert_eq!(processing_limit(mode, 0, None), 1);
        }
    }

    #[test]
    fn stored_page_telemetry_is_monotonic_across_completions() {
        let before = stored_progress();
        record_stored_page(512);
        let first = stored_progress();
        record_stored_page(1_024);
        let second = stored_progress();
        // Other download tests may complete pages concurrently in this process.
        assert!(first.0 >= before.0 + 512);
        assert!(first.1 > before.1);
        assert!(second.0 >= first.0 + 1_024);
        assert!(second.1 > first.1);
    }

    #[test]
    fn lowering_limit_drains_existing_work_and_raising_wakes_waiters() {
        let budget = ImageWorkBudget::new(2);
        let first = budget.acquire(None).unwrap();
        let second = budget.acquire(None).unwrap();
        budget.set_limit(1);
        thread::scope(|scope| {
            let (sent, received) = mpsc::channel();
            let budget = &budget;
            scope.spawn(move || {
                let _permit = budget.acquire(None).unwrap();
                sent.send(()).unwrap();
            });
            drop(first);
            assert!(received.recv_timeout(Duration::from_millis(60)).is_err());
            budget.set_limit(2);
            received.recv_timeout(Duration::from_secs(1)).unwrap();
            drop(second);
        });
        assert_eq!(budget.state.lock().unwrap().active, 0);
    }

    #[test]
    fn nested_image_stages_hold_one_slot_until_the_last_scope_leaves() {
        let budget: &'static ImageWorkBudget = Box::leak(Box::new(ImageWorkBudget::new(1)));
        let first = acquire_scoped(budget, None).unwrap();
        let nested = acquire_scoped(budget, None).unwrap();
        assert_eq!(budget.state.lock().unwrap().active, 1);
        let (sent, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _scope = acquire_scoped(budget, None).unwrap();
            sent.send(()).unwrap();
        });
        // An outer caller may drop its handle first; the nested stage still owns
        // the slot and background-priority lifetime.
        drop(first);
        assert!(received.recv_timeout(Duration::from_millis(60)).is_err());
        drop(nested);
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert_eq!(budget.state.lock().unwrap().active, 0);
    }

    #[test]
    fn scoped_waiters_and_nested_stages_respect_cancellation() {
        let budget: &'static ImageWorkBudget = Box::leak(Box::new(ImageWorkBudget::new(1)));
        let active = acquire_scoped(budget, None).unwrap();
        let cancellation = CancellationToken::new();
        let waiting_cancellation = cancellation.clone();
        let (sent, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            sent.send(acquire_scoped(budget, Some(&waiting_cancellation)).is_none())
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while budget.state.lock().unwrap().waiting == 0 && Instant::now() < deadline {
            thread::yield_now();
        }
        cancellation.cancel();
        assert!(received.recv_timeout(Duration::from_secs(1)).unwrap());
        assert!(acquire_scoped(budget, Some(&cancellation)).is_none());
        assert_eq!(budget.state.lock().unwrap().active, 1);
        drop(active);
        worker.join().unwrap();
        assert_eq!(budget.state.lock().unwrap().waiting, 0);
    }

    #[test]
    fn concurrent_nested_stages_share_a_single_global_ceiling() {
        let budget: &'static ImageWorkBudget = Box::leak(Box::new(ImageWorkBudget::new(2)));
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        thread::scope(|workers| {
            for _ in 0..12 {
                let active = &active;
                let peak = &peak;
                workers.spawn(move || {
                    let _scope = acquire_scoped(budget, None).unwrap();
                    let _nested = acquire_scoped(budget, None).unwrap();
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(5));
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= 2);
        assert_eq!(budget.state.lock().unwrap().active, 0);
        assert_eq!(budget.state.lock().unwrap().waiting, 0);
    }

    #[test]
    fn scoped_unwind_releases_slot_and_restores_thread_priority() {
        thread::spawn(|| {
            let budget: &'static ImageWorkBudget = Box::leak(Box::new(ImageWorkBudget::new(1)));
            #[cfg(windows)]
            let before = unsafe {
                windows::Win32::System::Threading::GetThreadPriority(
                    windows::Win32::System::Threading::GetCurrentThread(),
                )
            };
            let result = std::panic::catch_unwind(|| {
                let outer = acquire_scoped(budget, None).unwrap();
                let _nested = acquire_scoped(budget, None).unwrap();
                drop(outer);
                panic!("synthetic decoder failure");
            });
            assert!(result.is_err());
            assert_eq!(budget.state.lock().unwrap().active, 0);
            #[cfg(windows)]
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
                assert_eq!(GetThreadPriority(GetCurrentThread()), before);
            }
            let scope = acquire_scoped(budget, None).unwrap();
            #[cfg(windows)]
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
                assert!(scope._lease._priority.background_entered);
                assert!(GetThreadPriority(GetCurrentThread()) < before);
            }
            drop(scope);
            #[cfg(windows)]
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
                assert_eq!(GetThreadPriority(GetCurrentThread()), before);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    #[ignore = "manual isolated real-image contention profile; changes process-local budget"]
    fn profile_real_hashes_share_background_budget() {
        use crate::domain::{ArtifactSha256, GalleryId, HashProfile, SourcePageNumber};
        use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
        let mut pixels = vec![0_u8; 640 * 960 * 3];
        for (index, value) in pixels.iter_mut().enumerate() {
            *value = (index.wrapping_mul(31) % 251) as u8;
        }
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&pixels, 640, 960, ExtendedColorType::Rgb8)
            .unwrap();
        let hash = || {
            super::super::duplicate_analyzer::compute_page_hash(
                "resource-profile",
                GalleryId::new(1).unwrap(),
                SourcePageNumber::new(1).unwrap(),
                ArtifactSha256::new("01".repeat(32)).unwrap(),
                &bytes,
                &HashProfile::current(),
            )
        };
        let expected = hash().unwrap();
        configure(ProcessingMode::GamePriority);
        let limit = FULL_IMAGE_WORK.limit.load(Ordering::Acquire);
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let background_jobs = AtomicUsize::new(0);
        let started = Instant::now();
        let pages: Vec<_> = (0..32).collect();
        let results = super::super::processing_pool::try_map(&pages, limit + 4, |_| {
            let _scope = acquire(None).unwrap();
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            #[cfg(windows)]
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
                if GetThreadPriority(GetCurrentThread()) < 0 {
                    background_jobs.fetch_add(1, Ordering::SeqCst);
                }
            }
            let result = hash();
            active.fetch_sub(1, Ordering::SeqCst);
            result
        });
        let elapsed = started.elapsed();
        configure(ProcessingMode::Basic);
        let results = results.unwrap();
        assert_eq!(results.len(), 32);
        assert!(results.iter().all(|result| result == &expected));
        assert!(peak.load(Ordering::SeqCst) <= limit);
        #[cfg(windows)]
        assert_eq!(background_jobs.load(Ordering::SeqCst), 32);
        assert_eq!(FULL_IMAGE_WORK.state.lock().unwrap().active, 0);
        eprintln!("real_hash_resource_profile pages=32 dimensions=640x960 requested_workers={} shared_limit={} peak_active={} background_jobs={} elapsed_ms={} identical_hashes=true",
            limit + 4, limit, peak.load(Ordering::SeqCst), background_jobs.load(Ordering::SeqCst), elapsed.as_millis());
    }

    #[test]
    fn waiting_work_uses_the_new_policy_without_interrupting_the_current_image() {
        let budget: &'static ImageWorkBudget = Box::leak(Box::new(ImageWorkBudget::new(1)));
        budget.set_policy(ProcessingMode::AtsumiPriority, 1);
        let active = acquire_scoped(budget, None).unwrap();
        let (sent, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            let scope = acquire_scoped(budget, None).unwrap();
            sent.send(scope._lease._permit.mode).unwrap();
        });
        budget.set_policy(ProcessingMode::GamePriority, 1);
        let nested = acquire_scoped(budget, None).unwrap();
        assert_eq!(nested._lease._permit.mode, ProcessingMode::AtsumiPriority);
        assert!(received.recv_timeout(Duration::from_millis(60)).is_err());
        drop(active);
        drop(nested);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            ProcessingMode::GamePriority
        );
        worker.join().unwrap();
        assert_eq!(budget.state.lock().unwrap().active, 0);
    }

    #[test]
    #[cfg(windows)]
    fn windows_applies_each_mode_and_restores_reused_worker_after_success_and_unwind() {
        thread::spawn(|| {
            use windows::Win32::System::Threading::{
                GetCurrentThread, GetThreadPriority, SetThreadPriority,
                THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_IDLE,
            };
            let thread = unsafe { GetCurrentThread() };
            unsafe { SetThreadPriority(thread, THREAD_PRIORITY_BELOW_NORMAL).unwrap(); }
            let before = unsafe { GetThreadPriority(thread) };
            let budget: &'static ImageWorkBudget = Box::leak(Box::new(ImageWorkBudget::new(1)));
            for mode in [ProcessingMode::AtsumiPriority, ProcessingMode::GamePriority, ProcessingMode::Basic] {
                for unwind in [false, true] {
                    budget.set_policy(mode, 1);
                    let mut observed = false;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let scope = acquire_scoped(budget, None).unwrap();
                        let nested = acquire_scoped(budget, None).unwrap();
                        drop(scope);
                        let current = unsafe { GetThreadPriority(thread) };
                        assert_eq!(nested._lease._priority.background_entered, mode != ProcessingMode::AtsumiPriority);
                        match mode {
                            ProcessingMode::Basic => assert!(current < before),
                            ProcessingMode::GamePriority => assert_eq!(current, THREAD_PRIORITY_IDLE.0),
                            ProcessingMode::AtsumiPriority => assert_eq!(current, THREAD_PRIORITY_ABOVE_NORMAL.0),
                        }
                        eprintln!("processing_priority mode={} before={} during={} background={} unwind={}", mode.as_str(), before, current, nested._lease._priority.background_entered, unwind);
                        observed = true;
                        if unwind { panic!("synthetic processing failure"); }
                    }));
                    assert!(observed, "priority assertion must run before synthetic unwind");
                    assert_eq!(result.is_err(), unwind);
                    assert_eq!(unsafe { GetThreadPriority(thread) }, before);
                    assert_eq!(budget.state.lock().unwrap().active, 0);
                }
            }
        }).join().unwrap();
    }
}
