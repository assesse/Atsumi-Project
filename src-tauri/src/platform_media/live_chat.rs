//! One receive-only chat connection per live session. Recording adds a durable
//! sink, not another connection; detaching a sink synchronizes before returning.
use super::{
    chat_assets::{ArchiveAssets, SessionAssets},
    chat_parts::{self, ClosedParts, Stream, Writer},
    chat_status::{self, ChatArchiveSummary, Marker, Reason},
    replay_chat::{normalize_public_line, ChatPart, NormalizedChatEvent},
    storage, youtube_chat, MediaRef, PlatformMediaService,
};
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const RING_ITEMS: usize = 500;
const RING_BYTES: usize = 1024 * 1024;
#[cfg(test)]
const OLD_FILE_LIMIT: u64 = 128 * 1024 * 1024;
const LINE_LIMIT: usize = 512 * 1024;
pub use super::chat_assets::ChatAssetRead;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveChatRow {
    pub sequence: u64,
    pub kind: String,
    pub author: String,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ChatPart>,
    pub amount: Option<String>,
    pub badges: Vec<String>,
    pub color: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveChatPage {
    pub items: Vec<LiveChatRow>,
    pub cursor: u64,
    pub status: String,
    pub ended: bool,
    pub gap: bool,
}
struct Archive {
    marker: Marker,
    writer: Writer,
    directory: PathBuf,
    bytes: u64,
    error: Option<String>,
    failure_reason: Option<Reason>,
    assets: Option<Arc<ArchiveAssets>>,
    // Only losses preceding this attachment count. Evicting the shared recent
    // cache later does not lose rows already written to this archive.
    recent_loss: Option<RecentLoss>,
    coverage_warning: Option<&'static str>,
}
#[derive(Clone, Copy, Default)]
struct RecentLoss {
    latest_wall_ms: Option<u64>,
    unknown_until_ms: Option<u64>,
    unbounded_unknown: bool,
}
impl RecentLoss {
    fn merge(&mut self, other: Self) {
        self.latest_wall_ms = self.latest_wall_ms.max(other.latest_wall_ms);
        self.unknown_until_ms = self.unknown_until_ms.max(other.unknown_until_ms);
        self.unbounded_unknown |= other.unbounded_unknown;
    }
    fn unknown(received: Option<u64>) -> Self {
        Self { unknown_until_ms: received, unbounded_unknown: received.is_none(), ..Self::default() }
    }
    fn event(event: &NormalizedChatEvent) -> Self {
        let wall = event.source_timestamp_ms.unwrap_or(event.received_at_ms);
        if valid_wall(wall) {
            Self { latest_wall_ms: Some(wall), ..Self::default() }
        } else {
            Self::unknown(valid_wall(event.received_at_ms).then_some(event.received_at_ms))
        }
    }
    fn reason(self, origin: Option<u64>) -> Option<Reason> {
        if self.latest_wall_ms.is_none() && self.unknown_until_ms.is_none() && !self.unbounded_unknown {
            return None;
        }
        let Some(origin) = origin.filter(|value| valid_wall(*value)) else {
            return Some(Reason::StatusUnknown);
        };
        if self.latest_wall_ms.is_some_and(|wall| wall >= origin) {
            Some(Reason::ArchiveLimit)
        } else if self.unbounded_unknown || self.unknown_until_ms.is_some_and(|wall| wall >= origin) {
            Some(Reason::StatusUnknown)
        } else {
            None
        }
    }
}
fn valid_wall(value: u64) -> bool {
    (1_000_000_000_000..=9_000_000_000_000).contains(&value)
}
// Only recognized empty protocol envelopes are harmless. A parser returning
// zero normalized rows is not proof that an unfamiliar renderer has no content.
fn youtube_empty_poll(bytes: &[u8]) -> bool {
    use serde_json::Value;
    fn keys(value: &Value, allowed: &[&str]) -> bool {
        value.as_object().is_some_and(|map| map.keys().all(|key| allowed.contains(&key.as_str())))
    }
    fn empty_actions(value: &Value) -> bool {
        value["actions"].as_array().is_some_and(Vec::is_empty)
    }
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else { return false; };
    if value.get("isLive").is_some_and(|live| !live.is_boolean()) { return false; }
    if keys(&value, &["isLive", "replayChatItemAction"]) {
        let replay = &value["replayChatItemAction"];
        return keys(replay, &["actions", "videoOffsetTimeMsec"]) && empty_actions(replay)
            && replay.get("videoOffsetTimeMsec").is_none_or(|offset| offset.as_u64().is_some()
                || offset.as_str().is_some_and(|offset| offset.parse::<u64>().is_ok()));
    }
    if keys(&value, &["isLive", "actions"]) { return empty_actions(&value); }
    if !keys(&value, &["continuationContents"])
        || !keys(&value["continuationContents"], &["liveChatContinuation"]) { return false; }
    let poll = &value["continuationContents"]["liveChatContinuation"];
    if !keys(poll, &["actions", "continuations"]) || !empty_actions(poll) { return false; }
    poll["continuations"].as_array().is_some_and(|rows| !rows.is_empty() && rows.len() <= 8 && rows.iter().all(|row| {
        let Some(map) = row.as_object().filter(|map| map.len() == 1) else { return false; };
        map.iter().all(|(kind, body)| {
            matches!(kind.as_str(), "timedContinuationData" | "invalidationContinuationData")
                && keys(body, &["continuation", "timeoutMs"])
                && body["continuation"].as_str().is_some_and(|token| !token.is_empty() && token.len() <= 32768)
                && body.get("timeoutMs").is_none_or(|timeout| timeout.as_u64().is_some())
        })
    }))
}
#[derive(Clone)]
struct RecentRow {
    bytes: Vec<u8>,
    raw: bool,
    coverage: RecentLoss,
}
pub(super) struct ChatDetachOutcome {
    directory: PathBuf,
    marker: Marker,
    reason: Option<Reason>,
    assets_incomplete: bool,
    parts: ClosedParts,
}
impl ChatDetachOutcome {
    #[cfg(test)]
    pub(super) fn persist(
        self,
        directory: &Path,
        provider: &str,
        job_id: &str,
    ) -> Result<ChatArchiveSummary, String> {
        self.persist_cancellable(directory, provider, job_id, &AtomicBool::new(false))
    }
    pub(super) fn persist_cancellable(
        self,
        directory: &Path,
        provider: &str,
        job_id: &str,
        cancel: &AtomicBool,
    ) -> Result<ChatArchiveSummary, String> {
        if self.directory != directory {
            return Err(chat_status::ERROR.into());
        }
        chat_status::identity(&self.marker, provider, job_id)?;
        chat_status::finish_multipart(
            directory,
            &self.marker,
            self.parts,
            self.reason,
            self.assets_incomplete,
            cancel,
        )
    }
}

/// Raw recordings know the selected file origin only after verified remux.
/// This owns closed sinks, never a reference to the current receiver/archive.
pub(super) struct RawChatDetachOutcome {
    closed: ChatDetachOutcome,
    recent_loss: Option<RecentLoss>,
}
impl RawChatDetachOutcome {
    fn resolve_origin(mut self, origin: Option<u64>) -> ChatDetachOutcome {
        let origin = origin.filter(|value| valid_wall(*value));
        let reason = self.recent_loss.take().and_then(|loss| loss.reason(origin))
            .or_else(|| origin.is_none().then_some(Reason::StatusUnknown));
        // An earlier connection/write/flush/interruption failure is irreversible.
        if self.closed.reason.is_none() { self.closed.reason = reason; }
        self.closed
    }
    pub(super) fn persist_cancellable(
        self,
        directory: &Path,
        provider: &str,
        job_id: &str,
        timeline: Option<&super::youtube_spool::RawTimelineFacts>,
        cancel: &AtomicBool,
    ) -> Result<ChatArchiveSummary, String> {
        if provider != "youtube" || self.closed.directory != directory {
            return Err(chat_status::ERROR.into());
        }
        chat_status::identity(&self.closed.marker, provider, job_id)?;
        if cancel.load(Ordering::Acquire) { return Err(chat_status::ERROR.into()); }
        // A failed/missing timeline cannot manufacture a replay anchor or clear
        // earlier partial proof. Keep every chat part even on this failure.
        let origin = timeline.and_then(|facts| facts.publish(directory, &self.closed.marker, cancel).ok());
        self.resolve_origin(origin).persist_cancellable(directory, provider, job_id, cancel)
    }
}
struct State {
    ring: VecDeque<(LiveChatRow, usize)>,
    ring_bytes: usize,
    next: u64,
    ids: HashSet<String>,
    order: VecDeque<String>,
    archive: Option<Archive>,
    archive_error: Option<String>,
    receiver_failure: Option<String>,
    status: String,
    require_verified_connection: bool,
    verified_connection: bool,
    verify_from_first_event: bool,
    recent: VecDeque<RecentRow>,
    recent_bytes: usize,
    recent_loss: RecentLoss,
    preparation_started: Option<Instant>,
    youtube_availability: Option<bool>,
}
pub(super) struct SharedChat {
    state: Mutex<State>,
    stop: Arc<AtomicBool>,
    started: AtomicBool,
    done: AtomicBool,
    preserve_source: AtomicBool,
    assets: SessionAssets,
    asset_warning: AtomicBool,
}
struct ReceiverDone(Arc<SharedChat>);
impl Drop for ReceiverDone {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.stop.store(true, Ordering::Release);
            self.0
                .fail_receiver("채팅 수신 작업이 중단됐습니다. 기존 기록은 보존했습니다.".into());
        }
        self.0.assets.stop();
        self.0.done.store(true, Ordering::Release);
    }
}
impl SharedChat {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                ring: VecDeque::new(),
                ring_bytes: 0,
                next: 1,
                ids: HashSet::new(),
                order: VecDeque::new(),
                archive: None,
                archive_error: None,
                receiver_failure: None,
                status: "채팅 연결 준비 중".into(),
                require_verified_connection: false,
                verified_connection: false,
                verify_from_first_event: false,
                recent: VecDeque::new(),
                recent_bytes: 0,
                recent_loss: RecentLoss::default(),
                preparation_started: None,
                youtube_availability: None,
            }),
            stop: Arc::new(AtomicBool::new(false)),
            started: AtomicBool::new(false),
            done: AtomicBool::new(true),
            preserve_source: AtomicBool::new(false),
            assets: SessionAssets::new(),
            asset_warning: AtomicBool::new(false),
        })
    }
    pub(super) fn start(
        self: &Arc<Self>,
        service: PlatformMediaService,
        reference: MediaRef,
        directory: PathBuf,
        youtube_descriptor: Result<Option<youtube_chat::Descriptor>, String>,
    ) {
        self.start_bound(service, reference, directory, youtube_descriptor, None);
    }
    pub(super) fn start_bound(
        self: &Arc<Self>,
        service: PlatformMediaService,
        reference: MediaRef,
        directory: PathBuf,
        youtube_descriptor: Result<Option<youtube_chat::Descriptor>, String>,
        soop_broadcast: Option<String>,
    ) {
        if self.stop.load(Ordering::Acquire) || self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        if soop_broadcast.is_some() {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.require_verified_connection = true;
        }
        self.done.store(false, Ordering::Release);
        let this = self.clone();
        let worker = std::thread::Builder::new()
            .name("platform-live-chat".into())
            .spawn(move || {
                let _finished = ReceiverDone(this.clone());
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if reference.provider == "soop" {
                        this.soop(&reference.external_id, soop_broadcast.as_deref())
                    } else {
                        this.youtube(&service, youtube_descriptor, &directory)
                    }
                }))
                .unwrap_or_else(|_| {
                    Err("채팅 수신 작업이 중단됐습니다. 기존 기록은 보존했습니다.".into())
                });
                this.finish_receiver(result);
            });
        if worker.is_err() {
            self.finish_receiver(Err("채팅 수신을 준비하지 못했습니다.".into()));
            self.done.store(true, Ordering::Release);
        }
    }
    pub(super) fn start_official_youtube(
        self: &Arc<Self>,
        service: PlatformMediaService,
        reference: MediaRef,
        directory: PathBuf,
        descriptor: Result<Option<youtube_chat::Descriptor>, String>,
    ) {
        if self.observe_youtube_availability(Some(true)).is_err() { return; }
        self.start_bound(service, reference, directory, descriptor, None);
    }
    /// An observation is not a producer attempt. Unknown/disabled can later
    /// become available, but never restart a producer or its first deadline.
    pub(super) fn observe_youtube_availability(&self, available: Option<bool>) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        if self.stop.load(Ordering::Acquire) { return Err("채팅 연결을 종료하는 중입니다.".into()); }
        if self.started.load(Ordering::Acquire) || state.receiver_failure.is_some() { return Ok(()); }
        state.require_verified_connection = true;
        state.verify_from_first_event = true;
        state.preparation_started.get_or_insert_with(Instant::now);
        state.youtube_availability = available;
        state.status = state.archive_error.clone().unwrap_or_else(|| match available {
            Some(false) => "이 방송은 채팅을 제공하지 않습니다.",
            Some(true) => "채팅 연결 준비 중",
            None => "채팅 연결 확인 중",
        }.into());
        Ok(())
    }
    pub(super) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.assets.stop();
    }
    pub(super) fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire) && self.assets.is_done()
    }
    #[cfg(test)]
    pub(super) fn producer_started_for_test(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    fn set_status(&self, message: String) {
        if let Ok(mut state) = self.state.lock() {
            state.status = state.archive_error.clone().unwrap_or(message);
        }
    }
    fn fail_receiver(&self, message: String) {
        self.preserve_source.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            let failure = state.receiver_failure.get_or_insert(message).clone();
            // Share the attachment lock: a failed receiver can neither leave its
            // current archive complete nor make the next empty archive complete.
            if let Some(archive) = state.archive.as_mut() {
                archive.failure_reason.get_or_insert(Reason::Interrupted);
                archive.error.get_or_insert_with(|| failure.clone());
            }
            let error = state.archive_error.get_or_insert(failure).clone();
            state.status = error;
        }
    }
    fn finish_receiver(&self, result: Result<(), String>) {
        if let Err(error) = result {
            self.fail_receiver(error);
        } else if self.stop.load(Ordering::Acquire) {
            self.set_status("채팅 연결 종료".into());
        } else {
            self.set_status("채팅 수신 종료".into());
        }
    }
    fn run_tail(&self, read: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(read))
            .unwrap_or_else(|_| Err("채팅 처리 작업이 중단됐습니다.".into()));
        if let Err(error) = &result {
            // Latch before stopping/waiting for the producer, so a recording
            // detached during process drain cannot hide a known tail loss.
            self.fail_receiver(error.clone());
            self.stop.store(true, Ordering::Release);
        }
        result
    }
    pub(super) fn status(&self) -> String {
        self.state
            .lock()
            .map(|state| self.observed_status(&state))
            .unwrap_or_else(|_| "채팅 상태 오류".into())
    }
    fn observed_status(&self, state: &State) -> String {
        if !self.stop.load(Ordering::Acquire) && state.require_verified_connection
            && !state.verified_connection && state.youtube_availability != Some(false)
            && state.receiver_failure.is_none() && state.archive_error.is_none()
            && state.preparation_started.is_some_and(|at| at.elapsed() >= Duration::from_secs(15)) {
            "채팅 연결을 확인하지 못했습니다.".into()
        } else { state.status.clone() }
    }
    /// Nonblocking preparation for the already-running receiver. A failed or
    /// timed-out chat does not prevent video-only recording; attachment keeps
    /// its existing partial proof instead of inventing connection coverage.
    pub(super) fn is_preparing(&self) -> bool {
        if self.stop.load(Ordering::Acquire) { return false; }
        self.state.lock().map(|state| {
            state.require_verified_connection && !state.verified_connection
                && state.receiver_failure.is_none()
                && state.youtube_availability != Some(false)
                && state.preparation_started.is_some_and(|at| at.elapsed() < Duration::from_secs(15))
        }).unwrap_or(false)
    }
    fn accept_youtube_bootstrap(&self, rows: Vec<Vec<u8>>) -> Result<(), String> {
        self.run_tail(|| {
            let count = rows.len();
            for line in rows { self.ingest("youtube", &line)?; }
            let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
            if !self.stop.load(Ordering::Acquire) && state.receiver_failure.is_none() {
                state.verified_connection = true;
                state.status = state.archive_error.clone().unwrap_or_else(|| "채팅 연결됨".into());
                tracing::info!(action_count = count, "YouTube initial chat response verified");
            }
            Ok(())
        })
    }
    pub(super) fn attach_recording_archive(
        &self,
        directory: &Path,
        provider: &str,
        job_id: &str,
    ) -> Result<(), String> {
        storage::safe_path(directory)?;
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        if state.archive.is_some() {
            return Err("이 방송의 녹화 채팅을 마무리하는 중입니다.".into());
        }
        let marker = chat_status::begin_multipart(directory, provider, job_id)?;
        let writer = Writer::create(directory, &marker)?;
        let assets = ArchiveAssets::new(directory).ok();
        if assets.is_none() {
            self.asset_warning.store(true, Ordering::Release);
        }
        let receiver_failure = state.receiver_failure.clone();
        let initial_reason = if receiver_failure.is_some() {
            Some(Reason::Interrupted)
        } else if state.require_verified_connection && !state.verified_connection {
            // Preserve later rows, but never assert coverage of the interval
            // before a bound connection actually completed its handshake.
            Some(Reason::StatusUnknown)
        } else {
            None
        };
        let recent_loss = (provider == "youtube").then_some(state.recent_loss);
        state.archive = Some(Archive {
            marker,
            writer,
            directory: directory.to_owned(),
            bytes: 0,
            error: receiver_failure.clone(),
            failure_reason: initial_reason,
            assets,
            recent_loss,
            coverage_warning: None,
        });
        // The receiver starts when the public live identity is bound, before
        // recording. Retain a bounded overlap rather than dropping the initial
        // response while the video writer is being attached. Original server
        // timestamps remain intact; replay filters rows before the media anchor.
        if provider == "youtube" {
            let rows: Vec<_> = state.recent.iter().cloned().collect();
            if let Some(archive) = state.archive.as_mut() {
                for row in rows {
                    if !row.raw {
                        if let Ok(event) = serde_json::from_slice::<NormalizedChatEvent>(&row.bytes) {
                            self.assets.observe(&event.parts, archive.assets.as_ref());
                        }
                    }
                    archive.write(&row.bytes, row.raw);
                }
            }
        }
        state.archive_error = state.archive.as_ref().and_then(|archive| archive.error.clone()).or(receiver_failure);
        if let Some(error) = state.archive_error.clone() {
            self.preserve_source.store(true, Ordering::Release);
            state.status = error;
        }
        Ok(())
    }
    /// Resolve only the exact YouTube archive using the same approximate origin
    /// written to chat-timeline.json. This does not alter timestamps or stop the
    /// current sink: a lost prefix must not discard subsequent received rows.
    pub(super) fn resolve_youtube_archive_origin(&self, job_id: &str, origin: Option<u64>) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        let archive = state.archive.as_mut().ok_or(chat_status::ERROR)?;
        chat_status::identity(&archive.marker, "youtube", job_id)?;
        archive.resolve_recent_loss(origin);
        if let Some(warning) = archive.coverage_warning {
            self.preserve_source.store(true, Ordering::Release);
            // Keep any known receiver/write failure ahead of this warning.
            let warning = state.archive_error.get_or_insert_with(|| warning.to_owned()).clone();
            state.status = warning;
        }
        Ok(())
    }
    #[cfg(test)]
    fn attach_archive(&self, directory: &Path) -> Result<(), String> {
        self.attach_recording_archive(directory, "youtube", &uuid::Uuid::new_v4().to_string())
    }
    pub(super) fn detach_archive(&self) -> Result<Option<ChatDetachOutcome>, String> {
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        if let Some(mut archive) = state.archive.take() {
            // A failed/no-progress recording may never publish a media origin.
            // Do not silently certify an unresolved lost prefix in that case.
            archive.resolve_recent_loss(None);
            let assets_incomplete = archive.assets.as_ref().is_none_or(|a| a.close());
            if assets_incomplete {
                self.asset_warning.store(true, Ordering::Release);
                self.preserve_source.store(true, Ordering::Release);
            }
            // Always close both streams, even after an earlier receiver/write
            // failure. No open sink crosses the next recording attachment.
            let parts = archive.writer.close();
            let reason = archive.failure_reason.clone().or_else(|| parts.reason());
            if let Some(error) = archive.error.or_else(|| archive.coverage_warning.map(str::to_owned)).or_else(|| reason.as_ref().map(archive_error)) {
                self.preserve_source.store(true, Ordering::Release);
                state.archive_error = Some(error.clone());
                state.status = error.clone();
            }
            return Ok(Some(ChatDetachOutcome {
                directory: archive.directory,
                marker: archive.marker,
                reason,
                assets_incomplete,
                parts,
            }));
        }
        Ok(None)
    }
    /// Exact raw job only. Its pre-attachment coverage remains unresolved until
    /// the candidate proves the common A/V origin; all sinks close right now.
    pub(super) fn detach_youtube_raw_archive(&self, job_id: &str) -> Result<Option<RawChatDetachOutcome>, String> {
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        let Some(current) = state.archive.as_ref() else { return Ok(None); };
        chat_status::identity(&current.marker, "youtube", job_id)?;
        let archive = state.archive.take().ok_or(chat_status::ERROR)?;
        let assets_incomplete = archive.assets.as_ref().is_none_or(|assets| assets.close());
        if assets_incomplete {
            self.asset_warning.store(true, Ordering::Release);
            self.preserve_source.store(true, Ordering::Release);
        }
        let parts = archive.writer.close();
        let reason = archive.failure_reason.or_else(|| parts.reason());
        if let Some(error) = archive.error.or_else(|| archive.coverage_warning.map(str::to_owned))
            .or_else(|| reason.as_ref().map(archive_error)) {
            self.preserve_source.store(true, Ordering::Release);
            state.archive_error = Some(error.clone());
            state.status = error;
        }
        Ok(Some(RawChatDetachOutcome {
            closed: ChatDetachOutcome { directory: archive.directory, marker: archive.marker,
                reason, assets_incomplete, parts },
            recent_loss: archive.recent_loss,
        }))
    }
    pub(super) fn snapshot(&self, after: u64, limit: usize) -> Result<LiveChatPage, String> {
        let state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        let mut bytes = 0usize;
        let mut items = Vec::new();
        for (row, size) in state
            .ring
            .iter()
            .filter(|(row, _)| row.sequence > after)
            .take(limit.clamp(1, 120))
        {
            if bytes + size > 256 * 1024 {
                break;
            }
            bytes += size;
            items.push(row.clone());
        }
        let cursor = items.last().map_or(after, |row| row.sequence);
        let gap = after != 0
            && state
                .ring
                .front()
                .is_some_and(|(row, _)| row.sequence > after.saturating_add(1));
        let status = self.observed_status(&state);
        Ok(LiveChatPage {
            items,
            cursor,
            status: if self.asset_warning.load(Ordering::Acquire) || self.assets.incomplete() {
                format!("{status} · 일부 이모티콘은 문자로 표시")
            } else {
                status
            },
            ended: self.is_done() && self.started.load(Ordering::Acquire),
            gap,
        })
    }
    fn ingest(&self, provider: &str, line: &[u8]) -> Result<(), String> {
        if line.len() > LINE_LIMIT {
            return Err("채팅 행이 한도를 넘었습니다.".into());
        }
        let parsed = normalize_public_line(provider, line);
        let raw_coverage = if provider == "youtube" {
            match &parsed {
                Ok(events) if events.is_empty() && youtube_empty_poll(line) => RecentLoss::default(),
                Ok(events) if !events.is_empty() => {
                    let mut coverage = RecentLoss::default();
                    for event in events { coverage.merge(RecentLoss::event(event)); }
                    coverage
                }
                _ => RecentLoss::unknown(SystemTime::now().duration_since(UNIX_EPOCH).ok()
                    .and_then(|time| u64::try_from(time.as_millis()).ok()).filter(|value| valid_wall(*value))),
            }
        } else { RecentLoss::default() };
        // Attach/detach cannot split the raw source and its normalized rows into
        // different recordings. Unknown public fields stay in this private file.
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        if provider == "youtube" && state.verify_from_first_event {
            remember(&mut state, line.to_vec(), true, raw_coverage);
        }
        if let Some(archive) = state.archive.as_mut() {
            archive.write(line, true);
            if let Some(error) = archive.error.clone() {
                self.preserve_source.store(true, Ordering::Release);
                state.archive_error = Some(error);
            }
        }
        match parsed {
            Ok(events) => {
                for event in events {
                    self.push_locked(&mut state, event)?;
                }
            }
            Err(error) => {
                self.preserve_source.store(true, Ordering::Release);
                state.status = error.clone();
                return Err(error);
            }
        }
        if let Some(error) = state.archive_error.clone() {
            state.status = error;
        }
        Ok(())
    }
    fn push(&self, event: NormalizedChatEvent) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "채팅 상태 오류")?;
        self.push_locked(&mut state, event)
    }
    fn push_locked(&self, state: &mut State, event: NormalizedChatEvent) -> Result<(), String> {
        if state.require_verified_connection {
            if state.verify_from_first_event && !matches!(event.kind.as_str(), "gap" | "ended") {
                state.verified_connection = true;
            }
            match event.kind.as_str() {
                "connected" => state.verified_connection = true,
                "gap" => {
                    state.verified_connection = false;
                    if let Some(archive) = state.archive.as_mut() {
                        archive.failure_reason.get_or_insert(Reason::Interrupted);
                    }
                }
                "ended" => state.verified_connection = false,
                _ => (),
            }
        }
        let bytes = serde_json::to_vec(&event).map_err(|_| "채팅을 변환하지 못했습니다.")?;
        if bytes.len() > 64 * 1024 {
            return Err("채팅 메시지가 표시 한도를 넘었습니다.".into());
        }
        if let Some(id) = &event.message_id {
            if !state.ids.insert(id.clone()) {
                return Ok(());
            }
            state.order.push_back(id.clone());
            while state.order.len() > 2048 {
                if let Some(old) = state.order.pop_front() {
                    state.ids.remove(&old);
                }
            }
        }
        self.assets.observe(
            &event.parts,
            state.archive.as_ref().and_then(|a| a.assets.as_ref()),
        );
        if state.verify_from_first_event { remember(state, bytes.clone(), false, RecentLoss::event(&event)); }
        if let Some(archive) = state.archive.as_mut() {
            archive.write(&bytes, false);
            if let Some(error) = archive.error.clone() {
                self.preserve_source.store(true, Ordering::Release);
                state.archive_error = Some(error);
            }
        }
        state.status = state.archive_error.clone().unwrap_or_else(|| {
            if event.kind == "gap" {
                "채팅 재연결 중 · 수신 공백 있음"
            } else {
                "채팅 연결됨"
            }
            .into()
        });
        let row = LiveChatRow {
            sequence: state.next,
            kind: event.kind,
            author: event.author,
            text: event.text,
            parts: event.parts,
            amount: event.amount,
            badges: event.badges,
            color: event.color,
        };
        state.next = state.next.saturating_add(1);
        state.ring_bytes += bytes.len();
        state.ring.push_back((row, bytes.len()));
        while state.ring.len() > RING_ITEMS || state.ring_bytes > RING_BYTES {
            if let Some((_, size)) = state.ring.pop_front() {
                state.ring_bytes = state.ring_bytes.saturating_sub(size);
            }
        }
        Ok(())
    }
    pub(super) fn asset(&self, id: &str) -> Result<ChatAssetRead, String> {
        self.assets.reply(id)
    }
    fn soop(&self, channel: &str, expected_broadcast: Option<&str>) -> Result<(), String> {
        crate::soop::chat::capture_bound(
            channel,
            expected_broadcast,
            |event| {
                let bytes =
                    serde_json::to_vec(&event).map_err(|_| "채팅 메시지를 읽지 못했습니다.")?;
                self.ingest("soop", &bytes)
            },
            self.stop.clone(),
        )
        .map(|_| ())
    }
    fn youtube(
        self: &Arc<Self>,
        service: &PlatformMediaService,
        descriptor: Result<Option<youtube_chat::Descriptor>, String>,
        directory: &Path,
    ) -> Result<(), String> {
        let descriptor = descriptor?
            .ok_or("이 방송의 공개 채팅을 확인할 수 없습니다. 영상 수신은 계속됩니다.")?;
        let result = youtube_chat::receive(
            service,
            &descriptor,
            directory,
            &self.stop,
            |line| {
                // Latch callback loss before the producer finishes draining, so a
                // recording stopped concurrently cannot be marked complete.
                self.run_tail(|| self.ingest("youtube", line))
            },
            |rows| {
                // The pinned downloader sleeps for the server poll timeout
                // before flushing stdout. Its committed response proves the
                // handshake now, even when actions is an empty array.
                self.accept_youtube_bootstrap(rows)
            },
            |error| self.fail_receiver(error.to_owned()),
        );
        match result {
            Ok(()) => Ok(()),
            Err(error) if error == "cancelled" && self.stop.load(Ordering::Acquire) => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub(super) fn cleanup_source(&self, directory: &Path) {
        if !self.is_done() || self.preserve_source.load(Ordering::Acquire) {
            return;
        }
        let Ok(state) = self.state.lock() else {
            return;
        };
        if state.archive.is_some() {
            return;
        }
        drop(state);
        let path = directory.join("chat-source");
        if storage::safe_path(&path).is_err() {
            return;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            return;
        };
        for entry in entries.take(4096).filter_map(Result::ok) {
            let name = entry.file_name();
            if !owned_source_name(&name.to_string_lossy()) {
                continue;
            }
            let _ = remove_source_file(&entry.path());
        }
        // Empty leaf only. Unknown files, symlinks and archives are never removed.
        let _ = fs::remove_dir(&path);
    }
}

fn remember(state: &mut State, bytes: Vec<u8>, raw: bool, coverage: RecentLoss) {
    state.recent_bytes += bytes.len();
    state.recent.push_back(RecentRow { bytes, raw, coverage });
    while state.recent.len() > RING_ITEMS * 2 || state.recent_bytes > RING_BYTES {
        if let Some(row) = state.recent.pop_front() {
            state.recent_bytes -= row.bytes.len();
            state.recent_loss.merge(row.coverage);
        }
    }
}

impl Archive {
    fn resolve_recent_loss(&mut self, origin: Option<u64>) {
        let reason = self.recent_loss.take().and_then(|loss| loss.reason(origin));
        if let Some(reason) = reason {
            if self.failure_reason.is_none() {
                self.coverage_warning = Some(if reason == Reason::ArchiveLimit {
                    "녹화 시작 부근의 일부 채팅이 보관 한도를 넘어 누락됐습니다. 이후 채팅은 계속 저장됩니다."
                } else {
                    "녹화 시작 부근의 채팅 보관 범위를 확인하지 못했습니다. 이후 채팅은 계속 저장됩니다."
                });
                self.failure_reason = Some(reason);
            }
        }
    }
    fn write(&mut self, bytes: &[u8], raw: bool) {
        if self.error.is_some() {
            return;
        }
        let next = self.bytes.saturating_add(bytes.len() as u64 + 1);
        let result = if next > chat_parts::MAX_TOTAL_BYTES {
            Err(Reason::ArchiveLimit)
        } else {
            self.writer
                .append(if raw { Stream::Source } else { Stream::Events }, bytes)
        };
        match result {
            Ok(()) => self.bytes = next,
            Err(reason) => {
                self.error = Some(archive_error(&reason));
                self.failure_reason.get_or_insert(reason);
            }
        }
    }
}
fn archive_error(reason: &Reason) -> String {
    match reason {
        Reason::ArchiveLimit => "채팅 저장 한도에 도달했습니다. 이후 채팅은 저장되지 않습니다.",
        Reason::FlushFailed => "채팅 기록을 마무리하지 못했습니다. 기존 기록은 보존했습니다.",
        Reason::Interrupted => "채팅 수신 작업이 중단됐습니다. 기존 기록은 보존했습니다.",
        Reason::WriteFailed | Reason::StatusUnknown => {
            "채팅을 저장하지 못했습니다. 기존 기록은 보존했습니다."
        }
    }
    .into()
}
fn owned_source_name(name: &str) -> bool {
    matches!(
        name,
        "chat.live_chat.json"
            | "chat.live_chat.json.part"
            | "chat.live_chat.json.ytdl"
            | "chat.live_chat.json.part.ytdl"
    ) || youtube_chat::owned_source_name(name)
        || name
            .strip_prefix("chat.live_chat.json.part-Frag")
            .is_some_and(|n| {
                !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit())
            })
}
fn remove_source_file(path: &Path) -> Result<(), String> {
    let (_read, identity, _parents) = open_tail(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
                BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO,
            },
        };
        let file = OpenOptions::new()
            .access_mode(0x10080)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|_| "임시 채팅 기록을 정리하지 못했습니다.")?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| "임시 채팅 기록을 확인하지 못했습니다.")?;
        }
        let actual = (
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        );
        if actual != identity
            || info.nNumberOfLinks != 1
            || info.dwFileAttributes & (0x400 | 0x10) != 0
        {
            return Err("임시 채팅 기록이 변경됐습니다.".into());
        }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                &disposition as *const _ as *const _,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
            .map_err(|_| "임시 채팅 기록을 정리하지 못했습니다.".to_string())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = identity;
        fs::remove_file(path).map_err(|_| "임시 채팅 기록을 정리하지 못했습니다.".into())
    }
}
fn open_tail(path: &Path) -> Result<(File, (u64, u64), Vec<File>), String> {
    storage::safe_path(path)?;
    let mut ancestors = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let parts: Vec<_> = path.ancestors().skip(1).collect();
        for part in parts.into_iter().rev() {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(part)
                .map_err(|_| "채팅 원본 폴더를 잠그지 못했습니다.")?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                    .map_err(|_| "채팅 원본 폴더를 확인하지 못했습니다.")?;
            }
            if info.dwFileAttributes & 0x400 != 0 || info.dwFileAttributes & 0x10 == 0 {
                return Err("연결된 채팅 폴더는 열 수 없습니다.".into());
            }
            ancestors.push(file);
        }
        let file = OpenOptions::new()
            .read(true)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|_| "채팅 임시 기록을 읽지 못했습니다.")?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| "채팅 원본을 확인하지 못했습니다.")?;
        }
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & (0x400 | 0x10) != 0 {
            return Err("연결된 채팅 파일은 열 수 없습니다.".into());
        }
        Ok((
            file,
            (
                info.dwVolumeSerialNumber as u64,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            ),
            ancestors,
        ))
    }
    #[cfg(not(windows))]
    {
        let meta = storage::regular(path)?;
        let file = File::open(path).map_err(|_| "채팅 임시 기록을 읽지 못했습니다.")?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                return Err("연결된 채팅 파일은 열 수 없습니다.".into());
            }
            (meta.dev(), meta.ino())
        };
        #[cfg(not(unix))]
        let identity = (meta.len(), 0);
        Ok((file, identity, ancestors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    const COVERAGE_BASE: u64 = 1_700_000_000_000;
    fn coverage_chat() -> Arc<SharedChat> {
        let chat = SharedChat::new();
        {
            let mut state = chat.state.lock().unwrap();
            state.require_verified_connection = true;
            state.verify_from_first_event = true;
            state.verified_connection = true;
        }
        chat
    }
    fn coverage_event(id: usize, at: u64) -> NormalizedChatEvent {
        let mut row = event(id);
        row.source_timestamp_ms = Some(at);
        row.received_at_ms = at + 20;
        row
    }
    #[test]
    fn raw_detach_defers_old_loss_and_cannot_touch_successor() {
        let chat = coverage_chat();
        chat.state.lock().unwrap().recent_loss.latest_wall_ms = Some(COVERAGE_BASE);
        let first = tempfile::tempdir().unwrap(); let old_id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(first.path(), "youtube", &old_id).unwrap();
        let detached = chat.detach_youtube_raw_archive(&old_id).unwrap().unwrap();
        assert!(chat.state.lock().unwrap().archive.is_none());
        let next = tempfile::tempdir().unwrap(); let next_id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(next.path(), "youtube", &next_id).unwrap();
        chat.push(coverage_event(10, COVERAGE_BASE + 100)).unwrap();
        assert!(chat.detach_youtube_raw_archive(&old_id).is_err());
        let proof = detached.resolve_origin(Some(COVERAGE_BASE + 1)).persist(first.path(), "youtube", &old_id).unwrap();
        assert_eq!(proof.state, chat_status::TextState::Complete);
        assert_eq!(chat.state.lock().unwrap().archive.as_ref().unwrap().marker.job_id, next_id);
        let proof = chat.detach_youtube_raw_archive(&next_id).unwrap().unwrap()
            .resolve_origin(Some(COVERAGE_BASE)).persist(next.path(), "youtube", &next_id).unwrap();
        assert_eq!(proof.reasons, vec![Reason::ArchiveLimit]);
    }
    #[test]
    fn raw_detach_preserves_known_unknown_and_missing_timing_failures() {
        for case in 0..4 {
            let chat = coverage_chat();
            chat.state.lock().unwrap().recent_loss = match case {
                0 => RecentLoss::event(&coverage_event(1, COVERAGE_BASE)),
                1 => RecentLoss::unknown(Some(COVERAGE_BASE)),
                2 => RecentLoss::unknown(None),
                _ => RecentLoss::default(),
            };
            let root = tempfile::tempdir().unwrap(); let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
            let detached = chat.detach_youtube_raw_archive(&id).unwrap().unwrap();
            let proof = detached.resolve_origin((case != 3).then_some(COVERAGE_BASE))
                .persist(root.path(), "youtube", &id).unwrap();
            assert_eq!(proof.state, chat_status::TextState::Partial);
            assert_eq!(proof.reasons, vec![if case == 0 { Reason::ArchiveLimit } else { Reason::StatusUnknown }]);
        }
    }
    #[test]
    fn raw_detach_never_upgrades_earlier_failure() {
        for reason in [Reason::StatusUnknown, Reason::Interrupted, Reason::WriteFailed, Reason::FlushFailed, Reason::ArchiveLimit] {
            let chat = coverage_chat();
            let root = tempfile::tempdir().unwrap(); let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
            chat.state.lock().unwrap().archive.as_mut().unwrap().failure_reason = Some(reason.clone());
            let proof = chat.detach_youtube_raw_archive(&id).unwrap().unwrap()
                .resolve_origin(Some(COVERAGE_BASE)).persist(root.path(), "youtube", &id).unwrap();
            assert_eq!(proof.reasons, vec![reason]);
        }
    }
    #[test]
    fn raw_detach_rejects_other_provider_before_closing_sink() {
        let chat = coverage_chat(); let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(root.path(), "soop", &id).unwrap();
        assert!(chat.detach_youtube_raw_archive(&id).is_err());
        assert!(chat.state.lock().unwrap().archive.is_some());
        assert!(chat.detach_archive().unwrap().is_some());
    }
    #[test]
    fn raw_persist_without_timeline_is_partial_even_for_verified_quiet_chat() {
        let chat = coverage_chat(); let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
        let proof = chat.detach_youtube_raw_archive(&id).unwrap().unwrap()
            .persist_cancellable(root.path(), "youtube", &id, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(proof.reasons, vec![Reason::StatusUnknown]);
        assert_eq!(proof.state, chat_status::TextState::Partial);
    }
    #[test]
    fn youtube_recent_row_eviction_overlapping_origin_is_partial_but_keeps_writing() {
        let chat = coverage_chat();
        for id in 0..=RING_ITEMS * 2 { chat.push(coverage_event(id, COVERAGE_BASE + id as u64)).unwrap(); }
        assert_eq!(chat.state.lock().unwrap().recent.len(), RING_ITEMS * 2);
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
        chat.resolve_youtube_archive_origin(&id, Some(COVERAGE_BASE)).unwrap();
        chat.push(coverage_event(2000, COVERAGE_BASE + 2000)).unwrap();
        assert!(chat.state.lock().unwrap().archive.as_ref().unwrap().error.is_none());
        let proof = chat.detach_archive().unwrap().unwrap().persist(root.path(), "youtube", &id).unwrap();
        assert_eq!(proof.state, chat_status::TextState::Partial);
        assert_eq!(proof.reasons, vec![Reason::ArchiveLimit]);
        assert!(proof.originals_retained);
        assert_eq!(fs::read_to_string(root.path().join("chat.events.jsonl")).unwrap().lines().count(), RING_ITEMS * 2 + 1);
    }
    #[test]
    fn youtube_recent_byte_eviction_is_bounded_and_unknown_coverage_is_not_complete() {
        for known in [false, true] {
            let chat = coverage_chat();
            let coverage = if known { RecentLoss::event(&coverage_event(1, COVERAGE_BASE)) }
                else { RecentLoss::unknown(Some(COVERAGE_BASE)) };
            {
                let mut state = chat.state.lock().unwrap();
                for _ in 0..3 { remember(&mut state, vec![b' '; 400_000], true, coverage); }
                assert!(state.recent_bytes <= RING_BYTES);
                assert_eq!(state.recent.len(), 2);
            }
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
            chat.resolve_youtube_archive_origin(&id, Some(COVERAGE_BASE)).unwrap();
            let proof = chat.detach_archive().unwrap().unwrap().persist(root.path(), "youtube", &id).unwrap();
            assert_eq!(proof.state, chat_status::TextState::Partial);
            assert_eq!(proof.reasons, vec![if known { Reason::ArchiveLimit } else { Reason::StatusUnknown }]);
        }
    }
    #[test]
    fn youtube_old_known_and_unknown_eviction_does_not_poison_later_archives() {
        let chat = coverage_chat();
        chat.state.lock().unwrap().recent_loss = RecentLoss {
            latest_wall_ms: Some(COVERAGE_BASE), unknown_until_ms: Some(COVERAGE_BASE + 1), unbounded_unknown: false,
        };
        for (origin, partial) in [(COVERAGE_BASE, true), (COVERAGE_BASE + 2, false)] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
            chat.resolve_youtube_archive_origin(&id, Some(origin)).unwrap();
            let proof = chat.detach_archive().unwrap().unwrap().persist(root.path(), "youtube", &id).unwrap();
            assert_eq!(proof.state == chat_status::TextState::Partial, partial);
            assert_eq!(proof.state == chat_status::TextState::Complete, !partial);
        }
    }
    #[test]
    fn youtube_active_archive_eviction_is_not_loss_and_late_job_cannot_resolve_successor() {
        let chat = coverage_chat();
        let first = tempfile::tempdir().unwrap();
        let old_id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(first.path(), "youtube", &old_id).unwrap();
        for id in 0..=RING_ITEMS * 2 { chat.push(coverage_event(id, COVERAGE_BASE + id as u64)).unwrap(); }
        chat.resolve_youtube_archive_origin(&old_id, Some(COVERAGE_BASE)).unwrap();
        let proof = chat.detach_archive().unwrap().unwrap().persist(first.path(), "youtube", &old_id).unwrap();
        assert_eq!(proof.state, chat_status::TextState::Complete);
        let next = tempfile::tempdir().unwrap();
        let next_id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(next.path(), "youtube", &next_id).unwrap();
        assert!(chat.resolve_youtube_archive_origin(&old_id, Some(COVERAGE_BASE + 100_000)).is_err());
        chat.resolve_youtube_archive_origin(&next_id, Some(COVERAGE_BASE)).unwrap();
        let proof = chat.detach_archive().unwrap().unwrap().persist(next.path(), "youtube", &next_id).unwrap();
        assert_eq!(proof.reasons, vec![Reason::ArchiveLimit]);
    }
    #[test]
    fn youtube_unresolved_loss_is_unknown_and_stronger_prior_failures_remain() {
        for prior in [None, Some(Reason::Interrupted), Some(Reason::WriteFailed), Some(Reason::FlushFailed)] {
            let chat = coverage_chat();
            chat.state.lock().unwrap().recent_loss.latest_wall_ms = Some(COVERAGE_BASE);
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
            chat.state.lock().unwrap().archive.as_mut().unwrap().failure_reason = prior.clone();
            let proof = chat.detach_archive().unwrap().unwrap().persist(root.path(), "youtube", &id).unwrap();
            assert_eq!(proof.reasons, vec![prior.unwrap_or(Reason::StatusUnknown)]);
        }
    }
    #[test]
    fn youtube_raw_normalized_duplicates_share_coverage_and_soop_ignores_youtube_loss() {
        let chat = coverage_chat();
        let raw = serde_json::to_vec(&serde_json::json!({"isLive":true,"replayChatItemAction":{"actions":[{"addChatItemAction":{"item":{"liveChatTextMessageRenderer":{"id":"coverage-fixture","timestampUsec":"1700000000000000","authorName":{"simpleText":"Fixture"},"message":{"runs":[{"text":"fixture"}]}}}}}]}})).unwrap();
        chat.ingest("youtube", &raw).unwrap();
        {
            let mut state = chat.state.lock().unwrap();
            assert_eq!(state.recent.len(), 2);
            for row in &state.recent { assert_eq!(row.coverage.latest_wall_ms, Some(COVERAGE_BASE)); }
            let coverage = state.recent[0].coverage;
            for _ in 0..=RING_ITEMS * 2 { remember(&mut state, b"{}".to_vec(), true, coverage); }
        }
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        chat.attach_recording_archive(root.path(), "soop", &id).unwrap();
        assert!(chat.resolve_youtube_archive_origin(&id, Some(COVERAGE_BASE)).is_err());
        let proof = chat.detach_archive().unwrap().unwrap().persist(root.path(), "soop", &id).unwrap();
        assert_eq!(proof.state, chat_status::TextState::Complete);
    }
    #[test]
    fn youtube_recent_loss_uses_replay_timestamp_and_fails_closed_without_clock() {
        let row = coverage_event(1, COVERAGE_BASE);
        assert_eq!(RecentLoss::event(&row).reason(Some(COVERAGE_BASE + 1)), None);
        assert_eq!(RecentLoss::event(&row).reason(Some(COVERAGE_BASE)), Some(Reason::ArchiveLimit));
        assert_eq!(RecentLoss::unknown(None).reason(Some(COVERAGE_BASE)), Some(Reason::StatusUnknown));
        assert_eq!(RecentLoss::event(&row).reason(None), Some(Reason::StatusUnknown));
        assert_eq!(RecentLoss::default().reason(None), None);
        let mut untimed = row;
        untimed.source_timestamp_ms = None;
        assert_eq!(RecentLoss::event(&untimed).latest_wall_ms, Some(COVERAGE_BASE + 20));
    }
    #[test]
    fn youtube_verified_quiet_protocol_evictions_are_complete_but_unknown_renderers_are_not() {
        let polls: [&[u8]; 3] = [
            br#"{"isLive":true,"replayChatItemAction":{"actions":[]}}"#,
            br#"{"actions":[],"isLive":true}"#,
            br#"{"continuationContents":{"liveChatContinuation":{"actions":[],"continuations":[{"timedContinuationData":{"continuation":"fixture","timeoutMs":10000}}]}}}"#,
        ];
        for unknown in [false, true] {
            let chat = coverage_chat();
            chat.accept_youtube_bootstrap(Vec::new()).unwrap();
            let row = br#"{"isLive":true,"replayChatItemAction":{"actions":[{"addChatItemAction":{"item":{"futureChatRenderer":{"text":"fixture"}}}}]}}"#;
            for index in 0..=RING_ITEMS * 2 {
                chat.ingest("youtube", if unknown { row } else { polls[index % polls.len()] }).unwrap();
            }
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(root.path(), "youtube", &id).unwrap();
            chat.resolve_youtube_archive_origin(&id, Some(COVERAGE_BASE)).unwrap();
            let proof = chat.detach_archive().unwrap().unwrap().persist(root.path(), "youtube", &id).unwrap();
            assert_eq!(proof.state == chat_status::TextState::Complete, !unknown);
            if unknown { assert_eq!(proof.reasons, vec![Reason::StatusUnknown]); }
            assert!(chat.snapshot(0, 120).unwrap().items.is_empty());
        }
        assert!(!youtube_empty_poll(br#"{"actions":[],"unknownRenderer":{"text":"fixture"}}"#));
        assert!(!youtube_empty_poll(br#"{"isLive":true,"replayChatItemAction":{"actions":[],"unknownRenderer":{}}}"#));
        assert!(!youtube_empty_poll(br#"{"isLive":{},"actions":[]}"#));
        assert!(!youtube_empty_poll(br#"{}"#));
    }
    #[test]
    fn preparation_is_bounded_and_does_not_confuse_quiet_rooms_or_cancellation() {
        let chat=SharedChat::new();
        {let mut state=chat.state.lock().unwrap();state.require_verified_connection=true;state.preparation_started=Some(Instant::now());}
        assert!(chat.is_preparing());
        chat.accept_youtube_bootstrap(Vec::new()).unwrap();assert!(!chat.is_preparing());
        {let mut state=chat.state.lock().unwrap();state.verified_connection=false;state.preparation_started=Some(Instant::now()-Duration::from_secs(16));}
        assert!(!chat.is_preparing());
        {let mut state=chat.state.lock().unwrap();state.preparation_started=Some(Instant::now());}
        chat.stop(); assert!(!chat.is_preparing());
    }
    #[test]
    fn youtube_observations_share_first_deadline_without_consuming_producer() {
        let chat = SharedChat::new();
        chat.observe_youtube_availability(None).unwrap();
        let first = chat.state.lock().unwrap().preparation_started.unwrap();
        assert!(chat.is_preparing());
        for observation in [Some(false), None, Some(true), None] {
            chat.observe_youtube_availability(observation).unwrap();
            let state = chat.state.lock().unwrap();
            assert_eq!(state.preparation_started, Some(first));
            assert!(state.require_verified_connection && state.verify_from_first_event);
            assert!(state.receiver_failure.is_none() && state.archive_error.is_none());
            drop(state);
            assert_eq!(chat.is_preparing(), observation != Some(false));
            assert!(!chat.producer_started_for_test());
        }
        chat.state.lock().unwrap().preparation_started = Some(Instant::now() - Duration::from_secs(16));
        for observation in [Some(false), None, Some(true)] {
            chat.observe_youtube_availability(observation).unwrap();
            assert!(!chat.is_preparing());
            let expected = if observation == Some(false) { "이 방송은 채팅을 제공하지 않습니다." }
                else { "채팅 연결을 확인하지 못했습니다." };
            assert_eq!(chat.status(), expected);
            assert_eq!(chat.snapshot(0, 10).unwrap().status, expected);
        }
    }
    #[test]
    fn youtube_started_or_failed_producer_ignores_later_observations() {
        for failed in [false, true] {
            let chat = SharedChat::new();
            chat.observe_youtube_availability(Some(true)).unwrap();
            chat.started.store(true, Ordering::Release);
            if failed { chat.fail_receiver("fixture transport failure".into()); }
            else { chat.accept_youtube_bootstrap(Vec::new()).unwrap(); }
            let original = chat.status();
            let first = chat.state.lock().unwrap().preparation_started;
            for value in [None, Some(false), Some(true)] { chat.observe_youtube_availability(value).unwrap(); }
            let state = chat.state.lock().unwrap();
            assert_eq!(state.youtube_availability, Some(true));
            assert_eq!(state.preparation_started, first);
            assert_eq!(state.verified_connection, !failed);
            assert_eq!(state.status, original);
            assert!(chat.producer_started_for_test());
        }
    }
    #[test]
    fn youtube_late_positive_does_not_rewrite_attached_unknown_or_disabled_archive() {
        for observation in [None, Some(false)] {
            let chat = SharedChat::new();
            chat.observe_youtube_availability(observation).unwrap();
            let first = tempfile::tempdir().unwrap(); let first_id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(first.path(), "youtube", &first_id).unwrap();
            chat.observe_youtube_availability(Some(true)).unwrap();
            chat.started.store(true, Ordering::Release);
            chat.accept_youtube_bootstrap(Vec::new()).unwrap();
            let proof = chat.detach_archive().unwrap().unwrap().persist(first.path(), "youtube", &first_id).unwrap();
            assert_eq!(proof.state, chat_status::TextState::Partial);
            assert!(proof.reasons.contains(&Reason::StatusUnknown));
            let next = tempfile::tempdir().unwrap(); let next_id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(next.path(), "youtube", &next_id).unwrap();
            let proof = chat.detach_archive().unwrap().unwrap().persist(next.path(), "youtube", &next_id).unwrap();
            assert_eq!(proof.state, chat_status::TextState::Complete);
        }
    }
    #[test]
    fn youtube_stopped_observation_cannot_create_preparation_or_producer() {
        let chat = SharedChat::new(); chat.stop();
        for value in [None, Some(false), Some(true)] { assert!(chat.observe_youtube_availability(value).is_err()); }
        assert!(!chat.producer_started_for_test());
        assert!(chat.state.lock().unwrap().preparation_started.is_none());
    }
    #[test]
    fn empty_verified_response_precedes_archive_without_inventing_chat() {
        let directory = tempfile::tempdir().unwrap(); let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.state.lock().unwrap().require_verified_connection = true;
        chat.accept_youtube_bootstrap(Vec::new()).unwrap();
        chat.attach_recording_archive(directory.path(), "youtube", &id).unwrap();
        let proof = chat.detach_archive().unwrap().unwrap().persist(directory.path(), "youtube", &id).unwrap();
        assert_eq!(proof.state, chat_status::TextState::Complete);
        assert!(chat.snapshot(0,120).unwrap().items.is_empty());
    }
    #[test]
    fn cancelled_bootstrap_does_not_upgrade_connection_or_old_archive() {
        let directory = tempfile::tempdir().unwrap(); let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new(); chat.state.lock().unwrap().require_verified_connection = true;
        chat.attach_recording_archive(directory.path(),"youtube",&id).unwrap();
        chat.stop(); chat.accept_youtube_bootstrap(Vec::new()).unwrap();
        assert!(!chat.state.lock().unwrap().verified_connection);
        let proof=chat.detach_archive().unwrap().unwrap().persist(directory.path(),"youtube",&id).unwrap();
        assert!(proof.reasons.contains(&Reason::StatusUnknown));
    }
    #[test]
    fn youtube_initial_rows_survive_attach_and_stdout_duplicate() {
        let directory=tempfile::tempdir().unwrap(); let id=uuid::Uuid::new_v4().to_string();
        let chat=SharedChat::new();
        {let mut state=chat.state.lock().unwrap(); state.require_verified_connection=true; state.verify_from_first_event=true;}
        let row=serde_json::to_vec(&serde_json::json!({"isLive":true,"replayChatItemAction":{"actions":[{"addChatItemAction":{"item":{"liveChatTextMessageRenderer":{"id":"fixture-id","timestampUsec":"1700000000000000","authorName":{"simpleText":"Fixture"},"message":{"runs":[{"text":"fixture"}]}}}}}]}})).unwrap();
        chat.accept_youtube_bootstrap(vec![row.clone()]).unwrap();
        chat.attach_recording_archive(directory.path(),"youtube",&id).unwrap();
        chat.ingest("youtube",&row).unwrap();
        let proof=chat.detach_archive().unwrap().unwrap().persist(directory.path(),"youtube",&id).unwrap();
        assert_eq!(proof.state,chat_status::TextState::Complete);
        let events=fs::read_to_string(directory.path().join("chat.events.jsonl")).unwrap();
        assert_eq!(events.lines().count(),1);
        let event:serde_json::Value=serde_json::from_str(events.lines().next().unwrap()).unwrap();
        assert_eq!(event["sourceTimestampMs"],1700000000000u64);
    }
    #[test]
    fn bound_chat_pending_at_attach_stays_partial_even_after_later_connection() {
        for connect_later in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            chat.state.lock().unwrap().require_verified_connection = true;
            chat.attach_recording_archive(root.path(), "soop", &id)
                .unwrap();
            if connect_later {
                let mut connected = event(1);
                connected.kind = "connected".into();
                chat.push(connected).unwrap();
                chat.push(event(2)).unwrap();
            }
            let status = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "soop", &id)
                .unwrap();
            assert_ne!(status.state, chat_status::TextState::Complete);
            assert!(status.reasons.contains(&Reason::StatusUnknown));
            assert!(status.originals_retained);
        }
    }
    #[test]
    fn bound_chat_connected_before_attach_is_complete_unless_a_gap_occurs() {
        for gap in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            chat.state.lock().unwrap().require_verified_connection = true;
            let mut connected = event(1);
            connected.kind = "connected".into();
            chat.push(connected).unwrap();
            chat.attach_recording_archive(root.path(), "soop", &id)
                .unwrap();
            if gap {
                let mut row = event(2);
                row.kind = "gap".into();
                chat.push(row).unwrap();
            }
            let status = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "soop", &id)
                .unwrap();
            assert_eq!(status.state == chat_status::TextState::Complete, !gap);
            if gap {
                assert!(status.reasons.contains(&Reason::Interrupted));
                assert!(status.originals_retained);
            }
        }
    }
    #[test]
    fn official_youtube_requires_actual_public_event_not_status_label_for_complete_chat() {
        for first_event in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            {
                let mut state = chat.state.lock().unwrap();
                state.require_verified_connection = true;
                state.verify_from_first_event = true;
            }
            chat.set_status("채팅 연결됨".into());
            if first_event {
                chat.push(event(1)).unwrap();
            }
            chat.attach_recording_archive(root.path(), "youtube", &id)
                .unwrap();
            let status = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "youtube", &id)
                .unwrap();
            assert_eq!(
                status.state == chat_status::TextState::Complete,
                first_event
            );
        }
    }
    #[test]
    fn receiver_guard_finishes_on_unwind_without_leaving_active_work() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.attach_recording_archive(root.path(), "soop", &id)
            .unwrap();
        chat.done.store(false, Ordering::Release);
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _receiver = ReceiverDone(chat.clone());
            panic!("synthetic receiver failure");
        }));
        assert!(failed.is_err());
        assert!(chat.is_done());
        assert!(chat.stop.load(Ordering::Acquire));
        assert!(chat.preserve_source.load(Ordering::Acquire));
        let summary = chat
            .detach_archive()
            .unwrap()
            .unwrap()
            .persist(root.path(), "soop", &id)
            .unwrap();
        assert_eq!(summary.reasons, vec![Reason::Interrupted]);
        assert!(summary.originals_retained);
    }
    fn event(id: usize) -> NormalizedChatEvent {
        NormalizedChatEvent {
            format: "atsumi-chat-v1".into(),
            received_at_ms: id as u64,
            source_timestamp_ms: None,
            message_id: Some(format!("fixture-{id}")),
            kind: "chat".into(),
            author: "Fixture".into(),
            text: format!("Message {id}"),
            parts: Vec::new(),
            amount: None,
            badges: Vec::new(),
            color: None,
        }
    }
    #[test]
    fn terminal_failure_before_attach_keeps_empty_archive_partial() {
        // Exercise the common returned-error/spawn-failure completion path.
        for error in [
            "synthetic receiver error",
            "채팅 수신을 준비하지 못했습니다.",
        ] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            chat.finish_receiver(Err(error.into()));
            chat.attach_recording_archive(root.path(), "youtube", &id)
                .unwrap();
            chat.set_status("채팅 연결됨".into());
            assert_eq!(chat.status(), error);
            let summary = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "youtube", &id)
                .unwrap();
            assert_eq!(summary.state, chat_status::TextState::Partial);
            assert_eq!(summary.reasons, vec![Reason::Interrupted]);
            assert!(summary.originals_retained);
            assert_eq!(
                chat_status::recovery(root.path(), "youtube", &id).unwrap(),
                summary
            );
        }
    }
    #[test]
    fn tail_panic_marks_archive_partial_before_parent_receiver_drain() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.attach_recording_archive(root.path(), "youtube", &id)
            .unwrap();
        chat.done.store(false, Ordering::Release);
        let tail_chat = chat.clone();
        let tail =
            std::thread::spawn(move || tail_chat.run_tail(|| panic!("synthetic tail failure")));
        assert!(tail.join().unwrap().is_err());
        assert!(!chat.is_done());
        assert!(chat.stop.load(Ordering::Acquire));
        let summary = chat
            .detach_archive()
            .unwrap()
            .unwrap()
            .persist(root.path(), "youtube", &id)
            .unwrap();
        assert_eq!(summary.reasons, vec![Reason::Interrupted]);
        assert!(summary.originals_retained);
    }
    #[test]
    fn terminal_failure_during_archive_survives_detach_and_reattach() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("chat-source");
        fs::create_dir(&source).unwrap();
        let transient = source.join("chat.live_chat.json.part");
        fs::write(&transient, b"synthetic source\n").unwrap();
        let chat = SharedChat::new();
        for n in 0..2 {
            let directory = root.path().join(format!("record-{n}"));
            fs::create_dir(&directory).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            chat.attach_recording_archive(&directory, "youtube", &id)
                .unwrap();
            if n == 0 {
                chat.push(event(1)).unwrap();
                chat.finish_receiver(Err("synthetic source cap".into()));
            }
            let before = fs::read(directory.join("chat.events.jsonl")).unwrap();
            let summary = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(&directory, "youtube", &id)
                .unwrap();
            assert_eq!(summary.reasons, vec![Reason::Interrupted]);
            assert!(summary.originals_retained);
            assert_eq!(
                chat_status::recovery(&directory, "youtube", &id).unwrap(),
                summary
            );
            assert_eq!(
                fs::read(directory.join("chat.events.jsonl")).unwrap(),
                before
            );
        }
        chat.finish_receiver(Ok(()));
        assert_eq!(chat.status(), "synthetic source cap");
        chat.cleanup_source(root.path());
        assert!(transient.exists());
    }
    #[test]
    fn terminal_failure_keeps_stronger_archive_failure_reason() {
        for reason in [
            Reason::ArchiveLimit,
            Reason::WriteFailed,
            Reason::FlushFailed,
        ] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            chat.attach_recording_archive(root.path(), "soop", &id)
                .unwrap();
            {
                let mut state = chat.state.lock().unwrap();
                let archive = state.archive.as_mut().unwrap();
                archive.failure_reason = Some(reason.clone());
                archive.error = Some("synthetic archive error".into());
                state.archive_error = Some("synthetic archive error".into());
            }
            chat.finish_receiver(Err("synthetic receiver error".into()));
            assert_eq!(chat.status(), "synthetic archive error");
            let summary = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "soop", &id)
                .unwrap();
            assert_eq!(summary.reasons, vec![reason]);
            assert!(summary.originals_retained);
        }
    }
    #[test]
    fn intentional_stop_and_successful_receiver_end_keep_complete_proof() {
        for stop in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            chat.attach_recording_archive(root.path(), "soop", &id)
                .unwrap();
            chat.push(event(1)).unwrap();
            if stop {
                chat.stop();
            }
            chat.finish_receiver(Ok(()));
            let summary = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "soop", &id)
                .unwrap();
            assert_eq!(summary.state, chat_status::TextState::Complete);
            assert!(summary.reasons.is_empty());
            assert!(!summary.originals_retained);
            assert!(chat.state.lock().unwrap().receiver_failure.is_none());
        }
    }
    #[test]
    fn ring_is_bounded_and_cursor_indicates_trimmed_messages() {
        let chat = SharedChat::new();
        for i in 0..700 {
            chat.push(event(i)).unwrap();
        }
        let page = chat.snapshot(1, 120).unwrap();
        assert!(page.gap);
        assert_eq!(page.items.len(), 120);
        assert_eq!(chat.state.lock().unwrap().ring.len(), 500);
    }
    #[test]
    fn archive_detach_prevents_late_writes_and_next_recording_does_not_reconnect() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let chat = SharedChat::new();
        chat.attach_archive(&first).unwrap();
        chat.push(event(1)).unwrap();
        chat.detach_archive().unwrap();
        let before = fs::read(first.join("chat.events.jsonl")).unwrap();
        chat.push(event(2)).unwrap();
        assert_eq!(fs::read(first.join("chat.events.jsonl")).unwrap(), before);
        chat.attach_archive(&second).unwrap();
        chat.push(event(3)).unwrap();
        chat.detach_archive().unwrap();
        assert_eq!(
            fs::read_to_string(second.join("chat.events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(chat.snapshot(0, 120).unwrap().items.len(), 3);
        assert!(!chat.started.load(Ordering::Acquire));
    }
    #[test]
    fn duplicate_messages_do_not_enter_ring_or_archive_twice() {
        let chat = SharedChat::new();
        chat.push(event(1)).unwrap();
        chat.push(event(1)).unwrap();
        assert_eq!(chat.snapshot(0, 120).unwrap().items.len(), 1);
    }
    #[test]
    fn unsupported_public_events_keep_raw_archive_without_fabricating_chat() {
        let root = tempfile::tempdir().unwrap();
        let chat = SharedChat::new();
        chat.attach_archive(root.path()).unwrap();
        let raw=br#"{"replayChatItemAction":{"actions":[{"addChatItemAction":{"item":{"unknownPublicRenderer":{"text":"fixture"}}}}]}}"#;
        chat.ingest("youtube", raw).unwrap();
        chat.detach_archive().unwrap();
        assert!(chat.snapshot(0, 120).unwrap().items.is_empty());
        assert_eq!(
            fs::read(root.path().join("chat.source.jsonl")).unwrap(),
            [raw.as_slice(), b"\n"].concat()
        );
        assert!(fs::read(root.path().join("chat.events.jsonl"))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn archive_budget_failure_remains_sticky_while_live_ring_continues() {
        let root = tempfile::tempdir().unwrap();
        let chat = SharedChat::new();
        chat.attach_archive(root.path()).unwrap();
        chat.state.lock().unwrap().archive.as_mut().unwrap().bytes = chat_parts::MAX_TOTAL_BYTES;
        chat.push(event(1)).unwrap();
        let error = chat.status();
        assert!(error.contains("저장 한도"));
        chat.push(event(2)).unwrap();
        chat.set_status("채팅 연결됨".into());
        assert_eq!(chat.status(), error);
        assert_eq!(
            chat.detach_archive().unwrap().unwrap().reason,
            Some(Reason::ArchiveLimit)
        );
        chat.push(event(3)).unwrap();
        assert_eq!(chat.status(), error);
        assert_eq!(chat.snapshot(0, 120).unwrap().items.len(), 3);
        assert!(chat.preserve_source.load(Ordering::Acquire));
    }
    #[test]
    fn safely_detached_partial_persists_but_lock_failure_does_not_detach() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.attach_recording_archive(root.path(), "soop", &id)
            .unwrap();
        chat.state.lock().unwrap().archive.as_mut().unwrap().bytes = chat_parts::MAX_TOTAL_BYTES;
        chat.push(event(1)).unwrap();
        let detached = chat.detach_archive().unwrap().unwrap();
        let summary = detached.persist(root.path(), "soop", &id).unwrap();
        assert!(summary.originals_retained);
        assert_eq!(
            chat_status::recovery(root.path(), "soop", &id).unwrap(),
            summary
        );
        let poisoned = SharedChat::new();
        let other = poisoned.clone();
        let _ = std::thread::spawn(move || {
            let _guard = other.state.lock().unwrap();
            panic!("synthetic poison");
        })
        .join();
        assert!(poisoned.detach_archive().is_err());
    }
    #[test]
    fn failed_warning_publication_leaves_marker_and_chat_unmodified() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.attach_recording_archive(root.path(), "soop", &id)
            .unwrap();
        chat.push(event(1)).unwrap();
        let detached = chat.detach_archive().unwrap().unwrap();
        let before = fs::read(root.path().join("chat.events.jsonl")).unwrap();
        fs::create_dir(root.path().join("chat-status.final.json")).unwrap();
        assert!(detached.persist(root.path(), "soop", &id).is_err());
        assert_eq!(
            fs::read(root.path().join("chat.events.jsonl")).unwrap(),
            before
        );
        assert!(root.path().join("chat-status.json").is_file());
    }
    #[test]
    fn write_and_flush_failures_remain_partial_after_detach() {
        for reason in [Reason::WriteFailed, Reason::FlushFailed] {
            let root = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let chat = SharedChat::new();
            chat.attach_recording_archive(root.path(), "soop", &id)
                .unwrap();
            chat.push(event(1)).unwrap();
            {
                let mut state = chat.state.lock().unwrap();
                let archive = state.archive.as_mut().unwrap();
                archive.error = Some("synthetic I/O failure".into());
                archive.failure_reason = Some(reason.clone());
            }
            chat.push(event(2)).unwrap();
            let summary = chat
                .detach_archive()
                .unwrap()
                .unwrap()
                .persist(root.path(), "soop", &id)
                .unwrap();
            assert_eq!(summary.reasons, vec![reason]);
            assert!(summary.originals_retained);
            assert_eq!(
                fs::read_to_string(root.path().join("chat.events.jsonl"))
                    .unwrap()
                    .lines()
                    .count(),
                1
            );
            assert_eq!(chat.snapshot(0, 120).unwrap().items.len(), 2);
        }
    }
    #[test]
    #[ignore = "read-only existing isolated SOOP receive fixture; no network/account/content output"]
    fn actual_soop_fixture_keeps_partial_status_and_original_bytes() {
        let fixture=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".runtime/platform-media-live-tests/6086d286-5ebe-46d2-8908-a760fcc35bf8/media/SOOP/cecada28-4c0e-44ac-b9b2-0599da1d945e/chat.source.jsonl");
        let original = fs::read(&fixture).unwrap();
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.attach_recording_archive(root.path(), "soop", &id)
            .unwrap();
        // Exercise the exact production cap branch with only a few fixture rows.
        chat.state.lock().unwrap().archive.as_mut().unwrap().bytes =
            chat_parts::MAX_TOTAL_BYTES - 600;
        for line in original.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            chat.ingest("soop", line).unwrap();
        }
        assert_eq!(chat.snapshot(0, 120).unwrap().items.len(), 5);
        let summary = chat
            .detach_archive()
            .unwrap()
            .unwrap()
            .persist(root.path(), "soop", &id)
            .unwrap();
        assert_eq!(summary.reasons, vec![Reason::ArchiveLimit]);
        assert!(summary.originals_retained);
        assert_eq!(fs::read(&fixture).unwrap(), original);
    }
    #[test]
    fn combined_raw_and_normalized_budget_is_shared() {
        let root = tempfile::tempdir().unwrap();
        let chat = SharedChat::new();
        chat.attach_archive(root.path()).unwrap();
        let raw = br#"{"kind":"chat","receivedAtMs":1,"nickname":"Fixture","text":"fixture"}"#;
        chat.ingest("soop", raw).unwrap();
        let state = chat.state.lock().unwrap();
        let archive = state.archive.as_ref().unwrap();
        let total = fs::metadata(root.path().join("chat.events.jsonl"))
            .unwrap()
            .len()
            + fs::metadata(root.path().join("chat.source.jsonl"))
                .unwrap()
                .len();
        assert_eq!(archive.bytes, total);
        assert!(total > raw.len() as u64);
    }
    #[test]
    #[ignore = "isolated multipart integration writes >128MiB synthetic raw chat; no network or user files"]
    fn multipart_recording_beyond_old_cap_reopens_without_losing_chat() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let chat = SharedChat::new();
        chat.attach_recording_archive(root.path(), "soop", &id)
            .unwrap();
        // An unknown public field is intentionally retained in the raw source,
        // not propagated to the live ring or normalized replay DTO.
        let raw = serde_json::to_vec(&serde_json::json!({
            "kind": "chat", "receivedAtMs": 1, "nickname": "Fixture",
            "text": "fixture", "padding": "x".repeat(500_000),
        }))
        .unwrap();
        for _ in 0..270 {
            chat.ingest("soop", &raw).unwrap();
        }
        assert!(chat.state.lock().unwrap().archive.as_ref().unwrap().bytes > OLD_FILE_LIMIT);
        assert!(chat.state.lock().unwrap().archive_error.is_none());
        let summary = chat
            .detach_archive()
            .unwrap()
            .unwrap()
            .persist(root.path(), "soop", &id)
            .unwrap();
        assert_eq!(summary.state, chat_status::TextState::Complete);
        assert!(!summary.originals_retained);
        assert_eq!(
            chat_status::recovery(root.path(), "soop", &id).unwrap(),
            summary
        );
        let page = (0..2048)
            .find_map(|_| {
                let page = super::super::replay_chat::read_page_for_job(
                    root.path(),
                    "soop",
                    &id,
                    0,
                    Some("0"),
                    120,
                )
                .unwrap();
                (page.state != "indexing").then_some(page)
            })
            .expect("bounded multipart replay preparation must finish");
        assert_eq!(page.total, 270);
        assert_eq!(page.items.len(), 120);
        assert!(!page.partial_file);
        assert_eq!(chat.snapshot(0, 120).unwrap().items.len(), 120);
        super::super::replay_chat::release_directory(root.path());
        // Detaching is not a receiver restart or the end of the shared ring.
        chat.push(event(900)).unwrap();
        assert!(!chat.started.load(Ordering::Acquire));
        assert_eq!(chat.state.lock().unwrap().ring.len(), 271);
    }
    #[test]
    fn live_tail_allows_writer_but_rejects_hardlinked_source() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chat.live_chat.json.part");
        let mut writer = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        writer.write_all(b"{}\n").unwrap();
        let (mut file, _, _parents) = open_tail(&path).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "{}\n");
        drop(file);
        drop(writer);
        fs::hard_link(&path, root.path().join("linked")).unwrap();
        assert!(open_tail(&path).is_err());
    }
    #[test]
    fn cleanup_only_removes_exact_owned_transient_files_after_detach() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("chat-source");
        fs::create_dir(&source).unwrap();
        for name in [
            "chat.live_chat.json.part",
            "chat.live_chat.json.part-Frag2",
            "--Frag0",
            "--Frag1.part",
            "--Frag2.part.extra",
            "-Frag3.part",
            "unknown.json",
            "chat.live_chat.json.part-Frag2.extra",
        ] {
            fs::write(source.join(name), b"fixture").unwrap();
        }
        let chat = SharedChat::new();
        chat.attach_archive(root.path()).unwrap();
        chat.cleanup_source(root.path());
        assert!(source.join("chat.live_chat.json.part").exists());
        chat.detach_archive().unwrap();
        chat.cleanup_source(root.path());
        assert!(!source.join("chat.live_chat.json.part").exists());
        assert!(!source.join("chat.live_chat.json.part-Frag2").exists());
        assert!(!source.join("--Frag0").exists());
        assert!(!source.join("--Frag1.part").exists());
        assert!(source.join("--Frag2.part.extra").exists());
        assert!(source.join("-Frag3.part").exists());
        assert!(source.join("unknown.json").exists());
        assert!(source.join("chat.live_chat.json.part-Frag2.extra").exists());
        assert!(root.path().join("chat.events.jsonl").exists());
    }
}
