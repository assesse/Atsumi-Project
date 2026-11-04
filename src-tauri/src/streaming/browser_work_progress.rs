//! Ephemeral worker telemetry. Never persisted as a completion claim.
use std::{cell::RefCell, collections::HashMap, sync::{Arc, Mutex, OnceLock}};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct WorkProgress {
    pub phase: String,
    pub completed: u64,
    pub total: Option<u64>,
    pub unit: String,
    pub updated_at: u64,
}
type Entry = Arc<Mutex<WorkProgress>>;
fn entries() -> &'static Mutex<HashMap<String, Entry>> {
    static ENTRIES: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(Mutex::default)
}
thread_local! { static CURRENT: RefCell<Option<Entry>> = const { RefCell::new(None) }; }
pub(crate) struct Scope { id: String, entry: Entry, prior: Option<Entry> }
impl Scope {
    pub(crate) fn enter(id: &str, phase: &str) -> Self {
        let entry = Arc::new(Mutex::new(WorkProgress { phase: phase.into(), completed: 0, total: None, unit: "files".into(), updated_at: super::now_ms() }));
        entries().lock().unwrap_or_else(|p| p.into_inner()).insert(id.into(),entry.clone());
        let prior=CURRENT.with(|c| c.replace(Some(entry.clone())));
        Self { id:id.into(), entry, prior }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let mut map=entries().lock().unwrap_or_else(|p|p.into_inner());
        if map.get(&self.id).is_some_and(|v|Arc::ptr_eq(v,&self.entry)) { map.remove(&self.id); }
        CURRENT.with(|c| c.replace(self.prior.take()));
    }
}
pub(crate) fn snapshot(id: &str) -> Option<WorkProgress> {
    let entry=entries().lock().ok()?.get(id)?.clone();
    let value=entry.lock().ok()?.clone();
    Some(value)
}
pub(crate) fn phase(phase: &str, total: Option<u64>, unit: &str) {
    CURRENT.with(|c| { if let Some(entry)=c.borrow().as_ref() {
        *entry.lock().unwrap_or_else(|p|p.into_inner())=WorkProgress {phase:phase.into(),completed:0,total,unit:unit.into(),updated_at:super::now_ms()};
    }});
}
pub(crate) fn advance(amount:u64) {
    CURRENT.with(|c| { if let Some(entry)=c.borrow().as_ref() {
        let mut p=entry.lock().unwrap_or_else(|p|p.into_inner());
        p.completed=p.completed.saturating_add(amount).min(p.total.unwrap_or(u64::MAX));
        p.updated_at=super::now_ms();
    }});
}
pub(crate) fn advance_for(phases:&[&str],amount:u64) {
    CURRENT.with(|c| { if let Some(entry)=c.borrow().as_ref() {
        let mut p=entry.lock().unwrap_or_else(|p|p.into_inner());
        if phases.contains(&p.phase.as_str()) {
            p.completed=p.completed.saturating_add(amount).min(p.total.unwrap_or(u64::MAX));
            p.updated_at=super::now_ms();
        }
    }});
}
pub(crate) fn current() -> Option<Arc<Mutex<WorkProgress>>> { CURRENT.with(|c|c.borrow().clone()) }
pub(crate) fn tool_progress(entry:&Option<Entry>, micros:u64) {
    if let Some(entry)=entry {
        let mut p=entry.lock().unwrap_or_else(|p|p.into_inner());
        let value=(micros/1_000_000).min(p.total.unwrap_or(u64::MAX));
        if p.unit=="seconds" && value>p.completed {p.completed=value;p.updated_at=super::now_ms();}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn scope_is_ephemeral_and_monotonic_tool_updates_do_not_fake_completion() {
        { let _s=Scope::enter("work-progress-test","preparing");
          phase("verifying",Some(20),"seconds");
          let p=current();tool_progress(&p,9_000_000);tool_progress(&p,8_000_000);
          assert_eq!(snapshot("work-progress-test").unwrap().completed,9);
          assert_eq!(snapshot("work-progress-test").unwrap().phase,"verifying");
        }
        assert!(snapshot("work-progress-test").is_none());
    }
}
