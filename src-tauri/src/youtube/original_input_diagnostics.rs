//! Bounded technical observations only. Never ordinary keys, source URLs or IDs.
use crate::platform_native_geometry::InputGate;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Default)]
pub(super) struct Budget(AtomicU8);
impl Budget {
    fn claim(&self, key: i32) -> Option<&'static str> {
        let action = match key {
            1 => "mouse",
            0x1B => "escape",
            0x46 => "f",
            _ => return None,
        };
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < 32).then_some(count.saturating_add(1))
            })
            .ok()
            .map(|_| action)
    }
    pub(super) fn record(&self, vod: bool, key: i32, result: InputGate, receipt_used: bool) {
        let Some(action) = self.claim(key) else {
            return;
        };
        tracing::info!(
            diag_stage = if vod {
                "youtube_vod_fullscreen_input"
            } else {
                "youtube_fullscreen_input"
            },
            fullscreen_key = action,
            input_gate = result.diagnostic(),
            allowed = result.allowed(),
            receipt_used
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_is_bounded_per_bridge_and_never_wraps() {
        let budget = Budget::default();
        for key in [0, 13, 32, 0x53, 0x10, 0x11, 0x12, i32::MAX] {
            assert_eq!(budget.claim(key), None);
        }
        for index in 0..32 {
            assert_eq!(
                budget.claim([1, 0x1B, 0x46][index % 3]),
                Some(["mouse", "escape", "f"][index % 3])
            );
        }
        for _ in 0..1024 {
            assert_eq!(budget.claim(0x1B), None);
        }
        assert_eq!(budget.0.load(Ordering::Relaxed), 32);
        assert_eq!(Budget::default().claim(0x1B), Some("escape"));
    }
    #[test]
    fn parallel_claims_share_the_same_bounded_budget() {
        let budget = std::sync::Arc::new(Budget::default());
        let threads = (0..8)
            .map(|_| {
                let budget = budget.clone();
                std::thread::spawn(move || (0..32).filter(|_| budget.claim(1).is_some()).count())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .sum::<usize>(),
            32
        );
    }
}
