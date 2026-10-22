//! Closed, bounded native-document save intents. Source-supplied URLs are only
//! selectors within a fresh frozen capture, never arbitrary network/file input.
use super::*;
use super::super::archive::{archive_post_selection, ArchiveProgress, ArchiveProgressStage, CurrentTransferProgress, SaveIntent, SaveRequest};
use std::sync::OnceLock;
// Capturing is separate from the two execution slots: queued media is frozen
// before it waits for network execution, without flooding temporary drafts.
static CAPTURE_SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
static ACTIVE: OnceLock<Mutex<HashMap<(String, String), Arc<AtomicBool>>>> = OnceLock::new();
#[derive(Clone, Hash, PartialEq, Eq)]
struct ContentKey { profile: String, revision: u64, path: String, kind: String, media: Option<String> }
fn content_keys() -> &'static Mutex<std::collections::HashSet<ContentKey>> {
    static KEYS: OnceLock<Mutex<std::collections::HashSet<ContentKey>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}
struct NativeActionLease { key: (String, String), content: ContentKey }
impl Drop for NativeActionLease {
    fn drop(&mut self) {
        active().lock().unwrap_or_else(|p| p.into_inner()).remove(&self.key);
        content_keys().lock().unwrap_or_else(|p| p.into_inner()).remove(&self.content);
    }
}
type ReceiptScope = (String, String, u64);
static RECEIPTS: OnceLock<Mutex<HashMap<ReceiptScope, ReceiptBook>>> = OnceLock::new();
const RECEIPT_CAPACITY: usize = 4096;
fn admission_lane() -> &'static Mutex<()> {
    static LANE: OnceLock<Mutex<()>> = OnceLock::new();
    LANE.get_or_init(Default::default)
}
pub(super) fn base_post_path(path: &str) -> bool {
    post_path(path) && !path.contains("/photo/") && !path.contains("/video/")
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChooseRequest {
    id: String,
    generation: u64,
    account_id: String,
    feed: String,
    pane: &'static str,
    path: String,
    source_url: String,
}
fn choose_request(data: &Surfaces, pane: &Pane, path: &str, source: &Url) -> Option<ChooseRequest> {
    if !base_post_path(path) || !allowed(source) || source.as_str().len() > 2048
        || source.path().starts_with("/__atsumi_surface__/")
        || !pane.visible.load(Ordering::Acquire) || pane_state(pane) != "ready"
        || !registered_visible(data, pane) { return None; }
    Some(ChooseRequest {
        id: uuid::Uuid::new_v4().to_string(), generation: data.generation,
        account_id: pane.profile.clone(), feed: data.active.clone(),
        pane: if pane.feed == "preview" { "preview" } else { "feed" },
        path: path.into(), source_url: format!("{}{}", source.origin().ascii_serialization(), source.path()),
    })
}
pub(super) fn choose(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, path: &str) {
    if !interaction_allowed(session, pane) { return; }
    let Some(view) = app.get_webview(&pane.label) else { return; };
    let Ok(source) = view.url() else { return; };
    if !view.window().is_visible().unwrap_or(false) || view.window().is_minimized().unwrap_or(true) { return; }
    let request = {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        let Some(request) = choose_request(&data, pane, path, &source) else { return; };
        request
    };
    if !capture_pane_current(session, pane, request.generation, &request.account_id)
        || !view.url().is_ok_and(|url| url == source) { return; }
    // No content is trusted from this intent. The local selection tool freezes
    // the same pane and checks account/source/post before opening its overlay.
    let _ = app.emit_to("main", "x:archive-choose", request);
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Action {
    id: String,
    path: String,
    kind: String,
    media: Option<String>,
}
impl Action {
    pub(super) fn parse(value: &str) -> Option<Self> {
        let action: Self = serde_json::from_str(value).ok()?;
        if uuid::Uuid::parse_str(&action.id).ok()?.to_string() != action.id
            || !base_post_path(&action.path)
            || !matches!(action.kind.as_str(), "archive" | "all" | "one")
            || (action.kind == "one") != action.media.is_some()
            || action.media.as_ref().is_some_and(|v| v.is_empty() || v.len() > 2048) { return None; }
        Some(action)
    }
}
fn active() -> &'static Mutex<HashMap<(String,String), Arc<AtomicBool>>> { ACTIVE.get_or_init(Default::default) }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionProgress {
    sequence: u64,
    stage: &'static str,
    processed_media: usize,
    total_media: usize,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    current_transfer: Option<CurrentTransferProgress>,
    partial: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReceiptState { Active, Complete, Partial, Failed, Cancelled, Absent }
#[derive(Clone, Debug, Serialize)]
struct Receipt {
    state: ReceiptState,
    progress: Option<ActionProgress>,
    message: String,
}
impl Receipt {
    fn new(state: ReceiptState, message: &str) -> Self {
        Self { state, progress: None, message: message.chars().take(512).collect() }
    }
    fn capacity() -> Self {
        Self::new(ReceiptState::Failed, "이 화면의 저장 요청 기록 한도에 도달했습니다. 진행 중인 저장을 확인한 뒤 피드를 새로고침해 주세요.")
    }
}
#[derive(Default)]
struct ReceiptBook {
    entries: HashMap<String, Receipt>,
    final_progress: HashMap<String, ActionProgress>,
    sealed: bool,
}
impl ReceiptBook {
    fn claim(&mut self, id: &str) -> (Receipt, bool) {
        if let Some(receipt) = self.entries.get(id) { return (receipt.clone(), false); }
        if self.sealed || self.entries.len() >= RECEIPT_CAPACITY {
            self.sealed = true;
            return (Receipt::capacity(), false);
        }
        let receipt = Receipt::new(ReceiptState::Active, "저장 요청을 접수했습니다.");
        self.entries.insert(id.into(), receipt.clone());
        (receipt, true)
    }
    fn status(&mut self, id: &str) -> Receipt {
        if let Some(receipt) = self.entries.get(id) { return receipt.clone(); }
        if self.sealed || self.entries.len() >= RECEIPT_CAPACITY {
            self.sealed = true;
            return Receipt::capacity();
        }
        // A definitive absence is a tombstone, not a timeout guess. Never
        // discard it while this pane can still submit its original message.
        let receipt = Receipt::new(ReceiptState::Absent, "저장 요청이 접수되지 않았습니다. 필요하면 다시 선택해 주세요.");
        self.entries.insert(id.into(), receipt.clone());
        receipt
    }
    fn progress(&mut self, id: &str, value: ActionProgress) -> Option<Receipt> {
        let receipt = self.entries.get_mut(id)?;
        if receipt.state != ReceiptState::Active || self.final_progress.contains_key(id)
            || receipt.progress.as_ref().is_some_and(|old| old.sequence >= value.sequence
                || matches!(old.stage, "complete" | "failed" | "cancelled")) { return None; }
        // Terminal counters are truthful, but only the worker outcome has its
        // final explanatory error. Keep them private until that same atomic
        // terminal receipt, never emit active+terminal contradictory state.
        if matches!(value.stage, "complete" | "failed" | "cancelled") {
            self.final_progress.insert(id.into(), value);
            return None;
        }
        receipt.message = "저장 처리 중입니다.".into();
        receipt.progress = Some(value);
        Some(receipt.clone())
    }
    fn finish(&mut self, id: &str, state: ReceiptState, message: &str) -> Option<Receipt> {
        let receipt = self.entries.get_mut(id)?;
        if receipt.state != ReceiptState::Active && receipt.state != state { return Some(receipt.clone()); }
        if let Some(mut progress) = self.final_progress.remove(id) {
            progress.stage = match state { ReceiptState::Complete | ReceiptState::Partial => "complete",
                ReceiptState::Cancelled => "cancelled", _ => "failed" };
            progress.partial |= state == ReceiptState::Partial;
            receipt.progress = Some(progress);
        } else if let Some(progress) = &mut receipt.progress {
            progress.stage = match state { ReceiptState::Complete | ReceiptState::Partial => "complete",
                ReceiptState::Cancelled => "cancelled", _ => "failed" };
            progress.sequence = progress.sequence.saturating_add(1);
            progress.partial |= state == ReceiptState::Partial;
        }
        receipt.state = state;
        receipt.message = message.chars().take(512).collect();
        Some(receipt.clone())
    }
    fn has_active(&self) -> bool { self.entries.values().any(|r| r.state == ReceiptState::Active) }
    fn cancel_unstarted(&mut self, id: &str) -> Receipt {
        let (receipt, claimed) = self.claim(id);
        if claimed {
            self.finish(id, ReceiptState::Cancelled, "저장 요청을 시작하기 전에 취소했습니다.").unwrap()
        } else { receipt }
    }
}
fn receipts() -> &'static Mutex<HashMap<ReceiptScope, ReceiptBook>> { RECEIPTS.get_or_init(Default::default) }
fn receipt_scope(pane: &Pane) -> ReceiptScope { (pane.label.clone(), pane.profile.clone(), pane.revision) }
fn receipt_scope_current(session: &XSession, pane: &Pane) -> bool {
    if !is_current(session, pane) { return false; }
    // Known source identity/authentication loss is not readiness drift.
    // Raw loading (even a derived timeout) may still reconcile its own job.
    if matches!(pane.state.lock().unwrap_or_else(|p| p.into_inner()).as_str(), "ownerChanged" | "loginRequired") { return false; }
    session.registry.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|registry|
        registry.active == pane.profile && registry.accounts.iter().any(|account|
            account.id == pane.profile && account.handle.eq_ignore_ascii_case(&pane.owner)))
}
fn prune_receipts(session: &XSession) {
    let registered: std::collections::HashSet<_> = {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        data.feeds.values().chain(data.preview_tabs.iter().map(|tab| &tab.pane))
            .filter(|pane| pane.revision == session.revision.load(Ordering::Acquire))
            .map(|pane| receipt_scope(pane)).collect()
    };
    let active_panes: std::collections::HashSet<_> = active().lock().unwrap_or_else(|p| p.into_inner())
        .keys().map(|(label, _)| label.clone()).collect();
    receipts().lock().unwrap_or_else(|p| p.into_inner())
        .retain(|scope, book| registered.contains(scope) || active_panes.contains(&scope.0) || book.has_active());
}
fn deliver_receipt(session: &XSession, app: &AppHandle, pane: &Pane, id: &str, receipt: &Receipt) {
    if !receipt_scope_current(session, pane) { return; }
    let Some(view) = app.get_webview(&pane.label) else { return; };
    if !view.url().is_ok_and(|url| allowed(&url) && !url.path().starts_with("/__atsumi_surface__/")) { return; }
    let args = serde_json::json!([id,receipt]).to_string();
    let _ = view.eval(format!("window.__atsumiXActions?.receipt(...{args})"));
}
pub(super) fn canonical_request_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
pub(super) fn status(session: &XSession, app: &AppHandle, pane: &Pane, id: &str) {
    if !canonical_request_id(id) || !receipt_scope_current(session, pane) { return; }
    prune_receipts(session);
    let receipt = receipts().lock().unwrap_or_else(|p| p.into_inner())
        .entry(receipt_scope(pane)).or_default().status(id);
    deliver_receipt(session, app, pane, id, &receipt);
}
fn diagnostic(category: &'static str) {
    #[cfg(debug_assertions)]
    super::super::debug_shape("archive-action", category, &serde_json::json!({"observed":true}));
    #[cfg(not(debug_assertions))]
    let _ = category;
}
pub(super) fn rejected_submission() { diagnostic("submission-invalid"); }
fn project_progress(value: &ArchiveProgress, request: &str, capture: &str, account: &str) -> Option<ActionProgress> {
    const JS_INTEGER: u64 = 9_007_199_254_740_991;
    if value.request_id != request || value.capture_id != capture || value.account_id != account
        || value.sequence == 0 || value.sequence > JS_INTEGER || value.total_media > 4096
        || value.processed_media > value.total_media || value.downloaded_bytes > JS_INTEGER
        || value.total_bytes.is_some_and(|total| total < value.downloaded_bytes || total > JS_INTEGER)
        || value.current_transfer.as_ref().is_some_and(|transfer|
            transfer.received_bytes > value.downloaded_bytes || transfer.received_bytes > JS_INTEGER
            || transfer.total_bytes.is_some_and(|total| total < transfer.received_bytes || total > JS_INTEGER)) { return None; }
    Some(ActionProgress {
        sequence: value.sequence,
        stage: match value.stage { ArchiveProgressStage::Queued=>"queued", ArchiveProgressStage::Preparing=>"preparing", ArchiveProgressStage::Downloading=>"downloading",
            ArchiveProgressStage::Writing=>"writing", ArchiveProgressStage::Complete=>"complete",
            ArchiveProgressStage::Failed=>"failed", ArchiveProgressStage::Cancelled=>"cancelled" },
        processed_media:value.processed_media,total_media:value.total_media,downloaded_bytes:value.downloaded_bytes,
        total_bytes:value.total_bytes,current_transfer:value.current_transfer.clone(),partial:value.partial,
    })
}
fn progress(session: &XSession, app: &AppHandle, pane: &Pane, id: &str, value: ActionProgress) {
    // Retain frozen-job progress across route/visibility changes. The same
    // registered account/pane receives scalar state, never new save authority.
    let receipt = receipts().lock().unwrap_or_else(|p| p.into_inner())
        .get_mut(&receipt_scope(pane)).and_then(|book| book.progress(id, value));
    if let Some(receipt) = receipt { deliver_receipt(session, app, pane, id, &receipt); }
}
fn result(session: &XSession, app: &AppHandle, pane: &Pane, id: &str, state: ReceiptState, message: &str) {
    let receipt = receipts().lock().unwrap_or_else(|p| p.into_inner())
        .get_mut(&receipt_scope(pane)).and_then(|book| book.finish(id, state, message));
    diagnostic(match state { ReceiptState::Complete => "complete", ReceiptState::Partial => "partial",
        ReceiptState::Cancelled => "cancelled", _ => "failed" });
    if let Some(receipt) = receipt { deliver_receipt(session, app, pane, id, &receipt); }
}
pub(super) fn cancel(session: &XSession, app: &AppHandle, pane: &Pane, id: &str) {
    // A cancellation cannot fall between receipt claim and registration of
    // its exact queued work. No I/O or async wait occurs under this lane.
    let _admission=admission_lane().lock().unwrap_or_else(|p|p.into_inner());
    if !canonical_request_id(id) || !receipt_scope_current(session, pane) { return; }
    let receipt = receipts().lock().unwrap_or_else(|p| p.into_inner())
        .entry(receipt_scope(pane)).or_default().cancel_unstarted(id);
    if let Some(cancelled)=active().lock().unwrap_or_else(|p|p.into_inner()).get(&(pane.label.clone(),id.into())) {
        cancelled.store(true,Ordering::Release);
        session.archive.cancel(id);
    }
    diagnostic("cancel-requested");
    deliver_receipt(session, app, pane, id, &receipt);
}
pub(super) fn start(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, value: &str) {
    let Some(action)=Action::parse(value) else { return; };
    let _admission=admission_lane().lock().unwrap_or_else(|p|p.into_inner());
    if !receipt_scope_current(session, pane) { return; }
    prune_receipts(session);
    let (receipt, claimed) = receipts().lock().unwrap_or_else(|p| p.into_inner())
        .entry(receipt_scope(pane)).or_default().claim(&action.id);
    if !claimed {
        diagnostic("duplicate-or-tombstone");
        deliver_receipt(session, app, pane, &action.id, &receipt);
        return;
    }
    if !interaction_allowed(session, pane) {
        diagnostic("admission-not-ready");
        result(session, app, pane, &action.id, ReceiptState::Failed,
            "현재 화면을 확인할 수 없어 저장 요청을 시작하지 않았습니다. 화면이 준비된 뒤 다시 선택해 주세요.");
        return;
    }
    let Some(source)=app.get_webview(&pane.label).and_then(|view|view.url().ok()).filter(allowed) else {
        result(session,app,pane,&action.id,ReceiptState::Failed,STALE);return;
    };
    let content=ContentKey {profile:pane.profile.clone(),revision:pane.revision,
        path:action.path.to_ascii_lowercase(),kind:action.kind.clone(),media:action.media.clone()};
    let key=(pane.label.clone(),action.id.clone());
    let state=app.state::<AppState>();
    let pending=match state.begin_platform_work(|| {
        let mut contents=content_keys().lock().unwrap_or_else(|p|p.into_inner());
        if contents.contains(&content) { return Err("다른 화면에서 같은 저장 요청이 진행 중입니다. 기존 작업을 확인해 주세요.".into()); }
        let pending=session.archive.reserve_queued(action.id.clone())?;
        contents.insert(content.clone());
        Ok(pending)
    }) {
        Ok(pending)=>pending,
        Err(error)=>{diagnostic("admission-capacity");result(session,app,pane,&action.id,ReceiptState::Failed,&error);return;}
    };
    let cancelled=pending.cancellation();
    active().lock().unwrap_or_else(|p|p.into_inner()).insert(key.clone(),cancelled.clone());
    let lease=NativeActionLease{key,content};
    let generation=session.surfaces.lock().unwrap_or_else(|p|p.into_inner()).generation;
    diagnostic("accepted");
    deliver_receipt(session, app, pane, &action.id, &receipt);
    let host=session.clone();let app=app.clone();let pane=pane.clone();
    tauri::async_runtime::spawn(async move {
        let _lease=lease;
        let id=action.id.clone();
        let outcome=async {
            let acquire=CAPTURE_SLOTS.get_or_init(||Arc::new(tokio::sync::Semaphore::new(2))).clone().acquire_owned();
            tokio::pin!(acquire);
            let permit=loop {
                match tokio::time::timeout(std::time::Duration::from_millis(100), acquire.as_mut()).await {
                    Ok(value)=>break value.map_err(|_|ERROR.to_string())?,
                    Err(_)=>pending.check()?,
                }
            };
            let host=host.clone();let app=app.clone();let pane=pane.clone();
            tauri::async_runtime::spawn_blocking(move || -> Result<bool,String> {
                let capture_permit=permit;
                pending.check()?;
                if !capture_pane_current(&host,&pane,generation,&pane.profile)
                    || !app.get_webview(&pane.label).is_some_and(|view|view.url().is_ok_and(|current|current==source)) { return Err(STALE.into()); }
                diagnostic("capture-started");
                let target=if pane.feed=="preview" {"preview"} else {"feed"};
                let draft=inspect_archive(&host,&app,generation,target)?;
                let draft_lease=host.archive.lease_draft(&draft);
                diagnostic("capture-frozen");
                pending.check()?;
                let post=draft.document.records.iter().find(|p|p.url==format!("https://x.com{}",action.path)).ok_or("선택한 게시물이 화면에서 변경되었습니다. 다시 시도해 주세요.")?;
                let post_id=post.id.clone();
                let selected_ids=if action.kind=="archive" {archive_post_selection(&draft.document,&post_id)} else {vec![post_id.clone()]};
                let capture=draft.capture_id.clone();
                let request=SaveRequest {request_id:action.id.clone(),capture_id:draft.capture_id,
                    selected_ids,intent:if action.kind=="archive"{SaveIntent::Archive}else{SaveIntent::Media},include_media:true};
                let mut job=host.archive.prepare_queued(pending,request)?;
                drop(draft_lease);
                if let Some(media)=action.media.as_deref(){job.restrict_media(&post_id,media)?;}
                if !capture_pane_current(&host,&pane,generation,&pane.profile)
                    || !app.get_webview(&pane.label).is_some_and(|view|view.url().is_ok_and(|current|current==source)) { return Err(STALE.into()); }
                let root=app.path().app_data_dir().map_err(|_|ERROR)?.join("x-archives");
                progress(&host,&app,&pane,&action.id,ActionProgress {
                    sequence:0,stage:"queued",processed_media:0,total_media:0,downloaded_bytes:0,total_bytes:None,current_transfer:None,partial:false,
                });
                drop(capture_permit);
                // No DOM recapture or pane-visibility dependency after this
                // point. Epoch/cancel and the publication fence remain live.
                job.wait_for_turn()?;
                let progress_host=host.clone();let progress_app=app.clone();let progress_pane=pane.clone();let id=action.id.clone();
                diagnostic("save-started");
                let record=host.archive.save_reserved_with_progress(&root,job,move |value| {
                    if let Some(value)=project_progress(&value,&id,&capture,&progress_pane.profile) {
                        progress(&progress_host,&progress_app,&progress_pane,&id,value);
                    }
                })?;
                Ok(record.selected_content_complete)
            }).await.map_err(|_|ERROR.to_string())?
        }.await;
        let (state,message)=match outcome {Ok(true)=>(ReceiptState::Complete,"저장 완료".into()),
            Ok(false)=>(ReceiptState::Partial,"일부만 저장됨 · 누락된 내용이 있습니다.".into()),
            Err(error)=>(if cancelled.load(Ordering::Acquire){ReceiptState::Cancelled}else{ReceiptState::Failed},error)};
        result(&host,&app,&pane,&id,state,&message);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    fn receipt_progress(sequence: u64, stage: &'static str) -> ActionProgress {
        ActionProgress { sequence, stage, processed_media: 1, total_media: 2,
            downloaded_bytes: 1024, total_bytes: None, current_transfer: None, partial: false }
    }
    #[test]
    fn queued_receipt_precedes_worker_progress_and_survives_duplicate_ack_queries() {
        let mut book = ReceiptBook::default();
        assert!(book.claim("queued").1);
        let waiting = ActionProgress {sequence:0,stage:"queued",processed_media:0,total_media:0,
            downloaded_bytes:0,total_bytes:None,current_transfer:None,partial:false};
        assert_eq!(book.progress("queued", waiting.clone()).unwrap().progress.unwrap().stage, "queued");
        assert!(!book.claim("queued").1);
        assert_eq!(book.status("queued").progress.unwrap().stage, "queued");
        assert!(book.progress("queued", waiting).is_none());
        assert_eq!(book.progress("queued", receipt_progress(1, "preparing")).unwrap().progress.unwrap().sequence, 1);
        assert!(book.progress("queued", receipt_progress(0, "queued")).is_none());
        assert!(book.finish("queued", ReceiptState::Complete, "saved").is_some());
        assert!(book.progress("queued", receipt_progress(2, "downloading")).is_none());
    }
    #[test]
    fn queued_cancellation_keeps_receipt_tombstone_and_does_not_reexecute() {
        let mut book = ReceiptBook::default();book.claim("waiting");
        book.progress("waiting", receipt_progress(0, "queued"));
        assert_eq!(book.cancel_unstarted("waiting").state, ReceiptState::Active);
        let cancelled = book.finish("waiting", ReceiptState::Cancelled, "cancelled").unwrap();
        assert_eq!(cancelled.progress.unwrap().stage, "cancelled");
        assert!(!book.claim("waiting").1);
        assert_eq!(book.status("waiting").state, ReceiptState::Cancelled);
        assert_eq!(book.cancel_unstarted("not-yet-admitted").state, ReceiptState::Cancelled);
        assert!(!book.claim("not-yet-admitted").1);
    }
    #[test]
    fn pending_content_identity_is_account_revision_kind_and_exact_media_scoped() {
        let all = ContentKey {profile:"account".into(),revision:4,path:"/owner/status/12345".into(),kind:"all".into(),media:None};
        let mut keys = std::collections::HashSet::new();assert!(keys.insert(all.clone()));
        assert!(!keys.insert(all.clone()));
        assert!(keys.insert(ContentKey {profile:"other".into(),..all.clone()}));
        assert!(keys.insert(ContentKey {revision:5,..all.clone()}));
        assert!(keys.insert(ContentKey {kind:"archive".into(),..all.clone()}));
        assert!(keys.insert(ContentKey {kind:"one".into(),media:Some("https://pbs.twimg.com/media/one.jpg".into()),..all.clone()}));
        assert!(keys.insert(ContentKey {kind:"one".into(),media:Some("https://pbs.twimg.com/media/two.jpg".into()),..all}));
    }
    #[test]
    fn receipt_absence_tombstones_delayed_original_and_cannot_be_overwritten() {
        let mut book = ReceiptBook::default();
        assert_eq!(book.status("late").state, ReceiptState::Absent);
        assert!(!book.claim("late").1);
        assert!(book.progress("late", receipt_progress(1, "downloading")).is_none());
        assert_eq!(book.finish("late", ReceiptState::Complete, "late").unwrap().state, ReceiptState::Absent);
        assert_eq!(book.entries.len(), 1);
    }
    #[test]
    fn receipt_ack_loss_duplicate_and_terminal_loss_reconcile_without_reexecution() {
        let mut book = ReceiptBook::default();
        assert!(book.claim("job").1);
        assert_eq!(book.status("job").state, ReceiptState::Active);
        assert!(!book.claim("job").1);
        let first = book.progress("job", receipt_progress(3, "downloading")).unwrap();
        assert_eq!(first.progress.unwrap().downloaded_bytes, 1024);
        assert!(book.progress("job", receipt_progress(2, "downloading")).is_none());
        assert!(book.progress("job", receipt_progress(3, "downloading")).is_none());
        assert!(book.progress("job", receipt_progress(4, "complete")).is_none());
        assert_eq!(book.status("job").state, ReceiptState::Active);
        assert_eq!(book.status("job").progress.unwrap().stage, "downloading");
        assert!(book.progress("job", receipt_progress(5, "downloading")).is_none());
        book.finish("job", ReceiptState::Complete, "committed and reopened").unwrap();
        let restored = book.status("job");
        assert_eq!(restored.state, ReceiptState::Complete);
        assert_eq!(restored.message, "committed and reopened");
        assert_eq!(restored.progress.unwrap().sequence, 4);
        assert!(book.final_progress.is_empty());
        assert!(!book.claim("job").1);
    }
    #[test]
    fn receipt_failure_detail_and_partial_outcomes_survive_status_queries() {
        for state in [ReceiptState::Failed, ReceiptState::Cancelled, ReceiptState::Partial] {
            let mut book = ReceiptBook::default();
            book.claim("job");
            let mut progress = receipt_progress(1, if state == ReceiptState::Partial {"complete"} else {"failed"});
            progress.partial = state == ReceiptState::Partial;
            book.progress("job", progress);
            book.finish("job", state, "specific bounded reason");
            assert_eq!(book.status("job").state, state);
            assert_eq!(book.status("job").message, "specific bounded reason");
            assert!(book.progress("job", receipt_progress(9, "complete")).is_none());
            assert!(!book.claim("job").1);
        }
    }
    #[test]
    fn receipt_capacity_seals_unknown_ids_without_evicting_active_or_absence() {
        let mut book = ReceiptBook::default();
        book.claim("active");
        book.status("absent");
        for number in 2..RECEIPT_CAPACITY { book.status(&format!("missing-{number}")); }
        assert_eq!(book.entries.len(), RECEIPT_CAPACITY);
        assert_eq!(book.status("overflow").state, ReceiptState::Failed);
        assert!(book.sealed);
        assert!(!book.claim("overflow").1);
        assert!(!book.claim("another-new-id").1);
        assert_eq!(book.status("active").state, ReceiptState::Active);
        assert_eq!(book.status("absent").state, ReceiptState::Absent);
        assert!(book.has_active());
        assert_eq!(book.entries.len(), RECEIPT_CAPACITY);
    }
    #[test]
    fn receipt_cancel_before_submission_tombstones_without_cancelling_another_job() {
        let mut book = ReceiptBook::default();
        book.claim("other");
        assert_eq!(book.cancel_unstarted("late").state, ReceiptState::Cancelled);
        assert!(!book.claim("late").1);
        assert_eq!(book.status("other").state, ReceiptState::Active);
        // An accepted job stays active until its actual worker confirms cancel.
        assert_eq!(book.cancel_unstarted("other").state, ReceiptState::Active);
    }
    #[test]
    fn receipt_wire_is_closed_scalar_and_canonical_uuid_only() {
        let value = serde_json::to_value(Receipt::new(ReceiptState::Absent, "not admitted")).unwrap();
        assert_eq!(value, serde_json::json!({"state":"absent","progress":null,"message":"not admitted"}));
        assert_eq!(Receipt::new(ReceiptState::Failed, &"가".repeat(600)).message.chars().count(), 512);
        let id = "01234567-89ab-4cde-8123-456789abcdef";
        assert!(canonical_request_id(id));
        for bad in [id.to_uppercase(), id.replace('-', ""), "../path".into(), format!("{{{id}}}"), "".into()] {
            assert!(!canonical_request_id(&bad));
        }
        let source = Url::parse("https://x.com/home").unwrap();
        let body = format!("ATSUMI_X_SURFACE_V1:https://x.com/__atsumi_surface__/fixture?kind=archiveStatus&value={id}");
        assert_eq!(bridge_message(source.as_str(), &source, &body, "fixture"), Some(("archiveStatus".into(), id.into())));
        assert!(bridge_message("https://evil.test/home", &source, &body, "fixture").is_none());
        assert!(bridge_message(source.as_str(), &source, &body, "other").is_none());
        assert!(bridge_message(source.as_str(), &source, &body.replace(id, "not-uuid"), "fixture").is_none());
    }
    #[test]
    fn receipt_scope_allows_readiness_drift_but_not_other_account_owner_or_revision() {
        let session = XSession::default();
        let pane = Arc::new(Pane {
            label:"receipt-fixture".into(),nonce:"fixture".into(),feed:"foryou".into(),owner:"alice".into(),profile:"legacy".into(),
            revision:1,state:Mutex::new("loading".into()),loading_since:Mutex::new(Instant::now()),
            document_load:Mutex::new(DocumentLoad::default()),root_recovery:Mutex::new(RootRecovery::default()),
            visible:AtomicBool::new(false),fullscreen:AtomicBool::new(false),media:Arc::new(surface_media::Observations::new(1)),
        });
        session.surfaces.lock().unwrap().feeds.insert("foryou".into(), pane.clone());
        *session.registry.lock().unwrap() = Some(accounts::Registry { active:"legacy".into(),
            accounts:vec![accounts::Account { id:"legacy".into(), handle:"Alice".into() }] });
        assert!(receipt_scope_current(&session, &pane));
        assert!(!interaction_allowed(&session, &pane));
        *pane.loading_since.lock().unwrap() = Instant::now() - Duration::from_secs(60);
        assert!(receipt_scope_current(&session, &pane));
        for state in ["ownerChanged", "loginRequired"] {
            *pane.state.lock().unwrap() = state.into();
            assert!(!receipt_scope_current(&session, &pane));
        }
        *pane.state.lock().unwrap() = "loading".into();
        session.registry.lock().unwrap().as_mut().unwrap().active = "other".into();
        assert!(!receipt_scope_current(&session, &pane));
        session.registry.lock().unwrap().as_mut().unwrap().active = "legacy".into();
        for handle in ["bob", ""] {
            session.registry.lock().unwrap().as_mut().unwrap().accounts[0].handle = handle.into();
            assert!(!receipt_scope_current(&session, &pane));
        }
        session.registry.lock().unwrap().as_mut().unwrap().accounts[0].handle = "alice".into();
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!receipt_scope_current(&session, &pane));
        session.revision.store(1, Ordering::Release);
        session.surfaces.lock().unwrap().feeds.clear();
        assert!(!receipt_scope_current(&session, &pane));
    }
    #[test]
    fn action_progress_exposes_only_closed_real_counters_for_the_exact_frozen_request() {
        let mut value=ArchiveProgress {request_id:"request".into(),capture_id:"capture".into(),account_id:"account".into(),
            sequence:1,stage:ArchiveProgressStage::Downloading,processed_media:1,total_media:2,downloaded_bytes:4096,total_bytes:None,current_transfer:None,partial:false};
        let projected=serde_json::to_value(project_progress(&value,"request","capture","account").unwrap()).unwrap();
        assert_eq!(projected,serde_json::json!({"sequence":1,"stage":"downloading","processedMedia":1,"totalMedia":2,"downloadedBytes":4096,"totalBytes":null,"currentTransfer":null,"partial":false}));
        for (request,capture,account) in [("other","capture","account"),("request","other","account"),("request","capture","other")] {
            assert!(project_progress(&value,request,capture,account).is_none());
        }
        for stage in [ArchiveProgressStage::Preparing,ArchiveProgressStage::Writing,ArchiveProgressStage::Complete,ArchiveProgressStage::Failed,ArchiveProgressStage::Cancelled] {
            value.stage=stage;assert!(project_progress(&value,"request","capture","account").is_some());
        }
        value.partial=true;assert!(project_progress(&value,"request","capture","account").unwrap().partial);
    }
    #[test]
    fn action_progress_rejects_unknown_or_imprecise_counts_instead_of_fake_percentages() {
        let base=ArchiveProgress {request_id:"r".into(),capture_id:"c".into(),account_id:"a".into(),sequence:1,
            stage:ArchiveProgressStage::Downloading,processed_media:0,total_media:1,downloaded_bytes:3,total_bytes:None,current_transfer:None,partial:false};
        for changed in 0..7 {let mut value=base.clone();match changed {
            0=>value.sequence=0,1=>value.sequence=9_007_199_254_740_992,2=>value.processed_media=2,
            3=>value.total_media=4097,4=>value.downloaded_bytes=9_007_199_254_740_992,
            5=>value.total_bytes=Some(2),_=>value.total_bytes=Some(9_007_199_254_740_992),
        };assert!(project_progress(&value,"r","c","a").is_none());}
    }
    #[test]
    fn action_current_transfer_remains_separate_from_cumulative_and_rejects_invalid_lengths() {
        let mut value=ArchiveProgress {request_id:"r".into(),capture_id:"c".into(),account_id:"a".into(),sequence:1,
            stage:ArchiveProgressStage::Downloading,processed_media:0,total_media:1,downloaded_bytes:8192,total_bytes:None,
            current_transfer:Some(CurrentTransferProgress {received_bytes:1024,total_bytes:Some(2048)}),partial:false};
        let projected=serde_json::to_value(project_progress(&value,"r","c","a").unwrap()).unwrap();
        assert_eq!(projected["currentTransfer"],serde_json::json!({"receivedBytes":1024,"totalBytes":2048}));
        assert!(projected["totalBytes"].is_null());
        for (received,total) in [(8193,None),(1024,Some(1023)),(0,Some(9_007_199_254_740_992))] {
            value.current_transfer=Some(CurrentTransferProgress {received_bytes:received,total_bytes:total});
            assert!(project_progress(&value,"r","c","a").is_none());
        }
        value.current_transfer=Some(CurrentTransferProgress {received_bytes:1024,total_bytes:None});
        assert!(project_progress(&value,"r","c","a").is_some());
    }
    #[test]
    fn choose_intent_only_accepts_visible_registered_pane_and_exact_post_identity() {
        let pane = Arc::new(Pane {
            label:"choose-feed".into(),nonce:"fixture".into(),feed:"foryou".into(),owner:"alice".into(),profile:"legacy".into(),
            revision:0,state:Mutex::new("ready".into()),loading_since:Mutex::new(Instant::now()),
            document_load:Mutex::new(DocumentLoad::default()),root_recovery:Mutex::new(RootRecovery::default()),
            visible:AtomicBool::new(true),fullscreen:AtomicBool::new(false),
            media:Arc::new(surface_media::Observations::new(0)),
        });
        let mut data=Surfaces {active:"foryou".into(),generation:17,..Default::default()};
        data.feeds.insert("foryou".into(),pane.clone());
        data.viewport=Some(NativeViewport {sequence:1,visible:true,muted:false,occlusions:vec![],
            player:Some(Rect{x:0.,y:0.,width:600.,height:900.}),chat:None});
        let source=Url::parse("https://x.com/home?fixture=true").unwrap();
        let request=choose_request(&data,&pane,"/alice/status/123456",&source).unwrap();
        assert_eq!(request.generation,17);assert_eq!(request.account_id,"legacy");
        assert_eq!(request.pane,"feed");assert_eq!(request.source_url,"https://x.com/home");
        assert_eq!(request.path,"/alice/status/123456");
        for path in ["/home","/alice/status/123456/photo/1","https://x.com/alice/status/123456","/alice/status/123456?x=1"] {
            assert!(choose_request(&data,&pane,path,&source).is_none());
        }
        for url in ["https://evil.test/home","http://x.com/home","https://user@x.com/home","https://x.com/__atsumi_surface__/fixture"] {
            assert!(choose_request(&data,&pane,"/alice/status/123456",&Url::parse(url).unwrap()).is_none());
        }
        data.active="bookmarks".into();assert!(choose_request(&data,&pane,"/alice/status/123456",&source).is_none());
        data.active="foryou".into();pane.visible.store(false,Ordering::Release);
        assert!(choose_request(&data,&pane,"/alice/status/123456",&source).is_none());
        pane.visible.store(true,Ordering::Release);data.viewport.as_mut().unwrap().visible=false;
        assert!(choose_request(&data,&pane,"/alice/status/123456",&source).is_none());
    }
    #[test]
    fn native_actions_are_closed_and_no_arbitrary_url_or_path_is_accepted() {
        let id=uuid::Uuid::new_v4().to_string();
        let valid=serde_json::json!({"id":id,"path":"/example/status/123456","kind":"all","media":null});
        assert!(Action::parse(&valid.to_string()).is_some());
        for (key,value) in [("path","https://evil.test/a"),("path","/home"),("path","/example/status/123456/photo/1"),("kind","delete"),("id","../file")] {
            let mut changed=valid.clone();changed[key]=value.into();assert!(Action::parse(&changed.to_string()).is_none());
        }
        let mut one=valid;one["kind"]="one".into();assert!(Action::parse(&one.to_string()).is_none());
        one["media"]="https://pbs.twimg.com/media/known?format=jpg".into();assert!(Action::parse(&one.to_string()).is_some());
        one["filename"]="../escape".into();assert!(Action::parse(&one.to_string()).is_none());
    }
}
