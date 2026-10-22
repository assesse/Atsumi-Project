//! Bounded SOOP VOD fullscreen observations, not input authority or a key log.
use super::input_intent::Reason;
use crate::platform_native_geometry::InputGate;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Default)]
pub(super) struct Budget(AtomicU8);

/// One non-cloneable result ticket, issued only for a logged accepted input.
pub(super) struct Attempt;
impl Attempt {
    fn accepted(gate: InputGate) -> Option<Self> {
        gate.allowed().then_some(Self)
    }
    pub(super) fn finish(self, success: bool) {
        tracing::info!(diag_stage = "soop_vod_fullscreen_result", success);
    }
}

impl Budget {
    fn claim(&self, key: i32) -> Option<&'static str> {
        let action = match key {
            1 => "mouse",
            0x46 => "f",
            0x1B => "escape",
            _ => return None,
        };
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < 32).then_some(count.saturating_add(1))
            })
            .ok()
            .map(|_| action)
    }
    pub(super) fn observe(&self, key: i32, gate: InputGate) -> Option<Attempt> {
        self.observe_receipt(key, gate, false, None)
    }
    pub(super) fn observe_receipt(
        &self,
        key: i32,
        gate: InputGate,
        receipt_used: bool,
        reason: Option<Reason>,
    ) -> Option<Attempt> {
        let action = self.claim(key)?;
        if let Some(reason) = reason {
            tracing::info!(
                diag_stage = "soop_vod_fullscreen_input",
                fullscreen_key = action,
                input_gate = gate.diagnostic(),
                allowed = gate.allowed(),
                receipt_used,
                diag_operation = reason.diagnostic()
            );
        } else {
            tracing::info!(
                diag_stage = "soop_vod_fullscreen_input",
                fullscreen_key = action,
                input_gate = gate.diagnostic(),
                allowed = gate.allowed(),
                receipt_used
            );
        }
        Attempt::accepted(gate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_actions_consume_bounded_per_bridge_budget() {
        let budget = Budget::default();
        for key in [0, 13, 32, 0x53, 0x10, 0x11, 0x12, i32::MAX] {
            assert_eq!(budget.claim(key), None);
        }
        for index in 0..32 {
            assert_eq!(
                budget.claim([1, 0x46, 0x1B][index % 3]),
                Some(["mouse", "f", "escape"][index % 3])
            );
        }
        for _ in 0..1024 {
            assert_eq!(budget.claim(1), None);
        }
        assert_eq!(budget.0.load(Ordering::Relaxed), 32);
        assert_eq!(Budget::default().claim(1), Some("mouse"));
    }
    #[test]
    fn simultaneous_observations_cannot_exceed_thirty_two_attempts() {
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
                .map(|t| t.join().unwrap())
                .sum::<usize>(),
            32
        );
    }
    #[test]
    fn denied_inputs_never_issue_a_result_ticket() {
        for gate in [
            InputGate::InvalidInput,
            InputGate::WindowUnavailable,
            InputGate::KeyReleased,
            InputGate::FocusMismatch,
            InputGate::PointerOutside,
        ] {
            assert!(Attempt::accepted(gate).is_none());
        }
        assert!(Attempt::accepted(InputGate::Accepted).is_some());
        let budget = Budget::default();
        for _ in 0..32 {
            assert!(budget.observe(1, InputGate::Accepted).is_some());
        }
        assert!(budget.observe(1, InputGate::Accepted).is_none());
        assert!(Budget::default().observe(13, InputGate::Accepted).is_none());
    }
    #[test]
    fn fixed_receipt_reasons_share_the_existing_attempt_budget() {
        let budget = Budget::default();
        for _ in 0..32 {
            assert!(budget
                .observe_receipt(
                    1,
                    InputGate::InvalidInput,
                    false,
                    Some(Reason::NativePointer)
                )
                .is_none());
        }
        assert_eq!(budget.0.load(Ordering::Relaxed), 32);
        assert!(budget
            .observe_receipt(1, InputGate::Accepted, true, Some(Reason::Accepted))
            .is_none());
        for reason in [
            Reason::NoEvent,
            Reason::NativePointer,
            Reason::BindingUnavailable,
            Reason::ForwardFocus,
            Reason::Expired,
            Reason::BindingMismatch,
            Reason::Accepted,
        ] {
            let value = reason.diagnostic();
            assert!(
                !value.is_empty()
                    && value.len() <= 64
                    && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            );
        }
    }
    #[test]
    fn receipt_trace_contains_only_fixed_fields_with_sixty_four_row_ceiling() {
        use std::sync::{Arc, Mutex};
        use tracing::{
            field::{Field, Visit},
            Event, Subscriber,
        };
        use tracing_subscriber::{layer::Context, prelude::*, Layer};
        #[derive(Default)]
        struct Fields(serde_json::Map<String, serde_json::Value>);
        impl Visit for Fields {
            fn record_str(&mut self, f: &Field, v: &str) {
                self.0.insert(f.name().into(), serde_json::json!(v));
            }
            fn record_bool(&mut self, f: &Field, v: bool) {
                self.0.insert(f.name().into(), serde_json::json!(v));
            }
            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        }
        struct Capture(Arc<Mutex<Vec<serde_json::Value>>>);
        impl<S: Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
                let mut fields = Fields::default();
                event.record(&mut fields);
                self.0
                    .lock()
                    .unwrap()
                    .push(serde_json::Value::Object(fields.0));
            }
        }
        let rows = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::with_default(
            tracing_subscriber::registry().with(Capture(rows.clone())),
            || {
                let budget = Budget::default();
                for _ in 0..40 {
                    if let Some(attempt) =
                        budget.observe_receipt(1, InputGate::Accepted, true, Some(Reason::Accepted))
                    {
                        attempt.finish(true);
                    }
                }
            },
        );
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 64);
        for pair in rows.chunks_exact(2) {
            assert_eq!(
                pair[0],
                serde_json::json!({"diag_stage":"soop_vod_fullscreen_input","fullscreen_key":"mouse","input_gate":"accepted","allowed":true,"receipt_used":true,"diag_operation":"receipt_accepted"})
            );
            assert_eq!(
                pair[1],
                serde_json::json!({"diag_stage":"soop_vod_fullscreen_result","success":true})
            );
        }
    }
}
