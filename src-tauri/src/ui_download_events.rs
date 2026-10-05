//! UI notifications are projections, not the durable download journal. Keep
//! only the latest revision per entry while a renderer is busy or absent.
use crate::domain::{DownloadChangedEvent, DownloadJobProjection, JobEvent};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use tauri::{Manager, State};

const CAPACITY: usize = 4096;
const BATCH_SIZE: usize = 128;
#[derive(Default)]
pub struct DownloadEvents(Mutex<Pending>);
#[derive(Default)]
struct Pending {
    rows: BTreeMap<String, DownloadJobProjection>,
    order: VecDeque<String>,
    resync: bool,
    received: u64,
    coalesced: u64,
    overflow: u64,
    delivered: u64,
}
#[derive(Serialize)]
pub struct Batch {
    items: Vec<Row>,
    resync: bool,
}
#[derive(Serialize)]
pub struct Row {
    job: JobEvent,
    download: DownloadChangedEvent,
}
impl DownloadEvents {
    pub fn publish(&self, projection: DownloadJobProjection) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let key = projection.download.entry_id.clone();
        state.received += 1;
        if let Some(previous) = state.rows.get(&key) {
            if previous.download.revision >= projection.download.revision {
                return;
            }
            state.coalesced += 1;
        } else {
            if state.rows.len() >= CAPACITY {
                if let Some(old) = state.order.pop_front() {
                    state.rows.remove(&old);
                }
                state.resync = true;
                state.overflow += 1;
            }
            state.order.push_back(key.clone());
        }
        state.rows.insert(key, projection);
    }
    fn take(&self) -> Batch {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut items = Vec::new();
        while items.len() < BATCH_SIZE {
            let Some(key) = state.order.pop_front() else {
                break;
            };
            if let Some(p) = state.rows.remove(&key) {
                items.push(Row {
                    job: p.job,
                    download: p.download,
                });
            }
        }
        state.delivered += items.len() as u64;
        Batch {
            items,
            resync: std::mem::take(&mut state.resync),
        }
    }
    pub fn stats(&self) -> serde_json::Value {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        serde_json::json!({"pending":state.rows.len(),"received":state.received,"coalesced":state.coalesced,"overflow":state.overflow,"delivered":state.delivered})
    }
}
#[tauri::command]
pub fn download_events_take(
    window: tauri::Webview,
    state: State<'_, Arc<DownloadEvents>>,
    epoch: String,
) -> Result<Batch, String> {
    if window.label() != "main"
        || window
            .app_handle()
            .state::<Arc<crate::ui_diagnostics::UiDiagnostics>>()
            .epoch()
            != epoch
    {
        return Err("download notification document expired".into());
    }
    Ok(state.take())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn projection(entry: usize, revision: u64) -> DownloadJobProjection {
        let job = JobEvent {
            job_id: format!("job-{entry}"),
            gallery_id: Some(entry as i64),
            revision,
            state: crate::domain::JobState::Downloading,
            completed_units: Some(revision),
            total_units: Some(100),
            message: None,
        };
        DownloadJobProjection {
            job,
            download: DownloadChangedEvent {
                entry_id: entry.to_string(),
                gallery_id: entry as i64,
                revision,
                state: crate::domain::JobState::Downloading,
                progress: Some(revision as f64),
                attempt: Some(1),
                error_code: None,
                error_message: None,
                review_kind: None,
                review_id: None,
            },
        }
    }
    #[test]
    fn a_busy_renderer_retains_latest_state_not_every_progress_tick() {
        let queue = DownloadEvents::default();
        for i in 1..1000 {
            queue.publish(projection(1, i));
        }
        queue.publish(projection(1, 1));
        let batch = queue.take();
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].download.revision, 999);
        assert!(queue.take().items.is_empty());
    }
    #[test]
    fn absent_renderer_is_bounded_and_overflow_requests_durable_resync() {
        let queue = DownloadEvents::default();
        for i in 0..CAPACITY + 10 {
            queue.publish(projection(i, 1));
        }
        assert_eq!(queue.0.lock().unwrap().rows.len(), CAPACITY);
        let batch = queue.take();
        assert!(batch.resync);
        assert_eq!(batch.items.len(), BATCH_SIZE);
        assert!(!queue.take().resync);
    }
}
