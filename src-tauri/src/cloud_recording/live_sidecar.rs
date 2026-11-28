//! Native joined relay -> encrypted ownership -> immutable history capability.
//! This does not mint a completed media job or a YouTube playback clock.
use super::{live_relay::LiveRelaySnapshot, live_store::LiveReceipt, model::CloudError,
    relay_timing_read::{self as timing, FileProof, RelayNativeCapture, RelayTimingLease, RelayTimingProof}, CloudService};
use serde::{Deserialize, Serialize};
use std::{io::{BufRead, BufReader, Read, Seek, SeekFrom}, sync::{Arc, Mutex, OnceLock, Weak, atomic::{AtomicBool, AtomicUsize, Ordering}}};
#[path = "live_sidecar_files.rs"]
mod files;
#[path = "live_chat_source.rs"]
mod chat_source;
#[path="live_soop_chat_source.rs"]
mod soop_chat_source;
pub(crate) use soop_chat_source::{SoopSourceChatPage,SoopSourceChatItem,SoopSourceChatPosition};
pub(crate) use chat_source::{SourceChatItem, SourceChatPage, SourceChatPosition};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    capture: RelayNativeCapture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frame_reference: Option<crate::streaming::relay_status::FrameReferenceInspection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sealed: Option<Sealed>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Sealed { timing: RelayTimingProof, history: FileProof }
impl Record {
    pub(super) fn valid_for(&self, receipt: &LiveReceipt) -> bool {
        receipt.view.receipt_persisted && receipt.view.manager_joined && receipt.view.worker_joined && receipt.view.sidecar_joined
            && super::live_store::terminal(&receipt.view.state)
            && receipt.view.run_id.as_deref().zip(receipt.sidecar_id.as_deref()).is_some_and(|(run, sidecar)|
                self.capture.matches(&receipt.source, run, sidecar, receipt.view.segment_acks, receipt.view.end_list_acked))
            && self.sealed.as_ref().is_none_or(|s| s.timing.valid() && s.history.bytes <= 2 * 1024 * 1024)
            && self.frame_reference.as_ref().is_none_or(|r| r.valid() && r.state != "sampling")
    }
}
fn unavailable() -> CloudError {
    CloudError::new("CLOUD_LIVE_HISTORY", "이 전송과 저장된 채팅 기록의 연결을 확인하지 못했습니다. 원본 기록은 보존됩니다.", false)
}
fn cancelled() -> CloudError { CloudError::cancelled() }

#[derive(Clone)]
pub(super) struct WorkState {
    pub(super) cancel: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
}
impl WorkState { pub(super) fn active(&self) -> bool { self.active.load(Ordering::Acquire) != 0 } }
struct Work {
    service: Weak<CloudService>,
    id: String,
    cancel: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
}
struct Activity { service: Weak<CloudService>, active: Arc<AtomicUsize> }
impl Drop for Activity {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        if let Some(service) = self.service.upgrade() { service.wake.notify_all(); }
    }
}
impl Work {
    fn begin(&self) -> Result<Activity, CloudError> {
        if self.cancel.load(Ordering::Acquire) { return Err(cancelled()); }
        self.active.fetch_add(1, Ordering::AcqRel);
        let activity = Activity { service: self.service.clone(), active: self.active.clone() };
        if self.cancel.load(Ordering::Acquire) { return Err(cancelled()); }
        Ok(activity)
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        if let Some(service) = self.service.upgrade() {
            if let Ok(mut inner) = service.lock() { inner.live.sidecar_work.remove(&self.id); }
            service.wake.notify_all();
        }
    }
}
/// Native-only capability: no Deserialize and no caller-supplied filesystem path.
/// Manual history uses the actual provider shape while remote time remains unknown.
pub(crate) struct LiveHistoryLease {
    provider: String,
    sidecar_id: String,
    capture: RelayNativeCapture,
    timing: RelayTimingLease,
    history: files::HistoryPins,
    frame_reference: Option<crate::streaming::relay_status::FrameReferenceInspection>,
    read: Mutex<()>,
    references: OnceLock<Option<VerifiedLiveReferences>>,
    work: Work,
}
/// Borrowed only from a native history lease. No Deserialize, Clone, public
/// fields or factory from JSON/path. The coordinator must retain this lease
/// while using these source references and their acknowledged timing summary.
pub(crate) struct VerifiedLiveReferences {
    references: super::live_fingerprint::ReferenceSet,
    timing: timing::RelayTimingSummary,
}
impl VerifiedLiveReferences {
    pub(crate) fn references(&self) -> &super::live_fingerprint::ReferenceSet { &self.references }
    pub(crate) fn timing(&self) -> &timing::RelayTimingSummary { &self.timing }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveHistoryFacts {
    pub provider: String,
    pub files: usize,
    pub acknowledged_segments: u64,
    pub transport_complete: bool,
    pub history_incomplete: bool,
    pub remote_anchor: &'static str,
    pub source_reference: String,
    pub source_reference_reason: Option<String>,
}
#[derive(Serialize)]
#[serde(tag = "provider", content = "page", rename_all = "lowercase")]
pub(crate) enum LiveHistoryPage {
    Soop(crate::platform_media::replay_chat::ReplayChatPage),
    Chzzk { items: Vec<crate::streaming::model::ChatMessage>, cursor: Option<String> },
}
impl LiveHistoryLease {
    /// Cancel only this reader capability before its owner joins workers.
    /// Provider caches and other open histories retain their own lifetimes.
    pub(crate) fn cancel(&self) {
        self.work.cancel.store(true, Ordering::Release);
    }
    fn valid(&self) -> Result<(), CloudError> {
        if self.work.cancel.load(Ordering::Acquire) { return Err(cancelled()); }
        self.history.valid().map_err(|_| unavailable())
    }
    pub(crate) fn facts(&self) -> LiveHistoryFacts {
        let s = self.timing.summary();
        LiveHistoryFacts { provider: self.provider.clone(), files: self.history.files(),
            acknowledged_segments: s.acknowledged_segments, transport_complete: s.transport_complete,
            history_incomplete: s.sidecar_incomplete, remote_anchor: "unknown",
            source_reference: self.frame_reference.as_ref().map_or_else(|| "unavailable".into(), |r| r.state.clone()),
            source_reference_reason: self.frame_reference.as_ref().map_or_else(|| Some("not_captured".into()), |r| r.reason.clone()) }
    }
    pub(crate) fn timing(&self) -> &RelayTimingLease { &self.timing }
    /// Immutable numeric source evidence, not a measured remote anchor. An
    /// incomplete set remains incomplete; absence is not synthetic readiness.
    pub(crate) fn references(&self) -> Result<Option<&VerifiedLiveReferences>, CloudError> {
        let _active = self.work.begin()?;
        let _read = self.read.lock().map_err(|_| unavailable())?;
        self.valid()?;
        if let Some(references) = self.references.get() { return Ok(references.as_ref()); }
        let admitted = match self.frame_reference.as_ref().filter(|state| state.persisted) {
            None => None,
            Some(state) => {
                const MAX: u64 = 256 * 1024;
                let mut file = self.history.file(super::live_fingerprint::FILE_NAME).map_err(|_| unavailable())?;
                let length = file.metadata().map_err(|_| unavailable())?.len();
                if length == 0 || length > MAX { return Err(unavailable()); }
                file.seek(SeekFrom::Start(0)).map_err(|_| unavailable())?;
                let mut bytes = Vec::with_capacity(length as usize);
                (&mut file).take(MAX + 1).read_to_end(&mut bytes).map_err(|_| unavailable())?;
                if bytes.len() as u64 != length { return Err(unavailable()); }
                let reference: super::live_fingerprint::ReferenceSet = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
                let summary = self.timing.summary();
                let incomplete = reference.cancelled || reference.queue_dropped != 0
                    || reference.decode_failures != 0 || reference.windows.len() < 3;
                if reference.version != 1 || reference.remote_anchor != "unknown"
                    || !self.capture.owns_reference(reference.run_id, reference.source_id)
                    || reference.windows.len() > 3 || reference.sampled_segments > reference.acknowledged_segments
                    || reference.windows.len() as u64 > reference.sampled_segments
                    || reference.decode_failures.checked_add(reference.static_windows)
                        .and_then(|n| n.checked_add(reference.windows.len() as u64))
                        .is_none_or(|n| n > reference.sampled_segments)
                    || reference.acknowledged_segments != summary.acknowledged_segments
                    || reference.incomplete != incomplete
                    || crate::streaming::relay_status::FrameReferenceInspection::settled(&reference) != *state
                    || (!reference.windows.is_empty() && (reference.stream_id.is_none() || reference.stream_id != summary.stream_id))
                    || reference.stream_id.is_some_and(|stream| stream.is_nil() || Some(stream) != summary.stream_id) {
                    return Err(unavailable());
                }
                self.timing.verify_reference_windows(&reference.windows, &self.work.cancel).map_err(|_| unavailable())?;
                Some(VerifiedLiveReferences { references: reference, timing: summary.clone() })
            }
        };
        self.valid()?;
        self.references.set(admitted).map_err(|_| unavailable())?;
        Ok(self.references.get().ok_or_else(unavailable)?.as_ref())
    }
    pub(crate) fn page(&self, cursor: Option<&str>, limit: usize) -> Result<LiveHistoryPage, CloudError> {
        let _active = self.work.begin()?;
        let _read = self.read.lock().map_err(|_| unavailable())?;
        self.valid()?;
        let page = if self.provider == "soop" {
            LiveHistoryPage::Soop(crate::platform_media::replay_chat::read_relay_page(
                self.capture.root(), &self.sidecar_id, cursor, limit).map_err(|_| unavailable())?)
        } else {
            let offset = cursor.map(|s| {
                if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) { return Err(unavailable()); }
                s.parse::<u64>().map_err(|_| unavailable())
            }).transpose()?.unwrap_or(0);
            let mut file = self.history.file("chat.jsonl").map_err(|_| unavailable())?;
            let length = file.metadata().map_err(|_| unavailable())?.len();
            if offset > length { return Err(unavailable()); }
            if offset > 0 {
                file.seek(SeekFrom::Start(offset - 1)).map_err(|_| unavailable())?;
                let mut byte = [0]; file.read_exact(&mut byte).map_err(|_| unavailable())?;
                if byte[0] != b'\n' { return Err(unavailable()); }
            }
            file.seek(SeekFrom::Start(offset)).map_err(|_| unavailable())?;
            let mut reader = BufReader::with_capacity(8192, file);
            let mut items = Vec::new();
            let mut next = offset;
            for _ in 0..limit.clamp(1, 120) {
                if self.work.cancel.load(Ordering::Acquire) { return Err(cancelled()); }
                let mut line = Vec::new();
                (&mut reader).take(65537).read_until(b'\n', &mut line).map_err(|_| unavailable())?;
                if line.is_empty() { break; }
                if line.len() > 65536 { return Err(unavailable()); }
                // A disk-full/interrupted writer may leave an uncommitted last
                // line. Preserve all preceding records without inventing a row.
                if line.last() != Some(&b'\n') { next = length; break; }
                let item: crate::streaming::model::ChatMessage = serde_json::from_slice(&line).map_err(|_| unavailable())?;
                next = next.checked_add(line.len() as u64).ok_or_else(unavailable)?;
                items.push(item);
            }
            LiveHistoryPage::Chzzk { items, cursor: (next < length).then(|| next.to_string()) }
        };
        self.valid()?;
        Ok(page)
    }
    pub(crate) fn asset(&self, id: &str) -> Result<Option<String>, CloudError> {
        let _active = self.work.begin()?;
        self.valid()?;
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err(unavailable()); }
        let name = if self.provider == "soop" { format!("chat-assets/{id}") } else { format!("replay-assets/{id}.json") };
        if !self.history.contains(&name) { return Ok(None); }
        let result = if self.provider == "soop" {
            crate::platform_media::replay_chat::read_relay_asset(self.capture.root(), id).map_err(|_| unavailable())?
        } else {
            use base64::Engine;
            let asset = crate::streaming::replay_assets::read_recording(self.capture.root(), id).map_err(|_| unavailable())?;
            Some(format!("data:{};base64,{}", asset.mime, base64::engine::general_purpose::STANDARD.encode(asset.bytes)))
        };
        self.valid()?;
        Ok(result)
    }
}

impl CloudService {
    fn history_work(self: &Arc<Self>, cancel: Arc<AtomicBool>) -> Result<Work, CloudError> {
        let mut inner = self.lock()?;
        Self::admit(&inner)?;
        if cancel.load(Ordering::Acquire) { return Err(cancelled()); }
        if inner.live.sidecar_work.len() >= 8 { return Err(CloudError::new("CLOUD_BUSY", "열려 있는 채팅 기록을 먼저 닫아 주세요.", true)); }
        let id = uuid::Uuid::new_v4().to_string();
        let active = Arc::new(AtomicUsize::new(0));
        inner.live.sidecar_work.insert(id.clone(), WorkState { cancel: cancel.clone(), active: active.clone() });
        Ok(Work { service: Arc::downgrade(self), id, cancel, active })
    }
    /// Preserve native provenance before expensive hashing, including normal quit.
    /// A pending capture is not replay readiness. Restart can finish the same seal.
    pub(super) fn finish_live_sidecar(self: &Arc<Self>, id: &str, observed: &LiveRelaySnapshot) -> Result<(), CloudError> {
        let relay = observed.relay.as_ref().ok_or_else(unavailable)?;
        if !observed.manager_joined || !relay.worker_joined || !relay.sidecar.as_ref().is_some_and(|s| s.joined) { return Err(unavailable()); }
        let native = relay.sidecar_owner.as_ref().ok_or_else(unavailable)?;
        let root = relay.sidecar_directory.as_deref().ok_or_else(unavailable)?;
        let delivery = relay.delivery.as_ref().ok_or_else(unavailable)?;
        let source = self.lock()?.catalog.live_receipts.iter().find(|r| r.view.selection_id == id)
            .map(|r| r.source.clone()).ok_or_else(unavailable)?;
        if observed.selection_id != id || observed.provider != source.provider || observed.channel_id != source.channel_id
            || observed.broadcast_key != source.broadcast_key || observed.run_id.as_deref() != Some(relay.run_id.as_str()) {
            return Err(unavailable());
        }
        let capture = RelayNativeCapture::from_native(root, native, &source, delivery.segment_acks, delivery.end_list_acked).map_err(|_| unavailable())?;
        {
            let mut inner = self.lock()?;
            let mut candidate = inner.catalog.clone();
            let receipt = candidate.live_receipts.iter_mut().find(|r| r.view.selection_id == id).ok_or_else(unavailable)?;
            let record = Record { capture, frame_reference: relay.frame_reference.clone(), sealed: None };
            if !record.valid_for(receipt) { return Err(unavailable()); }
            match &receipt.sidecar {
                Some(old) if old.capture != record.capture => return Err(unavailable()),
                Some(_) => {},
                None => { receipt.sidecar = Some(record); receipt.changed()?; self.publish(&mut inner, candidate)?; }
            }
        }
        // A normal shutdown may cancel hashing; the encrypted native capture above
        // remains available for an explicit history-open after restart.
        let lease = self.open_live_history(id)?;
        let _facts = lease.facts();
        // Consume the actual original-platform parser, rather than leaving a new
        // proof factory disconnected from history. A long SOOP history may report
        // preparing and continue in the already bounded provider reader.
        let _first_page = lease.page(None, 1)?;
        let _source_references = lease.references()?;
        Ok(())
    }
    pub(crate) fn open_live_history(self: &Arc<Self>, id: &str) -> Result<LiveHistoryLease, CloudError> {
        self.open_live_history_cancellable(id, Arc::new(AtomicBool::new(false)))
    }
    /// A native replay owner passes its own cancellation flag before sealing
    /// starts, so closing an opening player also stops source verification.
    pub(super) fn open_live_history_cancellable(self: &Arc<Self>, id: &str, cancel: Arc<AtomicBool>) -> Result<LiveHistoryLease, CloudError> {
        if !uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) { return Err(unavailable()); }
        let work = self.history_work(cancel)?;
        let _active = work.begin()?;
        let (record, provider, sidecar_id) = {
            let inner = self.lock()?;
            let receipt = inner.catalog.live_receipts.iter().find(|r| r.view.selection_id == id).ok_or_else(unavailable)?;
            let record = receipt.sidecar.as_ref().filter(|s| s.valid_for(receipt)).ok_or_else(unavailable)?.clone();
            (record, receipt.source.provider.clone(), receipt.sidecar_id.clone().ok_or_else(unavailable)?)
        };
        let sealed = if let Some(sealed) = &record.sealed { sealed.clone() } else {
            let timing = record.capture.seal(&work.cancel).map_err(|_| unavailable())?;
            let history = files::seal(record.capture.root(), &work.cancel).map_err(|_| unavailable())?;
            Sealed { timing, history }
        };
        let timing = record.capture.open(&sealed.timing, &work.cancel).map_err(|_| unavailable())?;
        let history = files::HistoryPins::open(record.capture.root(), &sealed.history, &work.cancel).map_err(|_| unavailable())?;
        if record.frame_reference.as_ref().is_some_and(|r| r.persisted)
            && !history.contains(crate::cloud_recording::live_fingerprint::FILE_NAME) { return Err(unavailable()); }
        {
            let mut inner = self.lock()?;
            Self::admit(&inner)?;
            if work.cancel.load(Ordering::Acquire) { return Err(cancelled()); }
            let mut candidate = inner.catalog.clone();
            let receipt = candidate.live_receipts.iter_mut().find(|r| r.view.selection_id == id).ok_or_else(unavailable)?;
            if receipt.sidecar.as_ref() != Some(&record) || !record.valid_for(receipt) { return Err(unavailable()); }
            if record.sealed.is_none() {
                receipt.sidecar.as_mut().ok_or_else(unavailable)?.sealed = Some(sealed);
                receipt.changed()?;
                self.publish(&mut inner, candidate)?;
            }
        }
        Ok(LiveHistoryLease { provider, sidecar_id, capture: record.capture, timing, history,
            frame_reference: record.frame_reference, read: Mutex::new(()), references: OnceLock::new(), work })
    }
    /// Called after request_live_shutdown and owned thread joins. Idle leases
    /// cannot keep exit pending, but cancellation is not an in-flight I/O join.
    pub(super) fn wait_live_history(&self) -> Result<(), CloudError> {
        let mut inner = self.lock()?;
        while inner.live.sidecar_work.values().any(WorkState::active) {
            inner = self.wake.wait_timeout(inner, std::time::Duration::from_millis(100))
                .map_err(|_| CloudError::storage())?.0;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "live_sidecar_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::{verify_actual_joined_history, ActualHistoryReport};
#[cfg(all(test, windows))]
#[path = "live_sidecar_bridge_probe.rs"]
mod bridge_probe;
