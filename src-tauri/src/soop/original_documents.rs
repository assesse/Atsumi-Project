//! Provider-local document resource budget, never recording authorization.
use std::sync::{
    atomic::{AtomicU8, AtomicUsize, Ordering},
    Arc,
};

#[derive(Clone, Default)]
pub(super) struct Documents(Arc<AtomicUsize>);
pub(super) struct Permit {
    count: Arc<AtomicUsize>,
    state: AtomicU8,
}
impl Documents {
    pub fn acquire(&self) -> Result<Permit, String> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 4).then_some(count + 1)
            })
            .map_err(|_| "SOOP 방송과 영상은 최대 4개까지 열 수 있습니다.".to_string())?;
        Ok(Permit {
            count: self.0.clone(),
            state: AtomicU8::new(0),
        })
    }
}
impl Permit {
    pub fn release(&self) {
        if self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.count.fetch_sub(1, Ordering::AcqRel);
        }
    }
    /// A failed native close may leave its document alive. Fail closed until process exit.
    pub fn retain_after_close_failure(&self) {
        let _ = self
            .state
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_capacity_failure_cleanup_and_explicit_release_are_idempotent() {
        let budget = Documents::default();
        let held = (0..4)
            .map(|_| budget.acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(budget.acquire().is_err());
        held[0].release();
        held[0].release();
        let next = budget.acquire().unwrap();
        assert!(budget.acquire().is_err());
        drop(next);
        drop(held);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
        {
            let _creation_failure = budget.acquire().unwrap();
        }
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn failed_native_close_does_not_release_a_still_alive_document() {
        let budget = Documents::default();
        let permit = budget.acquire().unwrap();
        permit.retain_after_close_failure();
        permit.release();
        drop(permit);
        assert_eq!(budget.0.load(Ordering::Acquire), 1);
    }
    #[test]
    fn simultaneous_document_types_cannot_overbook_or_evict() {
        let budget = Documents::default();
        let scheduled = budget.acquire().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let workers = (0..16)
            .map(|_| {
                let b = budget.clone();
                let gate = barrier.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    b.acquire().ok()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let held = workers
            .into_iter()
            .filter_map(|w| w.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(held.len(), 3);
        assert_eq!(scheduled.state.load(Ordering::Acquire), 0);
        drop(held);
        drop(scheduled);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
}
