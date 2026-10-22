//! Interactive official X documents. Remote documents never receive Atsumi IPC
//! capabilities. The bridge accepts closed X paths and explicit validated links.
use super::{accounts, allowed, surface_media, trusted, valid_handle, valid_id, XSession, STALE};
use crate::{
    interface::AppState,
    platform_native_geometry::{NativeViewport, Rect},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, Webview, WebviewBuilder,
    WebviewUrl,
};
#[path = "x_surface_fullscreen.rs"]
mod fullscreen;
#[path = "x_surface_actions.rs"]
mod actions;
#[path = "x_surface_search.rs"]
pub mod search;

const ERROR: &str = "X 화면을 열지 못했습니다. 새로고침하거나 로그인 상태를 확인해 주세요.";
const LOGIN: &str = "X 로그인이 필요합니다. 계정 메뉴에서 로그인해 주세요.";
const OWNER: &str = "X 화면의 계정이 선택한 계정과 다릅니다. 계정 메뉴에서 다시 로그인해 주세요.";
const PREVIEW_LIMIT: usize = 16;
const PREVIEW_FULL: &str = "프리뷰 탭은 최대 16개까지 열 수 있습니다. 기존 탭을 닫은 뒤 다시 열어 주세요.";
#[derive(Default)]
pub(super) struct Surfaces {
    feeds: HashMap<String, Arc<Pane>>,
    preview: Option<Arc<Pane>>,
    preview_tabs: Vec<PreviewTab>,
    preview_error: Option<String>,
    route_intents: HashMap<String, RouteIntent>,
    active: String,
    generation: u64,
    order: u64,
    open_epoch: u64,
    viewport: Option<NativeViewport>,
    preview_intent: u64,
    preview_selection: u64,
    preview_paths: HashMap<String, String>,
    playing: Option<String>,
    manual_playing: bool,
    focused: Option<String>,
}
struct PreviewTab {
    pane: Arc<Pane>,
    path: String,
}
struct RouteIntent {
    source: Url,
    generation: u64,
    since: Instant,
    background: bool,
    origin: PreviewOpenOrigin,
}
#[derive(Clone, Copy)]
struct PreviewOpenOrigin {
    order: u64,
    selection: u64,
}
fn preview_origin(data: &Surfaces) -> PreviewOpenOrigin {
    PreviewOpenOrigin { order: data.order, selection: data.preview_selection }
}
fn preview_wants_foreground(data: &Surfaces, origin: PreviewOpenOrigin, background: bool) -> bool {
    !background && data.order == origin.order && data.preview_selection == origin.selection
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewSnapshot {
    id: String,
    path: String,
    title: String,
    state: String,
}
struct Pane {
    label: String,
    nonce: String,
    feed: String,
    owner: String,
    profile: String,
    revision: u64,
    state: Mutex<String>,
    loading_since: Mutex<Instant>,
    document_load: Mutex<DocumentLoad>,
    root_recovery: Mutex<RootRecovery>,
    visible: AtomicBool,
    fullscreen: AtomicBool,
    media: Arc<surface_media::Observations>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceSnapshot {
    generation: u64,
    feed: String,
    state: String,
    preview: bool,
    preview_state: String,
    previews: Vec<PreviewSnapshot>,
    active_preview_id: Option<String>,
    error: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentStatus {
    state: String,
    owner: Option<String>,
    path: String,
    #[serde(default)]
    proof: Option<DocumentProof>,
    #[serde(default, rename = "accountPresentation")]
    account_presentation: Option<super::account_presentation::ObservedPresentation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentProof {
    profile: bool,
    account: bool,
    primary: bool,
    #[serde(default)]
    photo: bool,
    login: bool,
    #[serde(default, rename = "photoShape")]
    photo_shape: Option<PhotoShape>,
    #[serde(default, rename = "accountShape")]
    account_shape: Option<AccountShape>,
    #[serde(default, rename = "adShape")]
    ad_shape: Option<AdShape>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdShape {
    ready: bool,
    articles: u16,
    outer_placements: u16,
    header_badges: u16,
    promoted_indicators: u16,
    hidden: u16,
    ambiguous: u16,
    multiple_placements: u16,
    shared_placements: u16,
    structure_limited: u16,
    metadata_missing: u16,
    limited: bool,
    #[serde(default)]
    header_shape: Option<AdHeaderShape>,
}
impl AdShape {
    fn diagnostic(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "ready":self.ready,"articles":self.articles.min(160),
            "outerPlacements":self.outer_placements.min(160),
            "headerBadges":self.header_badges.min(160),
            "promotedIndicators":self.promoted_indicators.min(160),
            "hidden":self.hidden.min(160),"ambiguous":self.ambiguous.min(160),
            "multiplePlacements":self.multiple_placements.min(160),
            "sharedPlacements":self.shared_placements.min(160),
            "structureLimited":self.structure_limited.min(160),
            "metadataMissing":self.metadata_missing.min(160),
            "limited":self.limited
        });
        if let Some(header) = &self.header_shape { value["headerShape"] = header.diagnostic(); }
        value
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdHeaderShape {
    inspected: u16, names_missing: u16, names_multiple: u16,
    stopped_content: u16, stopped_role_link: u16, role_link_in_name: u16, stopped_context: u16,
    caret_missing: u16, caret_multiple: u16, candidate_limit: u16, depth_limit: u16,
    label_absent: u16, label_author: u16, label_control: u16, label_scope: u16,
    label_non_leaf: u16, label_hidden: u16, outer_label: u16,
}
impl AdHeaderShape {
    fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "inspected":self.inspected.min(160),"namesMissing":self.names_missing.min(160),
            "namesMultiple":self.names_multiple.min(160),"stoppedContent":self.stopped_content.min(160),
            "stoppedRoleLink":self.stopped_role_link.min(160),"roleLinkInName":self.role_link_in_name.min(160),
            "stoppedContext":self.stopped_context.min(160),"caretMissing":self.caret_missing.min(160),
            "caretMultiple":self.caret_multiple.min(160),"candidateLimit":self.candidate_limit.min(160),
            "depthLimit":self.depth_limit.min(160),"labelAbsent":self.label_absent.min(160),
            "labelAuthor":self.label_author.min(160),"labelControl":self.label_control.min(160),
            "labelScope":self.label_scope.min(160),"labelNonLeaf":self.label_non_leaf.min(160),
            "labelHidden":self.label_hidden.min(160),"outerLabel":self.outer_label.min(160)
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountShape {
    nodes: u8,
    images: u8,
    image_sources: u8,
    loaded_images: u8,
    backgrounds: u8,
    text_length: u16,
    auto_text_nodes: u8,
    aria_label: bool,
    avatar_containers: u8,
    hidden: bool,
    has_box: bool,
    narrow: bool,
}
impl AccountShape {
    fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "nodes":self.nodes.min(128),"images":self.images.min(32),
            "imageSources":self.image_sources.min(32),"loadedImages":self.loaded_images.min(32),
            "backgrounds":self.backgrounds.min(32),"textLength":self.text_length.min(512),
            "autoTextNodes":self.auto_text_nodes.min(32),"ariaLabel":self.aria_label,
            "avatarContainers":self.avatar_containers.min(8),"hidden":self.hidden,
            "hasBox":self.has_box,"narrow":self.narrow
        })
    }
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum PhotoRoute {
    Auth, Photo, PhotoOther, Video, Post, Home, Collection, Profile, Other,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PhotoShape {
    route: PhotoRoute,
    segments: u8,
    dialogs: u8,
    dialog_images: u8,
    photo_candidates: u8,
    dialog_image_host: bool,
    dialog_image_media: bool,
    page_image_media: bool,
    hidden_dialog: bool,
    security_input: bool,
    #[serde(default)]
    action_shape: Option<PhotoActionShape>,
    #[serde(default)]
    sample: u8,
}
impl PhotoShape {
    fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "route":self.route,"segments":self.segments.min(16),"dialogs":self.dialogs.min(8),
            "dialogImages":self.dialog_images.min(32),"photoCandidates":self.photo_candidates.min(32),
            "dialogImageHost":self.dialog_image_host,"dialogImageMedia":self.dialog_image_media,
            "pageImageMedia":self.page_image_media,"hiddenDialog":self.hidden_dialog,"securityInput":self.security_input,
            "actionShape":self.action_shape.as_ref().map(PhotoActionShape::diagnostic),"sample":self.sample.min(4)
        })
    }
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum PhotoActionReason {
    Hidden, Route, Owner, DialogLimit, DialogAmbiguous, ArticleScope, ImageLimit, None, Ambiguous, Ready,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum PhotoCarouselProof {
    EmptyCandidates, CandidateLimit, Route, AncestorLimit, Absent, MixedRoots, HiddenRoot,
    NestedRoot, Lists, Items, NestedItems, CounterMissing, CounterMismatch, CurrentHidden,
    PositionConflict, SizeConflict, CurrentConflict, SelectedConflict, Detached, CandidateScope,
    ImageRect, FixedImage, FixedAncestor, AncestorRect, UnboundAbsolute, ClipDepth,
    CurrentDuplicate, CurrentClipped, NeighbourExposed, CurrentMissing, Ready,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PhotoActionShape {
    reason: PhotoActionReason,
    dialogs: u8,
    visible_dialogs: u8,
    leaf_dialogs: u8,
    article_present: bool,
    scanned_images: u8,
    allowed_images: u8,
    image_loaded: u8,
    image_complete: u8,
    nonzero_rect: u8,
    in_viewport: u8,
    directly_shown: u8,
    opacity_zero: u8,
    backgrounds: u8,
    exact_paint: u8,
    candidates: u8,
    control_present: bool,
    control_hovered: bool,
    #[serde(default)]
    slide_shape: Option<PhotoSlideShape>,
    #[serde(default)]
    carousel_proof: Option<PhotoCarouselProof>,
}
impl PhotoActionShape {
    fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "reason":self.reason,"dialogs":self.dialogs.min(128),"visibleDialogs":self.visible_dialogs.min(128),
            "leafDialogs":self.leaf_dialogs.min(128),"articlePresent":self.article_present,
            "scannedImages":self.scanned_images.min(128),"allowedImages":self.allowed_images.min(128),
            "imageLoaded":self.image_loaded.min(128),"imageComplete":self.image_complete.min(128),
            "nonzeroRect":self.nonzero_rect.min(128),"inViewport":self.in_viewport.min(128),
            "directlyShown":self.directly_shown.min(128),"opacityZero":self.opacity_zero.min(128),
            "backgrounds":self.backgrounds.min(128),"exactPaint":self.exact_paint.min(128),
            "candidates":self.candidates.min(128),"controlPresent":self.control_present,"controlHovered":self.control_hovered,
            "slideShape":self.slide_shape.as_ref().map(PhotoSlideShape::diagnostic),
            "carouselProof":self.carousel_proof
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PhotoSlideShape {
    route_ordinal: u8,
    carousels: u8,
    lists: u8,
    slides: u8,
    candidate_slides: u8,
    indexed_slides: u8,
    indexed_route_matches: u8,
    current_slides: u8,
    selected_slides: u8,
    current_route_matches: u8,
    selected_route_matches: u8,
    declared_sizes: u8,
    matching_set_sizes: u8,
    counter_labels: u8,
    counter_route_matches: u8,
    counter_size_matches: u8,
    ordinal_item_candidates: u8,
    fully_clipped: u8,
    partly_clipped: u8,
    unclipped: u8,
    clip_uncertain: u8,
    overflow: bool,
}
impl PhotoSlideShape {
    fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "routeOrdinal":self.route_ordinal.min(4),"carousels":self.carousels.min(8),
            "lists":self.lists.min(8),"slides":self.slides.min(8),"candidateSlides":self.candidate_slides.min(8),
            "indexedSlides":self.indexed_slides.min(32),"indexedRouteMatches":self.indexed_route_matches.min(32),
            "currentSlides":self.current_slides.min(32),"selectedSlides":self.selected_slides.min(32),
            "currentRouteMatches":self.current_route_matches.min(32),"selectedRouteMatches":self.selected_route_matches.min(32),
            "declaredSizes":self.declared_sizes.min(32),"matchingSetSizes":self.matching_set_sizes.min(32),
            "counterLabels":self.counter_labels.min(32),"counterRouteMatches":self.counter_route_matches.min(32),
            "counterSizeMatches":self.counter_size_matches.min(32),"ordinalItemCandidates":self.ordinal_item_candidates.min(128),
            "fullyClipped":self.fully_clipped.min(128),"partlyClipped":self.partly_clipped.min(128),
            "unclipped":self.unclipped.min(128),"clipUncertain":self.clip_uncertain.min(128),"overflow":self.overflow
        })
    }
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum RoutingReason { MissingPath, MissingScope }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum RoutingRole { Article, Link, Button, QuoteMarker }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoutingProbe {
    reason: RoutingReason,
    role: RoutingRole,
    has_tweet_article: bool,
    nested: bool,
    user_names: u8,
    own_user_names: u8,
    anchors: u8,
    times: u8,
    time_links: u8,
    media_links: u8,
    unique_media: bool,
    author_match: bool,
    target_anchor: bool,
    scoped_resolved: bool,
}
impl RoutingProbe {
    fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({
            "reason":self.reason,"role":self.role,"hasTweetArticle":self.has_tweet_article,"nested":self.nested,
            "userNames":self.user_names.min(32),"ownUserNames":self.own_user_names.min(32),"anchors":self.anchors.min(128),
            "times":self.times.min(32),"timeLinks":self.time_links.min(32),"mediaLinks":self.media_links.min(128),
            "uniqueMedia":self.unique_media,"authorMatch":self.author_match,"targetAnchor":self.target_anchor,"scopedResolved":self.scoped_resolved
        })
    }
}
#[derive(Default)]
struct DocumentLoad {
    pending: Option<Url>,
}
impl DocumentLoad {
    fn start(&mut self, url: &Url) -> bool {
        if !allowed(url) || url.path().starts_with("/__atsumi_surface__/") {
            return false;
        }
        self.pending = Some(url.clone());
        true
    }
    fn finish(&mut self, url: &Url) -> bool {
        if self.pending.as_ref() != Some(url) {
            return false;
        }
        self.pending = None;
        true
    }
}

#[derive(Default)]
struct RootRecovery {
    attempted: bool,
    pending: bool,
}
#[derive(Debug, PartialEq, Eq)]
enum RecoveryStep { Start, Pending, Failed }
impl RootRecovery {
    fn request(&mut self) -> RecoveryStep {
        if self.pending { return RecoveryStep::Pending; }
        if self.attempted { return RecoveryStep::Failed; }
        self.attempted = true;
        self.pending = true;
        RecoveryStep::Start
    }
}
#[derive(Debug, PartialEq, Eq)]
enum RootAction { Keep, ClosePreview, RestoreCollection }
fn collection_kind(path: &str, owner: &str) -> Option<&'static str> {
    if matches!(path, "/i/bookmarks" | "/i/history/bookmarks" | "/i/history") {
        Some("bookmarks")
    } else if path == "/i/history/likes" || path == format!("/{owner}/likes") {
        Some("likes")
    } else {
        None
    }
}
fn root_action(feed: &str, owner: &str, path: &str) -> RootAction {
    if feed == "preview" && path == "/home" {
        RootAction::ClosePreview
    } else if matches!(feed, "likes" | "bookmarks")
        && (path == "/home" || collection_kind(path, owner).is_some_and(|kind| kind != feed)) {
        RootAction::RestoreCollection
    } else {
        RootAction::Keep
    }
}

fn route(feed: &str, owner: &str) -> Result<String, String> {
    if !valid_handle(owner) {
        return Err(LOGIN.into());
    }
    Ok(match feed {
        "foryou" | "following" => "/home".into(),
        "communities" => "/i/communities".into(),
        "likes" => format!("/{owner}/likes"),
        "bookmarks" => "/i/bookmarks".into(),
        _ => return Err(ERROR.into()),
    })
}
fn post_path(value: &str) -> bool {
    if value.len() > 100 || !value.starts_with('/') {
        return false;
    }
    let p: Vec<_> = value.split('/').skip(1).collect();
    let i = if p.first() == Some(&"i") && p.get(1) == Some(&"web") {
        2
    } else {
        1
    };
    (i == 2 || p.first().is_some_and(|v| valid_handle(v)))
        && p.get(i) == Some(&"status")
        && p.get(i + 1).is_some_and(|v| valid_id(v))
        && (p.len() == i + 2
            || p.len() == i + 4
                && matches!(p[i + 2], "photo" | "video")
                && matches!(p[i + 3], "1" | "2" | "3" | "4"))
}
fn profile_path(value: &str) -> bool {
    let Some(handle) = value.strip_prefix('/') else { return false };
    valid_handle(handle)
        && !matches!(handle.to_ascii_lowercase().as_str(),
            "home" | "explore" | "search" | "notifications" | "messages" | "i" | "compose"
            | "settings" | "login" | "logout" | "signup" | "tos" | "privacy" | "help" | "about"
            | "download" | "jobs" | "communities" | "premium" | "grok" | "hashtag" | "intent"
            | "account" | "accounts")
}
fn navigation_path(value: &str) -> bool {
    post_path(value) || profile_path(value)
}
fn external_link_url(value: &str) -> Option<Url> {
    if value.is_empty() || value.len() > 2048
        || value.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\') {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "https" | "http") || !url.username().is_empty() || url.password().is_some()
        || url.host_str().is_none_or(|host| matches!(host.trim_end_matches('.'), "x.com" | "www.x.com")) {
        return None;
    }
    Some(url)
}
fn preview_title(path: &str) -> String {
    let handle = path.split('/').nth(1).unwrap_or_default();
    if handle == "i" { "게시물".into() }
    else if post_path(path) { format!("@{handle} · #{}", path.split('/').nth(3).unwrap_or_default()) }
    else { format!("@{handle}") }
}
fn signal(url: &Url, nonce: &str) -> Option<(String, String)> {
    if !allowed(url)
        || url.fragment().is_some()
        || url.path() != format!("/__atsumi_surface__/{nonce}")
        || url.as_str().len() > 4096
    {
        return None;
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    if pairs.len() != 2 || pairs[0].0 != "kind" || pairs[1].0 != "value" {
        return None;
    }
    let (kind, value) = (pairs[0].1.to_string(), pairs[1].1.to_string());
    if !matches!(kind.as_str(), "status" | "preview" | "previewBackground" | "close" | "external" | "externalLink" | "play" | "focus" | "wheel" | "key" | "archive" | "archiveCancel" | "archiveStatus" | "archiveChoose" | "search" | "routeIntent")
        && !(cfg!(debug_assertions) && matches!(kind.as_str(), "routeProbe" | "photoProbe")) {
        return None;
    }
    if kind == "routeProbe" && serde_json::from_str::<RoutingProbe>(&value).is_err() {
        return None;
    }
    if kind == "photoProbe" && !serde_json::from_str::<PhotoShape>(&value)
        .is_ok_and(|probe| matches!(probe.route, PhotoRoute::Photo) && (1..=4).contains(&probe.sample)) {
        return None;
    }
    if matches!(kind.as_str(), "preview" | "previewBackground" | "external") && !navigation_path(&value) {
        return None;
    }
    if kind == "externalLink" && external_link_url(&value).is_none() { return None; }
    if matches!(kind.as_str(), "close" | "focus" | "wheel") && !value.is_empty() {
        return None;
    }
    if kind == "key" && !matches!(value.as_str(), "Escape" | "ArrowLeft" | "ArrowRight") {
        return None;
    }
    if kind == "play" && !matches!(value.as_str(), "auto" | "manual") {
        return None;
    }
    if kind == "routeIntent" && !matches!(value.as_str(), "arm" | "armBackground" | "cancel") { return None; }
    if kind == "archive" && actions::Action::parse(&value).is_none() { actions::rejected_submission(); return None; }
    if kind == "archiveCancel" && !actions::canonical_request_id(&value) { return None; }
    if kind == "archiveStatus" && !actions::canonical_request_id(&value) { return None; }
    if kind == "archiveChoose" && !actions::base_post_path(&value) { return None; }
    if kind == "search" && search::Request::parse(&value).is_none() { return None; }
    Some((kind, value))
}
fn bridge_message(source: &str, current: &Url, body: &str, nonce: &str) -> Option<(String, String)> {
    if !allowed(current) || current.path().starts_with("/__atsumi_surface__/")
        || Url::parse(source).ok().as_ref() != Some(current) {
        return None;
    }
    let encoded = body.strip_prefix("ATSUMI_X_SURFACE_V1:").filter(|value| value.len() <= 4096)?;
    signal(&Url::parse(encoded).ok()?, nonce)
}
#[cfg(windows)]
fn install_bridge(view: &Webview, session: XSession, pane: Arc<Pane>) -> Result<(), String> {
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows::core::PWSTR;
    if view.label() != pane.label || view.window().label() != "main" {
        return Err(ERROR.into());
    }
    let app = view.app_handle().clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let installed = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let mut subscription = 0;
            core.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                if !is_current(&session, &pane) { return Ok(()); }
                let Some(args) = args else { return Ok(()); };
                let mut source = PWSTR::null();
                if args.Source(&mut source).is_err() { return Ok(()); }
                let source = CoTaskMemPWSTR::from(source).to_string();
                let Some(current) = app.get_webview(&pane.label).and_then(|view| view.url().ok()) else { return Ok(()); };
                let mut body = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut body).is_err() { return Ok(()); }
                let body = CoTaskMemPWSTR::from(body).to_string();
                if let Some((kind, value)) = bridge_message(&source, &current, &body, &pane.nonce) {
                    // Existing handler rechecks current pane/epoch, readiness,
                    // visibility and owner. Remote documents gain no main IPC.
                    handle_signal(&session, &app, &pane, &kind, &value);
                }
                Ok(())
            })), &mut subscription)?;
            Ok(())
        })();
        let _ = tx.send(installed.is_ok());
    }).map_err(|_| ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) { Ok(()) } else { Err(ERROR.into()) }
}
#[cfg(not(windows))]
fn install_bridge(_: &Webview, _: XSession, _: Arc<Pane>) -> Result<(), String> {
    // Do not revive the cancelled-navigation bridge on unsupported runtimes.
    Err(ERROR.into())
}
fn pane_state(pane: &Pane) -> String {
    let state = pane.state.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if state == "loading"
        && pane
            .loading_since
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
            > Duration::from_secs(30)
    {
        "timeout".into()
    } else {
        state
    }
}
fn changed(session: &XSession, app: &AppHandle) {
    // Only the trusted local frontend receives a signal; it reads a fresh
    // native snapshot instead of trusting event-supplied account/page data.
    let _ = app.emit_to(
        "main",
        "x:surface-changed",
        session.revision.load(Ordering::Acquire),
    );
}
fn snapshot(session: &XSession) -> SurfaceSnapshot {
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    let state = data
        .feeds
        .get(&data.active)
        .map(|p| pane_state(p))
        .unwrap_or_else(|| "loginRequired".into());
    let preview_state = data
        .preview
        .as_ref()
        .map(|p| pane_state(p))
        .unwrap_or_else(|| "empty".into());
    let error = match state.as_str() {
        "loginRequired" => Some(LOGIN.into()),
        "ownerChanged" => Some(OWNER.into()),
        "timeout" => {
            Some("X 화면 응답을 기다리는 중입니다. 계속 표시되지 않으면 새로고침해 주세요.".into())
        }
        "error" => Some(ERROR.into()),
        _ => match preview_state.as_str() {
            "loginRequired" => Some(LOGIN.into()),
            "ownerChanged" => Some(OWNER.into()),
            "timeout" | "error" => Some(
                "X 상세 화면을 불러오지 못했습니다. 닫은 뒤 게시물을 다시 선택해 주세요.".into(),
            ),
            _ => None,
        },
    };
    SurfaceSnapshot {
        generation: data.generation,
        feed: data.active.clone(),
        state,
        preview: data.preview.is_some(),
        preview_state,
        previews: data.preview_tabs.iter().map(|tab| PreviewSnapshot {
            id: tab.pane.label.clone(), path: tab.path.clone(),
            title: preview_title(&tab.path), state: pane_state(&tab.pane),
        }).collect(),
        active_preview_id: data.preview.as_ref().map(|pane| pane.label.clone()),
        error: error.or_else(|| data.preview_error.clone()),
    }
}
fn mute(view: &Webview, muted: bool, grant: Option<(XSession, Arc<Pane>)>) {
    #[cfg(windows)]
    let _ = view.with_webview(move |web| {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
        use windows::core::Interface;
        let authorized = || {
            grant.as_ref().is_some_and(|(host, pane)| playback_granted(host, pane))
        };
        unsafe {
            if let Ok(core) = web
                .controller()
                .CoreWebView2()
                .and_then(|c| c.cast::<ICoreWebView2_8>())
            {
                let _ = core.SetIsMuted(muted || !authorized());
                if !muted && !authorized() {
                    let _ = core.SetIsMuted(true);
                }
            }
        }
    });
    #[cfg(not(windows))]
    let _ = (view, muted, grant);
}
fn conceal(app: &AppHandle, pane: &Pane) {
    pane.visible.store(false, Ordering::Release);
    if let Some(view) = app.get_webview(&pane.label) {
        fullscreen::conceal(pane, &view);
        mute(&view, true, None);
        let _ = view.eval("window.__atsumiXSurface?.visibility(false)");
        let _ = view.hide();
    } else {
        pane.fullscreen.store(false, Ordering::Release);
    }
}
fn dispose(app: &AppHandle, pane: &Pane) {
    pane.media.revoke();
    conceal(app, pane);
    if let Some(view) = app.get_webview(&pane.label) {
        let _ = view.close();
    }
}
pub(super) fn close_all(session: &XSession, app: &AppHandle) {
    let panes = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        data.preview = None;
        let previews = std::mem::take(&mut data.preview_tabs);
        let panes = data
            .feeds
            .drain()
            .map(|(_, p)| p)
            .chain(previews.into_iter().map(|tab| tab.pane))
            .collect::<Vec<_>>();
        data.active.clear();
        data.viewport = None;
        data.generation += 1;
        data.preview_intent += 1;
        data.preview_selection += 1;
        data.preview_paths.clear();
        data.preview_error = None;
        data.route_intents.clear();
        data.playing = None;
        data.manual_playing = false;
        data.focused = None;
        panes
    };
    search::invalidate(session, app);
    for pane in panes {
        dispose(app, &pane);
    }
}
fn unobscured(rect: &Rect, viewport: &NativeViewport) -> bool {
    rect.width >= 100.0
        && rect.height >= 80.0
        && !viewport.occlusions.iter().any(|h| {
            h.width > 0.0
                && h.height > 0.0
                && h.x < rect.x + rect.width
                && h.x + h.width > rect.x
                && h.y < rect.y + rect.height
                && h.y + h.height > rect.y
        })
}
fn registered_visible(data: &Surfaces, pane: &Pane) -> bool {
    let selected = if pane.feed == "preview" {
        data.preview.as_ref().is_some_and(|p| p.label == pane.label)
    } else {
        pane.feed == data.active && data.feeds.get(&data.active).is_some_and(|p| p.label == pane.label)
    };
    fullscreen::allows(data, pane) && selected && data.viewport.as_ref().is_some_and(|v| {
        v.visible && (if pane.feed == "preview" { v.chat.as_ref() } else { v.player.as_ref() })
            .is_some_and(|rect| unobscured(rect, v))
    })
}
fn automatic_claim_allowed(data: &Surfaces, pane: &Pane) -> bool {
    let Some(label) = data.playing.as_deref() else { return true };
    if label == pane.label { return true; }
    let current = data.feeds.values().chain(data.preview.iter()).find(|p| p.label == label);
    let Some(current) = current.filter(|p| p.revision == pane.revision
        && p.visible.load(Ordering::Acquire) && pane_state(p) == "ready" && registered_visible(data, p)) else { return true };
    // A manual claim cannot be stolen by another document's autoplay. Among
    // autoplay documents the detail has fixed priority, so they cannot bounce
    // playback back and forth. Either pane's next manual input always wins.
    !data.manual_playing && pane.feed == "preview" && current.feed != "preview"
}
fn automatic_playback_allowed(session: &XSession, pane: &Pane) -> bool {
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    pane.revision == session.revision.load(Ordering::Acquire)
        && registered_visible(&data, pane) && automatic_claim_allowed(&data, pane)
}
fn claim_playback(session: &XSession, pane: &Pane, manual: bool) -> bool {
    if pane.revision != session.revision.load(Ordering::Acquire)
        || !pane.visible.load(Ordering::Acquire) || pane_state(pane) != "ready" {
        return false;
    }
    let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    if pane.revision != session.revision.load(Ordering::Acquire) || !registered_visible(&data, pane)
        || !manual && !automatic_claim_allowed(&data, pane) {
        return false;
    }
    data.manual_playing = manual || data.playing.as_deref() == Some(pane.label.as_str()) && data.manual_playing;
    data.playing = Some(pane.label.clone());
    true
}
fn playback_granted(session: &XSession, pane: &Pane) -> bool {
    if pane.revision != session.revision.load(Ordering::Acquire)
        || !pane.visible.load(Ordering::Acquire) || pane_state(pane) != "ready" {
        return false;
    }
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    pane.revision == session.revision.load(Ordering::Acquire)
        && registered_visible(&data, pane)
        && data.playing.as_deref() == Some(pane.label.as_str())
        && data.viewport.as_ref().is_some_and(|v| !v.muted)
}
fn claim_native_audio(session: &XSession, pane: &Pane, playing: bool) -> bool {
    if !playing { return false; }
    {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        // Already owned audio must not cause a paint/event loop. Native audio
        // is an automatic claim only; it never steals another manual player.
        if data.playing.as_deref() == Some(pane.label.as_str())
            || !data.viewport.as_ref().is_some_and(|v| v.visible && !v.muted) { return false; }
    }
    claim_playback(session, pane, false)
}
#[cfg(windows)]
fn observe_native_audio(view: &Webview, session: XSession, pane: Arc<Pane>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows::core::{Interface, BOOL};
    let app = view.app_handle().clone();
    let _ = view.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_8>()) else { return; };
        let mut playing = BOOL(0);
        if core.IsDocumentPlayingAudio(&mut playing).is_ok()
            && claim_native_audio(&session, &pane, playing.as_bool()) { paint(&session, &app); }
    });
}
#[cfg(not(windows))]
fn observe_native_audio(_: &Webview, _: XSession, _: Arc<Pane>) {}
#[cfg(windows)]
fn install_native_audio(view: &Webview, session: XSession, pane: Arc<Pane>) -> Result<(), String> {
    use webview2_com::{IsDocumentPlayingAudioChangedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2_8};
    use windows::core::{Interface, BOOL};
    let app = view.app_handle().clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.with_webview(move |platform| unsafe {
        let installed = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?.cast::<ICoreWebView2_8>()?;
            let mut token = 0;
            core.add_IsDocumentPlayingAudioChanged(&IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()); };
                let mut playing = BOOL(0);
                let core = sender.cast::<ICoreWebView2_8>()?;
                // Browser-owned audio observation covers descendant frames.
                // No frame gets a bridge, credential access or player commands.
                if core.IsDocumentPlayingAudio(&mut playing).is_ok()
                    && claim_native_audio(&session, &pane, playing.as_bool()) { paint(&session, &app); }
                Ok(())
            })), &mut token)?;
            Ok(())
        })();
        let _ = tx.send(installed.is_ok());
    }).map_err(|_| ERROR)?;
    if rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) { Ok(()) } else { Err(ERROR.into()) }
}
#[cfg(not(windows))]
fn install_native_audio(_: &Webview, _: XSession, _: Arc<Pane>) -> Result<(), String> { Ok(()) }
fn paint(session: &XSession, app: &AppHandle) {
    let host = session.clone();
    let owned = app.clone();
    let _ = app.run_on_main_thread(move || paint_current(&host, &owned));
}
fn paint_current(session: &XSession, app: &AppHandle) {
    let fullscreen = fullscreen::presentation(session, app);
    let (panes, preview, active, viewport) = {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        (
            data.feeds.values().cloned().collect::<Vec<_>>(),
            data.preview_tabs.iter().map(|tab| tab.pane.clone()).collect::<Vec<_>>(),
            data.active.clone(),
            data.viewport.clone(),
        )
    };
    for pane in panes.into_iter().chain(preview) {
        let mut rect = viewport.as_ref().filter(|v| v.visible).and_then(|v| {
            if pane.feed == "preview" {
                let selected = session.surfaces.lock().unwrap_or_else(|p| p.into_inner())
                    .preview.as_ref().is_some_and(|current| current.label == pane.label);
                selected.then_some(v.chat.as_ref()).flatten()
            } else if pane.feed == active {
                v.player.as_ref()
            } else {
                None
            }
        }).cloned();
        if let Some(fullscreen) = fullscreen.as_ref() {
            rect = fullscreen.rect(&pane);
        }
        let visible = pane.revision == session.revision.load(Ordering::Acquire)
            && pane_state(&pane) == "ready"
            && rect.as_ref()
                .zip(viewport.as_ref())
                .is_some_and(|(r, v)| unobscured(r, v));
        if !visible {
            conceal(app, &pane);
            continue;
        }
        if let Some(view) = app.get_webview(&pane.label) {
            let r = rect.as_ref().unwrap();
            // Bounds are local to the main window; never move or focus the host.
            let result = view.set_bounds(tauri::Rect {
                position: LogicalPosition::new(r.x, r.y).into(),
                size: LogicalSize::new(r.width, r.height).into(),
            });
            if result.is_err() {
                conceal(app, &pane);
                continue;
            }
            if pane.revision != session.revision.load(Ordering::Acquire)
                || !registered_visible(&session.surfaces.lock().unwrap_or_else(|p| p.into_inner()), &pane) {
                conceal(app, &pane);
                continue;
            }
            pane.visible.store(true, Ordering::Release);
            let _ = view.eval("window.__atsumiXSurface?.visibility(true)");
            // Keep both documents interactive; prevent losing autoplay from
            // reclaiming playback while allowing the next explicit input.
            let automatic = automatic_playback_allowed(session, &pane);
            let _ = view.eval(format!("window.__atsumiXSurface?.playback({automatic})"));
            mute(
                &view,
                !playback_granted(session, &pane),
                Some((session.clone(), pane.clone())),
            );
            let _ = view.show();
            // Audio may have started while identity/readiness was still being
            // proven. Recheck on presentation without polling or replaying it.
            observe_native_audio(&view, session.clone(), pane.clone());
        }
    }
}
fn is_current(session: &XSession, pane: &Pane) -> bool {
    if pane.revision != session.revision.load(Ordering::Acquire) {
        return false;
    }
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    data.feeds
        .values()
        .chain(data.preview_tabs.iter().map(|tab| &tab.pane))
        .any(|p| p.label == pane.label)
}
fn interaction_allowed(session: &XSession, pane: &Pane) -> bool {
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    pane.revision == session.revision.load(Ordering::Acquire)
        && pane.visible.load(Ordering::Acquire)
        && pane_state(pane) == "ready"
        && registered_visible(&data, pane)
}
fn input_target(data: &Surfaces, source: &Arc<Pane>) -> Option<Arc<Pane>> {
    let valid = |pane: &&Arc<Pane>| pane.revision == source.revision
        && pane.visible.load(Ordering::Acquire) && pane_state(pane) == "ready" && registered_visible(data, pane);
    data.focused.as_ref().and_then(|label| data.feeds.values().chain(data.preview.iter())
        .find(|pane| &pane.label == label).filter(valid).cloned())
        .or_else(|| data.feeds.values().chain(data.preview.iter())
            .find(|pane| pane.label == source.label).filter(valid).cloned())
}
fn claim_input(session: &XSession, pane: &Arc<Pane>) -> bool {
    let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    if pane.revision != session.revision.load(Ordering::Acquire)
        || !pane.visible.load(Ordering::Acquire) || pane_state(pane) != "ready"
        || !registered_visible(&data, pane) { return false; }
    data.focused = Some(pane.label.clone());
    data.route_intents.retain(|label, _| label == &pane.label);
    true
}
fn claim_scroll(session: &XSession, pane: &Arc<Pane>) -> bool {
    let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    if pane.revision != session.revision.load(Ordering::Acquire)
        || !pane.visible.load(Ordering::Acquire) || pane_state(pane) != "ready"
        || !registered_visible(&data, pane) { return false; }
    let changed = data.focused.as_deref() != Some(pane.label.as_str())
        || data.playing.as_deref() != Some(pane.label.as_str()) || !data.manual_playing;
    data.focused = Some(pane.label.clone());
    data.playing = Some(pane.label.clone());
    data.manual_playing = true;
    data.route_intents.retain(|label, _| label == &pane.label);
    changed
}
fn dismiss_menus(session: &XSession, app: &AppHandle, generation: u64, except: Option<String>, cancel_routing: bool) {
    let host = session.clone();
    let owned = app.clone();
    let _ = app.run_on_main_thread(move || {
        let panes = {
            let data = host.surfaces.lock().unwrap_or_else(|p| p.into_inner());
            if data.generation != generation { return; }
            if let Some(label) = except.as_deref() {
                if data.focused.as_deref() != Some(label)
                    || !data.feeds.values().chain(data.preview.iter()).any(|pane|
                        pane.label == label && pane.revision == host.revision.load(Ordering::Acquire)
                        && pane.visible.load(Ordering::Acquire) && pane_state(pane) == "ready"
                        && registered_visible(&data, pane)) { return; }
            }
            data.feeds.values().chain(data.preview_tabs.iter().map(|tab| &tab.pane))
                .filter(|pane| pane.revision == host.revision.load(Ordering::Acquire)
                    && except.as_deref() != Some(pane.label.as_str()))
                .map(|pane| pane.label.clone()).collect::<Vec<_>>()
        };
        for label in panes {
            if let Some(view) = owned.get_webview(&label) {
                // Idempotent bridge only; never synthesize a source click/focus.
                let script = if cancel_routing { "window.__atsumiXSurface?.dismissMenus(true)" }
                    else { "window.__atsumiXSurface?.dismissMenus(false)" };
                let _ = view.eval(script);
            }
        }
    });
}
fn route_intent_target(intent: &RouteIntent, generation: u64, current: &Url, target: &Url) -> bool {
    intent.generation == generation && intent.since.elapsed() <= Duration::from_millis(1000)
        && intent.source == *current && allowed(target)
        && target.query().is_none() && target.fragment().is_none()
        && target.path() != current.path()
        && (actions::base_post_path(target.path()) || profile_path(target.path()))
}
fn intercept_source_navigation(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, target: &Url) -> bool {
    // Only a narrowly armed genuine source click may divert its next native
    // navigation. No target is guessed from neighbouring post/author content.
    let intent = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        let Some(intent) = data.route_intents.remove(&pane.label) else { return false; };
        (intent, data.generation)
    };
    if pane.feed == "preview" && !intent.0.background || !interaction_allowed(session, pane) { return false; }
    let Some(current) = app.get_webview(&pane.label).and_then(|view| view.url().ok()) else { return false; };
    if !route_intent_target(&intent.0, intent.1, &current, target) { return false; }
    let host = session.clone(); let owned = app.clone(); let source = pane.clone();
    let path = target.path().to_owned();
    tauri::async_runtime::spawn(async move {
        let _lane = host.surface_lane.lock().await;
        if !interaction_allowed(&host, &source)
            || host.surfaces.lock().unwrap_or_else(|p| p.into_inner()).order != intent.0.origin.order
            || !owned.get_webview(&source.label).and_then(|view| view.url().ok()).is_some_and(|url| url == current) { return; }
        let feed = host.surfaces.lock().unwrap_or_else(|p| p.into_inner()).active.clone();
        let _ = open_preview(&host, &owned, &path, &source.owner, &feed, intent.0.background, intent.0.origin).await;
    });
    true
}
fn dispatch_key(session: &XSession, app: &AppHandle, source: &Arc<Pane>, key: &str) {
    if !matches!(key, "Escape" | "ArrowLeft" | "ArrowRight") { return; }
    let (target, generation) = {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        let Some(target) = input_target(&data, source) else { return; };
        (target, data.generation)
    };
    let script = format!("window.__atsumiXInput?.key({})", serde_json::to_string(key).unwrap());
    let host = session.clone();
    let owned = app.clone();
    let source = source.clone();
    let _ = app.run_on_main_thread(move || {
        if source.revision != host.revision.load(Ordering::Acquire) { return; }
        let admitted = {
            let data = host.surfaces.lock().unwrap_or_else(|p| p.into_inner());
            data.generation == generation && input_target(&data, &source).is_some_and(|current| current.label == target.label)
        };
        if !admitted { return; }
        let Some(view) = owned.get_webview(&target.label) else { return; };
        if !view.url().is_ok_and(|url| allowed(&url) && !url.path().starts_with("/__atsumi_surface__/"))
            || !view.window().is_visible().unwrap_or(false) || view.window().is_minimized().unwrap_or(true) { return; }
        // No host/window focus or keyboard injection: the last user-selected
        // native pane consumes only this closed, already authorized key.
        let _ = view.eval(script);
    });
}
fn capture_pane_current(session: &XSession, pane: &Pane, generation: u64, profile: &str) -> bool {
    if pane.revision != session.revision.load(Ordering::Acquire)
        || pane.profile != profile || !pane.visible.load(Ordering::Acquire) || pane_state(pane) != "ready" {
        return false;
    }
    let same_account = session.registry.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|registry| {
        registry.active == profile && registry.accounts.iter().any(|account| account.id == profile
            && account.handle.eq_ignore_ascii_case(&pane.owner))
    });
    if !same_account { return false; }
    let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
    generation == data.generation && pane.revision == session.revision.load(Ordering::Acquire)
        && registered_visible(&data, pane)
}
/// Called on a blocking worker by the trusted local archive IPC. Inspection
/// never creates/navigates a source document or reads a hidden/cached pane.
pub(crate) fn inspect_archive(
    session: &XSession,
    app: &AppHandle,
    generation: u64,
    target: &str,
) -> Result<super::archive::ArchiveDraft, String> {
    let pane = {
        let data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if data.generation != generation { return Err(STALE.into()); }
        match target {
            "feed" => data.feeds.get(&data.active),
            "preview" => data.preview.as_ref(),
            _ => return Err(ERROR.into()),
        }.cloned().ok_or(STALE)?
    };
    let profile = pane.profile.clone();
    if !capture_pane_current(session, &pane, generation, &profile) { return Err(STALE.into()); }
    let view = app.get_webview(&pane.label).ok_or(STALE)?;
    let expected = view.url().map_err(|_| STALE)?;
    if !allowed(&expected) { return Err(STALE.into()); }
    let current = || capture_pane_current(session, &pane, generation, &profile)
        && view.window().is_visible().unwrap_or(false)
        && !view.window().is_minimized().unwrap_or(true)
        && view.url().is_ok_and(|url| url == expected);
    if !current() { return Err(STALE.into()); }
    let observed = pane.media.snapshot(session.revision.load(Ordering::Acquire));
    let draft = session.archive.inspect_view_with_observed(&view, expected.as_str(), &profile, &pane.owner, &current, &observed)?;
    if !current() {
        // Freeze succeeded, but no caller will receive its identity to release
        // it. Do not leave an unreachable chooser slot until its TTL expires.
        let _lease = session.archive.lease_draft(&draft);
        return Err(STALE.into());
    }
    Ok(draft)
}
fn external(path: &str) {
    if !navigation_path(path) {
        return;
    }
    open_browser_url(&format!("https://x.com{path}"));
}
fn external_link(value: &str) {
    let Some(url) = external_link_url(value) else { return; };
    open_browser_url(url.as_str());
}
fn open_browser_url(value: &str) {
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        let url = value
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            let _ = ShellExecuteW(
                None,
                windows::core::w!("open"),
                PCWSTR(url.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = value;
}
fn guard_pane_root(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, status: &DocumentStatus) -> bool {
    // Never turn authentication/identity failure into a silent navigation.
    // Recheck the live URL as well as the authenticated native envelope, so a
    // delayed status cannot close or redirect a newer document.
    if status.state != "ready" || status.owner.as_deref() != Some(&pane.owner) { return false; }
    let Some(view) = app.get_webview(&pane.label) else { return false; };
    if !view.url().is_ok_and(|url| allowed(&url) && url.path() == status.path) { return false; }
    match root_action(&pane.feed, &pane.owner, &status.path) {
        RootAction::Keep => {
            if collection_kind(&status.path, &pane.owner) == Some(pane.feed.as_str()) {
                *pane.root_recovery.lock().unwrap_or_else(|p| p.into_inner()) = RootRecovery::default();
            }
            false
        }
        RootAction::ClosePreview => {
            close_preview_root(session, app, pane);
            true
        }
        RootAction::RestoreCollection => {
            let step = pane.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).request();
            if step == RecoveryStep::Pending { return true; }
            let state = if step == RecoveryStep::Start { "loading" } else { "error" };
            *pane.state.lock().unwrap_or_else(|p| p.into_inner()) = state.into();
            if step == RecoveryStep::Start {
                *pane.loading_since.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
            }
            conceal(app, pane);
            changed(session, app);
            if step == RecoveryStep::Failed { return true; }
            let Ok(path) = route(&pane.feed, &pane.owner) else { return true; };
            let Ok(target) = format!("https://x.com{path}").parse::<Url>() else { return true; };
            let host = session.clone();
            let owned = app.clone();
            let child = pane.clone();
            let previous_path = status.path.clone();
            let scheduled = app.run_on_main_thread(move || {
                if !is_current(&host, &child) {
                    child.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).pending = false;
                    return;
                }
                let Some(view) = owned.get_webview(&child.label) else {
                    child.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).pending = false;
                    *child.state.lock().unwrap_or_else(|p| p.into_inner()) = "error".into();
                    changed(&host, &owned);
                    return;
                };
                if !view.url().is_ok_and(|url| allowed(&url) && url.path() == previous_path) {
                    child.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).pending = false;
                    // The user/source already navigated elsewhere. Refresh
                    // that document's proof instead of clobbering its route.
                    let _ = view.eval("window.__atsumiXSurface?.refresh()");
                    return;
                }
                if view.navigate(target).is_err() {
                    child.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).pending = false;
                    *child.state.lock().unwrap_or_else(|p| p.into_inner()) = "error".into();
                    changed(&host, &owned);
                }
            });
            if scheduled.is_err() {
                pane.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).pending = false;
                *pane.state.lock().unwrap_or_else(|p| p.into_inner()) = "error".into();
                changed(session, app);
            }
            true
        }
    }
}
fn handle_signal(session: &XSession, app: &AppHandle, pane: &Arc<Pane>, kind: &str, value: &str) {
    if !is_current(session, pane) {
        return;
    }
    if kind == "status" {
        let Ok(status) = serde_json::from_str::<DocumentStatus>(value) else {
            return;
        };
        if !matches!(
            status.state.as_str(),
            "ready" | "loading" | "loginRequired" | "ownerChanged"
        ) || status.path.len() > 1000
            || !status.path.starts_with('/')
        {
            return;
        }
        if guard_pane_root(session, app, pane, &status) { return; }
        let state = if status.state == "ready" && status.owner.as_deref() != Some(&pane.owner) {
            "ownerChanged"
        } else {
            &status.state
        };
        #[cfg(debug_assertions)]
        if let Some(proof) = status.proof.as_ref() {
            let feed = match pane.feed.as_str() { "foryou"=>"foryou", "following"=>"following", "communities"=>"communities", "likes"=>"likes", "bookmarks"=>"bookmarks", "preview"=>"preview", _=>"other" };
            super::debug_shape("official-surface", "readiness", &serde_json::json!({
                "feed":feed,"state":state,"profileLink":proof.profile,"accountSwitcher":proof.account,
                "primaryColumn":proof.primary,"photoDialog":proof.photo,"login":proof.login,
                "photoShape":proof.photo_shape.as_ref().map(PhotoShape::diagnostic),
                "accountShape":proof.account_shape.as_ref().map(AccountShape::diagnostic),
                "adShape":proof.ad_shape.as_ref().map(AdShape::diagnostic)
            }));
        }
        let mut previous = pane.state.lock().unwrap_or_else(|p| p.into_inner());
        if previous.as_str() != state {
            tracing::info!(diag_stage="x_surface_state", feed=%pane.feed, state, "X official surface state changed");
            if state == "loading" {
                *pane.loading_since.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
            }
        }
        *previous = state.into();
        drop(previous);
        if matches!(state, "ready" | "loading") {
            if let Some(current) = app.get_webview(&pane.label).and_then(|view| view.url().ok()) {
                observe_preview_path(&mut session.surfaces.lock().unwrap_or_else(|p| p.into_inner()), pane, &status.path, &current);
            }
        }
        if state == "ready" {
            session.data.lock().unwrap_or_else(|p| p.into_inner()).owner = Some(pane.owner.clone());
            if status.proof.as_ref().is_some_and(|p|p.profile && p.account && !p.login) {
                if let Some(observed) = status.account_presentation {
                    super::account_presentation::observe(session, app, &pane.profile, &pane.owner, pane.revision, observed);
                }
            }
        }
        paint(session, app);
        changed(session, app);
        return;
    }
    // Receipt reconciliation grants no capture/download authority. Valid
    // submissions rejected by readiness still need a definitive failure.
    // The bridge has already authenticated the current source and pane nonce.
    if kind == "archiveStatus" { actions::status(session, app, pane, value); return; }
    if kind == "archive" { actions::start(session, app, pane, value); return; }
    if kind == "archiveCancel" { actions::cancel(session, app, pane, value); return; }
    if kind == "search" { search::start(session, app, pane, value); return; }
    if !interaction_allowed(session, pane) {
        return;
    }
    match kind {
        #[cfg(debug_assertions)]
        "photoProbe" => {
            // Same current pane/account/viewport fence as other interactions.
            // Only log the closed shape: no repaint, navigation or state write.
            if let Ok(probe) = serde_json::from_str::<PhotoShape>(value) {
                if matches!(&probe.route, PhotoRoute::Photo) && (1..=4).contains(&probe.sample) {
                    super::debug_shape("official-surface", "photoProbe", &probe.diagnostic());
                }
            }
        }
        "archiveChoose" => actions::choose(session, app, pane, value),
        #[cfg(debug_assertions)]
        "routeProbe" if pane.feed != "preview" => {
            // This read-only debug message has the same source/nonce/current
            // and current viewport admission as an interaction, without paint,
            // navigation, source strings or a general remote IPC capability.
            if let Ok(probe) = serde_json::from_str::<RoutingProbe>(value) {
                super::debug_shape("official-surface", "routing-unresolved", &probe.diagnostic());
            }
        }
        "play" => {
            let _ = claim_playback(session, pane, value == "manual");
            paint(session, app);
        }
        "focus" => {
            if claim_input(session, pane) {
                let generation = session.surfaces.lock().unwrap_or_else(|p| p.into_inner()).generation;
                dismiss_menus(session, app, generation, Some(pane.label.clone()), true);
            }
        }
        "wheel" => {
            if claim_scroll(session, pane) {
                let generation = session.surfaces.lock().unwrap_or_else(|p| p.into_inner()).generation;
                dismiss_menus(session, app, generation, Some(pane.label.clone()), true);
                // Only native pane audio ownership changes. No source mute,
                // media.play(), synthetic scroll, or keyboard focus request.
                paint(session, app);
            }
        }
        "routeIntent" if pane.feed != "preview" || value != "arm" => {
            let source = app.get_webview(&pane.label).and_then(|view| view.url().ok());
            let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
            data.route_intents.remove(&pane.label);
            if matches!(value, "arm" | "armBackground") {
                if let Some(source) = source.filter(|url| allowed(url)
                    && !url.path().starts_with("/__atsumi_surface__/")) {
                    let generation = data.generation;
                    let origin = preview_origin(&data);
                    data.route_intents.insert(pane.label.clone(), RouteIntent {
                        source, generation, since: Instant::now(), background: value == "armBackground", origin,
                    });
                }
            }
        }
        "key" => dispatch_key(session, app, pane, value),
        "external" => external(value),
        "externalLink" => external_link(value),
        "close" if pane.feed == "preview" => {
            let old = {
                let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
                if !input_target(&data, pane).is_some_and(|current| current.label == pane.label) { return; }
                let Ok(old) = take_preview(&mut data, Some(&pane.label), None) else { return; };
                old
            };
            finish_preview_close(session, app, old, true);
        }
        "preview" | "previewBackground" if pane.feed != "preview" || kind == "previewBackground" => {
            let host = session.clone();
            let app = app.clone();
            let pane = pane.clone();
            let path = value.to_owned();
            let background = kind == "previewBackground";
            let origin = preview_origin(&session.surfaces.lock().unwrap_or_else(|p| p.into_inner()));
            tauri::async_runtime::spawn(async move {
                let _lane = host.surface_lane.lock().await;
                if !is_current(&host, &pane) || !interaction_allowed(&host, &pane) {
                    return;
                }
                let feed = host.surfaces.lock().unwrap_or_else(|p| p.into_inner()).active.clone();
                let _ = open_preview(&host, &app, &path, &pane.owner, &feed, background, origin).await;
            });
        }
        _ => (),
    }
}
async fn create(
    session: &XSession,
    app: &AppHandle,
    feed: &str,
    owner: &str,
    path: &str,
) -> Result<Arc<Pane>, String> {
    let registry = session.registry(app)?;
    let directory = accounts::profile(app, &registry.active)?;
    let revision = session.revision.load(Ordering::Acquire);
    let pane = Arc::new(Pane {
        label: format!("x-surface-{}", uuid::Uuid::new_v4().simple()),
        nonce: uuid::Uuid::new_v4().simple().to_string(),
        feed: feed.into(),
        owner: owner.into(),
        profile: registry.active,
        revision,
        state: Mutex::new("loading".into()),
        loading_since: Mutex::new(Instant::now()),
        document_load: Mutex::new(DocumentLoad::default()),
        root_recovery: Mutex::new(RootRecovery::default()),
        visible: AtomicBool::new(false),
        fullscreen: AtomicBool::new(false),
        media: Arc::new(surface_media::Observations::new(revision)),
    });
    let config = serde_json::json!({"nonce":pane.nonce,"feed":feed,"owner":owner,"diagnostics":cfg!(debug_assertions),"searchEnabled":search::available(),"searchCapabilities":search::capabilities()}).to_string();
    let script = [
        include_str!("x_surface_ads.js").replace("__ATSUMI_X_ADS__", &config),
        include_str!("x_surface_actions.js").replace("__ATSUMI_X_ACTIONS__", &config),
        include_str!("x_surface_input.js").replace("__ATSUMI_X_INPUT__", &config),
        include_str!("x_surface.js").replace("__ATSUMI_X_SURFACE__", &config),
    ].join("\n");
    let target: Url = format!("https://x.com{path}").parse().map_err(|_| ERROR)?;
    let host = session.clone();
    let owned = app.clone();
    let child = pane.clone();
    let live = Arc::new(AtomicBool::new(true));
    let wanted = live.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        if !wanted.load(Ordering::Acquire)
            || host.revision.load(Ordering::Acquire) != child.revision
        {
            let _ = tx.send(Err(STALE.into()));
            return;
        }
        let built = (|| -> Result<(), String> {
            owned.state::<AppState>().begin_platform_work(|| Ok(()))?;
            let parent = owned.get_window("main").ok_or(ERROR)?;
            let page_app = owned.clone();
            let page_host = host.clone();
            let page_pane = child.clone();
            let navigation_app = owned.clone();
            let navigation_host = host.clone();
            let navigation_pane = child.clone();
            let popup_app = owned.clone();
            let popup_host = host.clone();
            let popup_pane = child.clone();
            let builder = WebviewBuilder::new(
                &child.label,
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .data_directory(directory)
            .focused(false)
            .browser_extensions_enabled(false)
            .additional_browser_args(crate::browser_identity::BROWSER_ARGS)
            .initialization_script(script)
            .on_navigation(move |url| {
                if url.path().starts_with("/__atsumi_surface__/") {
                    // Reserved legacy bridge URLs are never actual navigation
                    // or a second transport. Only native messages are handled.
                    return false;
                }
                if intercept_source_navigation(&navigation_host, &navigation_app, &navigation_pane, url) {
                    return false;
                }
                allowed(url) || url.as_str() == "about:blank"
            })
            .on_new_window(move |url, _| {
                let _ = intercept_source_navigation(&popup_host, &popup_app, &popup_pane, &url);
                tauri::webview::NewWindowResponse::Deny
            })
            .on_download(|_, _| false)
            .on_page_load(move |_, payload| {
                if page_pane.revision != page_host.revision.load(Ordering::Acquire) { return; }
                let mut load = page_pane.document_load.lock().unwrap_or_else(|p| p.into_inner());
                let started = payload.event() == tauri::webview::PageLoadEvent::Started && load.start(payload.url());
                let finished = payload.event() == tauri::webview::PageLoadEvent::Finished && load.finish(payload.url());
                drop(load);
                if started {
                    page_pane.root_recovery.lock().unwrap_or_else(|p| p.into_inner()).pending = false;
                    page_pane.media.clear();
                    *page_pane.state.lock().unwrap_or_else(|p| p.into_inner()) = "loading".into();
                    *page_pane
                        .loading_since
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = Instant::now();
                    conceal(&page_app, &page_pane);
                } else if finished && is_current(&page_host, &page_pane) {
                    if let Some(view) = page_app.get_webview(&page_pane.label) {
                        let _ = view.eval("window.__atsumiXSurface?.refresh()");
                    }
                }
            });
            let view = parent
                .add_child(
                    builder,
                    LogicalPosition::new(0., 0.),
                    LogicalSize::new(1024., 900.),
                )
                .map_err(|_| ERROR)?;
            view.hide().map_err(|_| ERROR)?;
            mute(&view, true, None);
            install_bridge(&view, host.clone(), child.clone())?;
            install_native_audio(&view, host.clone(), child.clone())?;
            fullscreen::install(&view, host.clone(), child.clone())?;
            // Passive only: unlike extracted readers, original documents keep
            // their normal source media network/playback behavior intact.
            if surface_media::install(&view, child.media.clone(), host.revision.clone()).is_err() {
                // Archive observation is optional for the official page. A
                // partially installed handler is revoked and remains inert;
                // missing media stays explicitly unavailable in the archive.
                child.media.revoke();
                #[cfg(debug_assertions)]
                super::debug_shape("official-surface", "media-observation-unavailable", &serde_json::json!({"passThrough":true}));
            }
            if !wanted.load(Ordering::Acquire)
                || host.revision.load(Ordering::Acquire) != child.revision
            {
                let _ = view.close();
                return Err(STALE.into());
            }
            view.navigate(target).map_err(|_| ERROR)?;
            Ok(())
        })();
        if built.is_err() {
            dispose(&owned, &child);
        }
        let _ = tx.send(built);
    })
    .map_err(|_| ERROR)?;
    let result = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| ERROR.to_string())
        .and_then(|r| r.map_err(|_| ERROR.to_string()))
        .and_then(|r| r);
    live.store(false, Ordering::Release);
    if let Err(e) = result {
        dispose(app, &pane);
        return Err(e);
    }
    if pane.revision != session.revision.load(Ordering::Acquire) {
        dispose(app, &pane);
        return Err(STALE.into());
    }
    Ok(pane)
}
async fn open_preview(
    session: &XSession,
    app: &AppHandle,
    path: &str,
    owner: &str,
    feed: &str,
    background: bool,
    origin: PreviewOpenOrigin,
) -> Result<(), String> {
    if !navigation_path(path) {
        return Err(ERROR.into());
    }
    let (existing, intent, order) = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if data.active != feed || data.order != origin.order {
            return Err(STALE.into());
        }
        let existing = existing_preview(&data, path, owner, session.revision.load(Ordering::Acquire));
        if let Some(label) = existing.as_deref() {
            if preview_wants_foreground(&data, origin, background) { select_preview(&mut data, label, None)?; }
        } else {
            if !preview_has_capacity(&data) {
                data.preview_error = Some(PREVIEW_FULL.into());
                drop(data);
                changed(session, app);
                return Err(PREVIEW_FULL.into());
            }
            data.preview_intent += 1;
        }
        (existing, data.preview_intent, data.order)
    };
    if existing.is_some() {
        let generation = session.surfaces.lock().unwrap_or_else(|p| p.into_inner()).generation;
        dismiss_menus(session, app, generation, None, false);
        paint(session, app);
        changed(session, app);
        return Ok(());
    }
    // Callers hold surface_lane: different rapid opens are retained in order,
    // and duplicate targets cannot allocate two loading WebViews.
    let pane = match create(session, app, "preview", owner, path).await {
        Ok(pane) => pane,
        Err(error) => {
            let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
            if data.order == order && data.active == feed && data.preview_intent == intent {
                data.preview_error = Some(ERROR.into());
                drop(data);
                changed(session, app);
            }
            return Err(error);
        }
    };
    {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if pane.revision != session.revision.load(Ordering::Acquire)
            || data.order != order
            || data.active != feed
        {
            drop(data);
            dispose(app, &pane);
            return Err(STALE.into());
        }
        let background = !preview_wants_foreground(&data, origin, background);
        if let Err(error) = register_preview_mode(&mut data, pane.clone(), path, intent, background) {
            drop(data);
            dispose(app, &pane);
            return Err(error);
        }
    }
    if let Some(view) = app.get_webview(&pane.label) {
        let _ = view.eval("window.__atsumiXSurface?.refresh()");
    }
    paint(session, app);
    changed(session, app);
    Ok(())
}
fn existing_preview(data: &Surfaces, path: &str, owner: &str, revision: u64) -> Option<String> {
    data.preview_tabs.iter().find(|tab| tab.path == path && tab.pane.revision == revision
        && tab.pane.owner == owner).map(|tab| tab.pane.label.clone())
}
fn observe_preview_path(data: &mut Surfaces, pane: &Pane, path: &str, current: &Url) {
    if pane.feed != "preview" || !allowed(current) || current.path() != path || !navigation_path(path) { return; }
    if let Some(tab) = data.preview_tabs.iter_mut().find(|tab| tab.pane.label == pane.label
        && tab.pane.revision == pane.revision && tab.pane.profile == pane.profile) {
        tab.path = path.into();
    }
}
fn preview_has_capacity(data: &Surfaces) -> bool { data.preview_tabs.len() < PREVIEW_LIMIT }
#[cfg(test)]
fn register_preview(data: &mut Surfaces, pane: Arc<Pane>, path: &str, intent: u64) -> Result<(), String> {
    register_preview_mode(data, pane, path, intent, false)
}
fn register_preview_mode(data: &mut Surfaces, pane: Arc<Pane>, path: &str, intent: u64, background: bool) -> Result<(), String> {
    if !navigation_path(path)
        || existing_preview(data, path, &pane.owner, pane.revision).is_some() { return Err(STALE.into()); }
    if !preview_has_capacity(data) || data.preview_tabs.iter().any(|tab| tab.pane.label == pane.label) {
        return Err(PREVIEW_FULL.into());
    }
    data.preview_tabs.push(PreviewTab { pane: pane.clone(), path: path.into() });
    // A newer explicit selection/close wins without silently discarding
    // the requested page whose creation was already in flight.
    if !background && data.preview_intent == intent {
        select_preview(data, &pane.label, None)?;
    } else {
        data.generation += 1;
    }
    data.preview_error = None;
    Ok(())
}
fn select_preview(data: &mut Surfaces, label: &str, expected_generation: Option<u64>) -> Result<(), String> {
    if expected_generation.is_some_and(|generation| generation != data.generation) { return Err(STALE.into()); }
    let pane = data.preview_tabs.iter().find(|tab| tab.pane.label == label)
        .map(|tab| tab.pane.clone()).ok_or(STALE)?;
    data.preview_intent += 1;
    data.route_intents.clear();
    if data.preview.as_ref().is_some_and(|current| current.label == label) {
        data.preview_error = None;
        return Ok(());
    }
    if data.preview.as_ref().is_some_and(|old| data.playing.as_deref() == Some(old.label.as_str())) {
        data.playing = None;
        data.manual_playing = false;
    }
    // Revoke hidden tab gates immediately; paint will also hide/mute its view.
    if let Some(old) = data.preview.as_ref() { old.visible.store(false, Ordering::Release); }
    data.focused = Some(pane.label.clone());
    data.preview = Some(pane);
    data.preview_error = None;
    data.generation += 1;
    if let Some(viewport) = data.viewport.as_mut() { viewport.chat = None; }
    Ok(())
}
fn select_preview_explicit(data: &mut Surfaces, label: &str, generation: u64) -> Result<(), String> {
    select_preview(data, label, Some(generation))?;
    data.preview_selection += 1;
    Ok(())
}
fn preview_matches(data: &Surfaces, expected: Option<&str>) -> bool {
    expected.is_none_or(|label| data.preview.as_ref().is_some_and(|pane| pane.label == label))
}
fn take_preview(
    data: &mut Surfaces,
    expected_label: Option<&str>,
    expected_generation: Option<u64>,
) -> Result<Option<Arc<Pane>>, String> {
    // The caller holds the surfaces lock across both admission and removal.
    // A rejected old UI command must not even clear the new feed's selection.
    if expected_generation.is_some_and(|generation| generation != data.generation)
        || !preview_matches(data, expected_label) {
        return Err(STALE.into());
    }
    let Some(label) = data.preview.as_ref().map(|pane| pane.label.clone()) else { return Ok(None); };
    remove_preview(data, &label, expected_generation).map(Some)
}
fn remove_preview(data: &mut Surfaces, label: &str, expected_generation: Option<u64>) -> Result<Arc<Pane>, String> {
    if expected_generation.is_some_and(|generation| generation != data.generation) { return Err(STALE.into()); }
    let index = data.preview_tabs.iter().position(|tab| tab.pane.label == label).ok_or(STALE)?;
    let active = data.preview.as_ref().is_some_and(|pane| pane.label == label);
    let old = data.preview_tabs.remove(index).pane;
    data.generation += 1;
    data.preview_intent += 1;
    data.preview_selection += 1;
    data.preview_error = None;
    data.route_intents.clear();
    let feed = data.active.clone();
    data.preview_paths.remove(&feed);
    if data.playing.as_deref() == Some(label) {
        data.playing = None;
        data.manual_playing = false;
    }
    old.visible.store(false, Ordering::Release);
    if active {
        let adjacent = index.min(data.preview_tabs.len().saturating_sub(1));
        data.preview = data.preview_tabs.get(adjacent).map(|tab| tab.pane.clone());
        data.focused = data.preview.as_ref().map(|pane| pane.label.clone())
            .or_else(|| data.feeds.get(&data.active).map(|pane| pane.label.clone()));
        if let Some(viewport) = data.viewport.as_mut() { viewport.chat = None; }
    }
    Ok(old)
}
fn finish_preview_close(session: &XSession, app: &AppHandle, old: Option<Arc<Pane>>, cancel_routing: bool) {
    search::invalidate(session, app);
    if let Some(old) = old {
        dispose(app, &old);
    }
    let generation = session.surfaces.lock().unwrap_or_else(|p| p.into_inner()).generation;
    dismiss_menus(session, app, generation, None, cancel_routing);
    paint(session, app);
    changed(session, app);
}
fn take_root_preview(data: &mut Surfaces, pane: &Arc<Pane>, revision: u64) -> Result<Arc<Pane>, String> {
    if pane.revision != revision || !data.preview_tabs.iter()
        .any(|tab| Arc::ptr_eq(&tab.pane, pane)) { return Err(STALE.into()); }
    remove_preview(data, &pane.label, None)
}
fn close_preview_root(session: &XSession, app: &AppHandle, pane: &Arc<Pane>) {
    let old = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        // Root recovery has current URL/owner proof. It may retire that exact
        // retained document even while hidden, never whichever tab is active.
        // Escape remains separately restricted to the current input target.
        let Ok(old) = take_root_preview(&mut data, pane, session.revision.load(Ordering::Acquire)) else { return; };
        old
    };
    finish_preview_close(session, app, Some(old), false);
}

#[tauri::command(rename_all = "camelCase")]
pub async fn x_surface_open(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    feed: String,
    order: u64,
    refresh: bool,
) -> Result<SurfaceSnapshot, String> {
    trusted(&window)?;
    if order == 0 || order > 9_007_199_254_740_991 {
        return Err(ERROR.into());
    }
    let revision = session.revision.load(Ordering::Acquire);
    let open_epoch = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if order <= data.order {
            return Err(STALE.into());
        }
        data.order = order;
        data.open_epoch
    };
    let _lane = session.surface_lane.lock().await;
    let registry = session.registry(&app)?;
    let owner = registry
        .accounts
        .iter()
        .find(|a| a.id == registry.active)
        .ok_or(LOGIN)?
        .handle
        .to_lowercase();
    let path = route(&feed, &owner)?;
    let old = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if !feed_open_current(&data, order, open_epoch) || revision != session.revision.load(Ordering::Acquire) {
            return Err(STALE.into());
        }
        data.active = feed.clone();
        data.generation += 1;
        data.viewport = None;
        data.playing = None;
        data.manual_playing = false;
        data.focused = None;
        data.preview_intent += 1;
        data.preview_selection += 1;
        data.preview_error = None;
        data.route_intents.clear();
        let replaced = if refresh {
            data.feeds.remove(&feed)
        } else {
            None
        };
        (
            replaced,
            data.feeds.values().chain(data.preview_tabs.iter().map(|tab| &tab.pane)).cloned().collect::<Vec<_>>(),
            data.feeds.contains_key(&feed),
        )
    };
    search::invalidate(&session, &app);
    if let Some(p) = old.0 {
        dispose(&app, &p);
    }
    for p in old.1 {
        conceal(&app, &p);
    }
    if !old.2 {
        let pane = create(&session, &app, &feed, &owner, &path).await?;
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if !feed_open_current(&data, order, open_epoch) || revision != session.revision.load(Ordering::Acquire) {
            drop(data);
            dispose(&app, &pane);
            return Err(STALE.into());
        }
        data.feeds.insert(feed.clone(), pane.clone());
        drop(data);
        if let Some(view) = app.get_webview(&pane.label) {
            let _ = view.eval("window.__atsumiXSurface?.refresh()");
        }
    }
    Ok(snapshot(&session))
}
fn feed_open_current(data: &Surfaces, order: u64, epoch: u64) -> bool {
    data.order == order && data.open_epoch == epoch
}
#[tauri::command]
pub fn x_surface_suspend(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
) -> Result<(), String> {
    trusted(&window)?;
    let panes = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        suspend_documents(&mut data)
    };
    search::invalidate(&session, &app);
    for pane in panes { conceal(&app, &pane); }
    Ok(())
}
fn suspend_documents(data: &mut Surfaces) -> Vec<Arc<Pane>> {
    // Retain the actual source documents (and their browser scroll/history),
    // but invalidate every viewport/route/selection callback from the old view.
    // Account resets and app shutdown still call close_all independently.
    data.generation += 1;
    // Independent of the shell's monotonic/clock request serial: revoking an
    // open must not consume the very next valid shell request number.
    data.open_epoch += 1;
    data.viewport = None;
    data.playing = None;
    data.manual_playing = false;
    data.focused = None;
    data.preview_intent += 1;
    data.preview_selection += 1;
    data.route_intents.clear();
    let panes = data.feeds.values().chain(data.preview_tabs.iter().map(|tab| &tab.pane))
        .cloned().collect::<Vec<_>>();
    for pane in &panes { pane.visible.store(false, Ordering::Release); }
    panes
}
#[tauri::command]
pub fn x_surface_status(
    window: Webview,
    session: State<'_, XSession>,
) -> Result<SurfaceSnapshot, String> {
    trusted(&window)?;
    Ok(snapshot(&session))
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_surface_viewport(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    generation: u64,
    viewport: NativeViewport,
) -> Result<(), String> {
    trusted(&window)?;
    viewport.validate()?;
    {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if data.generation != generation
            || data
                .viewport
                .as_ref()
                .is_some_and(|v| v.sequence >= viewport.sequence)
        {
            return Ok(());
        }
        data.viewport = Some(viewport);
    }
    paint(&session, &app);
    Ok(())
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_surface_close_preview(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    generation: u64,
    preview_id: Option<String>,
) -> Result<SurfaceSnapshot, String> {
    trusted(&window)?;
    let old = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        match preview_id.as_deref() {
            Some(label) => Some(remove_preview(&mut data, label, Some(generation))?),
            None => take_preview(&mut data, None, Some(generation))?,
        }
    };
    finish_preview_close(&session, &app, old, true);
    Ok(snapshot(&session))
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_surface_select_preview(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    generation: u64,
    preview_id: String,
) -> Result<SurfaceSnapshot, String> {
    trusted(&window)?;
    let generation = {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        select_preview_explicit(&mut data, &preview_id, generation)?;
        data.generation
    };
    search::invalidate(&session, &app);
    dismiss_menus(&session, &app, generation, None, true);
    paint(&session, &app);
    changed(&session, &app);
    Ok(snapshot(&session))
}
#[tauri::command(rename_all = "camelCase")]
pub fn x_surface_dismiss_menus(
    window: Webview,
    app: AppHandle,
    session: State<'_, XSession>,
    generation: u64,
) -> Result<(), String> {
    trusted(&window)?;
    {
        let mut data = session.surfaces.lock().unwrap_or_else(|p| p.into_inner());
        if data.generation != generation { return Err(STALE.into()); }
        data.route_intents.clear();
    }
    dismiss_menus(&session, &app, generation, None, true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_are_selected_account_scoped() {
        assert_eq!(route("likes", "alice").unwrap(), "/alice/likes");
        assert_eq!(route("bookmarks", "alice").unwrap(), "/i/bookmarks");
        assert_eq!(route("communities", "alice").unwrap(), "/i/communities");
        assert!(route("anything", "alice").is_err());
        assert!(route("likes", "../bob").is_err());
    }
    #[test]
    fn pane_roots_do_not_turn_into_a_second_home_or_another_collection() {
        assert_eq!(root_action("preview", "alice", "/home"), RootAction::ClosePreview);
        for feed in ["likes", "bookmarks"] {
            assert_eq!(root_action(feed, "alice", "/home"), RootAction::RestoreCollection);
        }
        for path in ["/i/bookmarks", "/i/history/bookmarks", "/i/history"] {
            assert_eq!(root_action("likes", "alice", path), RootAction::RestoreCollection);
            assert_eq!(root_action("bookmarks", "alice", path), RootAction::Keep);
        }
        for path in ["/alice/likes", "/i/history/likes"] {
            assert_eq!(root_action("bookmarks", "alice", path), RootAction::RestoreCollection);
            assert_eq!(root_action("likes", "alice", path), RootAction::Keep);
        }
        // Detail/profile/photo history, gates and unknown/foreign collections
        // are not redirected by this narrowly scoped root guard.
        for feed in ["preview", "foryou", "following", "likes", "bookmarks"] {
            for path in ["/creator", "/creator/status/123456", "/creator/status/123456/photo/1",
                "/creator/status/123456/video/1", "/i/flow/login", "/login", "/bob/likes",
                "/home/extra", "/HOME", "/home?token=fixture", "/i/bookmarks/extra"] {
                assert_eq!(root_action(feed, "alice", path), RootAction::Keep, "{feed}: {path}");
            }
        }
        assert_eq!(root_action("foryou", "alice", "/home"), RootAction::Keep);
        assert_eq!(root_action("following", "alice", "/home"), RootAction::Keep);
    }
    #[test]
    fn collection_root_recovery_is_bounded_and_can_reset_only_after_success() {
        let mut recovery = RootRecovery::default();
        assert_eq!(recovery.request(), RecoveryStep::Start);
        for _ in 0..4 { assert_eq!(recovery.request(), RecoveryStep::Pending); }
        // A real document load has started; a second wrong ready root is a
        // failed recovery, not another automatic reload loop.
        recovery.pending = false;
        for _ in 0..4 { assert_eq!(recovery.request(), RecoveryStep::Failed); }
        recovery = RootRecovery::default(); // A confirmed matching root.
        assert_eq!(recovery.request(), RecoveryStep::Start);
    }
    #[test]
    fn queued_preview_closes_match_the_current_preview_not_a_cached_feed_or_old_label() {
        let (session, feed, preview) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        assert!(preview_matches(&data, None)); // Explicit local close-current.
        assert!(preview_matches(&data, Some(&preview.label)));
        assert!(!preview_matches(&data, Some(&feed.label)));
        assert!(!preview_matches(&data, Some("old-preview")));
        data.preview = None;
        assert!(!preview_matches(&data, Some(&preview.label)));
    }
    #[test]
    fn stale_local_close_cannot_mutate_the_new_collection_or_preview() {
        let (session, _, preview) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        data.active = "bookmarks".into();
        data.generation = 9;
        data.preview_intent = 12;
        data.preview_paths.insert("bookmarks".into(), "/creator/status/123456".into());
        data.playing = Some(preview.label.clone());
        data.manual_playing = true;
        for (label, generation) in [(None, Some(8)), (None, Some(10)), (Some("old-preview"), Some(9))] {
            assert!(matches!(take_preview(&mut data, label, generation), Err(error) if error == STALE));
            assert_eq!(data.generation, 9);
            assert_eq!(data.preview_intent, 12);
            assert_eq!(data.active, "bookmarks");
            assert_eq!(data.preview_paths.get("bookmarks").map(String::as_str), Some("/creator/status/123456"));
            assert_eq!(data.playing.as_deref(), Some(preview.label.as_str()));
            assert!(data.manual_playing);
            assert!(data.preview.as_ref().is_some_and(|current| Arc::ptr_eq(current, &preview)));
        }
        let removed = take_preview(&mut data, None, Some(9)).unwrap().unwrap();
        assert!(Arc::ptr_eq(&removed, &preview));
        assert_eq!(data.generation, 10);
        assert_eq!(data.preview_intent, 13);
        assert!(data.preview.is_none());
        assert!(!data.preview_paths.contains_key("bookmarks"));
        assert!(data.playing.is_none());
        assert!(!data.manual_playing);
        // Duplicate delivery of the same UI request is rejected atomically.
        assert!(matches!(take_preview(&mut data, None, Some(9)), Err(error) if error == STALE));
        assert_eq!(data.generation, 10);
        assert_eq!(data.preview_intent, 13);
    }
    #[test]
    fn bridge_cannot_open_arbitrary_targets() {
        for p in ["/alice/status/123456", "/i/web/status/123456/photo/2"] {
            assert!(post_path(p), "{p}");
        }
        for p in [
            "//evil.test/a",
            "/alice/status/012345",
            "/alice/status/123456?secret=x",
            "/alice/status/123456/../logout",
            "/i/bookmarks",
            "https://x.com/a/status/123456",
            "/a/status/123456/photo/9",
        ] {
            assert!(!post_path(p), "{p}");
        }
        let url = Url::parse(
            "https://x.com/__atsumi_surface__/nonce?kind=preview&value=%2Falice%2Fstatus%2F123456",
        )
        .unwrap();
        assert!(signal(&url, "nonce").is_some());
        assert!(signal(&url, "other").is_none());
        for s in [
            "https://evil.test/__atsumi_surface__/nonce?kind=close&value=",
            "https://x.com/__atsumi_surface__/nonce?kind=delete&value=",
            "https://x.com/__atsumi_surface__/nonce?kind=external&value=file:///c:/secret",
            "https://x.com/__atsumi_surface__/nonce?kind=close&value=&kind=external",
        ] {
            assert!(signal(&Url::parse(s).unwrap(), "nonce").is_none());
        }
    }
    #[test]
    fn external_link_bridge_accepts_only_bounded_http_web_targets() {
        for value in ["https://example.org/article?ref=one%20two#section", "http://example.org:8080/article", "https://t.co/ObservedLink"] {
            let mut packet = Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
            packet.query_pairs_mut().append_pair("kind", "externalLink").append_pair("value", value);
            assert_eq!(signal(&packet, "nonce"), Some(("externalLink".into(), value.into())));
            assert!(signal(&packet, "other").is_none());
            // The same URL never gains source/preview navigation authority.
            assert!(!navigation_path(value));
        }
        for value in ["javascript:alert(1)", "file:///C:/secret", "data:text/html,hello", "ms-settings:",
            "blob:https://example.org/id", "https://user:secret@example.org/", "https://example.org/a\\b",
            "https://example.org/a\nb", "https://example.org/a b", "/creator", "//example.org/path",
            "https://x.com/creator", "http://www.x.com/home", "https://x.com./creator", "https://www.x.com./home"] {
            assert!(external_link_url(value).is_none(), "{value}");
            let mut packet = Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
            packet.query_pairs_mut().append_pair("kind", "externalLink").append_pair("value", value);
            assert!(signal(&packet, "nonce").is_none(), "{value}");
        }
        assert!(external_link_url(&format!("https://example.org/{}", "a".repeat(2048))).is_none());
    }
    #[test]
    fn search_bridge_accepts_only_small_exact_media_intents() {
        let request = serde_json::json!({"id":"a3b88765-d811-4bbc-8f3b-ec32bff741dc","kind":"image","provider":"saucenao","path":"/artist/status/123456"});
        let packet = |value: &serde_json::Value| {
            let mut url=Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
            url.query_pairs_mut().append_pair("kind","search").append_pair("value",&value.to_string());url
        };
        assert!(signal(&packet(&request),"nonce").is_some());
        assert!(signal(&packet(&request),"stale").is_none());
        for (key,value) in [("provider",serde_json::json!("lens")),("kind",serde_json::json!("file")),("id",serde_json::json!("bad")),("path",serde_json::json!("https://example.org/")),("url",serde_json::json!("https://example.org/private")),("dataUrl",serde_json::json!("data:image/jpeg;base64,AA=="))] {
            let mut invalid=request.clone();invalid[key]=value;assert!(signal(&packet(&invalid),"nonce").is_none());
        }
        let current=Url::parse("https://x.com/home").unwrap();let body=format!("ATSUMI_X_SURFACE_V1:{}",packet(&request));
        assert!(bridge_message(current.as_str(),&current,&body,"nonce").is_some());
        assert!(bridge_message("https://x.com/other/status/123456",&current,&body,"nonce").is_none());
    }
    #[test]
    fn external_link_bridge_keeps_current_source_nonce_and_packet_fences() {
        let current = Url::parse("https://x.com/home").unwrap();
        let mut packet = Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
        packet.query_pairs_mut().append_pair("kind", "externalLink").append_pair("value", "https://example.org/article");
        let body = format!("ATSUMI_X_SURFACE_V1:{packet}");
        assert_eq!(bridge_message(current.as_str(), &current, &body, "nonce"), Some(("externalLink".into(), "https://example.org/article".into())));
        for source in ["https://example.org/", "https://x.com/other/status/123456", "https://x.com/home?old=1"] {
            assert!(bridge_message(source, &current, &body, "nonce").is_none());
        }
        assert!(bridge_message(current.as_str(), &current, &body, "stale").is_none());
        packet.query_pairs_mut().append_pair("kind", "externalLink");
        assert!(signal(&packet, "nonce").is_none());
    }
    #[test]
    fn profile_navigation_is_exact_and_excludes_site_actions() {
        for path in ["/alice", "/Creator_123", "/reposter", "/alice/status/123456"] {
            assert!(navigation_path(path), "{path}");
        }
        for path in ["/home", "/HOME", "/explore", "/search", "/notifications", "/messages",
            "/i", "/compose", "/settings", "/login", "/logout", "/signup", "/tos", "/privacy",
            "/help", "/about", "/download", "/jobs", "/communities", "/premium", "/grok",
            "/hashtag", "/intent", "/account", "/accounts", "/alice/", "/alice/likes",
            "/alice?token=fixture", "/alice#fragment", "/alice%2fbob", "//evil.test/alice",
            "https://x.com/alice", "/sixteenlettersxx"] {
            assert!(!navigation_path(path), "{path}");
        }
        let profile = Url::parse("https://x.com/__atsumi_surface__/nonce?kind=preview&value=%2Falice").unwrap();
        assert_eq!(signal(&profile, "nonce"), Some(("preview".into(), "/alice".into())));
        assert!(signal(&profile, "wrong").is_none());
    }
    #[test]
    fn playback_bridge_has_no_payload_or_arbitrary_command() {
        for intent in ["manual", "auto"] {
            let valid = Url::parse(&format!("https://x.com/__atsumi_surface__/nonce?kind=play&value={intent}")).unwrap();
            assert_eq!(signal(&valid, "nonce"), Some(("play".into(), intent.into())));
        }
        for url in ["https://x.com/__atsumi_surface__/nonce?kind=play&value=",
            "https://x.com/__atsumi_surface__/nonce?kind=play&value=MANUAL",
            "https://x.com/__atsumi_surface__/nonce?kind=play&value=%2Falice",
            "https://x.com/__atsumi_surface__/nonce?kind=play&value=&token=fixture",
            "https://x.com/__atsumi_surface__/nonce?kind=play&value=#other",
            "https://x.com/__atsumi_surface__/nonce?kind=execute&value=play",
            "https://x.com.evil.test/__atsumi_surface__/nonce?kind=play&value=",
            "https://user@x.com/__atsumi_surface__/nonce?kind=play&value=",
            "https://x.com:444/__atsumi_surface__/nonce?kind=play&value="] {
            assert!(signal(&Url::parse(url).unwrap(), "nonce").is_none(), "{url}");
        }
    }
    #[test]
    fn input_bridge_accepts_only_no_payload_focus_and_three_closed_keys() {
        let focus = Url::parse("https://x.com/__atsumi_surface__/nonce?kind=focus&value=").unwrap();
        assert_eq!(signal(&focus, "nonce"), Some(("focus".into(), "".into())));
        for key in ["Escape", "ArrowLeft", "ArrowRight"] {
            let url = Url::parse(&format!("https://x.com/__atsumi_surface__/nonce?kind=key&value={key}")).unwrap();
            assert_eq!(signal(&url, "nonce"), Some(("key".into(), key.into())));
        }
        for (kind, value) in [("focus", "preview"), ("focus", "script"), ("key", "Delete"),
            ("key", "a"), ("key", ""), ("key", "Escape;execute"), ("key", "/creator/status/123456")] {
            let url = Url::parse(&format!("https://x.com/__atsumi_surface__/nonce?kind={kind}&value={value}")).unwrap();
            assert!(signal(&url, "nonce").is_none());
        }
    }
    #[test]
    fn message_bridge_requires_exact_live_source_and_reuses_closed_nonce_validator() {
        let current = Url::parse("https://x.com/home").unwrap();
        let raw = "ATSUMI_X_SURFACE_V1:https://x.com/__atsumi_surface__/nonce?kind=play&value=manual";
        assert_eq!(bridge_message(current.as_str(), &current, raw, "nonce"), Some(("play".into(), "manual".into())));
        assert!(bridge_message(current.as_str(), &current, raw, "other").is_none());
        for source in ["https://x.com/i/bookmarks", "https://x.com/home?other=true", "https://x.com/home#other", "https://www.x.com/home", "https://evil.test/home", "about:blank"] {
            assert!(bridge_message(source, &current, raw, "nonce").is_none(), "{source}");
        }
        for raw in [raw.replacen("ATSUMI_X_SURFACE_V1:", "", 1), raw.replace("kind=play", "kind=execute"), raw.replace("value=manual", "value=unknown"), format!("{raw}&extra=true"), format!("ATSUMI_X_SURFACE_V1:{}", "x".repeat(4097))] {
            assert!(bridge_message(current.as_str(), &current, &raw, "nonce").is_none());
        }
        for url in ["about:blank", "http://x.com/home", "https://user@x.com/home", "https://x.com:444/home", "https://evil.test/home", "https://x.com/__atsumi_surface__/nonce"] {
            let current = Url::parse(url).unwrap();
            assert!(bridge_message(current.as_str(), &current, raw, "nonce").is_none());
        }
    }
    #[test]
    fn only_matching_started_document_can_consume_a_page_completion() {
        let mut load = DocumentLoad::default();
        let home = Url::parse("https://x.com/home").unwrap();
        let detail = Url::parse("https://x.com/alice/status/123456").unwrap();
        // Wry also reports failed/cancelled NavigationCompleted with current
        // document URL. Without a content-start event, it is not a reload.
        assert!(!load.finish(&home));
        assert!(load.start(&home));
        assert!(!load.finish(&detail));
        assert!(load.finish(&home));
        assert!(!load.finish(&home));
        assert!(load.start(&home));
        assert!(load.start(&detail));
        assert!(!load.finish(&home));
        assert!(load.finish(&detail));
        for url in ["about:blank", "https://evil.test/home", "http://x.com/home", "https://x.com/__atsumi_surface__/nonce?kind=status&value=fixture"] {
            let url = Url::parse(url).unwrap();
            assert!(!load.start(&url));assert!(!load.finish(&url));
        }
    }
    fn playback_fixture() -> (XSession, Arc<Pane>, Arc<Pane>) {
        let session = XSession::default();
        let make = |label: &str, feed: &str| Arc::new(Pane {
            label: label.into(), nonce: "fixture".into(), feed: feed.into(), owner: "alice".into(),
            profile: "legacy".into(),
            revision: session.revision.load(Ordering::Acquire), state: Mutex::new("ready".into()),
            loading_since: Mutex::new(Instant::now()), visible: AtomicBool::new(true),
            fullscreen: AtomicBool::new(false),
            document_load: Mutex::new(DocumentLoad::default()),
            root_recovery: Mutex::new(RootRecovery::default()),
            media: Arc::new(surface_media::Observations::new(session.revision.load(Ordering::Acquire))),
        });
        let feed = make("feed-fixture", "foryou");
        let preview = make("preview-fixture", "preview");
        {
            let mut data = session.surfaces.lock().unwrap();
            data.active = "foryou".into();
            data.feeds.insert("foryou".into(), feed.clone());
            data.preview = Some(preview.clone());
            data.preview_tabs.push(PreviewTab { pane: preview.clone(), path: "/alice/status/123456".into() });
            data.viewport = Some(NativeViewport {
                sequence: 1, visible: true, muted: false, occlusions: vec![],
                player: Some(Rect {x: 0., y: 0., width: 600., height: 900.}),
                chat: Some(Rect {x: 600., y: 0., width: 600., height: 900.}),
            });
        }
        (session, feed, preview)
    }
    #[test]
    fn ordinary_suspend_retains_exact_documents_but_revokes_geometry_and_pending_intents() {
        let (session, feed, preview)=playback_fixture();
        let revision=session.revision.load(Ordering::Acquire);
        assert!(claim_scroll(&session,&feed));
        let mut data=session.surfaces.lock().unwrap();
        let generation=data.generation;
        let origin=preview_origin(&data);
        data.route_intents.insert(feed.label.clone(),RouteIntent{source:Url::parse("https://x.com/home").unwrap(),generation,since:Instant::now(),background:false,origin});
        let panes=suspend_documents(&mut data);
        assert_eq!(panes.len(),2);assert_eq!(data.generation,generation+1);
        assert_eq!(data.active,"foryou");assert!(Arc::ptr_eq(data.feeds.get("foryou").unwrap(),&feed));
        assert!(Arc::ptr_eq(data.preview.as_ref().unwrap(),&preview));
        assert!(Arc::ptr_eq(&data.preview_tabs[0].pane,&preview));
        assert_eq!(data.preview_tabs[0].path,"/alice/status/123456");
        assert!(data.viewport.is_none());assert!(data.playing.is_none());assert!(data.focused.is_none());
        assert!(!data.manual_playing);assert!(data.route_intents.is_empty());
        assert!(!preview_wants_foreground(&data,origin,false));
        assert!(panes.iter().all(|pane|!pane.visible.load(Ordering::Acquire)));
        assert_eq!(session.revision.load(Ordering::Acquire),revision);
        drop(data);
        assert!(!claim_scroll(&session,&feed));assert!(!claim_scroll(&session,&preview));
        assert!(is_current(&session,&feed));assert!(is_current(&session,&preview));
    }
    #[test]
    fn ordinary_suspend_invalidates_both_queued_and_creating_feed_opens_without_eating_next_order() {
        let mut data = Surfaces::default();
        data.order = 1_000;
        let queued_epoch = data.open_epoch;
        assert!(feed_open_current(&data, 1_000, queued_epoch));
        suspend_documents(&mut data);
        assert_eq!(data.order, 1_000);
        assert!(!feed_open_current(&data, 1_000, queued_epoch));
        // The next shell request may be exactly previous order + 1. It remains
        // valid and gets its own epoch; a later suspend also rejects its create
        // continuation, even though no newer shell request has arrived.
        data.order = 1_001;
        let creating_epoch = data.open_epoch;
        assert!(feed_open_current(&data, 1_001, creating_epoch));
        assert!(!feed_open_current(&data, 1_000, queued_epoch));
        data.generation += 1;
        assert!(feed_open_current(&data, 1_001, creating_epoch));
        suspend_documents(&mut data);
        assert!(!feed_open_current(&data, 1_001, creating_epoch));
        assert_eq!(data.order, 1_001);
    }
    #[test]
    fn wheel_transfers_input_and_audio_once_without_starting_a_source_player() {
        let (session,feed,preview)=playback_fixture();
        assert!(claim_playback(&session,&preview,true));assert!(claim_input(&session,&preview));
        assert!(claim_scroll(&session,&feed));assert!(playback_granted(&session,&feed));assert!(!playback_granted(&session,&preview));
        assert_eq!(session.surfaces.lock().unwrap().focused.as_deref(),Some(feed.label.as_str()));
        for _ in 0..10 {assert!(!claim_scroll(&session,&feed));}
        assert!(claim_scroll(&session,&preview));assert!(playback_granted(&session,&preview));
        assert!(!automatic_playback_allowed(&session,&feed));
        preview.visible.store(false,Ordering::Release);assert!(!claim_scroll(&session,&preview));
        preview.visible.store(true,Ordering::Release);
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible=false;
        assert!(!claim_scroll(&session,&feed));assert!(!playback_granted(&session,&preview));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible=true;
        session.revision.fetch_add(1,Ordering::AcqRel);assert!(!claim_scroll(&session,&feed));
    }
    #[test]
    fn wheel_bridge_has_no_scroll_values_or_arbitrary_payload() {
        let url=Url::parse("https://x.com/__atsumi_surface__/nonce?kind=wheel&value=").unwrap();
        assert_eq!(signal(&url,"nonce"),Some(("wheel".into(),"".into())));
        for value in ["manual","120","preview","https://private.invalid"] {
            let mut invalid=url.clone();invalid.set_query(None);
            invalid.query_pairs_mut().append_pair("kind","wheel").append_pair("value",value);
            assert!(signal(&invalid,"nonce").is_none());
        }
    }
    fn retained_test_pane(label: &str) -> Arc<Pane> {
        Arc::new(Pane {
            label:label.into(),nonce:"fixture".into(),feed:"preview".into(),owner:"alice".into(),
            profile:"legacy".into(),revision:1,state:Mutex::new("ready".into()),
            loading_since:Mutex::new(Instant::now()),document_load:Mutex::new(Default::default()),
            root_recovery:Mutex::new(Default::default()),visible:AtomicBool::new(false),
            fullscreen:AtomicBool::new(false),media:Arc::new(surface_media::Observations::new(1)),
        })
    }
    fn restore_test_preview_slot(data: &mut Surfaces) {
        data.viewport.as_mut().unwrap().chat = Some(Rect { x:600.,y:0.,width:600.,height:900. });
        if let Some(pane) = data.preview.as_ref() { pane.visible.store(true, Ordering::Release); }
    }
    #[test]
    fn retained_preview_tabs_preserve_independent_documents_and_select_exact_ids() {
        let (session, _, first) = playback_fixture();
        let second = retained_test_pane("preview-second");
        let third = retained_test_pane("preview-third");
        {
            let mut data = session.surfaces.lock().unwrap();
            let intent = data.preview_intent;
            register_preview(&mut data, second.clone(), "/bob/status/654321", intent).unwrap();
            let intent = data.preview_intent;
            register_preview(&mut data, third.clone(), "/carol", intent).unwrap();
            assert_eq!(data.preview_tabs.len(), 3);
            assert_eq!(data.preview.as_ref().unwrap().label, third.label);
            assert!(Arc::ptr_eq(&data.preview_tabs[0].pane, &first));
            assert!(Arc::ptr_eq(&data.preview_tabs[1].pane, &second));
            let generation = data.generation;
            select_preview(&mut data, &first.label, Some(generation)).unwrap();
            assert!(data.generation > generation);
            restore_test_preview_slot(&mut data);
            assert!(registered_visible(&data, &first));
            assert!(!registered_visible(&data, &second));
            assert!(!registered_visible(&data, &third));
        }
        assert!(is_current(&session, &second)); // Retained, but never interactive.
        assert!(!interaction_allowed(&session, &second));
        assert!(!claim_playback(&session, &second, true));
        assert!(!claim_input(&session, &second));
        let snapshot = snapshot(&session);
        assert_eq!(snapshot.previews.len(), 3);
        assert_eq!(snapshot.active_preview_id.as_deref(), Some(first.label.as_str()));
        assert_eq!(snapshot.previews[1].title, "@bob · #654321");
    }
    #[test]
    fn close_inactive_active_and_last_tabs_select_adjacent_without_replacing_documents() {
        let (session, _, first) = playback_fixture();
        let second = retained_test_pane("second");
        let third = retained_test_pane("third");
        let mut data = session.surfaces.lock().unwrap();
        let intent = data.preview_intent;
        register_preview(&mut data, second.clone(), "/bob", intent).unwrap();
        let intent = data.preview_intent;
        register_preview(&mut data, third.clone(), "/carol", intent).unwrap();
        restore_test_preview_slot(&mut data);
        data.playing = Some(third.label.clone()); data.manual_playing = true;
        let generation = data.generation;
        assert!(Arc::ptr_eq(&remove_preview(&mut data, &second.label, Some(generation)).unwrap(), &second));
        assert_eq!(data.preview.as_ref().unwrap().label, third.label);
        assert_eq!(data.playing.as_deref(), Some(third.label.as_str()));
        let generation = data.generation;
        remove_preview(&mut data, &third.label, Some(generation)).unwrap();
        assert_eq!(data.preview.as_ref().unwrap().label, first.label);
        assert!(data.playing.is_none()); assert!(!data.manual_playing);
        let generation = data.generation;
        remove_preview(&mut data, &first.label, Some(generation)).unwrap();
        assert!(data.preview.is_none()); assert!(data.preview_tabs.is_empty());
    }
    #[test]
    fn stale_tab_commands_never_change_selection_generation_or_retained_state() {
        let (session, _, first) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        data.generation = 4; data.preview_intent = 8;
        for generation in [3, 5] {
            assert!(select_preview(&mut data, &first.label, Some(generation)).is_err());
            assert!(remove_preview(&mut data, &first.label, Some(generation)).is_err());
        }
        assert!(select_preview(&mut data, "retired", Some(4)).is_err());
        assert!(remove_preview(&mut data, "retired", Some(4)).is_err());
        assert_eq!(data.generation, 4); assert_eq!(data.preview_intent, 8);
        assert_eq!(data.preview_tabs.len(), 1);
        assert!(Arc::ptr_eq(data.preview.as_ref().unwrap(), &first));
    }
    #[test]
    fn rapid_opens_are_retained_but_late_creation_cannot_override_new_selection_or_close() {
        let (session, _, first) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        let second = retained_test_pane("second");
        let intent = data.preview_intent;
        register_preview(&mut data, second.clone(), "/bob", intent).unwrap();
        let pending_intent = data.preview_intent;
        select_preview(&mut data, &first.label, None).unwrap();
        register_preview(&mut data, retained_test_pane("third"), "/carol", pending_intent).unwrap();
        assert_eq!(data.preview_tabs.len(), 3);
        assert_eq!(data.preview.as_ref().unwrap().label, first.label);
        let pending_intent = data.preview_intent;
        remove_preview(&mut data, &first.label, None).unwrap();
        let adjacent = data.preview.as_ref().unwrap().label.clone();
        register_preview(&mut data, retained_test_pane("fourth"), "/david", pending_intent).unwrap();
        assert_eq!(data.preview_tabs.len(), 3);
        assert_eq!(data.preview.as_ref().unwrap().label, adjacent);
    }
    #[test]
    fn duplicate_loading_target_reuses_exact_account_document_and_capacity_never_evicts() {
        let mut data = Surfaces::default();
        for index in 0..PREVIEW_LIMIT {
            let pane = retained_test_pane(&format!("preview-{index}"));
            *pane.state.lock().unwrap() = "loading".into();
            let intent = data.preview_intent;
            register_preview(&mut data, pane, &format!("/alice/status/{}", 123456 + index), intent).unwrap();
        }
        assert!(!preview_has_capacity(&data));
        let labels = data.preview_tabs.iter().map(|tab|tab.pane.label.clone()).collect::<Vec<_>>();
        assert_eq!(existing_preview(&data, "/alice/status/123456", "alice", 1).as_deref(), Some("preview-0"));
        assert!(existing_preview(&data, "/alice/status/123456", "bob", 1).is_none());
        assert!(existing_preview(&data, "/alice/status/123456", "alice", 2).is_none());
        let generation = data.generation;
        let intent = data.preview_intent;
        assert!(register_preview(&mut data, retained_test_pane("overflow"), "/zebra", intent).is_err());
        assert!(register_preview(&mut data, retained_test_pane("duplicate"), "/alice/status/123456", intent).is_err());
        assert_eq!(generation, data.generation);
        assert_eq!(labels, data.preview_tabs.iter().map(|tab|tab.pane.label.clone()).collect::<Vec<_>>());
        select_preview(&mut data, "preview-0", None).unwrap(); // Reuse works at capacity.
        assert_eq!(data.preview_tabs.len(), PREVIEW_LIMIT);
    }
    #[test]
    fn source_requested_navigation_intent_is_single_document_generation_and_deadline_scoped() {
        let current = Url::parse("https://x.com/home").unwrap();
        let intent = RouteIntent { source:current.clone(),generation:9,since:Instant::now(),background:false,
            origin: PreviewOpenOrigin { order:0, selection:0 } };
        let target = Url::parse("https://x.com/bob/status/123456").unwrap();
        assert!(route_intent_target(&intent, 9, &current, &target));
        for value in ["https://evil.test/bob/status/123456","https://x.com/bob/status/123456/photo/1",
            "https://x.com/bob/status/123456/video/1","https://x.com/home","https://x.com/settings",
            "https://x.com/bob?x=1","https://x.com/bob#section","http://x.com/bob"] {
            assert!(!route_intent_target(&intent, 9, &current, &Url::parse(value).unwrap()));
        }
        assert!(!route_intent_target(&intent, 10, &current, &target));
        assert!(!route_intent_target(&intent, 9, &Url::parse("https://x.com/i/bookmarks").unwrap(), &target));
        let expired = RouteIntent { since:Instant::now()-Duration::from_millis(1001),..intent };
        assert!(!route_intent_target(&expired, 9, &current, &target));
        for value in ["arm","armBackground","cancel"] {
            let url=Url::parse(&format!("https://x.com/__atsumi_surface__/nonce?kind=routeIntent&value={value}")).unwrap();
            assert!(signal(&url,"nonce").is_some());
        }
        for value in ["", "https://x.com/bob", "click", "/bob"] {
            let url=Url::parse(&format!("https://x.com/__atsumi_surface__/nonce?kind=routeIntent&value={value}")).unwrap();
            assert!(signal(&url,"nonce").is_none());
        }
    }
    #[test]
    fn background_tabs_retain_without_selecting_focusing_or_displacing_current_geometry() {
        let mut empty = Surfaces::default();
        register_preview_mode(&mut empty, retained_test_pane("background-first"), "/bob", 0, true).unwrap();
        assert!(empty.preview.is_none()); assert!(empty.focused.is_none());
        assert_eq!(empty.preview_tabs.len(), 1); assert_eq!(empty.generation, 1);
        let (session, feed, first) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        data.focused = Some(feed.label.clone());
        data.playing = Some(first.label.clone()); data.manual_playing = true;
        let geometry = data.viewport.as_ref().unwrap().chat.clone();
        let intent = data.preview_intent;
        register_preview_mode(&mut data, retained_test_pane("background-second"), "/bob", intent, true).unwrap();
        assert!(Arc::ptr_eq(data.preview.as_ref().unwrap(), &first));
        assert_eq!(data.focused.as_deref(), Some(feed.label.as_str()));
        assert_eq!(data.playing.as_deref(), Some(first.label.as_str()));
        assert!(data.manual_playing);
        assert_eq!(data.viewport.as_ref().unwrap().chat, geometry);
        assert!(registered_visible(&data, &first));
        let valid=Url::parse("https://x.com/__atsumi_surface__/nonce?kind=previewBackground&value=%2Fbob%2Fstatus%2F123456").unwrap();
        assert!(signal(&valid,"nonce").is_some());
        for value in ["https://evil.test/", "/settings", "/bob?bad=1", "javascript:alert(1)"] {
            let mut url=Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
            url.query_pairs_mut().append_pair("kind","previewBackground").append_pair("value",value);
            assert!(signal(&url,"nonce").is_none());
        }
    }
    #[test]
    fn retained_target_labels_follow_only_exact_observed_native_paths() {
        let (session, _, pane) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        let current = Url::parse("https://x.com/bob/status/654321").unwrap();
        observe_preview_path(&mut data, &pane, current.path(), &current);
        assert_eq!(existing_preview(&data, current.path(), "alice", pane.revision).as_deref(), Some(pane.label.as_str()));
        assert!(existing_preview(&data, "/alice/status/123456", "alice", pane.revision).is_none());
        for (path, url) in [
            ("/carol", "https://x.com/bob/status/654321"), ("/carol", "https://evil.test/carol"),
            ("/settings", "https://x.com/settings"), ("/home", "https://x.com/home"),
        ] {
            observe_preview_path(&mut data, &pane, path, &Url::parse(url).unwrap());
            assert_eq!(data.preview_tabs[0].path, current.path());
        }
    }
    #[test]
    fn queued_open_origin_survives_other_opens_but_not_a_newer_explicit_selection_or_feed() {
        let (session, _, first) = playback_fixture();
        let mut data = session.surfaces.lock().unwrap();
        let queued = preview_origin(&data);
        let intent = data.preview_intent;
        register_preview(&mut data, retained_test_pane("second"), "/bob", intent).unwrap();
        assert!(preview_wants_foreground(&data, queued, false)); // Rapid source clicks both retained/selected in order.
        let generation = data.generation;
        select_preview_explicit(&mut data, &first.label, generation).unwrap();
        assert!(!preview_wants_foreground(&data, queued, false));
        let intent = data.preview_intent;
        let background = !preview_wants_foreground(&data, queued, false);
        register_preview_mode(&mut data, retained_test_pane("third"), "/carol", intent, background).unwrap();
        assert_eq!(data.preview.as_ref().unwrap().label, first.label);
        assert_eq!(data.preview_tabs.len(), 3);
        let newer = preview_origin(&data);
        let generation = data.generation;
        select_preview_explicit(&mut data, &first.label, generation).unwrap(); // Even clicking already-active is an explicit choice.
        assert!(!preview_wants_foreground(&data, newer, false));
        let newest = preview_origin(&data);
        assert!(preview_wants_foreground(&data, newest, false));
        assert!(!preview_wants_foreground(&data, newest, true));
        remove_preview(&mut data, &first.label, None).unwrap();
        let adjacent = data.preview.as_ref().unwrap().label.clone();
        assert!(!preview_wants_foreground(&data, newest, false));
        let intent = data.preview_intent;
        let background = !preview_wants_foreground(&data, newest, false);
        register_preview_mode(&mut data, retained_test_pane("fourth"), "/david", intent, background).unwrap();
        assert_eq!(data.preview.as_ref().unwrap().label, adjacent);
        let newest = preview_origin(&data);
        data.order += 1;
        assert!(!preview_wants_foreground(&data, newest, false));
    }
    #[test]
    fn hidden_preview_home_recovery_retires_only_that_proved_document_not_the_active_tab() {
        let (session, _, hidden) = playback_fixture();
        let active = retained_test_pane("active");
        let mut data = session.surfaces.lock().unwrap();
        let intent = data.preview_intent;
        register_preview(&mut data, active.clone(), "/bob", intent).unwrap();
        restore_test_preview_slot(&mut data);
        data.playing = Some(active.label.clone()); data.manual_playing = true;
        let revision = session.revision.load(Ordering::Acquire);
        assert!(Arc::ptr_eq(&take_root_preview(&mut data, &hidden, revision).unwrap(), &hidden));
        assert!(Arc::ptr_eq(data.preview.as_ref().unwrap(), &active));
        assert_eq!(data.playing.as_deref(), Some(active.label.as_str()));
        assert_eq!(data.focused.as_deref(), Some(active.label.as_str()));
        assert!(registered_visible(&data, &active));
        let generation = data.generation;
        assert!(take_root_preview(&mut data, &hidden, revision).is_err());
        assert!(take_root_preview(&mut data, &active, revision + 1).is_err());
        let impostor = retained_test_pane(&active.label);
        assert!(take_root_preview(&mut data, &impostor, revision).is_err());
        assert_eq!(data.generation, generation);
        assert_eq!(data.preview_tabs.len(), 1);
    }
    #[test]
    fn latest_visible_native_play_can_return_to_feed_without_closing_preview() {
        let (session, feed, preview) = playback_fixture();
        assert!(!playback_granted(&session, &feed));
        assert!(claim_playback(&session, &feed, true));
        assert!(playback_granted(&session, &feed));
        assert!(!playback_granted(&session, &preview));
        assert!(claim_playback(&session, &preview, true));
        assert!(!playback_granted(&session, &feed)); // A delayed old unmute has no grant.
        assert!(playback_granted(&session, &preview));
        assert!(claim_playback(&session, &feed, true));
        assert!(playback_granted(&session, &feed));
        assert!(!playback_granted(&session, &preview));
        assert!(preview.visible.load(Ordering::Acquire));
        assert!(session.surfaces.lock().unwrap().preview.is_some());
    }
    #[test]
    fn frame_audio_recovers_missing_dom_play_without_stealing_manual_owner_or_looping() {
        let (session, feed, preview) = playback_fixture();
        assert!(!claim_native_audio(&session, &feed, false));
        assert!(claim_native_audio(&session, &feed, true));
        assert!(playback_granted(&session, &feed));
        assert!(!claim_native_audio(&session, &feed, true));
        assert!(claim_native_audio(&session, &preview, true));
        assert!(!claim_native_audio(&session, &feed, true));
        assert!(claim_playback(&session, &feed, true));
        assert!(!claim_native_audio(&session, &preview, true));
        assert!(playback_granted(&session, &feed));
        assert!(claim_playback(&session, &preview, true)); // Proven in-frame gesture.
        assert!(playback_granted(&session, &preview));
    }
    #[test]
    fn native_frame_audio_cannot_bypass_mute_privacy_readiness_epoch_or_occlusion() {
        let (session, feed, _) = playback_fixture();
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().muted = true;
        assert!(!claim_native_audio(&session, &feed, true));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().muted = false;
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = false;
        assert!(!claim_native_audio(&session, &feed, true));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = true;
        feed.visible.store(false, Ordering::Release);
        assert!(!claim_native_audio(&session, &feed, true));
        feed.visible.store(true, Ordering::Release);
        *feed.state.lock().unwrap() = "loading".into();
        assert!(!claim_native_audio(&session, &feed, true));
        *feed.state.lock().unwrap() = "ready".into();
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().occlusions.push(crate::platform_native_geometry::Hole {x: 0., y: 0., width: 100., height: 100., radii: [0.; 4]});
        assert!(!claim_native_audio(&session, &feed, true));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().occlusions.clear();
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!claim_native_audio(&session, &feed, true));
        assert!(session.surfaces.lock().unwrap().playing.is_none());
    }
    #[test]
    fn keys_follow_the_last_user_pane_without_closing_or_focusing_another_window() {
        let (session, feed, preview) = playback_fixture();
        assert!(claim_input(&session, &preview));
        assert!(claim_input(&session, &feed)); // Last pointer was the feed photo.
        {
            let data = session.surfaces.lock().unwrap();
            assert_eq!(input_target(&data, &preview).unwrap().label, feed.label);
            assert!(data.preview.is_some()); // Arbitration does not close anything.
            assert!(data.playing.is_none()); // It does not start/claim media.
        }
        assert!(claim_input(&session, &preview));
        let data = session.surfaces.lock().unwrap();
        assert_eq!(input_target(&data, &feed).unwrap().label, preview.label);
    }
    #[test]
    fn hidden_stale_obscured_or_unready_panes_cannot_claim_keys() {
        let (session, feed, preview) = playback_fixture();
        assert!(claim_input(&session, &feed));
        preview.visible.store(false, Ordering::Release);
        assert!(!claim_input(&session, &preview));
        preview.visible.store(true, Ordering::Release);
        *preview.state.lock().unwrap() = "loading".into();
        assert!(!claim_input(&session, &preview));
        *preview.state.lock().unwrap() = "ready".into();
        {
            let mut data = session.surfaces.lock().unwrap();
            data.viewport.as_mut().unwrap().visible = false;
        }
        assert!(!claim_input(&session, &preview));
        assert!(input_target(&session.surfaces.lock().unwrap(), &preview).is_none());
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = true;
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!claim_input(&session, &preview));
    }
    #[test]
    fn fullscreen_and_closed_preview_cannot_leave_keys_owned_by_an_ineligible_pane() {
        let (session, feed, preview) = playback_fixture();
        assert!(claim_input(&session, &preview));
        feed.fullscreen.store(true, Ordering::Release);
        {
            let data = session.surfaces.lock().unwrap();
            assert_eq!(input_target(&data, &feed).unwrap().label, feed.label);
        }
        feed.fullscreen.store(false, Ordering::Release);
        let mut data = session.surfaces.lock().unwrap();
        let generation = data.generation;
        take_preview(&mut data, Some(&preview.label), Some(generation)).unwrap();
        assert_eq!(data.focused.as_deref(), Some(feed.label.as_str()));
        assert_eq!(input_target(&data, &feed).unwrap().label, feed.label);
    }
    #[test]
    fn non_playback_interactions_recheck_latest_viewport_before_queued_conceal_runs() {
        let (session, feed, preview) = playback_fixture();
        assert!(interaction_allowed(&session, &feed));
        assert!(interaction_allowed(&session, &preview));
        // The native visible atomics can still be true while paint/conceal is
        // queued. Current viewport/active selection must decide admission.
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = false;
        assert!(!interaction_allowed(&session, &feed));
        assert!(!interaction_allowed(&session, &preview));
        {
            let mut data = session.surfaces.lock().unwrap();
            let viewport = data.viewport.as_mut().unwrap();
            viewport.visible = true;
            viewport.occlusions.push(crate::platform_native_geometry::Hole { x:10., y:10., width:20., height:20., radii:[0.;4] });
        }
        assert!(!interaction_allowed(&session, &feed));
        assert!(interaction_allowed(&session, &preview));
        {
            let mut data = session.surfaces.lock().unwrap();
            let viewport = data.viewport.as_mut().unwrap();
            viewport.occlusions.clear();
            viewport.player = None;
            viewport.chat = None;
        }
        assert!(!interaction_allowed(&session, &feed));
        assert!(!interaction_allowed(&session, &preview));
    }
    #[test]
    fn inactive_unregistered_owner_changed_or_stale_panes_cannot_send_interactions() {
        let (session, feed, preview) = playback_fixture();
        session.surfaces.lock().unwrap().active = "likes".into();
        assert!(!interaction_allowed(&session, &feed));
        assert!(interaction_allowed(&session, &preview));
        session.surfaces.lock().unwrap().active = "foryou".into();
        *feed.state.lock().unwrap() = "ownerChanged".into();
        assert!(!interaction_allowed(&session, &feed));
        *feed.state.lock().unwrap() = "ready".into();
        feed.visible.store(false, Ordering::Release);
        assert!(!interaction_allowed(&session, &feed));
        feed.visible.store(true, Ordering::Release);
        session.surfaces.lock().unwrap().feeds.remove("foryou");
        assert!(!interaction_allowed(&session, &feed));
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!interaction_allowed(&session, &preview));
    }
    #[test]
    fn hidden_obscured_unready_or_stale_panes_cannot_take_playback() {
        let (session, feed, preview) = playback_fixture();
        assert!(claim_playback(&session, &preview, true));
        feed.visible.store(false, Ordering::Release);
        assert!(!claim_playback(&session, &feed, true));
        feed.visible.store(true, Ordering::Release);
        *feed.state.lock().unwrap() = "loading".into();
        assert!(!claim_playback(&session, &feed, true));
        *feed.state.lock().unwrap() = "ready".into();
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = false;
        assert!(!claim_playback(&session, &feed, true));
        assert!(!playback_granted(&session, &preview));
        {
            let mut data = session.surfaces.lock().unwrap();
            let viewport = data.viewport.as_mut().unwrap();
            viewport.visible = true;
            viewport.occlusions.push(crate::platform_native_geometry::Hole {x: 10., y: 10., width: 20., height: 20., radii: [0.;4]});
        }
        assert!(!claim_playback(&session, &feed, true));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().occlusions.clear();
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().muted = true;
        assert!(!playback_granted(&session, &preview));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().muted = false;
        session.surfaces.lock().unwrap().feeds.remove("foryou");
        assert!(!claim_playback(&session, &feed, true));
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!claim_playback(&session, &preview, true));
        assert!(!playback_granted(&session, &preview));
    }
    #[test]
    fn autoplay_cannot_steal_manual_playback_or_ping_pong_between_panes() {
        let (session, feed, preview) = playback_fixture();
        assert!(claim_playback(&session, &feed, false));
        assert!(claim_playback(&session, &preview, false));
        for _ in 0..5 { assert!(!claim_playback(&session, &feed, false)); }
        assert!(playback_granted(&session, &preview));
        assert!(claim_playback(&session, &feed, true));
        for _ in 0..5 { assert!(!claim_playback(&session, &preview, false)); }
        assert!(playback_granted(&session, &feed));
        assert!(claim_playback(&session, &feed, false)); // Same pane's next clip preserves its manual priority.
        assert!(!claim_playback(&session, &preview, false));
        assert!(claim_playback(&session, &preview, true));
        assert!(!claim_playback(&session, &feed, false));
        preview.visible.store(false, Ordering::Release);
        assert!(claim_playback(&session, &feed, false)); // An unavailable owner cannot retain a grant.
    }
    #[test]
    fn overlay_and_small_bounds_hide_native_content() {
        let mut v = NativeViewport {
            sequence: 1,
            player: None,
            chat: None,
            visible: true,
            muted: false,
            occlusions: vec![],
        };
        let r = Rect {
            x: 220.,
            y: 80.,
            width: 600.,
            height: 800.,
        };
        assert!(unobscured(&r, &v));
        v.occlusions.push(crate::platform_native_geometry::Hole {
            x: 250.,
            y: 100.,
            width: 30.,
            height: 30.,
            radii: [0.; 4],
        });
        assert!(!unobscured(&r, &v));
    }
    #[test]
    fn only_registered_current_account_panes_can_send_messages() {
        let session = XSession::default();
        let pane = Arc::new(Pane {
            label: "fixture".into(),
            nonce: "nonce".into(),
            feed: "foryou".into(),
            owner: "alice".into(),
            profile: "legacy".into(),
            revision: session.revision.load(Ordering::Acquire),
            state: Mutex::new("ready".into()),
            loading_since: Mutex::new(Instant::now()),
            document_load: Mutex::new(DocumentLoad::default()),
            root_recovery: Mutex::new(RootRecovery::default()),
            visible: AtomicBool::new(true),
            fullscreen: AtomicBool::new(false),
            media: Arc::new(surface_media::Observations::new(session.revision.load(Ordering::Acquire))),
        });
        assert!(!is_current(&session, &pane));
        session
            .surfaces
            .lock()
            .unwrap()
            .feeds
            .insert("foryou".into(), pane.clone());
        assert!(is_current(&session, &pane));
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!is_current(&session, &pane));
    }
    #[test]
    fn readiness_cannot_be_forged_with_extra_document_fields() {
        assert!(serde_json::from_str::<DocumentStatus>(
            r#"{"state":"ready","owner":"alice","path":"/home","token":"secret"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<DocumentStatus>(
            r#"{"state":"ready","owner":"alice","path":"/home"}"#
        )
        .is_ok());
        assert!(serde_json::from_str::<DocumentStatus>(
            r#"{"state":"loading","owner":null,"path":"/home","proof":{"profile":false,"account":false,"primary":true,"login":false}}"#
        ).is_ok());
        for proof in [r#"{"profile":false,"account":false,"primary":true,"login":false,"url":"private"}"#,
            r#"{"profile":"alice","account":false,"primary":true,"login":false}"#] {
            let value = format!(r#"{{"state":"loading","owner":null,"path":"/home","proof":{proof}}}"#);
            assert!(serde_json::from_str::<DocumentStatus>(&value).is_err());
        }
    }
    #[test]
    fn photo_diagnostic_is_closed_bounded_and_contains_no_source_strings() {
        let mut raw = serde_json::json!({"route":"photo","segments":255,"dialogs":255,
            "dialogImages":255,"photoCandidates":255,"dialogImageHost":true,"dialogImageMedia":true,
            "pageImageMedia":false,"hiddenDialog":false,"securityInput":false});
        let shape: PhotoShape = serde_json::from_value(raw.clone()).unwrap();
        let value = shape.diagnostic();
        assert_eq!(value["route"], "photo");assert_eq!(value["segments"],16);assert_eq!(value["dialogs"],8);
        assert_eq!(value["dialogImages"],32);assert_eq!(value["photoCandidates"],32);
        raw["route"] = serde_json::json!("/private/status/123456/photo/1");
        assert!(serde_json::from_value::<PhotoShape>(raw.clone()).is_err());
        raw["route"] = serde_json::json!("photo");raw["url"] = serde_json::json!("https://private.invalid");
        assert!(serde_json::from_value::<PhotoShape>(raw.clone()).is_err());
        raw.as_object_mut().unwrap().remove("url");raw["segments"] = serde_json::json!(256);
        assert!(serde_json::from_value::<PhotoShape>(raw).is_err());
    }
    #[test]
    fn photo_action_diagnostic_is_closed_bounded_and_cannot_carry_source_or_css() {
        let mut raw = serde_json::json!({
            "reason":"ambiguous","dialogs":255,"visibleDialogs":255,"leafDialogs":255,
            "articlePresent":true,"scannedImages":255,"allowedImages":255,"imageLoaded":255,
            "imageComplete":255,"nonzeroRect":255,"inViewport":255,"directlyShown":255,
            "opacityZero":255,"backgrounds":255,"exactPaint":255,"candidates":255,
            "controlPresent":false,"controlHovered":false
        });
        let probe: PhotoActionShape = serde_json::from_value(raw.clone()).unwrap();
        let projected = probe.diagnostic();
        assert_eq!(projected["reason"], "ambiguous");
        assert!(projected["carouselProof"].is_null());
        for field in ["dialogs","visibleDialogs","leafDialogs","scannedImages","allowedImages",
            "imageLoaded","imageComplete","nonzeroRect","inViewport","directlyShown",
            "opacityZero","backgrounds","exactPaint","candidates"] {
            assert_eq!(projected[field], 128);
        }
        raw["reason"] = serde_json::json!("https://private.invalid");
        assert!(serde_json::from_value::<PhotoActionShape>(raw.clone()).is_err());
        raw["reason"] = serde_json::json!("none");
        for field in ["url","owner","text","css","html","src"] {
            let mut unexpected = raw.clone();unexpected[field]=serde_json::json!("private");
            assert!(serde_json::from_value::<PhotoActionShape>(unexpected).is_err());
        }
        raw["scannedImages"] = serde_json::json!(256);
        assert!(serde_json::from_value::<PhotoActionShape>(raw).is_err());
    }
    #[test]
    fn photo_carousel_diagnostic_accepts_only_closed_branch_names() {
        let raw=serde_json::json!({"reason":"ambiguous","dialogs":0,"visibleDialogs":0,"leafDialogs":0,
            "articlePresent":false,"scannedImages":0,"allowedImages":0,"imageLoaded":0,"imageComplete":0,
            "nonzeroRect":0,"inViewport":0,"directlyShown":0,"opacityZero":0,"backgrounds":0,"exactPaint":0,
            "candidates":0,"controlPresent":false,"controlHovered":false});
        for name in ["emptyCandidates","candidateLimit","route","ancestorLimit","absent","mixedRoots",
            "hiddenRoot","nestedRoot","lists","items","nestedItems","counterMissing","counterMismatch",
            "currentHidden","positionConflict","sizeConflict","currentConflict","selectedConflict","detached",
            "candidateScope","imageRect","fixedImage","fixedAncestor","ancestorRect","unboundAbsolute",
            "clipDepth","currentDuplicate","currentClipped","neighbourExposed","currentMissing","ready"] {
            let mut value=raw.clone();value["carouselProof"]=serde_json::json!(name);
            let shape: PhotoActionShape=serde_json::from_value(value).unwrap();
            assert_eq!(shape.diagnostic()["carouselProof"],name);
        }
        for invalid in [serde_json::json!("private text"),serde_json::json!("https://private.invalid"),
            serde_json::json!(1),serde_json::json!(true),serde_json::json!(["ready"]),serde_json::json!({"ready":true})] {
            let mut value=raw.clone();value["carouselProof"]=invalid;
            assert!(serde_json::from_value::<PhotoActionShape>(value).is_err());
        }
    }
    #[test]
    fn photo_slide_diagnostic_is_closed_bounded_without_labels_or_geometry() {
        let mut raw = serde_json::json!({
            "routeOrdinal":255,"carousels":255,"lists":255,"slides":255,"candidateSlides":255,
            "indexedSlides":255,"indexedRouteMatches":255,"currentSlides":255,"selectedSlides":255,
            "currentRouteMatches":255,"selectedRouteMatches":255,"declaredSizes":255,"matchingSetSizes":255,
            "counterLabels":255,"counterRouteMatches":255,"counterSizeMatches":255,"ordinalItemCandidates":255,
            "fullyClipped":255,"partlyClipped":255,"unclipped":255,"clipUncertain":255,"overflow":true
        });
        let shape: PhotoSlideShape = serde_json::from_value(raw.clone()).unwrap();
        let value = shape.diagnostic();
        assert_eq!(value["routeOrdinal"],4);
        for field in ["carousels","lists","slides","candidateSlides"] { assert_eq!(value[field],8); }
        for field in ["indexedSlides","indexedRouteMatches","currentSlides","selectedSlides","currentRouteMatches",
            "selectedRouteMatches","declaredSizes","matchingSetSizes","counterLabels","counterRouteMatches","counterSizeMatches"] {
            assert_eq!(value[field],32);
        }
        for field in ["ordinalItemCandidates","fullyClipped","partlyClipped","unclipped","clipUncertain"] {
            assert_eq!(value[field],128);
        }
        assert!(value.as_object().unwrap().values().all(|value|value.is_number()||value.is_boolean()));
        for field in ["label","text","url","owner","html","src","width","height","css","selectedId"] {
            let mut unexpected=raw.clone();unexpected[field]=serde_json::json!("not permitted");
            assert!(serde_json::from_value::<PhotoSlideShape>(unexpected).is_err());
        }
        raw["routeOrdinal"]=serde_json::json!(256);
        assert!(serde_json::from_value::<PhotoSlideShape>(raw).is_err());
    }
    #[test]
    fn delayed_photo_diagnostic_keeps_closed_source_nonce_and_sample_boundaries() {
        let raw = serde_json::json!({"route":"photo","segments":5,"dialogs":0,
            "dialogImages":0,"photoCandidates":0,"dialogImageHost":false,"dialogImageMedia":false,
            "pageImageMedia":true,"hiddenDialog":false,"securityInput":false,"sample":1});
        let mut envelope=Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
        envelope.query_pairs_mut().append_pair("kind","photoProbe").append_pair("value",&raw.to_string());
        let current=Url::parse("https://x.com/creator/status/123456/photo/1").unwrap();
        let body=format!("ATSUMI_X_SURFACE_V1:{envelope}");
        assert_eq!(signal(&envelope,"nonce").is_some(),cfg!(debug_assertions));
        assert_eq!(bridge_message(current.as_str(),&current,&body,"nonce").is_some(),cfg!(debug_assertions));
        assert!(bridge_message("https://x.com/home",&current,&body,"nonce").is_none());
        assert!(bridge_message(current.as_str(),&current,&body,"wrong").is_none());
        for (field,value) in [("sample",serde_json::json!(0)),("sample",serde_json::json!(5)),
            ("route",serde_json::json!("home")),("url",serde_json::json!("https://private.invalid"))] {
            let mut invalid=raw.clone();invalid[field]=value;
            envelope.set_query(None);
            envelope.query_pairs_mut().append_pair("kind","photoProbe").append_pair("value",&invalid.to_string());
            assert!(signal(&envelope,"nonce").is_none());
        }
    }
    #[test]
    fn ad_diagnostic_is_closed_bounded_and_never_contains_source_content() {
        let raw=serde_json::json!({"ready":true,"articles":65535,"outerPlacements":65535,
            "headerBadges":65535,"promotedIndicators":65535,"hidden":65535,"ambiguous":65535,
            "multiplePlacements":65535,"sharedPlacements":65535,"structureLimited":65535,
            "metadataMissing":65535,"limited":true});
        let shape: AdShape=serde_json::from_value(raw.clone()).unwrap();
        let value=shape.diagnostic();
        for field in ["articles","outerPlacements","headerBadges","promotedIndicators","hidden","ambiguous",
            "multiplePlacements","sharedPlacements","structureLimited","metadataMissing"] {
            assert_eq!(value[field],160);
        }
        assert!(value.as_object().unwrap().values().all(|v|v.is_boolean()||v.is_number()));
        for (field,extra) in [("url",serde_json::json!("https://private.invalid")),
            ("html",serde_json::json!("private")),("articles",serde_json::json!(65536))] {
            let mut invalid=raw.clone();invalid[field]=extra;
            assert!(serde_json::from_value::<AdShape>(invalid).is_err());
        }
    }
    #[test]
    fn ad_header_diagnostic_is_closed_bounded_and_optional_for_older_documents() {
        let fields=["inspected","namesMissing","namesMultiple","stoppedContent","stoppedRoleLink",
            "roleLinkInName","stoppedContext","caretMissing","caretMultiple","candidateLimit","depthLimit",
            "labelAbsent","labelAuthor","labelControl","labelScope","labelNonLeaf","labelHidden","outerLabel"];
        let raw=serde_json::Value::Object(fields.iter().map(|field|(field.to_string(),serde_json::json!(65535))).collect());
        let shape: AdHeaderShape=serde_json::from_value(raw.clone()).unwrap();
        let value=shape.diagnostic();
        assert_eq!(value.as_object().unwrap().len(),fields.len());
        for field in fields { assert_eq!(value[field],160); }
        assert!(value.as_object().unwrap().values().all(serde_json::Value::is_u64));
        for (field,extra) in [("url",serde_json::json!("https://private.invalid")),("text",serde_json::json!("private")),
            ("html",serde_json::json!("private")),("inspected",serde_json::json!(65536)),
            ("inspected",serde_json::json!(-1)),("inspected",serde_json::json!(true)),
            ("inspected",serde_json::json!("1")),("inspected",serde_json::json!([]))] {
            let mut invalid=raw.clone();invalid[field]=extra;
            assert!(serde_json::from_value::<AdHeaderShape>(invalid).is_err());
        }
        let mut envelope=serde_json::json!({"ready":true,"articles":1,"outerPlacements":1,"headerBadges":0,
            "promotedIndicators":0,"hidden":0,"ambiguous":0,"multiplePlacements":1,"sharedPlacements":0,
            "structureLimited":0,"metadataMissing":1,"limited":false});
        let old: AdShape=serde_json::from_value(envelope.clone()).unwrap();
        assert!(old.header_shape.is_none());assert!(old.diagnostic().get("headerShape").is_none());
        envelope["headerShape"]=raw;
        let current: AdShape=serde_json::from_value(envelope).unwrap();
        assert_eq!(current.diagnostic()["headerShape"],value);
    }
    #[test]
    fn account_diagnostic_is_closed_bounded_and_contains_no_identity_or_resource_strings() {
        let raw = serde_json::json!({"nodes":255,"images":255,"imageSources":255,"loadedImages":255,
            "backgrounds":255,"textLength":65535,"autoTextNodes":255,"ariaLabel":true,
            "avatarContainers":255,"hidden":true,"hasBox":false,"narrow":true});
        let shape: AccountShape = serde_json::from_value(raw.clone()).unwrap();
        let value = shape.diagnostic();
        assert_eq!(value["nodes"],128);assert_eq!(value["textLength"],512);assert_eq!(value["avatarContainers"],8);
        for field in ["images","imageSources","loadedImages","backgrounds","autoTextNodes"] { assert_eq!(value[field],32); }
        assert!(value.as_object().unwrap().values().all(|value|value.is_number()||value.is_boolean()));
        for (field, invalid) in [("name",serde_json::json!("Private name")),("url",serde_json::json!("https://private.invalid")),
            ("ariaLabel",serde_json::json!("Private label")),("images",serde_json::json!(256)),
            ("textLength",serde_json::json!(65536))] {
            let mut changed=raw.clone();changed[field]=invalid;
            assert!(serde_json::from_value::<AccountShape>(changed).is_err());
        }
        let envelope=serde_json::json!({"state":"ready","owner":"alice","path":"/home",
            "proof":{"profile":true,"account":true,"primary":true,"login":false,"accountShape":raw}});
        assert!(serde_json::from_value::<DocumentStatus>(envelope).is_ok());
    }
    #[test]
    fn routing_diagnostic_has_only_closed_bounded_fields_and_the_same_message_boundary() {
        let mut raw = serde_json::json!({"reason":"missingPath","role":"link","hasTweetArticle":true,"nested":true,
            "userNames":255,"ownUserNames":255,"anchors":255,"times":255,"timeLinks":255,"mediaLinks":255,
            "uniqueMedia":false,"authorMatch":false,"targetAnchor":false,"scopedResolved":false});
        let shape: RoutingProbe = serde_json::from_value(raw.clone()).unwrap();
        let value = shape.diagnostic();
        for field in ["userNames","ownUserNames","times","timeLinks"] { assert_eq!(value[field],32); }
        for field in ["anchors","mediaLinks"] { assert_eq!(value[field],128); }
        let mut envelope = Url::parse("https://x.com/__atsumi_surface__/nonce").unwrap();
        envelope.query_pairs_mut().append_pair("kind","routeProbe").append_pair("value",&raw.to_string());
        assert_eq!(signal(&envelope,"nonce").is_some(),cfg!(debug_assertions));
        let current = Url::parse("https://x.com/home").unwrap();
        let body = format!("ATSUMI_X_SURFACE_V1:{envelope}");
        assert_eq!(bridge_message(current.as_str(),&current,&body,"nonce").is_some(),cfg!(debug_assertions));
        assert!(bridge_message("https://x.com/i/bookmarks",&current,&body,"nonce").is_none());
        assert!(bridge_message(current.as_str(),&current,&body,"wrong").is_none());
        for (field, invalid) in [("reason",serde_json::json!("anything")),("role",serde_json::json!("source-private")),
            ("href",serde_json::json!("https://private.invalid")),("anchors",serde_json::json!(256))] {
            let mut invalid_raw=raw.clone();invalid_raw[field]=invalid;
            assert!(serde_json::from_value::<RoutingProbe>(invalid_raw.clone()).is_err());
            envelope.set_query(None);envelope.query_pairs_mut().append_pair("kind","routeProbe").append_pair("value",&invalid_raw.to_string());
            assert!(signal(&envelope,"nonce").is_none());
        }
        raw["reason"]=serde_json::json!("missingScope");raw["role"]=serde_json::json!("quoteMarker");
        assert!(serde_json::from_value::<RoutingProbe>(raw).is_ok());
    }
    #[test]
    fn archive_capture_admission_requires_same_account_epoch_generation_and_visible_ready_pane() {
        let (session, feed, preview) = playback_fixture();
        *session.registry.lock().unwrap() = Some(accounts::Registry {
            active: "legacy".into(), accounts: vec![accounts::Account { id: "legacy".into(), handle: "Alice".into() }],
        });
        assert!(capture_pane_current(&session, &feed, 0, "legacy"));
        assert!(capture_pane_current(&session, &preview, 0, "legacy"));
        assert!(!capture_pane_current(&session, &feed, 1, "legacy"));
        assert!(!capture_pane_current(&session, &feed, 0, "another"));
        session.registry.lock().unwrap().as_mut().unwrap().active = "another".into();
        assert!(!capture_pane_current(&session, &feed, 0, "legacy"));
        session.registry.lock().unwrap().as_mut().unwrap().active = "legacy".into();
        session.registry.lock().unwrap().as_mut().unwrap().accounts[0].handle = "bob".into();
        assert!(!capture_pane_current(&session, &feed, 0, "legacy"));
        session.registry.lock().unwrap().as_mut().unwrap().accounts[0].handle = "alice".into();
        feed.visible.store(false, Ordering::Release);
        assert!(!capture_pane_current(&session, &feed, 0, "legacy"));
        feed.visible.store(true, Ordering::Release);
        *feed.state.lock().unwrap() = "loading".into();
        assert!(!capture_pane_current(&session, &feed, 0, "legacy"));
        *feed.state.lock().unwrap() = "ready".into();
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = false;
        assert!(!capture_pane_current(&session, &feed, 0, "legacy"));
        session.surfaces.lock().unwrap().viewport.as_mut().unwrap().visible = true;
        session.surfaces.lock().unwrap().feeds.remove("foryou");
        assert!(!capture_pane_current(&session, &feed, 0, "legacy"));
        session.revision.fetch_add(1, Ordering::AcqRel);
        assert!(!capture_pane_current(&session, &preview, 0, "legacy"));
    }
}
