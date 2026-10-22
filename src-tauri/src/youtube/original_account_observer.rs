//! Bounded waiting for one already-issued account observation or main-thread action.
//! No browser operations, authentication values or arbitrary diagnostic payloads.
use std::{
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

const WAIT_SLICE: Duration = Duration::from_millis(250);
const FRESHNESS: Duration = Duration::from_secs(2);
pub(super) const DEADLINE: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WaitFailure { Deadline, Invalidated, Disconnected }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloseOutcome { NotReady, Requested, Failed }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Diagnostic {
    Deadline, Invalidated, EvalFailed, CallbackLost, InvalidResponse,
    StaleObservation, DispatchFailed, ActionLost, CloseFailed,
    CloseRequested, WindowDestroyed,
}
impl Diagnostic {
    pub fn stage(self) -> &'static str {
        match self {
            Self::Deadline => "youtube_account_probe_deadline",
            Self::Invalidated => "youtube_account_probe_invalidated",
            Self::EvalFailed => "youtube_account_probe_eval_failed",
            Self::CallbackLost => "youtube_account_probe_callback_lost",
            Self::InvalidResponse => "youtube_account_probe_invalid_response",
            Self::StaleObservation => "youtube_account_probe_stale_observation",
            Self::DispatchFailed => "youtube_account_probe_dispatch_failed",
            Self::ActionLost => "youtube_account_probe_action_lost",
            Self::CloseFailed => "youtube_account_close_failed",
            Self::CloseRequested => "youtube_account_close_requested",
            Self::WindowDestroyed => "youtube_account_window_destroyed",
        }
    }
}

pub(super) fn fresh(issued_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(issued_at) < FRESHNESS
}

pub(super) fn request_close<E>(issued_at: Instant, now: Instant, eligible: bool,
    close: impl FnOnce() -> Result<(), E>) -> CloseOutcome {
    if !eligible || !fresh(issued_at, now) { return CloseOutcome::NotReady; }
    if close().is_ok() { CloseOutcome::Requested } else { CloseOutcome::Failed }
}

pub(super) fn receive<T>(
    receiver: &Receiver<T>, deadline: Instant, active: impl FnMut() -> bool,
) -> Result<T, WaitFailure> {
    receive_with(deadline, Instant::now, active, |timeout| receiver.recv_timeout(timeout))
}

fn receive_with<T>(
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
    mut active: impl FnMut() -> bool,
    mut receive: impl FnMut(Duration) -> Result<T, RecvTimeoutError>,
) -> Result<T, WaitFailure> {
    loop {
        let remaining = deadline.checked_duration_since(now())
            .filter(|remaining| !remaining.is_zero()).ok_or(WaitFailure::Deadline)?;
        if !active() { return Err(WaitFailure::Invalidated); }
        match receive(remaining.min(WAIT_SLICE)) {
            Ok(value) => {
                if now() >= deadline { return Err(WaitFailure::Deadline); }
                if !active() { return Err(WaitFailure::Invalidated); }
                return Ok(value);
            }
            Err(RecvTimeoutError::Timeout) => {},
            Err(RecvTimeoutError::Disconnected) => return Err(WaitFailure::Disconnected),
        }
    }
}

/// A bounded inter-observation delay, with the same invalidation checks.
pub(super) fn pause(deadline: Instant, active: impl FnMut() -> bool) {
    let (_keep_sender_alive, receiver) = std::sync::mpsc::sync_channel::<()>(1);
    let until = deadline.min(Instant::now() + Duration::from_secs(1));
    let _ = receive(&receiver, until, active);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn slow_callback_waits_for_the_same_request_but_is_not_fresh() {
        let start = Instant::now(); let elapsed = Cell::new(Duration::ZERO);
        let calls = Cell::new(0);
        let result = receive_with(start + DEADLINE, || start + elapsed.get(), || true, |slice| {
            calls.set(calls.get() + 1);
            if elapsed.get() >= Duration::from_secs(3) { return Ok("same response"); }
            elapsed.set(elapsed.get() + slice); Err(RecvTimeoutError::Timeout)
        });
        assert_eq!(result, Ok("same response")); assert_eq!(calls.get(), 13);
        assert!(!fresh(start, start + elapsed.get()));
        let new_issue = start + elapsed.get(); assert!(fresh(new_issue, new_issue));
    }

    #[test]
    fn missing_callback_has_one_absolute_deadline_not_a_renewed_timeout() {
        let start = Instant::now(); let elapsed = Cell::new(Duration::ZERO);
        let calls = Cell::new(0);
        let result: Result<(), _> = receive_with(start + DEADLINE, || start + elapsed.get(), || true, |slice| {
            calls.set(calls.get() + 1); elapsed.set(elapsed.get() + slice);
            Err(RecvTimeoutError::Timeout)
        });
        assert_eq!(result, Err(WaitFailure::Deadline));
        assert_eq!(elapsed.get(), DEADLINE); assert_eq!(calls.get(), 480);
        // Production's active predicate also checks the same deadline; retain
        // the accurate timeout reason instead of calling expiry invalidation.
        elapsed.set(Duration::ZERO);
        let result: Result<(), _> = receive_with(start + DEADLINE, || start + elapsed.get(),
            || elapsed.get() < DEADLINE, |slice| {
                elapsed.set(elapsed.get() + slice); Err(RecvTimeoutError::Timeout)
            });
        assert_eq!(result, Err(WaitFailure::Deadline));
    }

    #[test]
    fn invalidation_or_disconnection_ends_wait_without_another_request() {
        let start = Instant::now(); let calls = Cell::new(0);
        let result: Result<(), _> = receive_with(start + DEADLINE, || start, || calls.get() < 2, |_| {
            calls.set(calls.get() + 1); Err(RecvTimeoutError::Timeout)
        });
        assert_eq!(result, Err(WaitFailure::Invalidated)); assert_eq!(calls.get(), 2);
        let disconnected: Result<(), _> = receive_with(start + DEADLINE, || start, || true,
            |_| Err(RecvTimeoutError::Disconnected));
        assert_eq!(disconnected, Err(WaitFailure::Disconnected));
    }

    #[test]
    fn a_late_success_does_not_escape_deadline_or_invalidation() {
        let start = Instant::now(); let elapsed = Cell::new(Duration::ZERO);
        assert_eq!(receive_with(start + DEADLINE, || start + elapsed.get(), || true, |_| {
            elapsed.set(DEADLINE); Ok(true)
        }), Err(WaitFailure::Deadline));
        let valid = Cell::new(true);
        assert_eq!(receive_with(start + DEADLINE, || start, || valid.get(), |_| {
            valid.set(false); Ok(true)
        }), Err(WaitFailure::Invalidated));
        assert!(fresh(start, start + Duration::from_millis(1999)));
        assert!(!fresh(start, start + Duration::from_secs(2)));
    }

    #[test]
    fn failed_close_can_retry_only_from_a_new_eligible_fresh_observation() {
        let start = Instant::now(); let calls = Cell::new(0);
        assert_eq!(request_close(start, start, true, || {
            calls.set(calls.get() + 1); Err("private error is never retained")
        }), CloseOutcome::Failed);
        let later = start + Duration::from_secs(3);
        assert_eq!(request_close(start, later, true, || {
            calls.set(calls.get() + 1); Ok::<_, ()>(())
        }), CloseOutcome::NotReady);
        assert_eq!(request_close(later, later, false, || {
            calls.set(calls.get() + 1); Ok::<_, ()>(())
        }), CloseOutcome::NotReady);
        assert_eq!(request_close(later, later, true, || {
            calls.set(calls.get() + 1); Ok::<_, ()>(())
        }), CloseOutcome::Requested);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn diagnostics_are_distinct_bounded_constant_stages_not_success_claims() {
        let stages = [Diagnostic::Deadline, Diagnostic::Invalidated, Diagnostic::EvalFailed,
            Diagnostic::CallbackLost, Diagnostic::InvalidResponse, Diagnostic::StaleObservation,
            Diagnostic::DispatchFailed, Diagnostic::ActionLost, Diagnostic::CloseFailed,
            Diagnostic::CloseRequested, Diagnostic::WindowDestroyed];
        let mut unique = std::collections::HashSet::new();
        for stage in stages {
            let value = stage.stage(); assert!(value.len() <= 64);
            assert!(value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
            assert!(unique.insert(value)); assert!(!value.contains("completed"));
        }
        assert_ne!(Diagnostic::CloseFailed.stage(), Diagnostic::CloseRequested.stage());
        assert_ne!(Diagnostic::CloseRequested.stage(), Diagnostic::WindowDestroyed.stage());
    }
}
