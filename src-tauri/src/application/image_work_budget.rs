use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Condvar, Mutex,
    },
    time::Duration,
};

use crate::thumbnail::CancellationToken;

// Network workers may hold encoded responses while waiting, but only two
// download stages may decode, transcode, or hash full images at a time.
static FULL_IMAGE_WORK: ImageWorkBudget = ImageWorkBudget::new(2);
static STORED_BYTES: AtomicU64 = AtomicU64::new(0);
static STORED_PAGES: AtomicU64 = AtomicU64::new(0);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(crate) fn acquire(
    cancellation: Option<&CancellationToken>,
) -> Option<ImageWorkPermit<'static>> {
    FULL_IMAGE_WORK.acquire(cancellation)
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
}

struct ImageWorkBudget {
    limit: usize,
    state: Mutex<ImageWorkState>,
    pressure_epoch: AtomicU64,
    wake: Condvar,
}

impl ImageWorkBudget {
    const fn new(limit: usize) -> Self {
        Self {
            limit,
            state: Mutex::new(ImageWorkState {
                active: 0,
                waiting: 0,
            }),
            pressure_epoch: AtomicU64::new(0),
            wake: Condvar::new(),
        }
    }

    fn is_backlogged(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.waiting > 0 || state.active >= self.limit
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
            if state.active < self.limit {
                if waiting {
                    state.waiting -= 1;
                }
                state.active += 1;
                return Some(ImageWorkPermit { budget: self });
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
    fn shared_full_image_budget_remains_two_slots() {
        assert_eq!(FULL_IMAGE_WORK.limit, 2);
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
}
