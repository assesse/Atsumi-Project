//! Immutable, user-selected X DOM snapshots. No authentication or private API access.
//! Call inspection only after native visible/current pane checks. The UI cannot
//! submit URLs or HTML at save time: it selects IDs from a native frozen draft.
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ERROR: &str = "X 보관 데이터를 안전하게 처리하지 못했습니다.";
const CANCELLED: &str = "X 보관 작업이 취소되었거나 계정 상태가 변경되었습니다.";
const MAX_JSON: u64 = 4 * 1024 * 1024;
const MAX_IMAGE: u64 = 32 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_ASSETS: usize = 800;
const SCRIPT: &str = include_str!("x_archive_capture.js");

fn capture_script(diagnostics: bool) -> String {
    if diagnostics {
        SCRIPT.replacen("const DIAGNOSTICS = false;", "const DIAGNOSTICS = true;", 1)
    } else {
        SCRIPT.to_owned()
    }
}
/// Transient debug shape. Never serialized to frozen drafts, IPC or archives.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
struct CaptureDiagnostic {
    photo_route: bool,
    articles: u16,
    dialogs: u16,
    images: u16,
    images_over_limit: bool,
    permitted_images: u16,
    css_visible_images: u16,
    in_viewport_images: u16,
    unique_candidates: u16,
    opacity_zero_images: u16,
    zero_size_images: u16,
}
#[cfg(any(debug_assertions, test))]
impl CaptureDiagnostic {
    fn closed_shape(&self) -> serde_json::Value {
        serde_json::json!({
            "photoRoute":self.photo_route,"articles":self.articles.min(81),
            "dialogs":self.dialogs.min(9),"images":self.images.min(129),
            "imagesOverLimit":self.images_over_limit,
            "permittedImages":self.permitted_images.min(128),
            "cssVisibleImages":self.css_visible_images.min(128),
            "inViewportImages":self.in_viewport_images.min(128),
            "uniqueCandidates":self.unique_candidates.min(2),
            "opacityZeroImages":self.opacity_zero_images.min(128),
            "zeroSizeImages":self.zero_size_images.min(128)
        })
    }
}
/// A single transient quote-avatar shape. No URLs, author identity, text or
/// source HTML can enter this DTO or leave it through a saved document.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
struct QuoteCaptureDiagnostic {
    detail_root: bool,
    quote_found: bool,
    quote_visible: bool,
    author_unambiguous: bool,
    conflicting_targets: bool,
    selected: bool,
    admitted: bool,
    media_full: bool,
    quote_roots: u16,
    nested_roots: u16,
    aliases: u16,
    author_nodes: u16,
    content_nodes: u16,
    links: u16,
    images: u16,
    known: u16,
    known_in_scope: u16,
    profiles: u16,
    not_profile: u16,
    body_or_media: u16,
    nested: u16,
    visible: u16,
    painted: u16,
    hidden: u16,
    aria_hidden: u16,
    ancestor_hidden: u16,
    opacity_zero: u16,
    paint_own_style: u16,
    paint_parent: u16,
    paint_rect: u16,
    paint_layers: u16,
    paint_shape: u16,
    paint_match: u16,
    candidates: u16,
    selected_parent_div: bool,
    selected_ancestor_complete: bool,
    selected_ancestor_limit: bool,
    selected_parent_children: u16,
    selected_ancestors: u16,
    selected_ancestor_aria_hidden: u16,
    selected_ancestor_hidden: u16,
    selected_ancestor_inert: u16,
    selected_ancestor_display_none: u16,
    selected_ancestor_visibility_hidden: u16,
    selected_ancestor_opacity_zero: u16,
}
#[cfg(any(debug_assertions, test))]
impl QuoteCaptureDiagnostic {
    fn closed_shape(&self) -> serde_json::Value {
        // Keep the diagnostic allowlist explicit without expanding a large
        // recursive JSON macro or raising the crate-wide recursion limit.
        let mut shape = serde_json::Map::new();
        for (key, value) in [
            ("detailRoot", self.detail_root),
            ("quoteFound", self.quote_found),
            ("quoteVisible", self.quote_visible),
            ("authorUnambiguous", self.author_unambiguous),
            ("conflictingTargets", self.conflicting_targets),
            ("selected", self.selected),
            ("admitted", self.admitted),
            ("mediaFull", self.media_full),
            ("selectedParentDiv", self.selected_parent_div),
            ("selectedAncestorComplete", self.selected_ancestor_complete),
            ("selectedAncestorLimit", self.selected_ancestor_limit),
        ] {
            shape.insert(key.into(), value.into());
        }
        for (key, value) in [
            ("quoteRoots", self.quote_roots),
            ("nestedRoots", self.nested_roots),
            ("aliases", self.aliases),
            ("authorNodes", self.author_nodes),
            ("contentNodes", self.content_nodes),
            ("links", self.links),
            ("images", self.images),
            ("known", self.known),
            ("knownInScope", self.known_in_scope),
            ("profiles", self.profiles),
            ("notProfile", self.not_profile),
            ("bodyOrMedia", self.body_or_media),
            ("nested", self.nested),
            ("visible", self.visible),
            ("painted", self.painted),
            ("hidden", self.hidden),
            ("ariaHidden", self.aria_hidden),
            ("ancestorHidden", self.ancestor_hidden),
            ("opacityZero", self.opacity_zero),
            ("paintOwnStyle", self.paint_own_style),
            ("paintParent", self.paint_parent),
            ("paintRect", self.paint_rect),
            ("paintLayers", self.paint_layers),
            ("paintShape", self.paint_shape),
            ("paintMatch", self.paint_match),
            ("candidates", self.candidates),
            ("selectedParentChildren", self.selected_parent_children),
            ("selectedAncestors", self.selected_ancestors),
            ("selectedAncestorAriaHidden", self.selected_ancestor_aria_hidden),
            ("selectedAncestorHidden", self.selected_ancestor_hidden),
            ("selectedAncestorInert", self.selected_ancestor_inert),
            ("selectedAncestorDisplayNone", self.selected_ancestor_display_none),
            ("selectedAncestorVisibilityHidden", self.selected_ancestor_visibility_hidden),
            ("selectedAncestorOpacityZero", self.selected_ancestor_opacity_zero),
        ] {
            shape.insert(key.into(), value.min(65).into());
        }
        serde_json::Value::Object(shape)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureMedia {
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub alt: String,
    #[serde(default)]
    pub poster: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureCounts {
    pub replies: String,
    pub reposts: String,
    pub likes: String,
    pub bookmarks: String,
    pub views: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureQuote {
    pub id: String,
    pub url: String,
    pub handle: String,
    pub display_name: String,
    pub verified: Option<bool>,
    pub body: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_urls: Vec<String>,
}
/// A rendered quote card with no proved canonical post permalink. Deliberately
/// has no id/url fields and cannot become a selectable post or loaded reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureQuotePresentation {
    pub handle: String,
    pub display_name: String,
    pub verified: Option<bool>,
    pub body: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_urls: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureRelation { Root, Reply, Unknown }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapturePresentation {
    pub handle: String,
    pub display_name: String,
    pub verified: Option<bool>,
    pub counts: CaptureCounts,
    /// The source's displayed label at capture time, never recalculated later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// None means a legacy capture without occurrence attribution. Some([])
    /// explicitly means no own media, while a quote may still contain media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_urls: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<CaptureQuote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_presentation: Option<CaptureQuotePresentation>,
    /// Present only when every observed own-media node lies on the same side
    /// of the quote card. Absence is not an inferred historical ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_before_quote: Option<bool>,
    pub relation: CaptureRelation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub order: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapturePost {
    pub id: String,
    pub url: String,
    pub author: String,
    pub body: String,
    pub timestamp: String,
    pub counts: String,
    pub context: String,
    pub quotes: Vec<String>,
    pub media: Vec<CaptureMedia>,
    pub truncated: bool,
    /// Only the visible full-page photo was observed, not post text or replies.
    #[serde(default)]
    pub photo_only: bool,
    /// Observed historical presentation, absent in pre-v2 captures. It grants
    /// no social-action capability and never fabricates missing source fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<CapturePresentation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureDocument {
    pub version: u32,
    pub source_url: String,
    pub owner: String,
    pub captured_at: String,
    pub coverage: String,
    pub records: Vec<CapturePost>,
    pub limit_reached: bool,
    #[serde(default, skip_serializing)]
    diagnostic: Option<CaptureDiagnostic>,
    #[serde(default, skip_serializing)]
    quote_diagnostic: Option<QuoteCaptureDiagnostic>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveDraft {
    pub capture_id: String,
    pub account_id: String,
    pub document: CaptureDocument,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SaveIntent {
    Archive,
    Media,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest {
    pub request_id: String,
    pub capture_id: String,
    pub selected_ids: Vec<String>,
    pub intent: SaveIntent,
    pub include_media: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveAsset {
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub sha256: String,
    pub post_id: String,
    pub kind: String,
    pub source_url: String,
    /// Observed source identity remains unchanged; this is only the validated
    /// original-rendition request used for newly saved body images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveIssue {
    pub post_id: String,
    pub kind: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveRecord {
    pub version: u32,
    pub id: String,
    pub capture_id: String,
    pub account_id: String,
    pub saved_at: u64,
    pub intent: SaveIntent,
    pub document: CaptureDocument,
    pub assets: Vec<ArchiveAsset>,
    pub issues: Vec<ArchiveIssue>,
    /// Complete refers only to the explicitly selected frozen DOM, never all replies.
    pub selected_content_complete: bool,
    pub media_requested: bool,
    pub html_sha256: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSummary {
    pub id: String,
    pub account_id: String,
    pub source_url: String,
    pub saved_at: u64,
    pub captured_at: String,
    pub posts: usize,
    pub assets: usize,
    pub issues: usize,
    pub intent: SaveIntent,
    pub selected_content_complete: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveProgressStage {
    Queued,
    Preparing,
    Downloading,
    Writing,
    Complete,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentTransferProgress {
    pub received_bytes: u64,
    /// Content-Length of only the currently read response, if supplied. This
    /// is never the whole job/HLS-tree size or a verified saved-file size.
    pub total_bytes: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveProgress {
    pub request_id: String,
    pub capture_id: String,
    pub account_id: String,
    pub sequence: u64,
    pub stage: ArchiveProgressStage,
    /// Selected pictures/videos processed (including explicitly reported
    /// failures). Avatars, posters and HLS children do not inflate this count.
    pub processed_media: usize,
    pub total_media: usize,
    pub downloaded_bytes: u64,
    /// A whole HLS tree has no trustworthy byte total before it is fetched.
    pub total_bytes: Option<u64>,
    pub current_transfer: Option<CurrentTransferProgress>,
    pub partial: bool,
}
fn logical_media(media: &CaptureMedia) -> bool {
    matches!(media.kind.as_str(), "image" | "video" | "unavailable_image" | "unavailable_video")
}
enum TransferChange { Keep, Begin(Option<u64>), End }
struct TransferGuard<'a>(&'a ProgressReporter);
impl Drop for TransferGuard<'_> {
    fn drop(&mut self) {
        self.0.advance(ArchiveProgressStage::Downloading, None, false, false, true, TransferChange::End);
    }
}
struct ProgressState {
    value: ArchiveProgress,
    last_emitted: Option<Instant>,
    terminal: bool,
}
struct ProgressReporter {
    state: Mutex<ProgressState>,
    callback: Box<dyn Fn(ArchiveProgress) + Send + Sync>,
}
impl ProgressReporter {
    fn new(reserved: &ArchiveJob, callback: impl Fn(ArchiveProgress) + Send + Sync + 'static) -> Self {
        let total_media = if reserved.request.include_media || reserved.request.intent == SaveIntent::Media {
            reserved.draft.document.records.iter().filter(|post| reserved.request.selected_ids.contains(&post.id))
                .map(|post| post.media.iter().filter(|media| logical_media(media)).count()).sum()
        } else { 0 };
        Self {
            state: Mutex::new(ProgressState {
                value: ArchiveProgress {
                    request_id: reserved.request.request_id.clone(), capture_id: reserved.draft.capture_id.clone(),
                    account_id: reserved.draft.account_id.clone(), sequence: 0, stage: ArchiveProgressStage::Preparing,
                    processed_media: 0, total_media, downloaded_bytes: 0, total_bytes: None, current_transfer: None, partial: false,
                },
                last_emitted: None, terminal: false,
            }),
            callback: Box::new(callback),
        }
    }
    fn update(&self, stage: ArchiveProgressStage, bytes: Option<u64>, processed: bool, partial: bool, force: bool) {
        self.advance(stage, bytes, processed, partial, force, TransferChange::Keep);
    }
    fn begin_transfer(&self, total: Option<u64>) -> TransferGuard<'_> {
        self.advance(ArchiveProgressStage::Downloading, None, false, false, true, TransferChange::Begin(total));
        TransferGuard(self)
    }
    fn advance(&self, stage: ArchiveProgressStage, bytes: Option<u64>, processed: bool, partial: bool, force: bool, transfer: TransferChange) {
        let value = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.terminal { return; }
            match transfer {
                TransferChange::Keep => {}
                TransferChange::Begin(total_bytes) => state.value.current_transfer = Some(CurrentTransferProgress { received_bytes: 0, total_bytes }),
                TransferChange::End => {
                    if state.value.current_transfer.take().is_none() { return; }
                }
            }
            state.value.stage = stage;
            if let Some(bytes) = bytes {
                state.value.downloaded_bytes = state.value.downloaded_bytes.saturating_add(bytes);
                if let Some(current) = state.value.current_transfer.as_mut() {
                    current.received_bytes = current.received_bytes.saturating_add(bytes);
                    // An overlong response invalidates its declared length;
                    // retain real received bytes, never clamp them to a header.
                    if current.total_bytes.is_some_and(|total| current.received_bytes > total) { current.total_bytes = None; }
                }
            }
            if stage != ArchiveProgressStage::Downloading { state.value.current_transfer = None; }
            if processed { state.value.processed_media = (state.value.processed_media + 1).min(state.value.total_media); }
            state.value.partial |= partial;
            state.terminal = matches!(stage, ArchiveProgressStage::Complete | ArchiveProgressStage::Failed | ArchiveProgressStage::Cancelled);
            if !force && !state.terminal && state.last_emitted.is_some_and(|at| at.elapsed() < Duration::from_millis(100)) {
                return;
            }
            state.value.sequence = state.value.sequence.saturating_add(1);
            state.last_emitted = Some(Instant::now());
            state.value.clone()
        };
        // Never call a UI/session callback while holding a job/store lock.
        (self.callback)(value);
    }
}

/// Native post-save expands only the exact current detail conversation. Other
/// feed posts, quotes, media-only saves and individually saved replies stay
/// independent. Every returned ID was present in the same frozen DOM read.
pub(crate) fn archive_post_selection(document: &CaptureDocument, post_id: &str) -> Vec<String> {
    let Some(root) = document.records.iter().find(|post| post.id == post_id) else { return vec![]; };
    let mut selected = vec![root.id.clone()];
    let Some(proof) = root.presentation.as_ref().filter(|proof|
        proof.relation == CaptureRelation::Root
        && proof.conversation_id.as_deref() == Some(root.id.as_str())
        && document.source_url == root.url && !root.photo_only) else { return selected; };
    selected.extend(document.records.iter().filter(|post| post.presentation.as_ref().is_some_and(|reply|
        reply.relation == CaptureRelation::Reply
        && reply.conversation_id.as_deref() == Some(root.id.as_str())
        && reply.order > proof.order && post.id != root.id && !post.photo_only)).map(|post| post.id.clone()));
    selected
}
fn include_observed_video_references(post: &mut CapturePost) {
    let Some(view) = post.presentation.as_mut() else { return; };
    let attach = |urls: &mut Vec<String>, media: &CaptureMedia| {
        if media.kind == "video" && media.poster.as_ref().is_some_and(|poster| urls.contains(poster))
            && !urls.contains(&media.url) && urls.len() < 16 {
            urls.push(media.url.clone());
        }
    };
    for media in &post.media {
        if let Some(urls) = &mut view.media_urls { attach(urls, media); }
        if let Some(quote) = &mut view.quote { attach(&mut quote.media_urls, media); }
        if let Some(quote) = &mut view.quote_presentation { attach(&mut quote.media_urls, media); }
    }
}

struct Frozen {
    draft: ArchiveDraft,
    epoch: u64,
    created: Instant,
}
#[derive(Default)]
struct Inner {
    epoch: AtomicU64,
    drafts: Mutex<HashMap<String, Frozen>>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}
#[derive(Clone, Default)]
pub struct XArchiveState {
    inner: Arc<Inner>,
}
/// A native immediate-save capture has no selection window to retain it.
/// Release only its temporary map entry; reserved jobs own an independent
/// immutable snapshot, and dropping this lease never cancels work or files.
pub(crate) struct DraftLease {
    state: XArchiveState,
    capture_id: String,
    account_id: String,
}
impl Drop for DraftLease {
    fn drop(&mut self) {
        let _ = self.state.release_draft(&self.capture_id, &self.account_id);
    }
}
fn registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}
const SAVE_RUNNING_LIMIT: usize = 2;
const SAVE_OUTSTANDING_LIMIT: usize = SAVE_RUNNING_LIMIT + 20;
const QUEUE_FULL: &str = "저장 대기열이 가득 찼습니다. 진행 중인 작업이 끝난 뒤 다시 시도해 주세요.";
#[derive(Default)]
struct QueueState {
    waiting: VecDeque<String>,
    running: HashSet<String>,
}
#[derive(Default)]
struct SaveQueue {
    state: Mutex<QueueState>,
    changed: Condvar,
}
impl SaveQueue {
    fn admit(&self, id: &str, queued: bool) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.running.contains(id) || state.waiting.iter().any(|value| value == id) { return Err(ERROR.into()); }
        if state.running.len() + state.waiting.len() >= SAVE_OUTSTANDING_LIMIT { return Err(QUEUE_FULL.into()); }
        if queued { state.waiting.push_back(id.into()); }
        else {
            // The explicit chooser cannot jump ahead of already admitted work.
            if !state.waiting.is_empty() || state.running.len() >= SAVE_RUNNING_LIMIT {
                return Err("X 보관 작업이 진행 중입니다.".into());
            }
            state.running.insert(id.into());
        }
        self.changed.notify_all();
        Ok(())
    }
    fn wait(&self, id: &str, current: impl Fn() -> bool) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if !current() { return Err(CANCELLED.into()); }
            if state.running.contains(id) { return Ok(()); }
            if !state.waiting.iter().any(|value| value == id) { return Err(CANCELLED.into()); }
            if state.waiting.front().is_some_and(|value| value == id) && state.running.len() < SAVE_RUNNING_LIMIT {
                state.waiting.pop_front();
                state.running.insert(id.into());
                self.changed.notify_all();
                return Ok(());
            }
            state = self.changed.wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(|p| p.into_inner()).0;
        }
    }
    fn running(&self, id: &str) -> bool {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).running.contains(id)
    }
    fn release(&self, id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.waiting.retain(|value| value != id);
        state.running.remove(id);
        self.changed.notify_all();
    }
}
fn save_queue() -> &'static SaveQueue {
    static QUEUE: OnceLock<SaveQueue> = OnceLock::new();
    QUEUE.get_or_init(Default::default)
}
pub fn active_ids() -> Vec<String> {
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .keys()
        .cloned()
        .collect()
}
pub fn cancel_all() {
    for cancel in registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
    {
        cancel.store(true, Ordering::Release);
    }
}
struct Job {
    state: Arc<Inner>,
    id: String,
    cancel: Arc<AtomicBool>,
    epoch: u64,
    progress: Option<Arc<ProgressReporter>>,
}
/// RAII admission ticket. Register it inside AppState::begin_platform_work;
/// move it to the blocking worker. Dropping it releases both active registries.
pub struct ArchiveJob {
    job: Job,
    request: SaveRequest,
    draft: ArchiveDraft,
}
/// Registered before DOM inspection so capture/waiting work participates in
/// application quit/update/cancel. It owns no URL authority until preparation.
pub(crate) struct QueuedArchiveWork {
    job: Job,
}
impl QueuedArchiveWork {
    pub(crate) fn check(&self) -> Result<(), String> { self.job.check() }
    pub(crate) fn cancellation(&self) -> Arc<AtomicBool> { self.job.cancel.clone() }
}
impl ArchiveJob {
    pub(crate) fn wait_for_turn(&self) -> Result<(), String> {
        save_queue().wait(&self.job.id, || self.job.check().is_ok())
    }
    /// Narrow an already frozen, selected post to one observed media resource.
    /// This never accepts a new download URL or modifies the frozen shared draft.
    pub(crate) fn restrict_media(&mut self, post_id: &str, observed: &str) -> Result<(), String> {
        if self.request.intent != SaveIntent::Media
            || self.request.selected_ids != [post_id]
            || observed.is_empty() || observed.len() > 2048 {
            return Err(ERROR.into());
        }
        let post = self.draft.document.records.iter_mut().find(|p| p.id == post_id).ok_or(ERROR)?;
        let matches: Vec<_> = post.media.iter().filter(|m|
            matches!(m.kind.as_str(), "image" | "video" | "unavailable_video")
            && (m.url == observed || m.poster.as_deref() == Some(observed))).cloned().collect();
        if matches.len() != 1 { return Err("선택한 미디어를 정확히 확인하지 못했습니다. 다시 시도해 주세요.".into()); }
        let selected = &matches[0];
        post.media.retain(|m| m.url == selected.url && m.kind == selected.kind
            && m.poster == selected.poster && m.alt == selected.alt
            || m.kind == "poster" && selected.poster.as_deref() == Some(m.url.as_str()));
        if let Some(view) = &mut post.presentation {
            if view.avatar_url.as_ref().is_some_and(|url| !post.media.iter().any(|media| media.kind == "avatar" && &media.url == url)) {
                view.avatar_url = None;
            }
            if let Some(urls) = &mut view.media_urls { urls.retain(|url| post.media.iter().any(|media| &media.url == url)); }
            if let Some(quote) = &mut view.quote {
                if quote.avatar_url.as_ref().is_some_and(|url| !post.media.iter().any(|media| media.kind == "avatar" && &media.url == url)) {
                    quote.avatar_url = None;
                }
                quote.media_urls.retain(|url| post.media.iter().any(|media| &media.url == url));
            }
            if let Some(quote) = &mut view.quote_presentation {
                if quote.avatar_url.as_ref().is_some_and(|url| !post.media.iter().any(|media| media.kind == "avatar" && &media.url == url)) {
                    quote.avatar_url = None;
                }
                quote.media_urls.retain(|url| post.media.iter().any(|media| &media.url == url));
            }
        }
        Ok(())
    }
}
impl Job {
    fn progress(&self, stage: ArchiveProgressStage, bytes: Option<u64>, processed: bool, partial: bool, force: bool) {
        if let Some(progress) = &self.progress { progress.update(stage, bytes, processed, partial, force); }
    }
    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire)
            || self.state.epoch.load(Ordering::Acquire) != self.epoch
        {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.state
            .jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
        registry()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
        save_queue().release(&self.id);
    }
}
impl XArchiveState {
    fn register_work(&self, id: String, epoch: u64, queued: bool) -> Result<Job, String> {
        if !uuid::Uuid::parse_str(&id).is_ok_and(|value| value.to_string() == id) { return Err(ERROR.into()); }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = self.inner.jobs.lock().unwrap_or_else(|p| p.into_inner());
            let mut global = registry().lock().unwrap_or_else(|p| p.into_inner());
            if jobs.contains_key(&id) || global.contains_key(&id) { return Err(ERROR.into()); }
            save_queue().admit(&id, queued)?;
            jobs.insert(id.clone(), cancel.clone());
            global.insert(id.clone(), cancel.clone());
        }
        let job = Job { state: self.inner.clone(), id, cancel, epoch, progress: None };
        job.check()?;
        Ok(job)
    }
    /// Must be called within AppState::begin_platform_work. The bounded global
    /// admission includes capture preparation and never starts network I/O.
    pub(crate) fn reserve_queued(&self, request_id: String) -> Result<QueuedArchiveWork, String> {
        let epoch = self.inner.epoch.load(Ordering::Acquire);
        Ok(QueuedArchiveWork { job: self.register_work(request_id, epoch, true)? })
    }
    fn frozen_request(&self, request: &SaveRequest) -> Result<(ArchiveDraft, u64), String> {
        if !uuid::Uuid::parse_str(&request.request_id).is_ok_and(|id| id.to_string() == request.request_id) { return Err(ERROR.into()); }
        let (mut draft, epoch) = {
            let drafts = self.inner.drafts.lock().unwrap_or_else(|p| p.into_inner());
            let frozen = drafts.get(&request.capture_id).ok_or("보관 선택창이 만료되었습니다. 다시 읽어 주세요.")?;
            if frozen.created.elapsed() >= Duration::from_secs(600) { return Err(CANCELLED.into()); }
            (frozen.draft.clone(), frozen.epoch)
        };
        let selected: HashSet<_> = request.selected_ids.iter().cloned().collect();
        if selected.is_empty() || selected.len() != request.selected_ids.len() || selected.len() > 80
            || selected.iter().any(|id| !draft.document.records.iter().any(|post| &post.id == id)) { return Err(ERROR.into()); }
        // A waiting download retains only explicitly selected content, not
        // every neighboring item from the bounded source capture.
        draft.document.records.retain(|post| selected.contains(&post.id));
        Ok((draft, epoch))
    }
    pub(crate) fn prepare_queued(&self, pending: QueuedArchiveWork, request: SaveRequest) -> Result<ArchiveJob, String> {
        if !Arc::ptr_eq(&self.inner, &pending.job.state) || pending.job.id != request.request_id { return Err(ERROR.into()); }
        pending.check()?;
        let (draft, epoch) = self.frozen_request(&request)?;
        if pending.job.epoch != epoch { return Err(CANCELLED.into()); }
        let job = ArchiveJob { job: pending.job, request, draft };
        job.job.check()?;
        Ok(job)
    }
    pub(crate) fn lease_draft(&self, draft: &ArchiveDraft) -> DraftLease {
        DraftLease {
            state: self.clone(),
            capture_id: draft.capture_id.clone(),
            account_id: draft.account_id.clone(),
        }
    }
    /// The trusted chooser may release a stale response after an account/view
    /// switch. Match the frozen owner, not the current login, and do not change
    /// the epoch or any active job. Unknown/already-released IDs are idempotent.
    pub(crate) fn release_draft(&self, capture_id: &str, account_id: &str) -> Result<bool, String> {
        if !uuid::Uuid::parse_str(capture_id).is_ok_and(|id| id.to_string() == capture_id)
            || account_id.is_empty() || account_id.len() > 80
            || !account_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
            return Err(ERROR.into());
        }
        let mut drafts = self.inner.drafts.lock().unwrap_or_else(|p| p.into_inner());
        if !drafts.get(capture_id).is_some_and(|frozen| frozen.draft.account_id == account_id) {
            return Ok(false);
        }
        drafts.remove(capture_id);
        Ok(true)
    }
    pub fn invalidate_drafts(&self) {
        let mut drafts = self.inner.drafts.lock().unwrap_or_else(|p| p.into_inner());
        self.inner.epoch.fetch_add(1, Ordering::AcqRel);
        drafts.clear();
        self.cancel_all();
    }
    pub fn cancel_all(&self) {
        for c in self
            .inner
            .jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            c.store(true, Ordering::Release);
        }
    }
    pub fn cancel(&self, id: &str) -> bool {
        if let Some(c) = self
            .inner
            .jobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
        {
            c.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
    /// Blocking worker only; caller's native guard must check pane owner/epoch,
    /// visibility and account before and after this read. No page state is changed.
    pub fn inspect_view(
        &self,
        view: &tauri::Webview,
        expected_url: &str,
        account_id: &str,
        expected_owner: &str,
        current: impl Fn() -> bool,
    ) -> Result<ArchiveDraft, String> {
        self.inspect_view_with_observed(
            view,
            expected_url,
            account_id,
            expected_owner,
            current,
            &[],
        )
    }
    pub fn inspect_view_with_observed(
        &self,
        view: &tauri::Webview,
        expected_url: &str,
        account_id: &str,
        expected_owner: &str,
        current: impl Fn() -> bool,
        observed: &[String],
    ) -> Result<ArchiveDraft, String> {
        let epoch = self.inner.epoch.load(Ordering::Acquire);
        if !current() || view.url().map_err(|_| ERROR)?.as_str() != expected_url {
            return Err(CANCELLED.into());
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        view.eval_with_callback(capture_script(cfg!(debug_assertions)), move |result| {
            let _ = tx.send(result);
        })
        .map_err(|_| capture_rejected("inspect_eval"))?;
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| capture_rejected("inspect_timeout"))?;
        if !current()
            || self.inner.epoch.load(Ordering::Acquire) != epoch
            || view.url().map_err(|_| ERROR)?.as_str() != expected_url
        {
            return Err(CANCELLED.into());
        }
        if result.len() as u64 > MAX_JSON {
            return Err(capture_rejected("inspect_json_size"));
        }
        let mut document: CaptureDocument =
            serde_json::from_str(&result).map_err(|_| capture_rejected("inspect_json_shape"))?;
        if document.owner.is_empty() || !document.owner.eq_ignore_ascii_case(expected_owner) {
            return Err(CANCELLED.into());
        }
        if let Some(diagnostic) = document.diagnostic.take() {
            #[cfg(debug_assertions)]
            if document.records.is_empty() {
                super::debug_shape("archive-capture", "empty-capture-shape", &diagnostic.closed_shape());
            }
            #[cfg(not(debug_assertions))]
            let _ = diagnostic;
        }
        if let Some(diagnostic) = document.quote_diagnostic.take() {
            #[cfg(debug_assertions)]
            super::debug_shape("archive-capture", "quote-avatar-shape", &diagnostic.closed_shape());
            #[cfg(not(debug_assertions))]
            let _ = diagnostic;
        }
        document.captured_at =
            chrono::DateTime::<chrono::Utc>::from(SystemTime::now()).to_rfc3339();
        // Native request observations supplement only a matching DOM poster;
        // unrelated resources cannot be associated by tweet IDs or URL guesses.
        for post in &mut document.records {
            for media in &mut post.media {
                if media.kind == "unavailable_video" {
                    if let Some(scope) = media.poster.as_deref().and_then(poster_scope) {
                        if let Some(url) = observed
                            .iter()
                            .take(500)
                            .filter_map(|v| media_url(v, true).ok())
                            .filter(|u| video_scope(u).as_deref() == Some(scope.as_str()))
                            .filter(|u| u.path().ends_with(".m3u8") || u.path().ends_with(".mp4"))
                            .min_by_key(|u| (!u.path().ends_with(".m3u8"), u.path().len()))
                        {
                            media.kind = "video".into();
                            media.url = url.to_string();
                        }
                    }
                }
            }
            include_observed_video_references(post);
        }
        let serialized = serde_json::to_string(&document).map_err(|_| ERROR)?;
        self.freeze(&serialized, expected_url, account_id, epoch)
    }
    fn freeze(
        &self,
        raw: &str,
        expected_url: &str,
        account_id: &str,
        epoch: u64,
    ) -> Result<ArchiveDraft, String> {
        if raw.len() as u64 > MAX_JSON
            || account_id.is_empty()
            || account_id.len() > 80
            || !account_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(ERROR.into());
        }
        let document: CaptureDocument = serde_json::from_str(raw).map_err(|_| ERROR)?;
        validate_document(&document)?;
        let mut expected = Url::parse(expected_url).map_err(|_| ERROR)?;
        expected.set_query(None);
        expected.set_fragment(None);
        if document.source_url != expected.as_str() {
            return Err(CANCELLED.into());
        }
        let draft = ArchiveDraft {
            capture_id: uuid::Uuid::new_v4().to_string(),
            account_id: account_id.into(),
            document,
        };
        let mut drafts = self.inner.drafts.lock().unwrap_or_else(|p| p.into_inner());
        if self.inner.epoch.load(Ordering::Acquire) != epoch {
            return Err(CANCELLED.into());
        }
        drafts.retain(|_, f| f.created.elapsed() < Duration::from_secs(600));
        if drafts.len() >= 8 {
            return Err("보관 준비 작업이 많습니다. 잠시 후 다시 시도해 주세요.".into());
        }
        drafts.insert(
            draft.capture_id.clone(),
            Frozen {
                draft: draft.clone(),
                epoch,
                created: Instant::now(),
            },
        );
        Ok(draft)
    }
    /// Blocking worker. `root` is the application-owned X archive directory,
    /// never a path supplied by the remote page. Global admission is caller-owned.
    pub fn reserve(&self, request: SaveRequest) -> Result<ArchiveJob, String> {
        let (draft, epoch) = self.frozen_request(&request)?;
        let job = self.register_work(request.request_id.clone(), epoch, false)?;
        Ok(ArchiveJob {
            job,
            request,
            draft,
        })
    }
    pub fn save(&self, root: &Path, request: SaveRequest) -> Result<ArchiveRecord, String> {
        let reserved = self.reserve(request)?;
        self.save_reserved(root, reserved)
    }
    pub fn save_reserved(
        &self,
        root: &Path,
        reserved: ArchiveJob,
    ) -> Result<ArchiveRecord, String> {
        if !save_queue().running(&reserved.job.id) { return Err("저장 실행 순서가 아직 준비되지 않았습니다.".into()); }
        let ArchiveJob {
            job,
            request,
            draft,
        } = reserved;
        if !Arc::ptr_eq(&self.inner, &job.state) {
            return Err(ERROR.into());
        }
        job.check()?;
        let selected: HashSet<_> = request.selected_ids.iter().cloned().collect();
        let mut document = draft.document;
        document.records.retain(|p| selected.contains(&p.id));
        if request.intent == SaveIntent::Archive
            && !request.include_media
            && document.records.iter().any(|p| p.photo_only)
        {
            return Err("사진만 표시된 화면은 사진 파일을 함께 보관해야 합니다.".into());
        }
        if request.intent == SaveIntent::Media
            && !document.records.iter().any(|p| {
                p.media.iter().any(|m| {
                    matches!(
                        m.kind.as_str(),
                        "image" | "video" | "unavailable_video" | "unavailable_image"
                    )
                })
            })
        {
            return Err("선택한 항목에 내려받을 미디어가 없습니다.".into());
        }
        let mut record = ArchiveRecord {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            capture_id: draft.capture_id,
            account_id: draft.account_id,
            saved_at: now(),
            intent: request.intent,
            document,
            assets: vec![],
            issues: vec![],
            selected_content_complete: false,
            media_requested: request.include_media || request.intent == SaveIntent::Media,
            html_sha256: String::new(),
        };
        safe_path(root)?;
        fs::create_dir_all(root).map_err(|_| ERROR)?;
        safe_path(root)?;
        let stage = root.join(format!(".partial-{}", record.id));
        fs::create_dir(&stage).map_err(|_| ERROR)?;
        // Failed/cancelled staging is intentionally retained, never a completed item.
        if record.media_requested {
            job.progress(ArchiveProgressStage::Downloading, None, false, false, true);
            let client = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|_| ERROR)?;
            let mut total = 0u64;
            for post in &record.document.records {
                for media in &post.media {
                    job.check()?;
                    let result = store_media(
                        &client,
                        &stage,
                        post,
                        media,
                        &job,
                        &mut total,
                        &mut record.assets,
                        &mut record.issues,
                    );
                    if let Err(reason) = result {
                        job.check()?;
                        append_issue(
                            &mut record.issues,
                            ArchiveIssue {
                                post_id: post.id.clone(),
                                kind: media.kind.clone(),
                                reason,
                            },
                        )?;
                    }
                    job.progress(ArchiveProgressStage::Downloading, None, logical_media(media), !record.issues.is_empty(), true);
                }
            }
        }
        for post in &record.document.records {
            if post.truncated {
                append_issue(
                    &mut record.issues,
                    ArchiveIssue {
                        post_id: post.id.clone(),
                        kind: "text".into(),
                        reason: "capture_text_truncated".into(),
                    },
                )?;
            }
        }
        validate_issue_count(record.issues.len())?;
        record.selected_content_complete = record.issues.is_empty();
        job.check()?;
        job.progress(ArchiveProgressStage::Writing, None, false, !record.selected_content_complete, true);
        record.saved_at = now();
        let html = render_html(&record);
        record.html_sha256 = digest(html.as_bytes());
        write_new(&stage.join("index.html"), html.as_bytes())?;
        let metadata = serde_json::to_vec_pretty(&record).map_err(|_| ERROR)?;
        if metadata.len() as u64 > MAX_JSON {
            return Err(ERROR.into());
        }
        write_new(&stage.join("archive.json"), &metadata)?;
        write_new(&stage.join("manifest.sha256"), digest(&metadata).as_bytes())?;
        // Publication fence serializes invalidation and commit without holding a
        // page/UI lock during network IO. Draft invalidation takes the same lock.
        let _fence = self.inner.drafts.lock().unwrap_or_else(|p| p.into_inner());
        job.check()?;
        safe_path(&stage)?;
        let destination = root.join(&record.id);
        if destination.exists() {
            return Err(ERROR.into());
        }
        publish(&stage, &destination)?;
        // A terminal success sample follows durable publication and the same
        // metadata/HTML validation used when reopening a saved capture.
        read(root, &record.id)
    }

    /// Actual save progress only. Callers keep their own pane/generation fence;
    /// the immutable request/capture/account identity travels with every sample.
    pub fn save_reserved_with_progress(
        &self,
        root: &Path,
        mut reserved: ArchiveJob,
        callback: impl Fn(ArchiveProgress) + Send + Sync + 'static,
    ) -> Result<ArchiveRecord, String> {
        let progress = Arc::new(ProgressReporter::new(&reserved, callback));
        reserved.job.progress = Some(progress.clone());
        if !save_queue().running(&reserved.job.id) {
            progress.update(ArchiveProgressStage::Queued, None, false, false, true);
        }
        let result = (|| {
            reserved.wait_for_turn()?;
            progress.update(ArchiveProgressStage::Preparing, None, false, false, true);
            self.save_reserved(root, reserved)
        })();
        match &result {
            Ok(record) => progress.update(ArchiveProgressStage::Complete, None, false, !record.selected_content_complete, true),
            Err(error) => progress.update(if error == CANCELLED { ArchiveProgressStage::Cancelled } else { ArchiveProgressStage::Failed }, None, false, false, true),
        }
        result
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn append_issue(issues: &mut Vec<ArchiveIssue>, issue: ArchiveIssue) -> Result<(), String> {
    validate_issue_count(issues.len().saturating_add(1))?;
    issues.push(issue);
    Ok(())
}
const MAX_ISSUES: usize = 1280;
fn validate_issue_count(count: usize) -> Result<(), String> {
    if count > MAX_ISSUES {
        return Err("archive_issue_limit".into());
    }
    Ok(())
}
fn hls_branch<T>(
    proof: &mut bool,
    run: impl FnOnce(&mut bool) -> Result<T, String>,
) -> Result<T, String> {
    // A discarded optional child never proves a reachable video. Preserve
    // evidence from earlier successful siblings while evaluating this child.
    let mut child_proof = false;
    let result = run(&mut child_proof)?;
    *proof |= child_proof;
    Ok(result)
}
struct HlsNode {
    name: String,
    has_video: bool,
}
impl HlsNode {
    fn reuse(&self, proof: &mut bool) -> String {
        *proof |= self.has_video;
        self.name.clone()
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 24 && value.bytes().all(|b| b.is_ascii_digit())
}
fn source_url(value: &str) -> Result<Url, String> {
    let u = Url::parse(value).map_err(|_| ERROR)?;
    if u.scheme() != "https"
        || u.host_str() != Some("x.com")
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || u.query().is_some()
        || u.as_str().len() > 2048
    {
        return Err(ERROR.into());
    }
    Ok(u)
}
fn validate_presentation(post: &CapturePost, source: &Url) -> Result<(), String> {
    let Some(view) = &post.presentation else { return Ok(()); };
    let identity = |url: &str, post_id: &str, handle: &str| -> bool {
        source_url(url).ok().is_some_and(|url| {
            let path = url.path().trim_matches('/').split('/').collect::<Vec<_>>();
            path.len() == 3 && path[1] == "status" && path[2] == post_id
                && id(post_id) && !handle.is_empty() && handle.len() <= 15
                && handle.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && path[0].eq_ignore_ascii_case(handle)
        })
    };
    if post.photo_only || !identity(&post.url, &post.id, &view.handle)
        || view.display_name.len() > 480 || view.order >= 80
        || view.timestamp_label.as_ref().is_some_and(|label| label.len() > 320)
        || [&view.counts.replies, &view.counts.reposts, &view.counts.likes,
            &view.counts.bookmarks, &view.counts.views].iter().any(|value| value.len() > 128)
        || view.avatar_url.as_ref().is_some_and(|avatar| !post.media.iter().any(|media| media.kind == "avatar" && &media.url == avatar))
        || view.media_urls.as_ref().is_some_and(|urls| urls.len() > 16
            || urls.iter().collect::<HashSet<_>>().len() != urls.len()
            || urls.iter().any(|url| !post.media.iter().any(|media| matches!(media.kind.as_str(), "image" | "video" | "poster") && &media.url == url)))
    { return Err(capture_rejected("presentation_identity_or_limits")); }
    match view.relation {
        CaptureRelation::Unknown if view.conversation_id.is_none() => {}
        CaptureRelation::Root | CaptureRelation::Reply => {
            let Some(conversation) = &view.conversation_id else { return Err(capture_rejected("presentation_conversation")); };
            let path = source.path().trim_matches('/').split('/').collect::<Vec<_>>();
            if !id(conversation) || path.len() != 3 || path[1] != "status" || path[2] != conversation
                || !identity(source.as_str(), conversation, path[0])
                || (view.relation == CaptureRelation::Root) != (post.url == source.as_str())
                || (view.relation == CaptureRelation::Root) != (post.id == *conversation)
            { return Err(capture_rejected("presentation_conversation")); }
        }
        _ => return Err(capture_rejected("presentation_conversation")),
    }
    if let Some(quote) = &view.quote {
        if !identity(&quote.url, &quote.id, &quote.handle)
            || quote.display_name.len() > 480 || quote.body.len() > 48000 || quote.timestamp.len() > 80
            || quote.timestamp_label.as_ref().is_some_and(|label| label.len() > 320)
            || quote.avatar_url.as_ref().is_some_and(|avatar| !post.media.iter().any(|media| media.kind == "avatar" && &media.url == avatar))
            || quote.media_urls.len() > 16
            || quote.media_urls.iter().collect::<HashSet<_>>().len() != quote.media_urls.len()
            || quote.media_urls.iter().any(|url| !post.media.iter().any(|media| matches!(media.kind.as_str(), "image" | "video" | "poster") && &media.url == url))
        { return Err(capture_rejected("presentation_quote")); }
    }
    if view.quote.is_some() && view.quote_presentation.is_some()
        || view.media_before_quote.is_some() && view.quote.is_none() && view.quote_presentation.is_none()
    { return Err(capture_rejected("presentation_quote_scope")); }
    if let Some(quote) = &view.quote_presentation {
        if quote.handle.len() > 15 || !quote.handle.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || quote.display_name.len() > 480 || quote.body.len() > 48000 || quote.timestamp.len() > 80
            || quote.timestamp_label.as_ref().is_some_and(|label| label.len() > 320)
            || quote.avatar_url.as_ref().is_some_and(|avatar| !post.media.iter().any(|media| media.kind == "avatar" && &media.url == avatar))
            || quote.media_urls.len() > 16
            || quote.media_urls.iter().collect::<HashSet<_>>().len() != quote.media_urls.len()
            || quote.media_urls.iter().any(|url| !post.media.iter().any(|media| matches!(media.kind.as_str(), "image" | "video" | "poster") && &media.url == url))
        { return Err(capture_rejected("presentation_quote_display")); }
    }
    Ok(())
}
fn validate_document(doc: &CaptureDocument) -> Result<(), String> {
    let source = source_url(&doc.source_url).map_err(|_| capture_rejected("document_source"))?;
    if doc.version != 1
        || doc.owner.is_empty()
        || doc.owner.len() > 15
        || !doc
            .owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || doc.coverage != "loaded_dom_only"
        || doc.captured_at.len() > 40
        || chrono::DateTime::parse_from_rfc3339(&doc.captured_at).is_err()
    {
        return Err(capture_rejected("document_header"));
    }
    if doc.records.is_empty() {
        return Err(capture_rejected("empty_records"));
    }
    if doc.records.len() > 80 {
        return Err(capture_rejected("record_limit"));
    }
    let mut seen = HashSet::new();
    let mut orders = HashSet::new();
    for post in &doc.records {
        let u = source_url(&post.url).map_err(|_| capture_rejected("post_source"))?;
        let p = u.path().trim_matches('/').split('/').collect::<Vec<_>>();
        if !id(&post.id)
            || !seen.insert(&post.id)
            || p.len() != 3
            || p[1] != "status"
            || p[2] != post.id
            || p[0].is_empty()
            || p[0].len() > 15
            || !p[0].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || post.author.len() > 4000
            || post.body.len() > 96000
            || post.counts.len() > 8000
            || post.context.len() > 4000
            || post.timestamp.len() > 80
            || post.quotes.len() > 2
            || post.quotes.iter().any(|q| q.len() > 48000)
            || post.media.len() > 16
        {
            return Err(capture_rejected("post_identity_or_limits"));
        }
        validate_presentation(post, &source)?;
        if post.presentation.as_ref().is_some_and(|view| !orders.insert(view.order)) {
            return Err(capture_rejected("presentation_order"));
        }
        if post.photo_only {
            let expected_prefix = format!("{}/photo/", u.path());
            let index = source.path().strip_prefix(&expected_prefix);
            let image = post.media.first();
            let photo_resource = image
                .filter(|m| m.kind == "image" && m.poster.is_none())
                .and_then(|m| media_url(&m.url, false).ok())
                .is_some_and(|u| {
                    u.path().strip_prefix("/media/").is_some_and(|name| {
                        let mut parts = name.split('.');
                        let base = parts.next().unwrap_or_default();
                        !base.is_empty()
                            && base.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                            && parts.next().is_none_or(|extension| !extension.is_empty() && extension.bytes().all(|b| b.is_ascii_alphanumeric()))
                            && parts.next().is_none()
                    })
                });
            if doc.records.len() != 1
                || u.path() != format!("/{}/status/{}", p[0], post.id)
                || !matches!(index, Some("1" | "2" | "3" | "4"))
                || !post.author.is_empty()
                || !post.body.is_empty()
                || !post.timestamp.is_empty()
                || !post.counts.is_empty()
                || !post.context.is_empty()
                || !post.quotes.is_empty()
                || post.truncated
                || post.media.len() != 1
                || !photo_resource
            {
                return Err(capture_rejected("photo_only_scope"));
            }
        }
        for m in &post.media {
            if m.alt.len() > 8000
                || !matches!(
                    m.kind.as_str(),
                    "image"
                        | "poster"
                        | "avatar"
                        | "video"
                        | "unavailable_video"
                        | "unavailable_image"
                )
            {
                return Err(capture_rejected("media_kind_or_limits"));
            }
            if let Some(poster) = &m.poster {
                media_url(poster, false)
                    .map_err(|_| capture_rejected(media_rejection_category(poster, false)))?;
            }
            if matches!(m.kind.as_str(), "unavailable_video" | "unavailable_image") {
                if !m.url.is_empty() {
                    return Err(capture_rejected("unavailable_media_has_url"));
                }
            } else {
                media_url(&m.url, m.kind == "video").map_err(|_| {
                    capture_rejected(media_rejection_category(&m.url, m.kind == "video"))
                })?;
            }
        }
    }
    Ok(())
}
fn capture_rejected(category: &'static str) -> String {
    // Static labels only: never log source URLs, text, identities or query data.
    #[cfg(debug_assertions)]
    super::debug_shape(
        "archive-capture",
        "validation-rejected",
        &serde_json::json!({"category": category}),
    );
    let _ = category;
    ERROR.into()
}
fn image_name_allowed(value: &str) -> bool {
    matches!(
        value,
        "orig" | "4096x4096" | "large" | "medium" | "small" | "thumb" | "900x900"
    ) || value.split_once('x').is_some_and(|(width, height)| {
        [width, height].iter().all(|dimension| {
            !dimension.is_empty()
                && dimension.len() <= 5
                && !dimension.starts_with('0')
                && dimension.bytes().all(|byte| byte.is_ascii_digit())
                && dimension
                    .parse::<u32>()
                    .is_ok_and(|size| (1..=16384).contains(&size))
        })
    })
}
fn media_rejection_category(value: &str, video: bool) -> &'static str {
    let Ok(url) = Url::parse(value) else {
        return "media_url_parse";
    };
    if video {
        return "media_video_url";
    }
    if url.scheme() != "https"
        || url.host_str() != Some("pbs.twimg.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return "media_image_origin";
    }
    let mut keys = HashSet::new();
    for (key, value) in url.query_pairs() {
        if !keys.insert(key.to_string()) {
            return "media_image_duplicate_query";
        }
        match key.as_ref() {
            "format" if !matches!(value.as_ref(), "jpg" | "jpeg" | "png" | "webp" | "gif") => {
                return "media_image_format"
            }
            "name" if !image_name_allowed(&value) => {
                return if value.split_once('x').is_some_and(|(w, h)| {
                    [w, h].iter().all(|v| {
                        !v.is_empty() && v.len() <= 5 && v.bytes().all(|b| b.is_ascii_digit())
                    })
                }) {
                    "media_image_name_dimensions"
                } else {
                    "media_image_name_other"
                };
            }
            "format" | "name" => (),
            _ => return "media_image_query_key",
        }
    }
    "media_image_path_or_shape"
}

/// These are public, observed CDN resources only. No cookies, signed queries,
/// credentials, guessed endpoints, redirects, or configurable destination host.
fn media_url(value: &str, video: bool) -> Result<Url, String> {
    let u = Url::parse(value).map_err(|_| ERROR)?;
    if u.scheme() != "https"
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || u.as_str().len() > 2048
        || u.path().contains('%')
        || u.path().split('/').any(|p| p == ".." || p == ".")
    {
        return Err(ERROR.into());
    }
    if video {
        video_scope(&u).ok_or(ERROR)?;
    } else {
        if u.host_str() != Some("pbs.twimg.com")
            || ![
                "/media/",
                "/profile_images/",
                "/ext_tw_video_thumb/",
                "/amplify_video_thumb/",
                "/tweet_video_thumb/",
            ]
            .iter()
            .any(|p| u.path().starts_with(p))
            || !u
                .path()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
        {
            return Err(ERROR.into());
        }
        let mut keys = HashSet::new();
        for (k, v) in u.query_pairs() {
            if !keys.insert(k.to_string())
                || !match k.as_ref() {
                    "format" => matches!(v.as_ref(), "jpg" | "jpeg" | "png" | "webp" | "gif"),
                    "name" => image_name_allowed(&v),
                    _ => false,
                }
            {
                return Err(ERROR.into());
            }
        }
    }
    Ok(u)
}
/// Save-only exception: request X's original-size rendition of this exact
/// observed media ID/format. This does not promise pre-upload camera bytes.
/// Display URLs, avatars, posters and video resources never use this helper.
fn original_image_url(value: &str) -> Result<Url, String> {
    let invalid = || "original_image_url_unavailable".to_string();
    if !value.starts_with("https://pbs.twimg.com/media/")
        || value.bytes().any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'%' | b'\\'))
    {
        return Err(invalid());
    }
    let mut url = media_url(value, false).map_err(|_| invalid())?;
    if url.as_str() != value { return Err(invalid()); }
    let file = url.path().strip_prefix("/media/").ok_or_else(invalid)?;
    let (media_id, extension) = match file.rsplit_once('.') {
        Some((media_id, extension)) => (media_id, Some(extension)),
        None => (file, None),
    };
    if media_id.is_empty() || media_id.len() > 256
        || !media_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || extension.is_some_and(|format| !matches!(format, "jpg" | "jpeg" | "png" | "webp" | "gif"))
    {
        return Err(invalid());
    }
    let observed_format = url.query_pairs().find(|(key, _)| key == "format").map(|(_, value)| value.into_owned());
    if extension.zip(observed_format.as_deref()).is_some_and(|(extension, format)| extension != format) {
        return Err(invalid());
    }
    let format = observed_format.as_deref().or(extension).ok_or_else(invalid)?.to_string();
    let path = format!("/media/{media_id}");
    url.set_path(&path);
    url.set_query(None);
    url.query_pairs_mut().append_pair("format", &format).append_pair("name", "orig");
    Ok(url)
}
fn original_image_mime(url: &Url) -> Option<&'static str> {
    match url.query_pairs().find(|(key, _)| key == "format")?.1.as_ref() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}
fn video_scope(u: &Url) -> Option<String> {
    if u.host_str() != Some("video.twimg.com")
        || u.query_pairs().any(|(k, v)| {
            !matches!(k.as_ref(), "tag" | "container" | "v" | "variant_version")
                || v.len() > 40
                || !v
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
    {
        return None;
    }
    let parts = u
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    if parts.iter().any(|p| {
        p.is_empty()
            || !p
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    }) || !["m3u8", "mp4", "m4s", "ts", "aac", "vtt"]
        .iter()
        .any(|ext| u.path().ends_with(&format!(".{ext}")))
    {
        return None;
    }
    match parts.as_slice() {
        ["subtitles", kind, value, track, file]
            if matches!(*kind, "ext_tw_video" | "amplify_video")
                && id(value)
                && id(track)
                && file.ends_with(".vtt")
                && u.query().is_none() =>
        {
            Some(format!("{kind}/{value}"))
        }
        _ if u.path().ends_with(".vtt") => None,
        [kind, value, ..] if matches!(*kind, "ext_tw_video" | "amplify_video") && id(value) => {
            Some(format!("{kind}/{value}"))
        }
        ["tweet_video", file] if file.ends_with(".mp4") => {
            Some(format!("tweet_video/{}", file.trim_end_matches(".mp4")))
        }
        _ => None,
    }
}
fn poster_scope(value: &str) -> Option<String> {
    let u = media_url(value, false).ok()?;
    let parts = u
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [kind, value, ..]
            if matches!(*kind, "ext_tw_video_thumb" | "amplify_video_thumb") && id(value) =>
        {
            Some(format!("{}/{}", kind.trim_end_matches("_thumb"), value))
        }
        ["tweet_video_thumb", file] => Some(format!("tweet_video/{}", file.split('.').next()?)),
        _ => None,
    }
}
fn fetch(
    client: &Client,
    url: &Url,
    limit: u64,
    job: &Job,
    total: &mut u64,
) -> Result<Vec<u8>, String> {
    job.check()?;
    let mut response = client
        .get(url.clone())
        .header("Referer", "https://x.com/")
        .header("Accept-Encoding", "identity")
        .send()
        .map_err(|_| "media_request_failed")?;
    if response.status().as_u16() != 200
        || response.headers().contains_key("content-range")
        || response
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
    {
        return Err("media_response_not_complete".into());
    }
    let declared = response.content_length();
    read_transfer(&mut response, declared, limit, job, total)
}
fn read_transfer(
    response: &mut impl Read,
    declared: Option<u64>,
    limit: u64,
    job: &Job,
    total: &mut u64,
) -> Result<Vec<u8>, String> {
    job.check()?;
    if declared.is_some_and(|n| n == 0 || n > limit) {
        return Err("media_size_limit".into());
    }
    // RAII clears only this response's metadata on every return path, including
    // read errors, cancellation and rejected/truncated bytes. Whole-job bytes
    // remain cumulative and whole-job total remains explicitly unknown.
    let _transfer = job.progress.as_ref().map(|reporter| reporter.begin_transfer(declared));
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        job.check()?;
        let count = response
            .read(&mut buffer)
            .map_err(|_| "media_read_failed")?;
        if count == 0 {
            break;
        }
        // Count bytes actually received even when the resource is subsequently
        // rejected by its size/content validation. Do not imply saved bytes.
        job.progress(ArchiveProgressStage::Downloading, Some(count as u64), false, false, false);
        if bytes.len() as u64 + count as u64 > limit || *total + count as u64 > MAX_TOTAL {
            return Err("media_size_limit".into());
        }
        *total += count as u64;
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.is_empty() || declared.is_some_and(|n| n != bytes.len() as u64) {
        return Err("media_truncated".into());
    }
    Ok(bytes)
}
fn image_mime(bytes: &[u8]) -> Result<(&'static str, &'static str), String> {
    let format = image::guess_format(bytes).map_err(|_| "image_invalid")?;
    let result = match format {
        image::ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        image::ImageFormat::Png => ("image/png", "png"),
        image::ImageFormat::WebP => ("image/webp", "webp"),
        image::ImageFormat::Gif => ("image/gif", "gif"),
        _ => return Err("image_format_unsupported".into()),
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| "image_decode_failed")?;
    Ok(result)
}
fn mp4_boxes(bytes: &[u8]) -> Option<(bool, bool, bool, bool)> {
    let (mut at, mut ftyp, mut moov, mut mdat, mut moof) = (0usize, false, false, false, false);
    while at < bytes.len() {
        if bytes.len() - at < 8 {
            return None;
        }
        let mut size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as u64;
        let kind = &bytes[at + 4..at + 8];
        let header = if size == 1 {
            if bytes.len() - at < 16 {
                return None;
            }
            size = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap());
            16
        } else {
            8
        };
        if size == 0 {
            size = (bytes.len() - at) as u64;
        }
        if size < header || size > (bytes.len() - at) as u64 {
            return None;
        }
        ftyp |= kind == b"ftyp";
        moov |= kind == b"moov";
        mdat |= kind == b"mdat" && size > header;
        moof |= kind == b"moof";
        at += size as usize;
    }
    Some((ftyp, moov, mdat, moof))
}
fn mp4_complete(bytes: &[u8]) -> bool {
    mp4_boxes(bytes).is_some_and(|(f, m, d, _)| f && m && d)
}
fn mp4_video_track(bytes: &[u8], depth: usize) -> bool {
    if depth > 6 {
        return false;
    }
    let mut at = 0usize;
    while bytes.len().saturating_sub(at) >= 8 {
        let mut size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as u64;
        let kind = &bytes[at + 4..at + 8];
        let header = if size == 1 {
            if bytes.len() - at < 16 {
                return false;
            }
            size = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap());
            16
        } else {
            8
        };
        if size == 0 {
            size = (bytes.len() - at) as u64;
        }
        if size < header || size > (bytes.len() - at) as u64 {
            return false;
        }
        let body = &bytes[at + header as usize..at + size as usize];
        if kind == b"hdlr" && body.len() >= 12 && &body[8..12] == b"vide" {
            return true;
        }
        if matches!(kind, b"moov" | b"trak" | b"mdia") && mp4_video_track(body, depth + 1) {
            return true;
        }
        at += size as usize;
    }
    false
}
fn segment_video_track(bytes: &[u8], ext: &str) -> bool {
    if matches!(ext, "mp4" | "m4s") {
        return mp4_video_track(bytes, 0);
    }
    ext == "ts"
        && bytes.chunks_exact(188).any(|p| {
            if p[0] != 0x47 || p[1] & 0x40 == 0 {
                return false;
            }
            let control = (p[3] >> 4) & 3;
            let offset = match control {
                1 => 4,
                3 => 5 + p[4] as usize,
                _ => return false,
            };
            offset + 4 <= p.len()
                && p[offset..offset + 3] == [0, 0, 1]
                && (0xe0..=0xef).contains(&p[offset + 3])
        })
}
fn valid_segment(bytes: &[u8], ext: &str) -> bool {
    match ext {
        "vtt" => valid_webvtt(bytes),
        "mp4" | "m4s" => {
            mp4_boxes(bytes).is_some_and(|(f, m, d, fragment)| (f && m) || (fragment && d))
        }
        "ts" => {
            !bytes.is_empty()
                && bytes.len() % 188 == 0
                && bytes.chunks_exact(188).all(|packet| packet[0] == 0x47)
        }
        "aac" => {
            let mut at = 0;
            while at < bytes.len() {
                if bytes.len() - at < 7 || bytes[at] != 0xff || bytes[at + 1] & 0xf6 != 0xf0 {
                    return false;
                }
                let size = (((bytes[at + 3] & 3) as usize) << 11)
                    | ((bytes[at + 4] as usize) << 3)
                    | ((bytes[at + 5] as usize) >> 5);
                if size < 7 || size > bytes.len() - at {
                    return false;
                }
                at += size;
            }
            at > 0
        }
        _ => false,
    }
}
fn vtt_time(value: &str) -> Option<u64> {
    let (clock, millis) = value.split_once('.')?;
    if millis.len() != 3 || !millis.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts = clock.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.len() < 2 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let seconds = parts.last()?.parse::<u64>().ok()?;
    let minutes = parts[parts.len() - 2].parse::<u64>().ok()?;
    if seconds >= 60 || minutes >= 60 {
        return None;
    }
    let hours = if parts.len() == 3 {
        parts[0].parse::<u64>().ok()?
    } else {
        0
    };
    hours
        .checked_mul(3_600_000)?
        .checked_add(minutes * 60_000)?
        .checked_add(seconds * 1000)?
        .checked_add(millis.parse::<u64>().ok()?)
}
fn valid_webvtt(bytes: &[u8]) -> bool {
    if bytes.len() > 2 * 1024 * 1024 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return false;
    }
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut blocks = normalized.split("\n\n");
    let Some(header) = blocks.next() else {
        return false;
    };
    if header != "WEBVTT" && !header.starts_with("WEBVTT ") && !header.starts_with("WEBVTT\t") {
        return false;
    }
    if header.contains("-->") || header.contains('\n') {
        return false;
    }
    for block in blocks {
        let block = block.trim();
        if block.is_empty()
            || block == "NOTE"
            || block.starts_with("NOTE ")
            || block.starts_with("NOTE\n")
        {
            continue;
        }
        let lines = block.lines().collect::<Vec<_>>();
        let timing_index = if lines[0].contains("-->") { 0 } else { 1 };
        let Some(timing) = lines.get(timing_index) else {
            return false;
        };
        let Some((from, tail)) = timing.split_once(" --> ") else {
            return false;
        };
        let Some(to) = tail.split_whitespace().next() else {
            return false;
        };
        let (Some(from), Some(to)) = (vtt_time(from), vtt_time(to)) else {
            return false;
        };
        if to <= from
            || lines.len() <= timing_index + 1
            || lines[timing_index + 1..].iter().any(|l| l.contains("-->"))
        {
            return false;
        }
    }
    true
}

// Keep original attribute spelling when removing an unavailable optional track;
// quoted commas in CODECS/CHARACTERISTICS are not attribute separators.
fn hls_attributes(line: &str) -> Result<Vec<(&str, &str, &str)>, String> {
    let source = line.split_once(':').ok_or("hls_invalid_attributes")?.1;
    let (mut fields, mut start, mut quoted) = (Vec::new(), 0, false);
    for (at, c) in source.char_indices() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == ',' && !quoted {
            fields.push(source[start..at].trim());
            start = at + 1;
        }
    }
    if quoted {
        return Err("hls_invalid_attributes".into());
    }
    fields.push(source[start..].trim());
    let mut seen = HashSet::new();
    fields
        .into_iter()
        .map(|field| {
            let (name, raw) = field.split_once('=').ok_or("hls_invalid_attributes")?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
                || !seen.insert(name)
            {
                return Err("hls_invalid_attributes".into());
            }
            let value = if raw.starts_with('"') {
                raw.strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .ok_or("hls_invalid_attributes")?
            } else {
                raw
            };
            if value.is_empty() || value.contains('"') {
                return Err("hls_invalid_attributes".into());
            }
            Ok((name, value, field))
        })
        .collect()
}
fn subtitle_reference(source: &str, reference: &str) -> Result<bool, String> {
    let mut subtitle = false;
    for line in source.lines().map(str::trim) {
        if line == reference && !line.starts_with('#') {
            return Ok(false);
        }
        if !line.contains("URI=") {
            continue;
        }
        let attrs = hls_attributes(line)?;
        if attrs.iter().any(|(k, v, _)| *k == "URI" && *v == reference) {
            if !line.starts_with("#EXT-X-MEDIA:")
                || !attrs
                    .iter()
                    .any(|(k, v, _)| *k == "TYPE" && *v == "SUBTITLES")
            {
                return Ok(false);
            }
            subtitle = true;
        }
    }
    Ok(subtitle)
}
fn prune_subtitle_reference(source: &str, reference: &str) -> Result<String, String> {
    let mut retained = Vec::new();
    let mut groups = HashSet::new();
    for line in source.lines() {
        if line.trim().starts_with("#EXT-X-MEDIA:") {
            let attrs = hls_attributes(line.trim())?;
            if attrs
                .iter()
                .any(|(k, v, _)| *k == "TYPE" && *v == "SUBTITLES")
            {
                if attrs.iter().any(|(k, v, _)| *k == "URI" && *v == reference) {
                    continue;
                }
                if let Some((_, group, _)) = attrs.iter().find(|(k, _, _)| *k == "GROUP-ID") {
                    groups.insert(*group);
                }
            }
        }
        retained.push(line);
    }
    retained
        .into_iter()
        .map(|line| {
            if line.trim().starts_with("#EXT-X-STREAM-INF:") {
                let attrs = hls_attributes(line.trim())?;
                return Ok(format!(
                    "#EXT-X-STREAM-INF:{}",
                    attrs
                        .into_iter()
                        .filter(|(k, v, _)| *k != "SUBTITLES" || groups.contains(v))
                        .map(|(_, _, raw)| raw)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            Ok(line.to_string())
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|lines| lines.join("\n"))
}
fn validate_local_playlist(
    source: &str,
    post_id: &str,
    assets: &[ArchiveAsset],
) -> Result<(), String> {
    for reference in playlist_refs(source)? {
        let generated = reference
            .strip_prefix("asset-")
            .and_then(|name| name.rsplit_once('.'))
            .is_some_and(|(id, _)| {
                uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
            });
        if !generated
            || !asset_name(&reference)
            || !assets.iter().any(|asset| {
                asset.name == reference
                    && asset.post_id == post_id
                    && matches!(asset.kind.as_str(), "video_playlist" | "video_segment")
                    && matches!(
                        asset.mime.as_str(),
                        "application/vnd.apple.mpegurl"
                            | "video/mp4"
                            | "video/mp2t"
                            | "audio/aac"
                            | "text/vtt"
                    )
                    && valid_asset_type(asset)
                    && asset.bytes > 0
                    && asset.bytes <= MAX_FILE
                    && asset.sha256.len() == 64
                    && asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err("hls_uncommitted_reference".into());
        }
    }
    Ok(())
}
fn store_asset(
    stage: &Path,
    bytes: &[u8],
    mime: &str,
    ext: &str,
    post: &CapturePost,
    kind: &str,
    source: &str,
    download_url: Option<&str>,
    assets: &mut Vec<ArchiveAsset>,
) -> Result<String, String> {
    if assets.len() >= MAX_ASSETS {
        return Err("media_count_limit".into());
    }
    let name = format!("asset-{}.{}", uuid::Uuid::new_v4(), ext);
    write_new(&stage.join(&name), bytes)?;
    let hash = digest(bytes);
    if digest(&read_regular(&stage.join(&name), MAX_FILE)?) != hash {
        return Err("media_readback_failed".into());
    }
    assets.push(ArchiveAsset {
        name: name.clone(),
        mime: mime.into(),
        bytes: bytes.len() as u64,
        sha256: hash,
        post_id: post.id.clone(),
        kind: kind.into(),
        source_url: source.into(),
        download_url: download_url.map(str::to_string),
    });
    Ok(name)
}
fn store_image_media(
    stage: &Path,
    post: &CapturePost,
    media: &CaptureMedia,
    assets: &mut Vec<ArchiveAsset>,
    fetch_image: impl FnOnce(&Url) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let original = media.kind == "image";
    let url = if original { original_image_url(&media.url)? } else { media_url(&media.url, false)? };
    // One request only. A failed original must never silently fall back to the
    // observed thumbnail, another format, or another image ID.
    let bytes = fetch_image(&url).map_err(|reason| {
        if original && reason != CANCELLED { "original_image_request_failed".into() } else { reason }
    })?;
    let (mime, ext) = image_mime(&bytes).map_err(|reason| {
        if original { "original_image_validation_failed".into() } else { reason }
    })?;
    if original && original_image_mime(&url) != Some(mime) {
        return Err("original_image_validation_failed".into());
    }
    store_asset(stage, &bytes, mime, ext, post, &media.kind, &media.url,
        original.then_some(url.as_str()), assets)?;
    Ok(())
}
fn store_media(
    client: &Client,
    stage: &Path,
    post: &CapturePost,
    media: &CaptureMedia,
    job: &Job,
    total: &mut u64,
    assets: &mut Vec<ArchiveAsset>,
    issues: &mut Vec<ArchiveIssue>,
) -> Result<(), String> {
    if media.kind == "unavailable_video" {
        return Err("video_resource_not_observed".into());
    }
    if media.kind == "unavailable_image" {
        return Err("image_resource_not_observed".into());
    }
    if media.kind != "video" {
        return store_image_media(stage, post, media, assets, |url| fetch(client, url, MAX_IMAGE, job, total));
    }
    let url = media_url(&media.url, true)?;
    if url.path().ends_with(".m3u8") {
        let mut graph = HashMap::new();
        let mut visiting = HashSet::new();
        let mut has_video = false;
        archive_hls(
            client,
            stage,
            post,
            &url,
            &video_scope(&url).ok_or(ERROR)?,
            job,
            total,
            assets,
            &mut graph,
            &mut visiting,
            &mut has_video,
            issues,
            0,
        )?;
    } else {
        let bytes = fetch(client, &url, MAX_FILE, job, total)?;
        if !mp4_complete(&bytes) || !mp4_video_track(&bytes, 0) {
            return Err("video_container_incomplete".into());
        }
        store_asset(
            stage,
            &bytes,
            "video/mp4",
            "mp4",
            post,
            "video",
            &media.url,
            None,
            assets,
        )?;
    }
    Ok(())
}

// Preserve the finite, unencrypted playlist graph locally. Every URI (including
// AUDIO and MAP attributes) is rewritten to a committed local asset. Never pass
// a remote playlist to ffmpeg or preserve a live/sliding playlist as complete.
#[allow(clippy::too_many_arguments)]
fn archive_hls(
    client: &Client,
    stage: &Path,
    post: &CapturePost,
    url: &Url,
    scope: &str,
    job: &Job,
    total: &mut u64,
    assets: &mut Vec<ArchiveAsset>,
    graph: &mut HashMap<String, HlsNode>,
    visiting: &mut HashSet<String>,
    has_video: &mut bool,
    issues: &mut Vec<ArchiveIssue>,
    depth: usize,
) -> Result<String, String> {
    if depth > 4 || graph.len() >= MAX_ASSETS || !visiting.insert(url.to_string()) {
        return Err("hls_graph_limit".into());
    }
    let bytes = fetch(
        client,
        url,
        if url.path().ends_with(".m3u8") || url.path().ends_with(".vtt") {
            2 * 1024 * 1024
        } else {
            32 * 1024 * 1024
        },
        job,
        total,
    )?;
    if !url.path().ends_with(".m3u8") {
        let ext = url.path().rsplit('.').next().ok_or(ERROR)?;
        if !valid_segment(&bytes, ext) {
            return Err(if ext == "vtt" {
                "hls_subtitle_invalid"
            } else {
                "hls_segment_invalid"
            }
            .into());
        }
        *has_video |= segment_video_track(&bytes, ext);
        let mime = match ext {
            "mp4" | "m4s" => "video/mp4",
            "ts" => "video/mp2t",
            "aac" => "audio/aac",
            "vtt" => "text/vtt",
            _ => return Err("hls_segment_type".into()),
        };
        let name = store_asset(
            stage,
            &bytes,
            mime,
            ext,
            post,
            "video_segment",
            url.as_str(),
            None,
            assets,
        )?;
        visiting.remove(url.as_str());
        graph.insert(
            url.to_string(),
            HlsNode {
                name: name.clone(),
                has_video: *has_video,
            },
        );
        return Ok(name);
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| "hls_invalid_text")?;
    let refs = playlist_refs(source)?;
    let mut rewritten = source.to_string();
    // Replace full lines or quoted URI fields only, never substring host edits.
    for reference in refs {
        let optional_subtitle = subtitle_reference(source, &reference)?;
        let before = visiting.clone();
        let attempt = hls_branch(has_video, |branch_proof| {
            let child = url.join(&reference).map_err(|_| "hls_url_invalid")?;
            media_url(child.as_str(), true).map_err(|_| "hls_media_url_unsupported")?;
            if video_scope(&child).as_deref() != Some(scope) {
                return Err("hls_scope_changed".into());
            }
            if let Some(node) = graph.get(child.as_str()) {
                return Ok(node.reuse(branch_proof));
            }
            archive_hls(
                client,
                stage,
                post,
                &child,
                scope,
                job,
                total,
                assets,
                graph,
                visiting,
                branch_proof,
                issues,
                depth + 1,
            )
        });
        let local = match attempt {
            Ok(local) => local,
            Err(reason) if optional_subtitle => {
                job.check()?;
                *visiting = before;
                rewritten = prune_subtitle_reference(&rewritten, &reference)?;
                append_issue(
                    issues,
                    ArchiveIssue {
                        post_id: post.id.clone(),
                        kind: "subtitle".into(),
                        reason,
                    },
                )?;
                continue;
            }
            Err(reason) => return Err(reason),
        };
        rewritten = rewritten
            .lines()
            .map(|line| {
                if line.trim() == reference {
                    local.clone()
                } else {
                    line.replace(&format!("URI=\"{reference}\""), &format!("URI=\"{local}\""))
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if depth == 0 && !*has_video {
        return Err("hls_video_track_unverified".into());
    }
    validate_local_playlist(&rewritten, &post.id, assets)?;
    let name = store_asset(
        stage,
        rewritten.as_bytes(),
        "application/vnd.apple.mpegurl",
        "m3u8",
        post,
        if depth == 0 {
            "video_hls"
        } else {
            "video_playlist"
        },
        url.as_str(),
        None,
        assets,
    )?;
    visiting.remove(url.as_str());
    graph.insert(
        url.to_string(),
        HlsNode {
            name: name.clone(),
            has_video: *has_video,
        },
    );
    Ok(name)
}
fn playlist_refs(source: &str) -> Result<Vec<String>, String> {
    if !source.trim_start().starts_with("#EXTM3U") || source.len() > 2 * 1024 * 1024 {
        return Err("hls_invalid".into());
    }
    let master = source
        .lines()
        .any(|line| line.starts_with("#EXT-X-STREAM-INF:"));
    if !master && !source.lines().any(|line| line.trim() == "#EXT-X-ENDLIST") {
        return Err("hls_live_not_archivable".into());
    }
    let mut refs = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.len() > 8192
            || line.contains("#EXT-X-BYTERANGE")
            || line.contains("BYTERANGE=")
            || line.starts_with("#EXT-X-KEY:") && !line.contains("METHOD=NONE")
            || line.starts_with("#EXT-X-SESSION-KEY")
            || line.starts_with("#EXT-X-DEFINE")
            || line.starts_with("#EXT-X-PART")
            || line.starts_with("#EXT-X-PRELOAD")
        {
            return Err("hls_restricted_or_unsupported".into());
        }
        if !line.is_empty() && !line.starts_with('#') {
            refs.push(line.to_string());
        }
        if line.contains("URI=") {
            if !["#EXT-X-MAP:", "#EXT-X-MEDIA:", "#EXT-X-I-FRAME-STREAM-INF:"]
                .iter()
                .any(|p| line.starts_with(p))
            {
                return Err("hls_unknown_uri".into());
            }
            let tail = line.split_once("URI=\"").ok_or("hls_invalid_uri")?.1;
            let value = tail.split_once('"').ok_or("hls_invalid_uri")?.0;
            if value.is_empty() || value.len() > 2048 || tail.matches("URI=").count() > 0 {
                return Err("hls_invalid_uri".into());
            }
            refs.push(value.into());
        }
    }
    if refs.is_empty() || refs.len() > MAX_ASSETS {
        return Err("hls_graph_limit".into());
    }
    Ok(refs)
}

fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(ERROR.into());
    }
    for parent in path.ancestors() {
        if let Ok(m) = fs::symlink_metadata(parent) {
            if redirected(&m) {
                return Err(ERROR.into());
            }
        }
    }
    Ok(())
}
fn redirected(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}
fn open_regular(path: &Path) -> Result<File, String> {
    safe_path(path)?;
    let before = fs::symlink_metadata(path).map_err(|_| ERROR)?;
    if !before.is_file() || redirected(&before) {
        return Err(ERROR.into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000).share_mode(1);
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    let m = file.metadata().map_err(|_| ERROR)?;
    if !m.is_file() || redirected(&m) {
        return Err(ERROR.into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| ERROR)?;
        if info.nNumberOfLinks != 1 {
            return Err(ERROR.into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() != 1 {
            return Err(ERROR.into());
        }
    }
    Ok(file)
}
fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = open_regular(path)?;
    if file.metadata().map_err(|_| ERROR)?.len() > limit {
        return Err(ERROR.into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 > limit {
        return Err(ERROR.into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    safe_path(path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| ERROR)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| ERROR.into())
}
fn publish(stage: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let from = stage
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let to = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|_| ERROR)?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(stage, destination).map_err(|_| ERROR)?;
        File::open(destination.parent().ok_or(ERROR)?)
            .and_then(|f| f.sync_all())
            .map_err(|_| ERROR)?;
    }
    Ok(())
}
fn folder(root: &Path, value: &str) -> Result<PathBuf, String> {
    if uuid::Uuid::parse_str(value).map_err(|_| ERROR)?.to_string() != value {
        return Err(ERROR.into());
    }
    safe_path(root)?;
    let path = root.join(value);
    safe_path(&path)?;
    Ok(path)
}
fn asset_name(value: &str) -> bool {
    value.len() < 100
        && value.starts_with("asset-")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
        && !value.contains("..")
}
pub fn read(root: &Path, id: &str) -> Result<ArchiveRecord, String> {
    let folder = folder(root, id)?;
    let bytes = read_regular(&folder.join("archive.json"), MAX_JSON)?;
    let hash = read_regular(&folder.join("manifest.sha256"), 64)?;
    if hash != digest(&bytes).as_bytes() {
        return Err("보관 기록의 검증값이 다릅니다.".into());
    }
    let record: ArchiveRecord = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    if record.version != 1
        || record.id != id
        || record.assets.len() > MAX_ASSETS
        || record.issues.len() > MAX_ISSUES
    {
        return Err(ERROR.into());
    }
    validate_document(&record.document)?;
    let mut names = HashSet::new();
    for asset in &record.assets {
        if !asset_name(&asset.name)
            || !names.insert(&asset.name)
            || asset.bytes == 0
            || asset.bytes > MAX_FILE
            || asset.sha256.len() != 64
            || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !valid_asset_type(asset)
            || !valid_asset_provenance(&record.document, asset)
            || !record
                .document
                .records
                .iter()
                .any(|p| p.id == asset.post_id)
        {
            return Err(ERROR.into());
        }
    }
    if digest(&read_regular(&folder.join("index.html"), MAX_JSON)?) != record.html_sha256 {
        return Err("보관 화면의 검증값이 다릅니다.".into());
    }
    Ok(record)
}
fn valid_asset_provenance(document: &CaptureDocument, asset: &ArchiveAsset) -> bool {
    let Some(download_url) = asset.download_url.as_deref() else {
        // Absence is the legacy format, not evidence that it used originals.
        return true;
    };
    asset.kind == "image" && valid_asset_type(asset)
        && document.records.iter().any(|post| post.id == asset.post_id
            && post.media.iter().any(|media| media.kind == "image" && media.url == asset.source_url))
        && original_image_url(&asset.source_url).is_ok_and(|expected| expected.as_str() == download_url
            && original_image_mime(&expected) == Some(asset.mime.as_str()))
}
fn valid_asset_type(asset: &ArchiveAsset) -> bool {
    let ext = asset.name.rsplit('.').next().unwrap_or_default();
    let pair = match asset.mime.as_str() {
        "image/jpeg" => ext == "jpg",
        "image/png" => ext == "png",
        "image/webp" => ext == "webp",
        "image/gif" => ext == "gif",
        "video/mp4" => matches!(ext, "mp4" | "m4s"),
        "video/mp2t" => ext == "ts",
        "audio/aac" => ext == "aac",
        "text/vtt" => ext == "vtt" && asset.kind == "video_segment",
        "application/vnd.apple.mpegurl" => ext == "m3u8",
        _ => false,
    };
    pair && matches!(
        asset.kind.as_str(),
        "image" | "poster" | "avatar" | "video" | "video_hls" | "video_playlist" | "video_segment"
    )
}
pub fn list(root: &Path) -> Result<Vec<ArchiveSummary>, String> {
    safe_path(root)?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut output = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| ERROR)?.take(10001) {
        let entry = entry.map_err(|_| ERROR)?;
        let name = entry.file_name().to_string_lossy().to_string();
        if uuid::Uuid::parse_str(&name).is_err() {
            continue;
        }
        if output.len() >= 10000 {
            return Err("X 보관 목록 한도를 초과했습니다.".into());
        }
        let r = read(root, &name)?;
        output.push(ArchiveSummary {
            id: r.id,
            account_id: r.account_id,
            source_url: r.document.source_url,
            saved_at: r.saved_at,
            captured_at: r.document.captured_at,
            posts: r.document.records.len(),
            assets: r.assets.len(),
            issues: r.issues.len(),
            intent: r.intent,
            selected_content_complete: r.selected_content_complete,
        });
    }
    output.sort_by_key(|r| std::cmp::Reverse(r.saved_at));
    Ok(output)
}
/// Only expose a fixed archive folder after immutable-record/path validation.
pub fn verified_folder(root: &Path, id: &str) -> Result<PathBuf, String> {
    read(root, id)?;
    let path = folder(root, id)?;
    let canonical_root = fs::canonicalize(root).map_err(|_| ERROR)?;
    let canonical_path = fs::canonicalize(&path).map_err(|_| ERROR)?;
    if canonical_path.parent() != Some(canonical_root.as_path()) || !canonical_path.is_dir() {
        return Err(ERROR.into());
    }
    safe_path(&path)?;
    safe_path(&canonical_path)?;
    Ok(path)
}
/// A verified file handle held through the range read. No absolute file path is
/// exposed to IPC; the protocol chooses an asset only from a verified manifest.
pub struct ArchiveFile {
    file: File,
    pub mime: String,
    pub kind: String,
    pub bytes: u64,
}
impl ArchiveFile {
    pub fn read_range(&mut self, start: u64, length: usize) -> Result<Vec<u8>, String> {
        if length > 4 * 1024 * 1024 || start > self.bytes || length as u64 > self.bytes - start {
            return Err(ERROR.into());
        }
        self.file.seek(SeekFrom::Start(start)).map_err(|_| ERROR)?;
        let mut result = vec![0; length];
        self.file.read_exact(&mut result).map_err(|_| ERROR)?;
        Ok(result)
    }
}
pub fn open_asset(root: &Path, id: &str, name: &str) -> Result<ArchiveFile, String> {
    let record = read(root, id)?;
    let (mime, bytes, hash) = if name == "index.html" {
        (
            "text/html; charset=utf-8".to_string(),
            None,
            record.html_sha256,
        )
    } else {
        let asset = record.assets.iter().find(|a| a.name == name).ok_or(ERROR)?;
        (asset.mime.clone(), Some(asset.bytes), asset.sha256.clone())
    };
    let mut file = open_regular(&folder(root, id)?.join(name))?;
    let length = file.metadata().map_err(|_| ERROR)?.len();
    if length > MAX_FILE || bytes.is_some_and(|n| n != length) {
        return Err(ERROR.into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| ERROR)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if format!("{:x}", hasher.finalize()) != hash {
        return Err("보관 미디어의 검증값이 다릅니다.".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| ERROR)?;
    Ok(ArchiveFile {
        file,
        mime,
        kind: record
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.kind.clone())
            .unwrap_or_else(|| "html".into()),
        bytes: length,
    })
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn render_html(record: &ArchiveRecord) -> String {
    let mut html=String::from("<!doctype html><html lang=\"ko\"><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'none'; connect-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-src 'none'\"><meta name=\"viewport\" content=\"width=device-width\"><title>X 보관본</title><style>body{max-width:760px;margin:24px auto;padding:0 16px;font:16px system-ui;background:#18191c;color:#eee}article{border-bottom:1px solid #555;padding:20px 0}p,blockquote{white-space:pre-wrap;overflow-wrap:anywhere}small{color:#aaa}img,video{max-width:100%;max-height:80vh}blockquote{border-left:2px solid #777;padding-left:12px}</style><body>");
    html.push_str(&format!("<header><h1>X 보관본</h1><p>{}</p><small>{} · 화면에 로드된 내용만 보관 · 선택 {}개 · 누락 {}</small></header>",escape(&record.document.source_url),escape(&record.document.captured_at),record.document.records.len(),record.issues.len()));
    for post in &record.document.records {
        if post.photo_only {
            html.push_str(&format!(
                "<article><h2>현재 사진</h2><p>현재 사진만 보관 · 본문·댓글 미포함</p><small>{}</small>",
                escape(&post.url)
            ));
        } else {
        html.push_str(&format!(
            "<article><small>{}</small><h2>{}</h2><small>{} · {}</small><p>{}</p>",
            escape(&post.context),
            escape(&post.author),
            escape(&post.timestamp),
            escape(&post.url),
            escape(&post.body)
        ));
        }
        for quote in &post.quotes {
            html.push_str(&format!("<blockquote>{}</blockquote>", escape(quote)));
        }
        html.push_str(&format!("<small>{}</small>", escape(&post.counts)));
        for a in record.assets.iter().filter(|a| a.post_id == post.id) {
            if matches!(a.kind.as_str(), "image" | "poster") {
                html.push_str(&format!(
                    "<p><img src=\"{}\" alt=\"보관 이미지\"></p>",
                    a.name
                ));
            } else if a.kind == "video" {
                html.push_str(&format!(
                    "<video src=\"{}\" controls preload=\"metadata\"></video>",
                    a.name
                ));
            } else if a.kind == "video_hls" {
                html.push_str("<p>HLS 영상과 모든 조각이 로컬에 보관되었습니다. Atsumi 보관 뷰어에서 재생하세요.</p>");
            }
        }
        if post.truncated {
            html.push_str("<p>더 보기로 접힌 본문은 캡처 당시 표시된 부분만 보관되었습니다.</p>");
        }
        html.push_str("</article>");
    }
    html.push_str("</body></html>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> CaptureDocument {
        CaptureDocument {
            version: 1,
            source_url: "https://x.com/demo/status/123".into(),
            owner: "demo".into(),
            captured_at: "2026-10-20T01:00:00Z".into(),
            coverage: "loaded_dom_only".into(),
            records: vec![CapturePost {
                id: "123".into(),
                url: "https://x.com/demo/status/123".into(),
                author: "Example".into(),
                body: "<script>alert(1)</script> & text".into(),
                timestamp: "".into(),
                counts: "4 replies".into(),
                context: "".into(),
                quotes: vec![],
                media: vec![],
                truncated: false,
                photo_only: false,
                presentation: None,
            }],
            limit_reached: false,
            diagnostic: None,
            quote_diagnostic: None,
        }
    }
    fn draft(state: &XArchiveState) -> ArchiveDraft {
        let d = document();
        state
            .freeze(
                &serde_json::to_string(&d).unwrap(),
                &d.source_url,
                "account_test",
                0,
            )
            .unwrap()
    }
    fn request(d: &ArchiveDraft) -> SaveRequest {
        SaveRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            capture_id: d.capture_id.clone(),
            selected_ids: vec!["123".into()],
            intent: SaveIntent::Archive,
            include_media: false,
        }
    }
    #[test]
    fn isolated_save_queue_bounds_two_running_and_twenty_waiting_without_chooser_overtake() {
        let queue = SaveQueue::default();
        queue.admit("running-a", false).unwrap();
        queue.admit("running-b", false).unwrap();
        for index in 0..20 { queue.admit(&format!("waiting-{index}"), true).unwrap(); }
        assert_eq!(queue.admit("overflow", true).unwrap_err(), QUEUE_FULL);
        assert!(queue.admit("waiting-0", true).is_err());
        queue.release("running-a");
        assert!(queue.admit("chooser", false).is_err());
        queue.wait("waiting-0", || true).unwrap();
        assert!(queue.running("waiting-0"));
        assert_eq!(queue.state.lock().unwrap().running.len(), 2);
        queue.release("waiting-0");
        queue.release("running-b");
        for index in 1..20 { queue.release(&format!("waiting-{index}")); }
        queue.admit("chooser", false).unwrap();
        assert!(queue.running("chooser"));
    }
    #[test]
    fn isolated_save_queue_fifo_is_preserved_when_later_workers_arrive_first() {
        let queue = Arc::new(SaveQueue::default());
        for id in ["a", "b", "c", "d"] { queue.admit(id, true).unwrap(); }
        let (events, receive) = std::sync::mpsc::channel();
        let workers: Vec<_> = ["d", "c"].into_iter().map(|id| {
            let queue = queue.clone();let events = events.clone();
            std::thread::spawn(move || {
                let deadline = Instant::now();
                queue.wait(id, || deadline.elapsed() < Duration::from_secs(3)).unwrap();
                events.send(id).unwrap();
            })
        }).collect();
        assert!(receive.recv_timeout(Duration::from_millis(30)).is_err());
        queue.wait("a", || true).unwrap();queue.wait("b", || true).unwrap();
        assert!(receive.recv_timeout(Duration::from_millis(30)).is_err());
        queue.release("a");
        assert_eq!(receive.recv_timeout(Duration::from_secs(1)).unwrap(), "c");
        assert_eq!(queue.state.lock().unwrap().running.len(), 2);
        assert!(receive.recv_timeout(Duration::from_millis(30)).is_err());
        queue.release("b");
        assert_eq!(receive.recv_timeout(Duration::from_secs(1)).unwrap(), "d");
        assert_eq!(queue.state.lock().unwrap().running.len(), 2);
        queue.release("c");queue.release("d");
        for worker in workers { worker.join().unwrap(); }
    }
    #[test]
    fn isolated_save_queue_cancelled_head_releases_only_its_place() {
        let queue = Arc::new(SaveQueue::default());
        queue.admit("head", true).unwrap();queue.admit("next", true).unwrap();
        assert!(queue.wait("head", || false).is_err());
        assert!(!queue.running("head"));
        queue.release("head");queue.wait("next", || true).unwrap();
        assert!(queue.running("next"));
        assert_eq!(queue.state.lock().unwrap().running.len(), 1);
    }
    #[test]
    fn queued_capture_admission_registers_all_work_and_releases_overflow_cancel_and_early_errors() {
        let state = XArchiveState::default();
        let mut pending: Vec<_> = (0..SAVE_OUTSTANDING_LIMIT).map(|_| state.reserve_queued(uuid::Uuid::new_v4().to_string()).unwrap()).collect();
        let ids: Vec<_> = pending.iter().map(|entry| entry.job.id.clone()).collect();
        assert!(ids.iter().all(|id| active_ids().contains(id)));
        assert!(matches!(state.reserve_queued(uuid::Uuid::new_v4().to_string()), Err(error) if error == QUEUE_FULL));
        assert!(state.reserve_queued(ids[0].clone()).is_err());
        assert!(state.cancel(&ids[0]));assert!(pending[0].check().is_err());
        drop(pending.remove(0));assert!(!active_ids().contains(&ids[0]));
        let replacement = state.reserve_queued(uuid::Uuid::new_v4().to_string()).unwrap();
        state.invalidate_drafts();
        assert!(pending.iter().all(|entry| entry.check().is_err()));assert!(replacement.check().is_err());
        drop(replacement);drop(pending);
        assert!(ids.iter().all(|id| !active_ids().contains(id)));
    }
    #[test]
    fn queued_snapshot_owns_selected_content_after_draft_release_and_expiry() {
        let state = XArchiveState::default();
        let mut doc = document();let mut other = doc.records[0].clone();
        other.id = "456".into();other.url = "https://x.com/owner/status/456".into();
        doc.records.push(other);
        let capture = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
        let lease = state.lease_draft(&capture);let request = request(&capture);
        let pending = state.reserve_queued(request.request_id.clone()).unwrap();
        let ticket = state.prepare_queued(pending, request).unwrap();
        assert_eq!(ticket.draft.document.records.len(), 1);assert_eq!(ticket.draft.document.records[0].id, "123");
        state.inner.drafts.lock().unwrap().get_mut(&capture.capture_id).unwrap().created = Instant::now() - Duration::from_secs(601);
        drop(lease);assert!(!state.inner.drafts.lock().unwrap().contains_key(&capture.capture_id));
        ticket.wait_for_turn().unwrap();
        let root = tempfile::tempdir().unwrap();let saved = state.save_reserved(root.path(), ticket).unwrap();
        assert_eq!(saved.document.source_url, doc.source_url);assert_eq!(saved.document.records.len(), 1);
        assert_eq!(saved.capture_id, capture.capture_id);
    }
    #[test]
    fn queued_epoch_invalidation_rejects_waiting_snapshot_before_any_publication() {
        let state = XArchiveState::default();let capture = draft(&state);let request = request(&capture);
        let pending = state.reserve_queued(request.request_id.clone()).unwrap();
        let ticket = state.prepare_queued(pending, request).unwrap();
        state.invalidate_drafts();assert!(ticket.wait_for_turn().is_err());
        let root = tempfile::tempdir().unwrap();assert!(state.save_reserved(root.path(), ticket).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn queued_preparation_rejects_wrong_identity_and_unstarted_save_cannot_write() {
        let state = XArchiveState::default();let capture = draft(&state);let save_request = request(&capture);
        let wrong = state.reserve_queued(uuid::Uuid::new_v4().to_string()).unwrap();let id = wrong.job.id.clone();
        assert!(state.prepare_queued(wrong, save_request).is_err());assert!(!active_ids().contains(&id));
        let save_request = request(&capture);let pending = state.reserve_queued(save_request.request_id.clone()).unwrap();
        let ticket = state.prepare_queued(pending, save_request).unwrap();let id = ticket.job.id.clone();
        let root = tempfile::tempdir().unwrap();assert!(state.save_reserved(root.path(), ticket).is_err());
        assert!(!active_ids().contains(&id));assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn queued_work_and_direct_chooser_share_global_execution_slots_without_overtake() {
        let first = XArchiveState::default();let second = XArchiveState::default();
        let first_draft = draft(&first);let second_draft = draft(&second);
        let first_job = first.reserve(request(&first_draft)).unwrap();
        let second_job = second.reserve(request(&second_draft)).unwrap();
        let save_request = request(&second_draft);let pending = second.reserve_queued(save_request.request_id.clone()).unwrap();
        let queued = second.prepare_queued(pending, save_request).unwrap();
        assert!(first.reserve(request(&first_draft)).is_err());
        drop(first_job);assert!(first.reserve(request(&first_draft)).is_err());
        queued.wait_for_turn().unwrap();
        assert!(save_queue().running(&queued.job.id));assert!(save_queue().running(&second_job.job.id));
        drop(queued);drop(second_job);
        assert!(first.reserve(request(&first_draft)).is_ok());
    }
    #[test]
    fn immediate_draft_leases_allow_repeated_complete_and_partial_saves_without_eviction() {
        let state = XArchiveState::default();
        let chooser = draft(&state);
        let root = tempfile::tempdir().unwrap();
        for index in 0..12 {
            let mut doc = document();
            doc.records[0].truncated = index % 2 == 1;
            let capture = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
            let lease = state.lease_draft(&capture);
            let ticket = state.reserve(request(&capture)).unwrap();
            drop(lease);
            assert!(ticket.job.check().is_ok());
            assert!(!state.inner.drafts.lock().unwrap().contains_key(&capture.capture_id));
            let record = state.save_reserved(root.path(), ticket).unwrap();
            assert_eq!(record.capture_id, capture.capture_id);
            assert_eq!(record.selected_content_complete, index % 2 == 0);
            let drafts = state.inner.drafts.lock().unwrap();
            assert_eq!(drafts.len(), 1);
            assert!(drafts.contains_key(&chooser.capture_id));
        }
        assert_eq!(list(root.path()).unwrap().len(), 12);
        assert!(state.reserve(request(&chooser)).is_ok());
    }
    #[test]
    fn immediate_draft_lease_releases_early_selection_failure_cancel_and_stale_returns() {
        let state = XArchiveState::default();
        let chooser = draft(&state);
        for early_exit in 0..12 {
            let capture = draft(&state);
            let result = (|| -> Result<(), String> {
                let _lease = state.lease_draft(&capture);
                match early_exit % 3 {
                    0 => {
                        let mut invalid = request(&capture);
                        invalid.selected_ids = vec!["not-in-frozen-capture".into()];
                        let _ticket = state.reserve(invalid)?;
                    }
                    1 => return Err(CANCELLED.into()),
                    _ => return Err("source changed after freeze".into()),
                }
                Ok(())
            })();
            assert!(result.is_err());
            let drafts = state.inner.drafts.lock().unwrap();
            assert_eq!(drafts.len(), 1);
            assert!(drafts.contains_key(&chooser.capture_id));
        }
        assert_eq!(state.inner.epoch.load(Ordering::Acquire), 0);
        assert!(state.inner.jobs.lock().unwrap().is_empty());
    }
    #[test]
    fn chooser_draft_remains_retryable_after_save_failure_until_exact_release() {
        let state = XArchiveState::default();
        let capture = draft(&state);
        let root = tempfile::tempdir().unwrap();
        let mut invalid = request(&capture);
        invalid.intent = SaveIntent::Media; // The fixture has no media.
        let first = state.reserve(invalid).unwrap();
        assert!(state.save_reserved(root.path(), first).is_err());
        assert!(state.inner.drafts.lock().unwrap().contains_key(&capture.capture_id));
        let retry = state.reserve(request(&capture)).unwrap();
        let record = state.save_reserved(root.path(), retry).unwrap();
        assert_eq!(record.capture_id, capture.capture_id);
        let metadata_before = fs::read(root.path().join(&record.id).join("archive.json")).unwrap();
        assert!(state.release_draft(&capture.capture_id, &capture.account_id).unwrap());
        assert!(!state.release_draft(&capture.capture_id, &capture.account_id).unwrap());
        assert!(state.reserve(request(&capture)).is_err());
        assert_eq!(fs::read(root.path().join(&record.id).join("archive.json")).unwrap(), metadata_before);
        assert_eq!(read(root.path(), &record.id).unwrap().capture_id, capture.capture_id);
    }
    #[test]
    fn draft_release_requires_exact_canonical_identity_and_preserves_reserved_jobs() {
        let state = XArchiveState::default();
        let capture = draft(&state);
        let other = draft(&state);
        let first = state.reserve(request(&capture)).unwrap();
        let second = state.reserve(request(&other)).unwrap();
        for account in ["", "../account", "https://x.com", &"a".repeat(81)] {
            assert!(state.release_draft(&capture.capture_id, account).is_err());
        }
        for id in ["not-a-uuid".into(), format!("{{{}}}", capture.capture_id), capture.capture_id.replace('-', "")] {
            assert!(state.release_draft(&id, &capture.account_id).is_err());
        }
        assert!(!state.release_draft(&capture.capture_id, "different_account").unwrap());
        assert!(!state.release_draft(&uuid::Uuid::new_v4().to_string(), &capture.account_id).unwrap());
        assert!(state.release_draft(&capture.capture_id, &capture.account_id).unwrap());
        assert!(first.job.check().is_ok());
        assert!(second.job.check().is_ok());
        assert_eq!(state.inner.jobs.lock().unwrap().len(), 2);
        assert_eq!(state.inner.epoch.load(Ordering::Acquire), 0);
        assert!(state.inner.drafts.lock().unwrap().contains_key(&other.capture_id));
        // Slot release does not weaken the independent two-active-job limit.
        assert!(state.reserve(request(&other)).is_err());
        drop(first);
        drop(second);
        assert!(state.inner.jobs.lock().unwrap().is_empty());
        assert!(state.reserve(request(&other)).is_ok());
    }
    #[test]
    fn draft_leases_preserve_eight_live_chooser_limit_and_expiry() {
        let state = XArchiveState::default();
        let choices: Vec<_> = (0..8).map(|_| draft(&state)).collect();
        let doc = document();
        let raw = serde_json::to_string(&doc).unwrap();
        assert_eq!(state.freeze(&raw, &doc.source_url, "account_test", 0).unwrap_err(),
            "보관 준비 작업이 많습니다. 잠시 후 다시 시도해 주세요.");
        assert_eq!(state.inner.drafts.lock().unwrap().len(), 8);
        assert!(state.release_draft(&choices[0].capture_id, &choices[0].account_id).unwrap());
        let replacement = state.freeze(&raw, &doc.source_url, "account_test", 0).unwrap();
        assert_eq!(state.inner.drafts.lock().unwrap().len(), 8);
        state.inner.drafts.lock().unwrap().get_mut(&choices[1].capture_id).unwrap().created = Instant::now() - Duration::from_secs(601);
        assert!(state.reserve(request(&choices[1])).is_err());
        let fresh = state.freeze(&raw, &doc.source_url, "account_test", 0).unwrap();
        let drafts = state.inner.drafts.lock().unwrap();
        assert_eq!(drafts.len(), 8);
        assert!(drafts.contains_key(&replacement.capture_id));
        assert!(drafts.contains_key(&fresh.capture_id));
        for choice in choices.iter().skip(2) { assert!(drafts.contains_key(&choice.capture_id)); }
    }
    #[test]
    fn old_draft_lease_cannot_revoke_new_epoch_or_another_capture() {
        let state = XArchiveState::default();
        let old = draft(&state);
        let lease = state.lease_draft(&old);
        let old_job = state.reserve(request(&old)).unwrap();
        state.invalidate_drafts();
        assert!(old_job.job.check().is_err());
        drop(old_job);
        let doc = document();
        let fresh = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 1).unwrap();
        let current_job = state.reserve(request(&fresh)).unwrap();
        drop(lease);
        assert_eq!(state.inner.epoch.load(Ordering::Acquire), 1);
        assert!(state.inner.drafts.lock().unwrap().contains_key(&fresh.capture_id));
        assert!(current_job.job.check().is_ok());
    }
    fn media_job(media: Vec<CaptureMedia>) -> ArchiveJob {
        let state = XArchiveState::default();
        let mut doc = document();
        doc.records[0].media = media;
        let capture = state.freeze(
            &serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0,
        ).unwrap();
        let mut req = request(&capture);
        req.intent = SaveIntent::Media;
        req.include_media = true;
        state.reserve(req).unwrap()
    }
    fn captured_media(kind: &str, url: &str, poster: Option<&str>) -> CaptureMedia {
        CaptureMedia { kind: kind.into(), url: url.into(), alt: "".into(), poster: poster.map(Into::into) }
    }
    #[test]
    fn restrict_media_keeps_only_the_exact_observed_frozen_resource() {
        let first = "https://pbs.twimg.com/media/first?format=jpg&name=large";
        let other = "https://pbs.twimg.com/media/other?format=jpg&name=large";
        let mut job = media_job(vec![
            captured_media("image", first, None),
            captured_media("image", other, None),
        ]);
        let shared = job.draft.clone();
        assert!(job.restrict_media("123", first).is_ok());
        assert_eq!(job.draft.document.records[0].media.len(), 1);
        assert_eq!(job.draft.document.records[0].media[0].url, first);
        assert_eq!(shared.document.records[0].media.len(), 2);
        assert!(job.restrict_media("123", other).is_err());
        assert_eq!(job.draft.document.records[0].media[0].url, first);
    }
    #[test]
    fn restrict_media_distinguishes_unavailable_videos_with_different_posters() {
        let first = "https://pbs.twimg.com/ext_tw_video_thumb/123/pu/img/first.jpg";
        let other = "https://pbs.twimg.com/ext_tw_video_thumb/456/pu/img/other.jpg";
        let mut job = media_job(vec![
            captured_media("unavailable_video", "", Some(first)),
            captured_media("poster", first, None),
            captured_media("unavailable_video", "", Some(other)),
            captured_media("poster", other, None),
        ]);
        assert!(job.restrict_media("123", first).is_ok());
        let kept = &job.draft.document.records[0].media;
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].kind, "unavailable_video");
        assert_eq!(kept[0].poster.as_deref(), Some(first));
        assert_eq!(kept[1].kind, "poster");
        assert_eq!(kept[1].url, first);
    }
    #[test]
    fn restrict_media_never_admits_unknown_or_ambiguous_selectors() {
        let image = "https://pbs.twimg.com/media/first?format=jpg";
        let poster = "https://pbs.twimg.com/ext_tw_video_thumb/123/pu/img/first.jpg";
        let mut job = media_job(vec![
            captured_media("image", image, None),
            captured_media("unavailable_video", "", Some(poster)),
            captured_media("unavailable_video", "", Some(poster)),
        ]);
        let before = serde_json::to_value(&job.draft.document).unwrap();
        for observed in ["", "https://evil.test/image.jpg", "file:///C:/private.jpg",
            "https://pbs.twimg.com/media/not_captured?format=jpg", poster] {
            assert!(job.restrict_media("123", observed).is_err());
            assert_eq!(serde_json::to_value(&job.draft.document).unwrap(), before);
        }
        assert!(job.restrict_media("999", image).is_err());
        job.request.intent = SaveIntent::Archive;
        assert!(job.restrict_media("123", image).is_err());
        assert_eq!(serde_json::to_value(&job.draft.document).unwrap(), before);
    }
    #[test]
    fn frozen_ids_only_and_epoch_revocation() {
        let s = XArchiveState::default();
        let d = draft(&s);
        let root = tempfile::tempdir().unwrap();
        let mut q = request(&d);
        q.selected_ids = vec!["999".into()];
        assert!(s.save(root.path(), q).is_err());
        s.invalidate_drafts();
        assert!(s.save(root.path(), request(&d)).is_err());
    }
    #[test]
    fn reservation_is_immediately_counted_cancellable_and_released() {
        let state = XArchiveState::default();
        let draft = draft(&state);
        let req = request(&draft);
        let id = req.request_id.clone();
        let ticket = state.reserve(req).unwrap();
        assert!(active_ids().contains(&id));
        assert!(state.cancel(&id));
        let root = tempfile::tempdir().unwrap();
        assert!(state.save_reserved(root.path(), ticket).is_err());
        assert!(!active_ids().contains(&id));
        assert!(list(root.path()).unwrap().is_empty());
    }
    #[test]
    fn archive_progress_reports_scoped_ordered_stages_and_success_only_after_reopen() {
        let state = XArchiveState::default();
        let draft = draft(&state);
        let request = request(&draft);
        let request_id = request.request_id.clone();
        let reserved = state.reserve(request).unwrap();
        let root = tempfile::tempdir().unwrap();
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let sink = snapshots.clone();
        let saved_root = root.path().to_path_buf();
        let record = state.save_reserved_with_progress(root.path(), reserved, move |value| {
            if value.stage == ArchiveProgressStage::Complete {
                let records = list(&saved_root).unwrap();
                assert_eq!(records.len(), 1);
                assert_eq!(read(&saved_root, &records[0].id).unwrap().capture_id, value.capture_id);
            }
            sink.lock().unwrap().push(value);
        }).unwrap();
        let snapshots = snapshots.lock().unwrap();
        assert_eq!(snapshots.iter().map(|p| p.stage).collect::<Vec<_>>(), vec![
            ArchiveProgressStage::Preparing, ArchiveProgressStage::Writing, ArchiveProgressStage::Complete,
        ]);
        for (index, value) in snapshots.iter().enumerate() {
            assert_eq!(value.sequence, index as u64 + 1);
            assert_eq!(value.request_id, request_id);
            assert_eq!(value.capture_id, draft.capture_id);
            assert_eq!(value.account_id, draft.account_id);
            assert_eq!((value.processed_media, value.total_media, value.downloaded_bytes), (0, 0, 0));
            assert_eq!(value.total_bytes, None);
            assert!(!value.partial);
        }
        assert_eq!(list(root.path()).unwrap()[0].account_id, record.account_id);
        let serialized = serde_json::to_value(&snapshots[0]).unwrap();
        assert_eq!(serialized["stage"], "preparing");
        assert_eq!(serialized["accountId"], "account_test");
        assert_eq!(serialized["totalBytes"], serde_json::Value::Null);
        assert_eq!(serialized["currentTransfer"], serde_json::Value::Null);
    }
    #[test]
    fn chooser_snapshot_joins_fifo_and_reports_waiting_before_verified_completion() {
        let state = XArchiveState::default();
        let draft = draft(&state);
        let first = state.reserve(request(&draft)).unwrap();
        let second = state.reserve(request(&draft)).unwrap();
        let request = request(&draft);
        let pending = state.reserve_queued(request.request_id.clone()).unwrap();
        let reserved = state.prepare_queued(pending, request).unwrap();
        let holders = Arc::new(Mutex::new(Some((first, second))));
        let samples = Arc::new(Mutex::new(Vec::new()));
        let sink = samples.clone();
        let root = tempfile::tempdir().unwrap();
        let result = state.save_reserved_with_progress(root.path(), reserved, move |value| {
            if value.stage == ArchiveProgressStage::Queued {
                drop(holders.lock().unwrap().take());
            }
            sink.lock().unwrap().push(value);
        }).unwrap();
        let samples = samples.lock().unwrap();
        assert_eq!(samples.iter().map(|value| value.stage).collect::<Vec<_>>(), [
            ArchiveProgressStage::Queued, ArchiveProgressStage::Preparing,
            ArchiveProgressStage::Writing, ArchiveProgressStage::Complete,
        ]);
        assert!(samples.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));
        assert_eq!(read(root.path(), &result.id).unwrap().capture_id, draft.capture_id);
    }
    #[test]
    fn chooser_waiting_cancellation_reports_terminal_without_starting_a_save() {
        let state = XArchiveState::default();
        let draft = draft(&state);
        let _first = state.reserve(request(&draft)).unwrap();
        let _second = state.reserve(request(&draft)).unwrap();
        let request = request(&draft);
        let id = request.request_id.clone();
        let pending = state.reserve_queued(id.clone()).unwrap();
        let reserved = state.prepare_queued(pending, request).unwrap();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let sink = samples.clone();
        let cancel_state = state.clone();
        let root = tempfile::tempdir().unwrap();
        assert_eq!(state.save_reserved_with_progress(root.path(), reserved, move |value| {
            if value.stage == ArchiveProgressStage::Queued { assert!(cancel_state.cancel(&id)); }
            sink.lock().unwrap().push(value);
        }).unwrap_err(), CANCELLED);
        assert_eq!(samples.lock().unwrap().iter().map(|value| value.stage).collect::<Vec<_>>(), [
            ArchiveProgressStage::Queued, ArchiveProgressStage::Cancelled,
        ]);
        assert!(list(root.path()).unwrap().is_empty());
    }
    fn reporting_media_job(media: Vec<CaptureMedia>) -> (ArchiveJob, Arc<ProgressReporter>, Arc<Mutex<Vec<ArchiveProgress>>>) {
        let mut reserved = media_job(media);
        let samples = Arc::new(Mutex::new(Vec::new()));
        let sink = samples.clone();
        let reporter = Arc::new(ProgressReporter::new(&reserved, move |sample| sink.lock().unwrap().push(sample)));
        reserved.job.progress = Some(reporter.clone());
        (reserved, reporter, samples)
    }
    #[test]
    fn logical_progress_excludes_avatar_and_poster_without_removing_saved_helpers() {
        let image = captured_media("image", "https://pbs.twimg.com/media/one.jpg", None);
        let avatar = captured_media("avatar", "https://pbs.twimg.com/profile_images/123/a_normal.jpg", None);
        let other_avatar = captured_media("avatar", "https://pbs.twimg.com/profile_images/456/b_normal.jpg", None);
        let poster_url = "https://pbs.twimg.com/ext_tw_video_thumb/123/pu/img/one.jpg";
        let video = captured_media("video", "https://video.twimg.com/ext_tw_video/123/pu/pl/root.m3u8", Some(poster_url));
        let poster = captured_media("poster", poster_url, None);
        for media in [vec![image.clone(), avatar.clone(), other_avatar], vec![video, poster, avatar.clone()]] {
            let (reserved, progress, _) = reporting_media_job(media);
            assert_eq!(reserved.draft.document.records[0].media.len(), 3);
            assert_eq!(progress.state.lock().unwrap().value.total_media, 1);
            for item in &reserved.draft.document.records[0].media {
                progress.update(ArchiveProgressStage::Downloading, None, logical_media(item), false, true);
            }
            assert_eq!(progress.state.lock().unwrap().value.processed_media, 1);
            assert_eq!(reserved.draft.document.records[0].media.len(), 3);
        }
        let mut reserved = media_job(vec![image, avatar]);
        assert_eq!(ProgressReporter::new(&reserved, |_| {}).state.lock().unwrap().value.total_media, 1);
        reserved.restrict_media("123", "https://pbs.twimg.com/media/one.jpg").unwrap();
        assert_eq!(ProgressReporter::new(&reserved, |_| {}).state.lock().unwrap().value.total_media, 1);
    }
    #[test]
    fn logical_progress_counts_unavailable_media_as_processed_not_successful() {
        let (reserved, progress, _) = reporting_media_job(vec![
            captured_media("image", "https://pbs.twimg.com/media/one.jpg", None),
            captured_media("image", "https://pbs.twimg.com/media/two.jpg", None),
            captured_media("unavailable_image", "", None),
            captured_media("unavailable_video", "", None),
            captured_media("avatar", "https://pbs.twimg.com/profile_images/123/a.jpg", None),
        ]);
        assert_eq!(progress.state.lock().unwrap().value.total_media, 4);
        for item in &reserved.draft.document.records[0].media {
            progress.update(ArchiveProgressStage::Downloading, None, logical_media(item), item.kind.starts_with("unavailable_"), true);
        }
        let state = progress.state.lock().unwrap();
        assert_eq!((state.value.processed_media, state.value.total_media), (4, 4));
        assert!(state.value.partial);assert_eq!(state.value.stage, ArchiveProgressStage::Downloading);
    }
    #[test]
    fn current_transfer_has_only_its_own_bytes_and_never_claims_a_whole_job_total() {
        let (_, progress, samples) = reporting_media_job(vec![captured_media("image", "https://pbs.twimg.com/media/one.jpg", None)]);
        {
            let _transfer = progress.begin_transfer(Some(10));
            progress.update(ArchiveProgressStage::Downloading, Some(4), false, false, true);
            let sample = samples.lock().unwrap().last().unwrap().clone();
            assert_eq!(sample.current_transfer, Some(CurrentTransferProgress { received_bytes: 4, total_bytes: Some(10) }));
            assert_eq!(sample.downloaded_bytes, 4);assert_eq!(sample.total_bytes, None);
            let wire = serde_json::to_value(sample).unwrap();
            assert_eq!(wire["currentTransfer"], serde_json::json!({"receivedBytes":4,"totalBytes":10}));
        }
        assert_eq!(samples.lock().unwrap().last().unwrap().current_transfer, None);
        {
            let _transfer = progress.begin_transfer(None);
            progress.update(ArchiveProgressStage::Downloading, Some(6), false, false, true);
            let sample = samples.lock().unwrap().last().unwrap().clone();
            assert_eq!(sample.current_transfer, Some(CurrentTransferProgress { received_bytes: 6, total_bytes: None }));
            assert_eq!(sample.downloaded_bytes, 10);assert_eq!(sample.total_bytes, None);
        }
        assert_eq!(samples.lock().unwrap().last().unwrap().current_transfer, None);
        let samples = samples.lock().unwrap();
        assert!(samples.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));
        assert!(samples.iter().all(|sample| sample.processed_media == 0 && sample.total_bytes.is_none()));
    }
    #[test]
    fn overlong_current_response_keeps_real_bytes_and_revokes_the_wrong_header_total() {
        let (_, progress, samples) = reporting_media_job(vec![captured_media("image", "https://pbs.twimg.com/media/one.jpg", None)]);
        let transfer = progress.begin_transfer(Some(2));
        progress.update(ArchiveProgressStage::Downloading, Some(3), false, false, true);
        let sample = samples.lock().unwrap().last().unwrap().clone();
        assert_eq!(sample.current_transfer, Some(CurrentTransferProgress { received_bytes: 3, total_bytes: None }));
        assert_eq!(sample.downloaded_bytes, 3);
        progress.update(ArchiveProgressStage::Failed, None, false, true, true);
        assert_eq!(samples.lock().unwrap().last().unwrap().current_transfer, None);
        let count = samples.lock().unwrap().len();drop(transfer);
        assert_eq!(samples.lock().unwrap().len(), count);
    }
    struct ChunkedProgressReader {
        bytes: std::io::Cursor<Vec<u8>>,
        progress: Arc<ProgressReporter>,
        seen: Vec<Option<CurrentTransferProgress>>,
    }
    impl Read for ChunkedProgressReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.seen.push(self.progress.state.lock().unwrap().value.current_transfer.clone());
            let length = buffer.len().min(2);
            self.bytes.read(&mut buffer[..length])
        }
    }
    #[test]
    fn response_read_tracks_header_and_real_chunks_then_clears_before_asset_validation() {
        let (reserved, progress, samples) = reporting_media_job(vec![captured_media("image", "https://pbs.twimg.com/media/one.jpg", None)]);
        let mut reader = ChunkedProgressReader { bytes: std::io::Cursor::new(vec![1, 2, 3, 4]), progress, seen: vec![] };
        let mut total = 0;
        assert_eq!(read_transfer(&mut reader, Some(4), 8, &reserved.job, &mut total).unwrap(), [1, 2, 3, 4]);
        assert_eq!(reader.seen, [0, 2, 4].map(|received_bytes| Some(CurrentTransferProgress { received_bytes, total_bytes: Some(4) })));
        assert_eq!(total, 4);
        let samples = samples.lock().unwrap();let last = samples.last().unwrap();
        assert_eq!(last.downloaded_bytes, 4);assert_eq!(last.current_transfer, None);
        assert_eq!(last.processed_media, 0);assert_eq!(last.stage, ArchiveProgressStage::Downloading);
        assert!(!samples.iter().any(|sample| sample.stage == ArchiveProgressStage::Complete));
    }
    #[test]
    fn missing_and_inaccurate_lengths_do_not_invent_totals_or_accept_truncated_bytes() {
        for (declared, limit, success, received) in [(None, 8, true, 3), (Some(5), 8, false, 3), (Some(2), 8, false, 3), (None, 2, false, 3), (Some(0), 8, false, 0), (Some(9), 8, false, 0)] {
            let (reserved, progress, _) = reporting_media_job(vec![captured_media("image", "https://pbs.twimg.com/media/one.jpg", None)]);
            let result = read_transfer(&mut std::io::Cursor::new(vec![1, 2, 3]), declared, limit, &reserved.job, &mut 0);
            assert_eq!(result.is_ok(), success);
            let state = progress.state.lock().unwrap();
            assert_eq!(state.value.downloaded_bytes, received);assert_eq!(state.value.current_transfer, None);
            assert_eq!(state.value.total_bytes, None);assert_eq!(state.value.processed_media, 0);
        }
    }
    struct InterruptedProgressReader {
        first: bool,
        cancel: Option<Arc<AtomicBool>>,
    }
    impl Read for InterruptedProgressReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.first { return Err(std::io::Error::other("fixture read failure")); }
            self.first = false;buffer[0] = 1;
            if let Some(cancel) = &self.cancel { cancel.store(true, Ordering::Release); }
            Ok(1)
        }
    }
    #[test]
    fn transfer_metadata_clears_on_read_error_and_cancel_without_discarding_received_bytes() {
        for cancel in [false, true] {
            let (reserved, progress, samples) = reporting_media_job(vec![captured_media("image", "https://pbs.twimg.com/media/one.jpg", None)]);
            let mut reader = InterruptedProgressReader { first: true, cancel: cancel.then(|| reserved.job.cancel.clone()) };
            let error = read_transfer(&mut reader, Some(4), 8, &reserved.job, &mut 0).unwrap_err();
            assert_eq!(error, if cancel { CANCELLED } else { "media_read_failed" });
            let state = progress.state.lock().unwrap();
            assert_eq!(state.value.downloaded_bytes, 1);assert_eq!(state.value.current_transfer, None);
            assert_eq!(state.value.processed_media, 0);
            assert!(!samples.lock().unwrap().iter().any(|sample| sample.stage == ArchiveProgressStage::Complete));
        }
    }
    #[test]
    fn sequential_hls_responses_reset_transfer_bytes_but_count_one_logical_video() {
        let (reserved, progress, samples) = reporting_media_job(vec![captured_media("video", "https://video.twimg.com/ext_tw_video/123/pu/pl/root.m3u8", None)]);
        let mut total = 0;
        for size in [1, 2, 3] {
            read_transfer(&mut std::io::Cursor::new(vec![1; size]), Some(size as u64), 8, &reserved.job, &mut total).unwrap();
        }
        progress.update(ArchiveProgressStage::Downloading, None, true, false, true);
        let samples = samples.lock().unwrap();let last = samples.last().unwrap();
        assert_eq!((last.processed_media, last.total_media, last.downloaded_bytes), (1, 1, 6));
        assert_eq!(last.current_transfer, None);assert_eq!(last.total_bytes, None);
        assert_eq!(samples.iter().filter(|sample| sample.current_transfer.as_ref().is_some_and(|transfer| transfer.received_bytes == 0)).count(), 3);
        assert!(samples.iter().all(|sample| sample.total_bytes.is_none()));
    }
    #[test]
    fn archive_progress_preserves_real_byte_counts_throttles_chunks_and_ignores_late_updates() {
        let reserved = media_job(vec![captured_media("image", "https://pbs.twimg.com/media/one.jpg", None)]);
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let sink = snapshots.clone();
        let progress = ProgressReporter::new(&reserved, move |value| sink.lock().unwrap().push(value));
        progress.update(ArchiveProgressStage::Preparing, None, false, false, true);
        progress.update(ArchiveProgressStage::Downloading, Some(64), false, false, false);
        progress.update(ArchiveProgressStage::Downloading, Some(64), false, false, false);
        assert_eq!(snapshots.lock().unwrap().len(), 1);
        progress.update(ArchiveProgressStage::Downloading, None, true, true, true);
        progress.update(ArchiveProgressStage::Writing, None, false, false, true);
        progress.update(ArchiveProgressStage::Complete, None, false, true, true);
        progress.update(ArchiveProgressStage::Downloading, Some(256), true, false, true);
        progress.update(ArchiveProgressStage::Failed, None, false, false, true);
        let snapshots = snapshots.lock().unwrap();
        assert_eq!(snapshots.len(), 4);
        let final_state = snapshots.last().unwrap();
        assert_eq!(final_state.sequence, 4);
        assert_eq!(final_state.downloaded_bytes, 128);
        assert_eq!((final_state.processed_media, final_state.total_media), (1, 1));
        assert!(final_state.partial);
        assert_eq!(final_state.total_bytes, None);
        assert_eq!(final_state.stage, ArchiveProgressStage::Complete);
    }
    #[test]
    fn archive_progress_cancel_and_write_failure_never_emit_complete() {
        for cancel in [false, true] {
            let state = XArchiveState::default();
            let draft = draft(&state);
            let request = request(&draft);
            let request_id = request.request_id.clone();
            let reserved = state.reserve(request).unwrap();
            let root = tempfile::tempdir().unwrap();
            let file_root = root.path().join("not-a-directory");
            fs::write(&file_root, b"keep").unwrap();
            if cancel { state.cancel(&request_id); }
            let snapshots = Arc::new(Mutex::new(Vec::new()));
            let sink = snapshots.clone();
            assert!(state.save_reserved_with_progress(&file_root, reserved, move |value| sink.lock().unwrap().push(value)).is_err());
            let snapshots = snapshots.lock().unwrap();
            // A ticket cancelled before execution never reports that file
            // preparation started. A real write failure follows preparation.
            assert_eq!(snapshots.iter().map(|value| value.stage).collect::<Vec<_>>(), if cancel {
                vec![ArchiveProgressStage::Cancelled]
            } else { vec![ArchiveProgressStage::Preparing, ArchiveProgressStage::Failed] });
            assert_eq!(snapshots.last().unwrap().stage, if cancel { ArchiveProgressStage::Cancelled } else { ArchiveProgressStage::Failed });
            assert!(!snapshots.iter().any(|value| value.stage == ArchiveProgressStage::Complete));
            assert_eq!(fs::read(&file_root).unwrap(), b"keep");
        }
    }
    #[test]
    fn archive_progress_partial_is_a_truthful_committed_result_not_full_success() {
        let state = XArchiveState::default();
        let mut document = document();
        document.records[0].truncated = true;
        let draft = state.freeze(&serde_json::to_string(&document).unwrap(), &document.source_url, "account_test", 0).unwrap();
        let reserved = state.reserve(request(&draft)).unwrap();
        let root = tempfile::tempdir().unwrap();
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let sink = snapshots.clone();
        let record = state.save_reserved_with_progress(root.path(), reserved, move |value| sink.lock().unwrap().push(value)).unwrap();
        assert!(!record.selected_content_complete);
        let snapshots = snapshots.lock().unwrap();
        assert_eq!(snapshots.last().unwrap().stage, ArchiveProgressStage::Complete);
        assert!(snapshots.last().unwrap().partial);
        assert!(snapshots.iter().find(|value| value.stage == ArchiveProgressStage::Writing).unwrap().partial);
    }
    #[test]
    fn duplicate_reservation_cannot_drop_the_original() {
        let state = XArchiveState::default();
        let draft = draft(&state);
        let req = request(&draft);
        let id = req.request_id.clone();
        let ticket = state.reserve(req.clone()).unwrap();
        assert!(state.reserve(req).is_err());
        assert!(active_ids().contains(&id));
        drop(ticket);
        assert!(!active_ids().contains(&id));
    }
    #[test]
    fn never_labels_html_error_payload_a_video_segment() {
        for ext in ["mp4", "m4s", "ts", "aac", "vtt"] {
            assert!(!valid_segment(b"<html>server error</html>", ext));
        }
        let mut packet = vec![0u8; 188];
        packet[0] = 0x47;
        assert!(valid_segment(&packet, "ts"));
        packet.pop();
        assert!(!valid_segment(&packet, "ts"));
    }
    #[test]
    fn observed_x_subtitle_path_keeps_the_exact_video_scope() {
        let root = media_url(
            "https://video.twimg.com/amplify_video/123/pl/root.m3u8?tag=29&v=030&variant_version=1",
            true,
        )
        .unwrap();
        let subtitle = media_url(
            "https://video.twimg.com/subtitles/amplify_video/123/0/captions.vtt",
            true,
        )
        .unwrap();
        assert_eq!(video_scope(&root), video_scope(&subtitle));
        let other = media_url(
            "https://video.twimg.com/subtitles/amplify_video/456/0/captions.vtt",
            true,
        )
        .unwrap();
        assert_ne!(video_scope(&root), video_scope(&other));
        for url in [
            "https://evil.test/subtitles/amplify_video/123/0/captions.vtt",
            "https://video.twimg.com/amplify_video/123/captions.vtt",
            "https://video.twimg.com/subtitles/amplify_video/123/no-track/captions.vtt",
            "https://video.twimg.com/subtitles/amplify_video/123/0/captions.vtt?token=secret",
            "https://video.twimg.com/subtitles/amplify_video/123/0/captions.html",
        ] {
            assert!(media_url(url, true).is_err(), "{url}");
        }
    }
    #[test]
    fn webvtt_requires_text_header_and_well_formed_cue_times() {
        let source = "WEBVTT\r\n\r\n00:00:00.920 --> 00:00:01.920\r\n<X-word-ms ms=120,60 index=1>Caption</X-word-ms>\r\n\r\n2\r\n00:02.460 --> 00:03.501 align:start\r\nAnother caption\r\n";
        assert!(valid_segment(source.as_bytes(), "vtt"));
        for source in [
            "<html>server error</html>",
            "WEBVTT\n\n<script>alert(1)</script>",
            "WEBVTT\n\n00:00:02.000 --> 00:00:01.000\nBackwards",
            "WEBVTT\n\n00:00:61.000 --> 00:00:62.000\nInvalid seconds",
            "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nNul\0",
        ] {
            assert!(!valid_segment(source.as_bytes(), "vtt"));
        }
        assert!(!valid_segment(&[0xff, 0xfe], "vtt"));
        assert!(!segment_video_track(source.as_bytes(), "vtt"));
    }
    #[test]
    fn unavailable_optional_subtitles_do_not_destroy_video_audio_or_surviving_tracks() {
        let master = "#EXTM3U\n#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"en\",URI=\"sub.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=100,CODECS=\"mp4a.40.2,avc1.64001F\",SUBTITLES=\"subs\",AUDIO=\"audio\"\nvideo.m3u8";
        assert!(subtitle_reference(master, "sub.m3u8").unwrap());
        assert!(!subtitle_reference(master, "audio.m3u8").unwrap());
        let repaired = prune_subtitle_reference(master, "sub.m3u8").unwrap();
        assert!(!repaired.contains("SUBTITLES"));
        assert!(!repaired.contains("sub.m3u8"));
        assert!(repaired.contains("CODECS=\"mp4a.40.2,avc1.64001F\""));
        assert_eq!(
            playlist_refs(&repaired).unwrap(),
            ["audio.m3u8", "video.m3u8"]
        );
        let two = master.replace("#EXT-X-MEDIA:TYPE=AUDIO", "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"ko\",URI=\"ko.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO");
        let repaired = prune_subtitle_reference(&two, "sub.m3u8").unwrap();
        assert!(repaired.contains("SUBTITLES=\"subs\""));
        assert!(repaired.contains("ko.m3u8"));
        assert!(hls_attributes("#EXT-X-MEDIA:URI=\"one\",URI=\"two\"").is_err());
    }
    #[test]
    fn mixed_use_hls_uri_is_never_an_optional_subtitle() {
        let subtitles =
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"en\",URI=\"bad.m3u8\"";
        for required in [
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",URI=\"bad.m3u8\"",
            "#EXT-X-STREAM-INF:BANDWIDTH=50\nbad.m3u8",
            "#EXT-X-MAP:URI=\"bad.m3u8\"",
            "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=50,URI=\"bad.m3u8\"",
        ] {
            for pair in [
                format!("{subtitles}\n{required}"),
                format!("{required}\n{subtitles}"),
            ] {
                let source = format!("#EXTM3U\n{pair}\n#EXT-X-STREAM-INF:BANDWIDTH=100\ngood.m3u8");
                assert_eq!(
                    playlist_refs(&source).unwrap(),
                    ["bad.m3u8", "bad.m3u8", "good.m3u8"]
                );
                assert!(
                    !subtitle_reference(&source, "bad.m3u8").unwrap(),
                    "required use cannot be pruned as optional: {required}"
                );
            }
        }
        let only = format!(
            "#EXTM3U\n{subtitles}\n{}\n#EXT-X-STREAM-INF:BANDWIDTH=100\ngood.m3u8",
            subtitles.replace("NAME=\"en\"", "NAME=\"ko\"")
        );
        assert!(subtitle_reference(&only, "bad.m3u8").unwrap());
        assert_eq!(
            playlist_refs(&prune_subtitle_reference(&only, "bad.m3u8").unwrap()).unwrap(),
            ["good.m3u8"]
        );
    }
    #[test]
    fn published_hls_references_only_registered_local_assets_for_the_same_post() {
        let local = "asset-00000000-0000-4000-8000-000000000001.m3u8";
        let asset = ArchiveAsset {
            name: local.into(),
            mime: "application/vnd.apple.mpegurl".into(),
            bytes: 100,
            sha256: "a".repeat(64),
            post_id: "123".into(),
            kind: "video_playlist".into(),
            source_url: "https://video.twimg.com/amplify_video/123/pl/good.m3u8".into(),
            download_url: None,
        };
        let master =
            |reference: &str| format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\n{reference}");
        assert!(validate_local_playlist(&master(local), "123", &[asset.clone()]).is_ok());
        for reference in [
            "https://video.twimg.com/amplify_video/123/pl/bad.m3u8",
            "bad.m3u8",
            "../bad.m3u8",
            "asset-00000000-0000-4000-8000-000000000002.m3u8",
        ] {
            assert!(
                validate_local_playlist(&master(reference), "123", &[asset.clone()]).is_err(),
                "unreceived reference cannot be published"
            );
        }
        assert!(validate_local_playlist(&master(local), "456", &[asset.clone()]).is_err());
        let mut wrong_kind = asset.clone();
        wrong_kind.kind = "image".into();
        assert!(validate_local_playlist(&master(local), "123", &[wrong_kind]).is_err());
        let mut image = asset.clone();
        image.name = local.replace(".m3u8", ".jpg");
        image.mime = "image/jpeg".into();
        image.kind = "video_segment".into();
        assert!(validate_local_playlist(&master(&image.name), "123", &[image.clone()]).is_err());
        let mut invalid = asset;
        invalid.bytes = 0;
        assert!(validate_local_playlist(&master(local), "123", &[invalid]).is_err());
    }
    #[test]
    fn failed_optional_branch_cannot_contribute_video_evidence() {
        let mut root_proof = false;
        let failed: Result<(), String> = hls_branch(&mut root_proof, |branch| {
            *branch = true;
            Err("later_subtitle_child_failed".into())
        });
        assert!(failed.is_err());
        assert!(!root_proof, "discarded child must not prove root video");
        hls_branch(&mut root_proof, |branch| {
            *branch = true;
            Ok(())
        })
        .unwrap();
        assert!(root_proof);
        let _: Result<(), String> = hls_branch(&mut root_proof, |branch| {
            *branch = false;
            Err("unavailable".into())
        });
        assert!(
            root_proof,
            "later failed child must not erase earlier success"
        );
    }
    #[test]
    fn issue_cap_cannot_publish_a_record_that_read_will_reject() {
        let issue = || ArchiveIssue {
            post_id: "123".into(),
            kind: "subtitle".into(),
            reason: "unavailable".into(),
        };
        let mut issues = (0..1280).map(|_| issue()).collect();
        assert!(append_issue(&mut issues, issue()).is_err());
        assert_eq!(issues.len(), 1280);
        assert!(validate_issue_count(1280).is_ok());
        assert!(validate_issue_count(1281).is_err());
    }
    #[test]
    fn successful_cached_child_retains_proof_without_discarded_branch_leakage() {
        let node = HlsNode {
            name: "asset-video-init.mp4".into(),
            has_video: true,
        };
        let mut root_proof = false;
        let discarded: Result<String, String> = hls_branch(&mut root_proof, |child| {
            let _ = node.reuse(child);
            Err("later_child_failed".into())
        });
        assert!(discarded.is_err());
        assert!(!root_proof);
        assert_eq!(
            hls_branch(&mut root_proof, |child| Ok(node.reuse(child))).unwrap(),
            node.name
        );
        assert!(root_proof);
    }
    #[test]
    fn audio_only_container_is_not_a_proven_video() {
        fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(kind);
            v.extend_from_slice(body);
            v
        }
        let mut handler = vec![0; 8];
        handler.extend_from_slice(b"vide");
        let video = atom(
            b"moov",
            &atom(b"trak", &atom(b"mdia", &atom(b"hdlr", &handler))),
        );
        assert!(mp4_video_track(&video, 0));
        let mut audio = video.clone();
        let p = audio.windows(4).position(|x| x == b"vide").unwrap();
        audio[p..p + 4].copy_from_slice(b"soun");
        assert!(!mp4_video_track(&audio, 0));
        assert!(!mp4_video_track(b"garbage vide", 0));
    }
    #[test]
    fn immutable_offline_reopen_with_sanitized_html() {
        let s = XArchiveState::default();
        let d = draft(&s);
        let root = tempfile::tempdir().unwrap();
        let first = s.save(root.path(), request(&d)).unwrap();
        let second = s.save(root.path(), request(&d)).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(list(root.path()).unwrap().len(), 2);
        assert_eq!(
            read(root.path(), &first.id).unwrap().document.records[0].body,
            document().records[0].body
        );
        let mut html = open_asset(root.path(), &first.id, "index.html").unwrap();
        let len = html.bytes as usize;
        let text = String::from_utf8(html.read_range(0, len).unwrap()).unwrap();
        assert!(text.contains("&lt;script&gt;"));
        assert!(!text.contains("<script>"));
        assert!(text.contains("connect-src 'none'"));
        assert!(text.contains("script-src 'none'"));
        assert!(!text.contains("href=\"https:"));
        assert!(open_asset(root.path(), &first.id, "../archive.json").is_err());
    }
    fn presentation_document() -> CaptureDocument {
        let mut raw = serde_json::to_value(document()).unwrap();
        raw["records"][0]["presentation"] = serde_json::json!({
            "handle":"demo","displayName":"Observed name","verified":true,"timestampLabel":"오후 1:00",
            "counts":{"replies":"4","reposts":"456","likes":"3.5천","bookmarks":"1.4천","views":"85.5만"},
            "quote":{"id":"777","url":"https://x.com/quoted/status/777","handle":"quoted",
                "displayName":"Quote name","verified":null,"body":"<b>quoted</b>","timestamp":"","timestampLabel":"8시간"},
            "relation":"root","conversationId":"123","order":0
        });
        let mut reply = raw["records"][0].clone();
        reply["id"] = "456".into();
        reply["url"] = "https://x.com/replier/status/456".into();
        reply["body"] = "Already loaded reply".into();
        reply["presentation"]["handle"] = "replier".into();
        reply["presentation"]["displayName"] = "Reply name".into();
        reply["presentation"]["timestampLabel"] = "44분".into();
        reply["presentation"]["relation"] = "reply".into();
        reply["presentation"]["order"] = 1.into();
        reply["presentation"].as_object_mut().unwrap().remove("quote");
        raw["records"].as_array_mut().unwrap().push(reply);
        serde_json::from_value(raw).unwrap()
    }
    #[test]
    fn historical_presentation_and_loaded_reply_survive_immutable_save_and_reopen() {
        let state = XArchiveState::default();
        let doc = presentation_document();
        assert!(validate_document(&doc).is_ok());
        let captured = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
        let mut request = request(&captured);
        request.selected_ids = archive_post_selection(&captured.document, "123");
        assert_eq!(request.selected_ids, ["123", "456"]);
        let root = tempfile::tempdir().unwrap();
        let saved = state.save(root.path(), request).unwrap();
        let restored = read(root.path(), &saved.id).unwrap();
        assert_eq!(serde_json::to_value(restored.document).unwrap(), serde_json::to_value(doc).unwrap());
        assert!(saved.selected_content_complete);
        // No live page, account, script or remote embed is needed to reopen.
        assert!(render_html(&saved).contains("script-src 'none'"));
        assert!(!render_html(&saved).contains("<b>quoted</b>"));
    }
    fn display_quote_document() -> CaptureDocument {
        let mut raw = serde_json::to_value(presentation_document()).unwrap();
        let view = &mut raw["records"][0]["presentation"];
        let mut quote = view.as_object_mut().unwrap().remove("quote").unwrap();
        quote.as_object_mut().unwrap().remove("id");
        quote.as_object_mut().unwrap().remove("url");
        quote["displayName"] = "asa 🍫".into();
        view["quotePresentation"] = quote;
        view["displayName"] = "🍫".into();
        view["mediaBeforeQuote"] = true.into();
        serde_json::from_value(raw).unwrap()
    }
    #[test]
    fn display_only_quote_emoji_and_observed_order_survive_new_immutable_save_and_reopen() {
        let doc = display_quote_document();
        assert!(validate_document(&doc).is_ok());
        assert_eq!(archive_post_selection(&doc, "123"), ["123", "456"]);
        assert!(archive_post_selection(&doc, "777").is_empty());
        let state = XArchiveState::default();
        let draft = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
        let mut save_request = request(&draft);
        save_request.selected_ids = vec!["123".into(), "456".into()];
        let root = tempfile::tempdir().unwrap();
        let saved = state.save(root.path(), save_request).unwrap();
        let restored = read(root.path(), &saved.id).unwrap();
        assert_eq!(serde_json::to_value(&restored.document).unwrap(), serde_json::to_value(&doc).unwrap());
        let view = restored.document.records[0].presentation.as_ref().unwrap();
        assert_eq!(view.display_name, "🍫");
        assert_eq!(view.media_before_quote, Some(true));
        assert!(view.quote.is_none());
        let quote = serde_json::to_value(view.quote_presentation.as_ref().unwrap()).unwrap();
        assert_eq!(quote["displayName"], "asa 🍫");
        assert!(quote.get("id").is_none() && quote.get("url").is_none());
    }
    #[test]
    fn display_only_quote_cannot_smuggle_identity_relationships_or_foreign_resources() {
        let raw = serde_json::to_value(display_quote_document()).unwrap();
        for field in ["id", "url", "relation", "conversationId", "script"] {
            let mut changed = raw.clone();
            changed["records"][0]["presentation"]["quotePresentation"][field] = "untrusted".into();
            assert!(serde_json::from_value::<CaptureDocument>(changed).is_err(), "{field}");
        }
        for (field, value) in [
            ("handle", serde_json::json!("not/a_handle")),
            ("handle", serde_json::json!("x".repeat(16))),
            ("displayName", serde_json::json!("x".repeat(481))),
            ("body", serde_json::json!("x".repeat(48001))),
            ("timestamp", serde_json::json!("x".repeat(81))),
            ("timestampLabel", serde_json::json!("x".repeat(321))),
            ("avatarUrl", serde_json::json!("https://pbs.twimg.com/profile_images/123/not-observed.jpg")),
            ("mediaUrls", serde_json::json!(["https://pbs.twimg.com/media/not-observed.jpg"])),
        ] {
            let mut changed = raw.clone();
            changed["records"][0]["presentation"]["quotePresentation"][field] = value;
            assert!(validate_document(&serde_json::from_value(changed).unwrap()).is_err(), "{field}");
        }
        let mut both = raw.clone();
        both["records"][0]["presentation"]["quote"] =
            serde_json::to_value(presentation_document()).unwrap()["records"][0]["presentation"]["quote"].clone();
        assert!(validate_document(&serde_json::from_value(both).unwrap()).is_err());
        let mut blank_handle = raw.clone();
        blank_handle["records"][0]["presentation"]["quotePresentation"]["handle"] = "".into();
        assert!(validate_document(&serde_json::from_value(blank_handle).unwrap()).is_ok());
        let mut no_quote = raw;
        no_quote["records"][0]["presentation"].as_object_mut().unwrap().remove("quotePresentation");
        assert!(validate_document(&serde_json::from_value(no_quote).unwrap()).is_err());
    }
    #[test]
    fn display_only_quote_references_stay_same_post_and_are_pruned_for_single_media() {
        let mut doc = display_quote_document();
        let avatar = "https://pbs.twimg.com/profile_images/123/observed.jpg";
        let image = "https://pbs.twimg.com/media/observed.jpg";
        let other = "https://pbs.twimg.com/media/other.jpg";
        doc.records[0].media = vec![
            CaptureMedia {kind:"avatar".into(), url:avatar.into(), alt:"".into(), poster:None},
            CaptureMedia {kind:"image".into(), url:image.into(), alt:"".into(), poster:None},
            CaptureMedia {kind:"image".into(), url:other.into(), alt:"".into(), poster:None},
        ];
        let quote = doc.records[0].presentation.as_mut().unwrap().quote_presentation.as_mut().unwrap();
        quote.avatar_url = Some(avatar.into());
        quote.media_urls = vec![image.into(), other.into()];
        assert!(validate_document(&doc).is_ok());
        let state = XArchiveState::default();
        let draft = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
        let mut save_request = request(&draft); save_request.intent = SaveIntent::Media;
        let mut job = state.reserve(save_request).unwrap();
        job.restrict_media("123", image).unwrap();
        let quote = job.draft.document.records[0].presentation.as_ref().unwrap().quote_presentation.as_ref().unwrap();
        assert!(quote.avatar_url.is_none());
        assert_eq!(quote.media_urls, [image]);
        assert!(validate_document(&job.draft.document).is_ok());
        // A file captured under another selected post is not this quote's asset.
        doc.records[1].media = std::mem::take(&mut doc.records[0].media);
        assert!(validate_document(&doc).is_err());
    }
    #[test]
    fn display_only_quote_video_enrichment_is_poster_scoped_and_legacy_fields_stay_absent() {
        let mut doc = display_quote_document();
        let poster = "https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/p.jpg";
        let video = "https://video.twimg.com/ext_tw_video/111/pu/pl/master.m3u8";
        let other = "https://video.twimg.com/ext_tw_video/222/pu/pl/master.m3u8";
        doc.records[0].media = vec![
            CaptureMedia {kind:"poster".into(),url:poster.into(),alt:"".into(),poster:None},
            CaptureMedia {kind:"video".into(),url:video.into(),alt:"".into(),poster:Some(poster.into())},
            CaptureMedia {kind:"video".into(),url:other.into(),alt:"".into(),poster:None},
        ];
        doc.records[0].presentation.as_mut().unwrap().quote_presentation.as_mut().unwrap().media_urls = vec![poster.into()];
        include_observed_video_references(&mut doc.records[0]);
        include_observed_video_references(&mut doc.records[0]);
        assert_eq!(doc.records[0].presentation.as_ref().unwrap().quote_presentation.as_ref().unwrap().media_urls, [poster, video]);
        assert!(validate_document(&doc).is_ok());
        let legacy = serde_json::to_value(presentation_document()).unwrap();
        let view = &legacy["records"][0]["presentation"];
        assert!(view.get("quotePresentation").is_none() && view.get("mediaBeforeQuote").is_none());
    }
    #[test]
    fn native_default_save_expands_only_the_exact_loaded_detail_conversation() {
        let mut doc = presentation_document();
        let mut unknown = doc.records[1].clone();
        unknown.id = "999".into(); unknown.url = "https://x.com/replier/status/999".into();
        let proof = unknown.presentation.as_mut().unwrap();
        proof.relation = CaptureRelation::Unknown; proof.conversation_id = None; proof.order = 2;
        doc.records.push(unknown);
        assert_eq!(archive_post_selection(&doc, "123"), ["123", "456"]);
        assert_eq!(archive_post_selection(&doc, "456"), ["456"]);
        assert_eq!(archive_post_selection(&doc, "999"), ["999"]);
        assert!(archive_post_selection(&doc, "555").is_empty());
        doc.source_url = "https://x.com/home".into();
        assert_eq!(archive_post_selection(&doc, "123"), ["123"]);
        doc.source_url = "https://x.com/demo/status/123/photo/1".into();
        assert_eq!(archive_post_selection(&doc, "123"), ["123"]);
        doc.source_url = "https://x.com/demo/status/123".into();
        doc.records[0].presentation = None;
        assert_eq!(archive_post_selection(&doc, "123"), ["123"]);
    }
    #[test]
    fn historical_presentation_rejects_forged_identity_relationships_and_unbounded_values() {
        let raw = serde_json::to_value(presentation_document()).unwrap();
        for (field, value) in [
            ("handle", serde_json::json!("someone_else")),
            ("displayName", serde_json::json!("x".repeat(481))),
            ("timestampLabel", serde_json::json!("x".repeat(321))),
            ("order", serde_json::json!(80)),
            ("conversationId", serde_json::json!("456")),
            ("relation", serde_json::json!("reply")),
            ("relation", serde_json::json!("unknown")),
            ("counts", serde_json::json!({"replies":"x".repeat(129),"reposts":"","likes":"","bookmarks":"","views":""})),
            ("avatarUrl", serde_json::json!("https://pbs.twimg.com/profile_images/123/not-observed.jpg")),
            ("mediaUrls", serde_json::json!(["https://pbs.twimg.com/media/not-observed.jpg"])),
        ] {
            let mut changed = raw.clone();
            changed["records"][0]["presentation"][field] = value;
            assert!(validate_document(&serde_json::from_value(changed).unwrap()).is_err(), "{field}");
        }
        for (field, value) in [
            ("url", serde_json::json!("https://evil.test/quoted/status/777")),
            ("handle", serde_json::json!("not_quoted")),
            ("body", serde_json::json!("x".repeat(48001))),
            ("timestampLabel", serde_json::json!("x".repeat(321))),
            ("avatarUrl", serde_json::json!("https://pbs.twimg.com/profile_images/123/uncaptured.jpg")),
            ("mediaUrls", serde_json::json!(["https://pbs.twimg.com/media/not_observed.jpg"])),
        ] {
            let mut changed = raw.clone();
            changed["records"][0]["presentation"]["quote"][field] = value;
            assert!(validate_document(&serde_json::from_value(changed).unwrap()).is_err(), "{field}");
        }
        let mut duplicate = raw.clone();
        duplicate["records"][1]["presentation"]["order"] = 0.into();
        assert!(validate_document(&serde_json::from_value(duplicate).unwrap()).is_err());
        let mut unknown = raw;
        unknown["records"][0]["presentation"]["script"] = "not admitted".into();
        assert!(serde_json::from_value::<CaptureDocument>(unknown).is_err());
    }
    #[test]
    fn a_selected_loaded_reply_remains_readable_without_its_root_record() {
        let mut doc = presentation_document();
        doc.records.remove(0);
        assert!(validate_document(&doc).is_ok());
        assert_eq!(doc.records[0].presentation.as_ref().unwrap().relation, CaptureRelation::Reply);
        let state = XArchiveState::default();
        let captured = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
        let mut request = request(&captured); request.selected_ids = vec!["456".into()];
        let root = tempfile::tempdir().unwrap();
        let saved = state.save(root.path(), request).unwrap();
        assert_eq!(read(root.path(), &saved.id).unwrap().document.records.len(), 1);
    }
    #[test]
    fn quote_avatar_is_only_an_existing_same_post_asset_and_single_media_narrowing_clears_it() {
        let mut doc = presentation_document();
        let avatar = "https://pbs.twimg.com/profile_images/123/observed.jpg";
        let image = "https://pbs.twimg.com/media/observed.jpg";
        doc.records[0].media = vec![
            CaptureMedia {kind:"avatar".into(), url:avatar.into(), alt:"".into(), poster:None},
            CaptureMedia {kind:"image".into(), url:image.into(), alt:"".into(), poster:None},
        ];
        doc.records[0].presentation.as_mut().unwrap().quote.as_mut().unwrap().avatar_url = Some(avatar.into());
        doc.records[0].presentation.as_mut().unwrap().quote.as_mut().unwrap().media_urls = vec![image.into()];
        doc.records[0].presentation.as_mut().unwrap().avatar_url = Some(avatar.into());
        doc.records[0].presentation.as_mut().unwrap().media_urls = Some(vec![image.into()]);
        assert!(validate_document(&doc).is_ok());
        let state = XArchiveState::default();
        let captured = state.freeze(&serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0).unwrap();
        let mut request = request(&captured); request.intent = SaveIntent::Media;
        let mut job = state.reserve(request).unwrap();
        job.restrict_media("123", image).unwrap();
        assert!(job.draft.document.records[0].presentation.as_ref().unwrap().quote.as_ref().unwrap().avatar_url.is_none());
        assert_eq!(job.draft.document.records[0].presentation.as_ref().unwrap().quote.as_ref().unwrap().media_urls, [image]);
        assert!(job.draft.document.records[0].presentation.as_ref().unwrap().avatar_url.is_none());
        assert_eq!(job.draft.document.records[0].presentation.as_ref().unwrap().media_urls.as_deref(), Some([image.to_owned()].as_slice()));
        assert!(validate_document(&job.draft.document).is_ok());
    }
    #[test]
    fn shared_own_and_quote_media_occurrences_do_not_duplicate_the_captured_resource() {
        let mut doc = presentation_document();
        let image = "https://pbs.twimg.com/media/shared.jpg";
        let avatar = "https://pbs.twimg.com/profile_images/123/shared.jpg";
        doc.records[0].media = vec![
            CaptureMedia {kind:"image".into(),url:image.into(),alt:"Shared".into(),poster:None},
            CaptureMedia {kind:"avatar".into(),url:avatar.into(),alt:"Author".into(),poster:None},
        ];
        let view = doc.records[0].presentation.as_mut().unwrap();
        view.media_urls = Some(vec![image.into()]); view.avatar_url = Some(avatar.into());
        let quote = view.quote.as_mut().unwrap();
        quote.media_urls = vec![image.into()]; quote.avatar_url = Some(avatar.into());
        assert!(validate_document(&doc).is_ok());
        assert_eq!(doc.records[0].media.len(), 2);
        let raw = serde_json::to_value(&doc).unwrap();
        assert_eq!(raw["records"][0]["presentation"]["mediaUrls"], raw["records"][0]["presentation"]["quote"]["mediaUrls"]);
        assert_eq!(raw["records"][0]["presentation"]["avatarUrl"], raw["records"][0]["presentation"]["quote"]["avatarUrl"]);
        let restored: CaptureDocument = serde_json::from_value(raw).unwrap();
        assert!(validate_document(&restored).is_ok());
        assert_eq!(restored.records[0].media.len(), 2);
    }
    #[test]
    fn own_media_occurrence_absence_differs_from_an_explicit_empty_capture() {
        let mut doc = presentation_document();
        assert!(doc.records[0].presentation.as_ref().unwrap().media_urls.is_none());
        assert!(serde_json::to_value(&doc).unwrap()["records"][0]["presentation"].get("mediaUrls").is_none());
        doc.records[0].presentation.as_mut().unwrap().media_urls = Some(vec![]);
        let raw = serde_json::to_value(&doc).unwrap();
        assert_eq!(raw["records"][0]["presentation"]["mediaUrls"], serde_json::json!([]));
        let restored: CaptureDocument = serde_json::from_value(raw).unwrap();
        assert!(validate_document(&restored).is_ok());
        assert_eq!(restored.records[0].presentation.as_ref().unwrap().media_urls, Some(vec![]));
    }
    #[test]
    fn native_observed_video_enrichment_keeps_both_proved_occurrences_and_no_unrelated_video() {
        let mut doc = presentation_document();
        let poster = "https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/p.jpg";
        let video = "https://video.twimg.com/ext_tw_video/111/pu/pl/master.m3u8";
        let other = "https://video.twimg.com/ext_tw_video/222/pu/pl/master.m3u8";
        doc.records[0].media = vec![
            CaptureMedia {kind:"poster".into(),url:poster.into(),alt:"".into(),poster:None},
            CaptureMedia {kind:"video".into(),url:video.into(),alt:"".into(),poster:Some(poster.into())},
            CaptureMedia {kind:"video".into(),url:other.into(),alt:"".into(),poster:None},
        ];
        let view = doc.records[0].presentation.as_mut().unwrap();
        view.media_urls = Some(vec![poster.into()]);view.quote.as_mut().unwrap().media_urls = vec![poster.into()];
        include_observed_video_references(&mut doc.records[0]);
        include_observed_video_references(&mut doc.records[0]);
        let view = doc.records[0].presentation.as_ref().unwrap();
        assert_eq!(view.media_urls.as_ref().unwrap(), &[poster, video]);
        assert_eq!(view.quote.as_ref().unwrap().media_urls, [poster, video]);
        assert!(validate_document(&doc).is_ok());
    }
    fn photo_document_value() -> serde_json::Value {
        let mut value = serde_json::to_value(document()).unwrap();
        value["sourceUrl"] = serde_json::json!("https://x.com/demo/status/123/photo/1");
        let post = &mut value["records"][0];
        for field in ["author", "body", "timestamp", "counts", "context"] {
            post[field] = serde_json::json!("");
        }
        post["quotes"] = serde_json::json!([]);
        post["truncated"] = serde_json::json!(false);
        post["photoOnly"] = serde_json::json!(true);
        post["media"] = serde_json::json!([{
            "kind":"image","url":"https://pbs.twimg.com/media/example?format=png&name=large","alt":"Observed photo"
        }]);
        value
    }
    #[test]
    fn photo_only_capture_preserves_exact_scope_without_fabricated_text() {
        let doc: CaptureDocument = serde_json::from_value(photo_document_value()).unwrap();
        assert!(validate_document(&doc).is_ok());
        let encoded = serde_json::to_value(&doc).unwrap();
        assert_eq!(encoded["records"][0]["photoOnly"], true);
        for (field, value) in [
            ("body", serde_json::json!("not observed")),
            ("author", serde_json::json!("invented author")),
            ("timestamp", serde_json::json!("2026-10-20T00:00:00Z")),
            ("counts", serde_json::json!("1 like")),
            ("context", serde_json::json!("unobserved context")),
            ("quotes", serde_json::json!(["not observed"])),
            ("truncated", serde_json::json!(true)),
        ] {
            let mut raw = photo_document_value();
            raw["records"][0][field] = value;
            let changed: CaptureDocument = serde_json::from_value(raw).unwrap();
            assert!(validate_document(&changed).is_err(), "{field}");
        }
        for source in [
            "https://x.com/demo/status/456/photo/1",
            "https://x.com/other/status/123/photo/1",
            "https://x.com/demo/status/123",
            "https://x.com/demo/status/123/photo/5",
        ] {
            let mut raw = photo_document_value();
            raw["sourceUrl"] = serde_json::json!(source);
            assert!(validate_document(&serde_json::from_value(raw).unwrap()).is_err(), "{source}");
        }
        let mut raw = photo_document_value();
        raw["records"][0]["media"][0]["url"] = serde_json::json!("https://pbs.twimg.com/profile_images/avatar.jpg");
        assert!(validate_document(&serde_json::from_value(raw).unwrap()).is_err());
        let mut raw = photo_document_value();
        raw["records"][0]["media"].as_array_mut().unwrap().push(serde_json::json!({
            "kind":"image","url":"https://pbs.twimg.com/media/second.jpg","alt":""
        }));
        assert!(validate_document(&serde_json::from_value(raw).unwrap()).is_err());
        let mut raw = photo_document_value();
        raw["records"][0]["url"] = serde_json::json!("https://x.com/demo/status/123/");
        raw["sourceUrl"] = serde_json::json!("https://x.com/demo/status/123//photo/1");
        assert!(validate_document(&serde_json::from_value(raw).unwrap()).is_err());
    }
    #[test]
    fn older_captures_default_to_normal_post_scope() {
        let mut raw = serde_json::to_value(document()).unwrap();
        raw["records"][0].as_object_mut().unwrap().remove("photoOnly");
        let old: CaptureDocument = serde_json::from_value(raw).unwrap();
        assert!(validate_document(&old).is_ok());
        assert!(old.records[0].presentation.is_none());
        assert_eq!(serde_json::to_value(old).unwrap()["records"][0]["photoOnly"], false);
        let state = XArchiveState::default();
        let capture = draft(&state);
        let root = tempfile::tempdir().unwrap();
        let saved = state.save(root.path(), request(&capture)).unwrap();
        let mut old_record = serde_json::to_value(&saved).unwrap();
        old_record["document"]["records"][0].as_object_mut().unwrap().remove("photoOnly");
        let bytes = serde_json::to_vec_pretty(&old_record).unwrap();
        let folder = root.path().join(&saved.id);
        fs::write(folder.join("archive.json"), &bytes).unwrap();
        fs::write(folder.join("manifest.sha256"), digest(&bytes)).unwrap();
        let reopened = read(root.path(), &saved.id).unwrap();
        assert!(!reopened.document.records[0].photo_only);
        assert_eq!(reopened.html_sha256, saved.html_sha256);
    }
    #[test]
    fn transient_capture_diagnostic_is_closed_and_never_serialized() {
        let mut raw = serde_json::to_value(document()).unwrap();
        let shape = serde_json::json!({
            "photoRoute":true,"articles":0,"dialogs":0,"images":65535,"imagesOverLimit":true,
            "permittedImages":65535,"cssVisibleImages":0,"inViewportImages":0,"uniqueCandidates":65535,
            "opacityZeroImages":0,"zeroSizeImages":65535
        });
        raw["diagnostic"] = shape.clone();
        let parsed: CaptureDocument = serde_json::from_value(raw.clone()).unwrap();
        assert!(validate_document(&parsed).is_ok());
        let bounded = parsed.diagnostic.as_ref().unwrap().closed_shape();
        assert_eq!(bounded["images"], 129);
        assert_eq!(bounded["permittedImages"], 128);
        assert_eq!(bounded["uniqueCandidates"], 2);
        assert_eq!(bounded["zeroSizeImages"], 128);
        let serialized = serde_json::to_value(&parsed).unwrap();
        assert!(serialized.get("diagnostic").is_none());
        let restored: CaptureDocument = serde_json::from_value(serialized).unwrap();
        assert!(validate_document(&restored).is_ok());
        raw["diagnostic"]["url"] = serde_json::json!("not permitted");
        assert!(serde_json::from_value::<CaptureDocument>(raw.clone()).is_err());
        raw["diagnostic"] = shape;
        raw["diagnostic"]["images"] = serde_json::json!("not a count");
        assert!(serde_json::from_value::<CaptureDocument>(raw).is_err());
        assert_eq!(capture_script(false), SCRIPT);
        assert_eq!(SCRIPT.matches("const DIAGNOSTICS = false;").count(), 1);
        assert!(capture_script(true).contains("const DIAGNOSTICS = true;"));
    }
    fn quote_diagnostic_shape() -> serde_json::Value {
        let mut shape = serde_json::json!({
            "detailRoot": true, "quoteFound": true, "quoteVisible": true,
            "authorUnambiguous": true, "conflictingTargets": false,
            "selected": true, "admitted": false, "mediaFull": false,
            "quoteRoots": 65535, "nestedRoots": 65535, "aliases": 65535,
            "authorNodes": 65535, "contentNodes": 65535, "links": 65535,
            "images": 65535, "known": 65535, "knownInScope": 65535,
            "profiles": 65535, "notProfile": 65535, "bodyOrMedia": 65535,
            "nested": 65535, "visible": 65535, "painted": 65535,
            "hidden": 65535, "ariaHidden": 65535, "ancestorHidden": 65535,
            "opacityZero": 65535, "paintOwnStyle": 65535, "paintParent": 65535,
            "paintRect": 65535, "paintLayers": 65535, "paintShape": 65535,
            "paintMatch": 65535, "candidates": 65535
        });
        let object = shape.as_object_mut().unwrap();
        object.insert("selectedParentDiv".into(), true.into());
        object.insert("selectedAncestorComplete".into(), true.into());
        object.insert("selectedAncestorLimit".into(), false.into());
        for key in [
            "selectedParentChildren", "selectedAncestors", "selectedAncestorAriaHidden",
            "selectedAncestorHidden", "selectedAncestorInert", "selectedAncestorDisplayNone",
            "selectedAncestorVisibilityHidden", "selectedAncestorOpacityZero",
        ] {
            object.insert(key.into(), 65535.into());
        }
        shape
    }
    #[test]
    fn transient_quote_diagnostic_is_closed_bounded_and_never_serialized() {
        let mut raw = serde_json::to_value(document()).unwrap();
        let shape = quote_diagnostic_shape();
        raw["quoteDiagnostic"] = shape.clone();
        let parsed: CaptureDocument = serde_json::from_value(raw).unwrap();
        assert!(validate_document(&parsed).is_ok());
        let bounded = parsed.quote_diagnostic.as_ref().unwrap().closed_shape();
        let object = bounded.as_object().unwrap();
        assert_eq!(object.len(), 45);
        assert_eq!(object.values().filter(|value| value.is_boolean()).count(), 11);
        assert_eq!(object.values().filter(|value| value.is_u64()).count(), 34);
        for (key, original) in shape.as_object().unwrap() {
            if original.is_boolean() {
                assert_eq!(&bounded[key], original, "boolean {key} changed");
            } else {
                assert_eq!(bounded[key], 65, "counter {key} was not clamped");
            }
        }
        assert!(object.values().all(|value| value.is_boolean() || value.as_u64() == Some(65)));
        let serialized = serde_json::to_value(&parsed).unwrap();
        assert!(serialized.get("quoteDiagnostic").is_none());
        assert!(serialized.get("diagnostic").is_none());
        let restored: CaptureDocument = serde_json::from_value(serialized).unwrap();
        assert!(validate_document(&restored).is_ok());
        assert!(restored.quote_diagnostic.is_none());
        let draft = ArchiveDraft {
            capture_id: "capture_test".into(),
            account_id: "account_test".into(),
            document: parsed,
        };
        assert!(serde_json::to_value(draft).unwrap()["document"].get("quoteDiagnostic").is_none());
    }
    #[test]
    fn transient_quote_diagnostic_rejects_unknown_identity_text_and_nonclosed_types() {
        let mut base = serde_json::to_value(document()).unwrap();
        base["quoteDiagnostic"] = quote_diagnostic_shape();
        for key in ["url", "text", "owner", "postId", "html", "labels"] {
            let mut raw = base.clone();
            raw["quoteDiagnostic"][key] = serde_json::json!("not permitted");
            assert!(serde_json::from_value::<CaptureDocument>(raw).is_err(), "{key} accepted");
        }
        for (key, original) in quote_diagnostic_shape().as_object().unwrap() {
            let invalid = if original.is_boolean() {
                vec![
                    serde_json::json!(0), serde_json::json!("false"),
                    serde_json::json!([]), serde_json::json!({}), serde_json::Value::Null,
                ]
            } else {
                vec![
                    serde_json::json!(-1), serde_json::json!(65536), serde_json::json!(1.5),
                    serde_json::json!("5"), serde_json::json!(false),
                    serde_json::json!([]), serde_json::json!({}), serde_json::Value::Null,
                ]
            };
            for value in invalid {
                let mut raw = base.clone();
                raw["quoteDiagnostic"][key] = value;
                assert!(serde_json::from_value::<CaptureDocument>(raw).is_err(), "{key} type accepted");
            }
            let mut missing = base.clone();
            missing["quoteDiagnostic"].as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<CaptureDocument>(missing).is_err(), "{key} missing accepted");
        }
        for value in [serde_json::json!([]), serde_json::json!("not a shape"), serde_json::json!(false)] {
            let mut raw = base.clone();
            raw["quoteDiagnostic"] = value;
            assert!(serde_json::from_value::<CaptureDocument>(raw).is_err());
        }
    }
    #[test]
    fn absent_quote_diagnostic_preserves_legacy_document_shape() {
        let fixture = document();
        assert!(fixture.quote_diagnostic.is_none());
        let mut raw = serde_json::to_value(&fixture).unwrap();
        assert!(raw.get("quoteDiagnostic").is_none());
        let restored: CaptureDocument = serde_json::from_value(raw.clone()).unwrap();
        assert!(restored.quote_diagnostic.is_none());
        assert_eq!(serde_json::to_value(restored).unwrap(), raw);
        raw["quoteDiagnostic"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<CaptureDocument>(raw).unwrap().quote_diagnostic.is_none());
    }
    #[test]
    fn photo_only_requires_media_and_discloses_scope_in_portable_html() {
        let state = XArchiveState::default();
        let doc: CaptureDocument = serde_json::from_value(photo_document_value()).unwrap();
        let capture = state.freeze(
            &serde_json::to_string(&doc).unwrap(), &doc.source_url, "account_test", 0
        ).unwrap();
        let root = tempfile::tempdir().unwrap();
        assert_eq!(state.save(root.path(), request(&capture)).unwrap_err(),
            "사진만 표시된 화면은 사진 파일을 함께 보관해야 합니다.");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        let normal = draft(&state);
        let mut record = state.save(root.path(), request(&normal)).unwrap();
        record.document = doc;
        let html = render_html(&record);
        assert!(html.contains("현재 사진만 보관 · 본문·댓글 미포함"));
        assert!(html.contains("<h2>현재 사진</h2>"));
        assert!(!html.contains("Example"));
    }
    #[test]
    fn altered_file_or_manifest_rejected() {
        let s = XArchiveState::default();
        let d = draft(&s);
        let root = tempfile::tempdir().unwrap();
        let saved = s.save(root.path(), request(&d)).unwrap();
        fs::write(root.path().join(&saved.id).join("index.html"), b"changed").unwrap();
        assert!(read(root.path(), &saved.id).is_err());
    }
    #[test]
    fn generic_media_urls_remain_observed_and_never_gain_credentials() {
        let raw = "https://pbs.twimg.com/media/abc?format=jpg&name=large";
        assert_eq!(media_url(raw, false).unwrap().as_str(), raw);
        for u in [
            "https://pbs.twimg.com.evil.test/media/a.jpg",
            "https://u:p@pbs.twimg.com/media/a.jpg",
            "https://pbs.twimg.com/media/a.jpg?token=x",
            "https://127.0.0.1/a.jpg",
            "https://video.twimg.com/ext_tw_video/123/a.mp4?token=x",
        ] {
            assert!(media_url(u, u.contains("video.twimg")).is_err());
        }
    }
    #[test]
    fn original_image_request_preserves_observed_id_and_format_and_is_idempotent() {
        for (source, expected) in [
            ("https://pbs.twimg.com/media/Ab_c-1?format=jpg&name=small", "https://pbs.twimg.com/media/Ab_c-1?format=jpg&name=orig"),
            ("https://pbs.twimg.com/media/Ab_c-1.jpeg?name=thumb", "https://pbs.twimg.com/media/Ab_c-1?format=jpeg&name=orig"),
            ("https://pbs.twimg.com/media/Ab_c-1.png?format=png&name=900x900", "https://pbs.twimg.com/media/Ab_c-1?format=png&name=orig"),
            ("https://pbs.twimg.com/media/Ab_c-1?name=4096x4096&format=webp", "https://pbs.twimg.com/media/Ab_c-1?format=webp&name=orig"),
            ("https://pbs.twimg.com/media/Ab_c-1.gif", "https://pbs.twimg.com/media/Ab_c-1?format=gif&name=orig"),
            ("https://pbs.twimg.com/media/Ab_c-1?format=jpg&name=orig", "https://pbs.twimg.com/media/Ab_c-1?format=jpg&name=orig"),
        ] {
            assert_eq!(original_image_url(source).unwrap().as_str(), expected);
            assert_eq!(original_image_url(expected).unwrap().as_str(), expected);
            assert_eq!(media_url(source, false).unwrap().as_str(), source);
        }
    }
    #[test]
    fn original_image_request_rejects_unknown_format_id_query_or_origin() {
        for source in [
            "https://pbs.twimg.com/media/one", "https://pbs.twimg.com/media/one?name=large",
            "https://pbs.twimg.com/media/one.avif", "https://pbs.twimg.com/media/one.JPG",
            "https://pbs.twimg.com/media/one.jpg?format=png", "https://pbs.twimg.com/media/one.jpg?format=jpeg",
            "https://pbs.twimg.com/media/one.jpg:orig", "https://pbs.twimg.com/media/one.two.jpg",
            "https://pbs.twimg.com/media/path/one.jpg", "https://pbs.twimg.com/media/.jpg",
            "https://pbs.twimg.com/media/one?format=jpg&format=png", "https://pbs.twimg.com/media/one?format=jpg&name=small&name=orig",
            "https://pbs.twimg.com/media/one?format=jpg&token=value", "https://pbs.twimg.com/media/one?format=jpg&name=unseen",
            "https://pbs.twimg.com/media/one?format=%6apg", "https://pbs.twimg.com/media/%6fne?format=jpg",
            "https://pbs.twimg.com/media/../media/one.jpg", "https://pbs.twimg.com/media/one\\two.jpg",
            "https://pbs.twimg.com/media/one.jpg#fragment", "https://pbs.twimg.com/media/one.jpg\n",
            "https://pbs.twimg.com:443/media/one.jpg", "http://pbs.twimg.com/media/one.jpg",
            "https://user@pbs.twimg.com/media/one.jpg", "https://pbs.twimg.com.evil.test/media/one.jpg",
            "https://pbs.twimg.com/profile_images/one/photo.jpg", "https://pbs.twimg.com/ext_tw_video_thumb/123/pu/img/one.jpg",
            "https://video.twimg.com/ext_tw_video/123/one.jpg",
        ] {
            assert_eq!(original_image_url(source).unwrap_err(), "original_image_url_unavailable", "{source}");
        }
        let too_long = format!("https://pbs.twimg.com/media/{}?format=png", "a".repeat(257));
        assert!(original_image_url(&too_long).is_err());
    }
    fn original_test_png() -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2).write_to(&mut output, image::ImageFormat::Png).unwrap();
        output.into_inner()
    }
    #[test]
    fn original_image_saves_received_bytes_with_observed_identity_and_request_provenance() {
        let root = tempfile::tempdir().unwrap();
        let post = document().records.remove(0);
        let source = "https://pbs.twimg.com/media/one.png?name=small";
        let media = captured_media("image", source, None);
        let png = original_test_png();
        let mut assets = Vec::new();
        let mut requests = Vec::new();
        store_image_media(root.path(), &post, &media, &mut assets, |url| {
            requests.push(url.to_string()); Ok(png.clone())
        }).unwrap();
        assert_eq!(requests, ["https://pbs.twimg.com/media/one?format=png&name=orig"]);
        assert_eq!(assets.len(), 1);
        let asset = &assets[0];
        assert_eq!(asset.source_url, source);
        assert_eq!(asset.download_url.as_deref(), Some(requests[0].as_str()));
        assert_eq!(asset.mime, "image/png");
        assert_eq!(asset.bytes, png.len() as u64);
        assert_eq!(asset.sha256, digest(&png));
        assert_eq!(read_regular(&root.path().join(&asset.name), MAX_IMAGE).unwrap(), png);
        assert_eq!(media.url, source);
    }
    #[test]
    fn original_image_has_one_attempt_and_never_retries_display_size_or_hides_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let post = document().records.remove(0);
        let media = captured_media("image", "https://pbs.twimg.com/media/one?format=png&name=small", None);
        for failure in ["media_request_failed", "media_response_not_complete", "media_size_limit", "media_truncated", CANCELLED] {
            let mut count = 0;
            let mut assets = Vec::new();
            let result = store_image_media(root.path(), &post, &media, &mut assets, |url| {
                count += 1; assert_eq!(url.query(), Some("format=png&name=orig")); Err(failure.into())
            });
            assert_eq!(count, 1);
            assert_eq!(result.unwrap_err(), if failure == CANCELLED { CANCELLED } else { "original_image_request_failed" });
            assert!(assets.is_empty());
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn original_image_rejects_unproven_url_before_fetch_and_invalid_or_different_format_after_fetch() {
        let root = tempfile::tempdir().unwrap();
        let post = document().records.remove(0);
        let mut assets = Vec::new();
        let unknown = captured_media("image", "https://pbs.twimg.com/media/one?name=small", None);
        assert_eq!(store_image_media(root.path(), &post, &unknown, &mut assets, |_| panic!("unproved URL must not fetch")).unwrap_err(), "original_image_url_unavailable");
        let image = captured_media("image", "https://pbs.twimg.com/media/one?format=jpg&name=small", None);
        for bytes in [b"truncated image".to_vec(), original_test_png()] {
            let mut count = 0;
            assert_eq!(store_image_media(root.path(), &post, &image, &mut assets, |_| { count += 1; Ok(bytes) }).unwrap_err(), "original_image_validation_failed");
            assert_eq!(count, 1);
        }
        assert!(assets.is_empty());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn original_image_policy_does_not_upgrade_avatar_or_poster() {
        let root = tempfile::tempdir().unwrap();
        let post = document().records.remove(0);
        let mut assets = Vec::new();
        for (kind, source) in [
            ("avatar", "https://pbs.twimg.com/profile_images/123/own_normal.png"),
            ("poster", "https://pbs.twimg.com/ext_tw_video_thumb/123/pu/img/frame.png"),
            ("poster", "https://pbs.twimg.com/media/also_observed.png?name=small"),
        ] {
            store_image_media(root.path(), &post, &captured_media(kind, source, None), &mut assets, |url| {
                assert_eq!(url.as_str(), source); Ok(original_test_png())
            }).unwrap();
            let asset = assets.last().unwrap();
            assert_eq!(asset.source_url, source);
            assert_eq!(asset.download_url, None);
            assert!(serde_json::to_value(asset).unwrap().get("downloadUrl").is_none());
        }
    }
    #[test]
    fn original_provenance_is_same_captured_post_image_only_and_legacy_metadata_is_unchanged() {
        let source = "https://pbs.twimg.com/media/one?format=png&name=small";
        let mut document = document();
        document.records[0].media = vec![captured_media("image", source, None)];
        let mut asset = ArchiveAsset {
            name: format!("asset-{}.png", uuid::Uuid::new_v4()), mime: "image/png".into(), bytes: 1,
            sha256: "a".repeat(64), post_id: "123".into(), kind: "image".into(), source_url: source.into(),
            download_url: Some(original_image_url(source).unwrap().to_string()),
        };
        assert!(valid_asset_provenance(&document, &asset));
        asset.mime = "image/jpeg".into();
        assert!(!valid_asset_provenance(&document, &asset));
        asset.mime = "image/png".into();
        let png_name = asset.name.clone();
        asset.name = asset.name.replace(".png", ".jpg");
        assert!(!valid_asset_provenance(&document, &asset));
        asset.name = png_name;
        for changed in ["https://pbs.twimg.com/media/two?format=png&name=orig", "https://pbs.twimg.com/media/one?format=jpg&name=orig",
            "https://pbs.twimg.com/media/one?format=png&name=small", "https://pbs.twimg.com/media/one.png?name=orig",
            "https://other.test/media/one?format=png&name=orig", "file:///one.png", ""] {
            asset.download_url = Some(changed.into());
            assert!(!valid_asset_provenance(&document, &asset));
        }
        asset.download_url = Some(original_image_url(source).unwrap().to_string());
        for kind in ["avatar", "poster", "video", "video_hls", "video_segment"] {
            asset.kind = kind.into(); assert!(!valid_asset_provenance(&document, &asset));
        }
        asset.kind = "image".into(); asset.post_id = "456".into();
        assert!(!valid_asset_provenance(&document, &asset));
        asset.post_id = "123".into(); document.records[0].media[0].kind = "poster".into();
        assert!(!valid_asset_provenance(&document, &asset));
        asset.download_url = None;
        let legacy = serde_json::to_value(&asset).unwrap();
        assert!(legacy.get("downloadUrl").is_none());
        let decoded: ArchiveAsset = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(decoded.download_url, None);
        assert_eq!(serde_json::to_value(&decoded).unwrap(), legacy);
        assert!(valid_asset_provenance(&document, &decoded));
    }
    #[test]
    fn new_and_legacy_original_records_reopen_and_serve_local_verified_bytes() {
        let state = XArchiveState::default();
        let draft = draft(&state);
        let root = tempfile::tempdir().unwrap();
        let mut record = state.save(root.path(), request(&draft)).unwrap();
        let folder = root.path().join(&record.id);
        let source = "https://pbs.twimg.com/media/one.png?name=small";
        record.document.records[0].media.push(captured_media("image", source, None));
        let png = original_test_png();
        store_image_media(&folder, &record.document.records[0], &record.document.records[0].media[0], &mut record.assets, |_| Ok(png.clone())).unwrap();
        let publish = |record: &ArchiveRecord| {
            let bytes = serde_json::to_vec(record).unwrap();
            fs::write(folder.join("archive.json"), &bytes).unwrap();
            fs::write(folder.join("manifest.sha256"), digest(&bytes)).unwrap();
        };
        publish(&record);
        assert_eq!(read(root.path(), &record.id).unwrap().assets[0].source_url, source);
        let mut local = open_asset(root.path(), &record.id, &record.assets[0].name).unwrap();
        assert_eq!(local.read_range(0, png.len()).unwrap(), png);
        record.assets[0].download_url = Some("https://pbs.twimg.com/media/another?format=png&name=orig".into());
        publish(&record);
        assert!(read(root.path(), &record.id).is_err());
        assert!(open_asset(root.path(), &record.id, &record.assets[0].name).is_err());
        record.assets[0].download_url = None;
        publish(&record);
        let before = fs::read(folder.join("archive.json")).unwrap();
        assert_eq!(read(root.path(), &record.id).unwrap().assets[0].download_url, None);
        let mut local = open_asset(root.path(), &record.id, &record.assets[0].name).unwrap();
        assert_eq!(local.read_range(0, png.len()).unwrap(), png);
        assert_eq!(fs::read(folder.join("archive.json")).unwrap(), before);
    }
    #[test]
    fn inspect_diagnostics_are_fixed_categories_preserving_closed_query_validation() {
        for (url, category) in [
            (
                "https://pbs.twimg.com/media/example?format=jpg&name=16385x360",
                "media_image_name_dimensions",
            ),
            (
                "https://pbs.twimg.com/media/example?format=jpg&name=private-value",
                "media_image_name_other",
            ),
            (
                "https://pbs.twimg.com/media/example?token=secret-value",
                "media_image_query_key",
            ),
            ("https://other.test/media/example", "media_image_origin"),
            (
                "https://pbs.twimg.com/media/example?name=small&name=large",
                "media_image_duplicate_query",
            ),
        ] {
            assert!(media_url(url, false).is_err());
            assert_eq!(media_rejection_category(url, false), category);
            assert!(
                !category.contains("example")
                    && !category.contains("private-value")
                    && !category.contains("secret-value")
            );
        }
        assert!(media_url(
            "https://pbs.twimg.com/media/example?format=jpg&name=4096x4096",
            false
        )
        .is_ok());
    }
    #[test]
    fn observed_image_dimensions_are_bounded_and_never_rewritten_or_guessed() {
        for dimension in ["360x360", "640x360", "4096x4096", "16384x1", "1x16384"] {
            let observed =
                format!("https://pbs.twimg.com/media/example?format=jpg&name={dimension}");
            assert_eq!(media_url(&observed, false).unwrap().as_str(), observed);
            let mut captured = document();
            captured.records[0].media.push(CaptureMedia {
                kind: "image".into(),
                url: observed,
                alt: "".into(),
                poster: None,
            });
            assert!(validate_document(&captured).is_ok());
        }
        for dimension in [
            "0x1",
            "1x0",
            "01x10",
            "10x01",
            "-1x2",
            "+1x2",
            "16385x1",
            "1x16385",
            "999999999x1",
            "1x1x1",
            "1X1",
            "１x１",
            "1 x1",
            "1x1 ",
            "1xorig",
            "2e2x200",
        ] {
            let observed =
                format!("https://pbs.twimg.com/media/example?format=jpg&name={dimension}");
            assert!(!image_name_allowed(dimension), "{dimension}");
            // URL parsing trims a trailing literal space; the actual query
            // escape below preserves that input for the native validator.
            let escaped = observed.replace(' ', "%20");
            assert!(media_url(&escaped, false).is_err(), "{dimension}");
        }
        for observed in [
            "https://pbs.twimg.com.evil.test/media/example?format=jpg&name=360x360",
            "https://pbs.twimg.com/media/example?format=jpg&name=360x360&token=secret",
            "https://pbs.twimg.com/media/example?format=jpg&name=360x360&name=640x360",
            "https://pbs.twimg.com/media/example?format=html&name=360x360",
        ] {
            assert!(media_url(observed, false).is_err());
        }
    }
    #[test]
    fn live_encrypted_byterange_or_remote_unknown_hls_not_complete() {
        for p in [
            "#EXTM3U\n#EXTINF:2\na.ts",
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"secret\"\n#EXT-X-ENDLIST\na.ts",
            "#EXTM3U\n#EXT-X-BYTERANGE:5\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#UNKNOWN:URI=\"x\"\na.ts\n#EXT-X-ENDLIST",
        ] {
            assert!(playlist_refs(p).is_err());
        }
        let p = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2\na.m4s\n#EXT-X-ENDLIST";
        assert_eq!(playlist_refs(p).unwrap(), vec!["init.mp4", "a.m4s"]);
    }
    #[test]
    fn mp4_requires_complete_box_graph() {
        let mut data = vec![];
        for (kind, body) in [
            (b"ftyp", b"test".as_slice()),
            (b"moov", b"".as_slice()),
            (b"mdat", b"media".as_slice()),
        ] {
            data.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
            data.extend_from_slice(kind);
            data.extend_from_slice(body);
        }
        assert!(mp4_complete(&data));
        data.pop();
        assert!(!mp4_complete(&data));
        assert!(!mp4_complete(b"not video"));
    }
    #[test]
    fn no_network_or_private_state_in_capture_script() {
        for denied in [
            "fetch(",
            "XMLHttpRequest",
            "document.cookie",
            "localStorage",
            "sessionStorage",
            "__react",
            "innerHTML",
            "outerHTML",
        ] {
            assert!(!SCRIPT.contains(denied), "{denied}");
        }
        assert!(SCRIPT.contains("getEntriesByType('resource')"));
        assert!(SCRIPT.contains("loaded_dom_only"));
    }
    #[test]
    fn hidden_stage_is_not_a_completed_archive() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".partial-test")).unwrap();
        assert!(list(root.path()).unwrap().is_empty());
    }
}
