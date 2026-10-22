//! YouTube-only same-document recording intent. The host arm mutex guards it.
use crate::platform_media::live::RecordingOwner;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Run {
    owner: Option<RecordingOwner>,
    epoch: u64,
    active: bool,
    no_progress: u8,
    attempts: Vec<Instant>,
}
impl Run {
    pub fn start(&mut self, owner: RecordingOwner, epoch: u64) {
        *self = Self { owner: Some(owner), epoch, active: true, ..Self::default() };
    }
    pub fn permits(&self, owner: &RecordingOwner, epoch: u64) -> bool {
        self.active && self.owner.as_ref() == Some(owner) && self.epoch == epoch
    }
    pub fn cancel(&mut self, owner: Option<&RecordingOwner>) {
        if owner.is_none() || self.owner.as_ref() == owner { self.active = false; }
    }
    pub fn next(&mut self, owner: &RecordingOwner, epoch: u64, progress: bool, now: Instant) -> bool {
        if !self.permits(owner, epoch) { return false; }
        self.attempts.retain(|at| now.saturating_duration_since(*at) < Duration::from_secs(60));
        self.no_progress = if progress { 0 } else { self.no_progress.saturating_add(1) };
        if self.no_progress > 2 || self.attempts.len() >= 3 { self.active = false; return false; }
        self.attempts.push(now);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn owner(generation: u64) -> RecordingOwner { RecordingOwner::Scheduled { schedule_id: "UC1234567890123456789012".into(), broadcast_key: "AbCdEf123_-".into(), run_id: "30000000-0000-4000-8000-000000000001".into(), generation } }
    #[test] fn owner_epoch_and_cancellation_fence_future_files() {
        let mut run=Run::default();let a=owner(1);let b=owner(2);let now=Instant::now();run.start(a.clone(),5);
        assert!(!run.next(&b,5,true,now));assert!(!run.next(&a,6,true,now));run.cancel(Some(&b));assert!(run.next(&a,5,true,now));
        run.cancel(Some(&a));assert!(!run.next(&a,5,true,now));run.start(b.clone(),6);run.cancel(Some(&a));assert!(run.permits(&b,6));run.cancel(None);assert!(!run.permits(&b,6));
    }
    #[test] fn only_two_consecutive_empty_boundaries_and_three_per_minute() {
        let mut run=Run::default();let a=owner(1);let now=Instant::now();run.start(a.clone(),1);
        assert!(run.next(&a,1,false,now));assert!(run.next(&a,1,false,now));assert!(!run.next(&a,1,false,now));
        run.start(a.clone(),1);assert!(run.next(&a,1,false,now));assert!(run.next(&a,1,true,now));assert!(run.next(&a,1,false,now));assert!(!run.next(&a,1,true,now));
        run.start(a.clone(),1);for _ in 0..3 {assert!(run.next(&a,1,true,now));}assert!(run.next(&a,1,true,now+Duration::from_secs(60)));
    }
}
